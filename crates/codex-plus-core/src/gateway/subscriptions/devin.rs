//! Devin：devin CLI 用的 Connect RPC 接口（protobuf）。
//!
//! - 聊天：`GetChatMessage`，一帧 Connect envelope 发出，回来是一串 envelope（protobuf 消息 +
//!   结尾的 JSON 帧）。字段号见下面的编码函数。
//! - 登录：浏览器 PKCE（和 `devin auth login` 一样回调到本机 `/callback`）、导入
//!   `~/.local/share/devin/credentials.toml`、粘贴 `devin-session-token$…`。token 不按时间过期，
//!   没有刷新；导入本机登录的账号每次请求前重读文件，跟着 CLI 的重新登录走。
//! - 上游的几个脾气：系统提示和工具描述不能放在 system / 工具描述字段里（会回「internal error」），
//!   要放进第一条 user 消息；工具 schema 根部不能是 anyOf / oneOf / allOf；没回答的工具调用要补结果；
//!   最后一条不能是不带调用的 assistant；temperature / top_p 不能是精确的 0。

use std::collections::HashSet;

use anyhow::Context;
use base64::Engine;
use serde_json::{Value, json};

use crate::gateway::subscriptions::b_shared::{
    self, Decoder, content_text, decode_stream, gzip_stored, object_root, system_texts,
};
use crate::gateway::subscriptions::chat::{ChatSse, Usage};
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const SERVER: &str = "https://server.codeium.com";
const AUTHORIZE: &str = "https://app.devin.ai/auth/cli/continue";
const EXCHANGE: &str = "/exa.seat_management_pb.SeatManagementService/ExchangeDevinCLIPKCECode";
const USER_STATUS: &str = "/exa.seat_management_pb.SeatManagementService/GetUserStatus";
const CHAT_RPC: &str = "/exa.api_server_pb.ApiServerService/GetChatMessage";
const MODEL_CONFIGS_RPC: &str = "/exa.api_server_pb.ApiServerService/GetCliModelConfigs";
const CLI_VERSION: &str = "3000.11.3";
/// 请求体超过这么大时按官方 CLI 的做法 gzip。
const GZIP_AT: usize = 8 << 10;
const CALLBACK_PATH: &str = "/callback";
const INTERRUPTED: &str = "Tool use was interrupted and did not produce a result.";
const PROCEED: &str = "Please proceed with the task.";
const DEFAULT_MAX_TOKENS: u64 = 128_000;

/// 拉不到模型列表时的默认变体（上游只认变体 id）。
const DEFAULT_MODELS: &[&str] = &[
    "swe-2-high",
    "swe-2-medium",
    "claude-opus-5-5-high",
    "claude-sonnet-5-5-high",
    "claude-haiku-5-5-medium",
    "gpt-5-5-high",
    "gpt-5-3-codex-high",
    "gemini-3-8-flash-high",
    "kimi-k3-high",
    "glm-5-3-high",
    "deepseek-v4-pro-high",
];

const ROLE_USER: u64 = 1;
const ROLE_ASSISTANT: u64 = 2;
const ROLE_TOOL: u64 = 4;

pub struct Devin;

pub static DEVIN: Devin = Devin;

// ---------------------------------------------------------------------------
// protobuf
// ---------------------------------------------------------------------------

/// 极简 protobuf 编码器。
#[derive(Debug, Default, Clone)]
struct Pb(Vec<u8>);

impl Pb {
    fn raw_varint(&mut self, mut value: u64) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                self.0.push(byte);
                break;
            }
            self.0.push(byte | 0x80);
        }
    }

    fn key(&mut self, field: u32, wire: u8) {
        self.raw_varint(((field as u64) << 3) | wire as u64);
    }

    fn uint(&mut self, field: u32, value: u64) -> &mut Self {
        self.key(field, 0);
        self.raw_varint(value);
        self
    }

    fn bytes(&mut self, field: u32, data: &[u8]) -> &mut Self {
        self.key(field, 2);
        self.raw_varint(data.len() as u64);
        self.0.extend_from_slice(data);
        self
    }

    fn string(&mut self, field: u32, text: &str) -> &mut Self {
        self.bytes(field, text.as_bytes())
    }

    fn double(&mut self, field: u32, value: f64) -> &mut Self {
        self.key(field, 1);
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    fn message(&mut self, field: u32, message: &Pb) -> &mut Self {
        self.bytes(field, &message.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum PbValue<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed64(u64),
    Fixed32(u32),
}

fn read_varint(data: &[u8], position: &mut usize) -> anyhow::Result<u64> {
    let mut value = 0_u64;
    let mut shift = 0;
    loop {
        let byte = *data.get(*position).context("protobuf 数据被截断")?;
        *position += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
        shift += 7;
        if shift > 63 {
            anyhow::bail!("protobuf varint 过长");
        }
    }
}

/// 顶层字段列表。
fn pb_fields(data: &[u8]) -> anyhow::Result<Vec<(u32, PbValue<'_>)>> {
    let mut out = Vec::new();
    let mut position = 0;
    while position < data.len() {
        let key = read_varint(data, &mut position)?;
        let field = (key >> 3) as u32;
        let value = match key & 7 {
            0 => PbValue::Varint(read_varint(data, &mut position)?),
            1 => {
                let bytes = data
                    .get(position..position + 8)
                    .context("protobuf 数据被截断")?;
                position += 8;
                PbValue::Fixed64(u64::from_le_bytes(bytes.try_into()?))
            }
            2 => {
                let len = read_varint(data, &mut position)? as usize;
                let bytes = data
                    .get(position..position + len)
                    .context("protobuf 数据被截断")?;
                position += len;
                PbValue::Bytes(bytes)
            }
            5 => {
                let bytes = data
                    .get(position..position + 4)
                    .context("protobuf 数据被截断")?;
                position += 4;
                PbValue::Fixed32(u32::from_le_bytes(bytes.try_into()?))
            }
            wire => anyhow::bail!("不支持的 protobuf 类型 {wire}"),
        };
        out.push((field, value));
    }
    Ok(out)
}

fn pb_text(value: &PbValue<'_>) -> String {
    match value {
        PbValue::Bytes(bytes) => String::from_utf8_lossy(bytes).to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------------

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        _ => "linux",
    }
}

/// 请求元数据：`chisel` 两个字段必须带，否则模型列表只返回一个模型。
fn metadata(token: &str) -> Pb {
    let mut pb = Pb::default();
    pb.string(1, "devin-cli")
        .string(2, CLI_VERSION)
        .string(3, token)
        .string(4, "en")
        .string(5, os_name())
        .string(7, CLI_VERSION)
        .string(12, "chisel")
        .string(28, "chisel");
    pb
}

/// 上游的工具调用 id 形如 `Bash:0#a65b…`，客户端只认 `[A-Za-z0-9_-]`：编码成 `dv_` + base64url。
fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn out_id(id: &str) -> String {
    if safe_id(id) && !id.starts_with("dv_") {
        return id.to_string();
    }
    format!(
        "dv_{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.as_bytes())
    )
}

fn in_id(id: &str) -> String {
    if let Some(encoded) = id.strip_prefix("dv_").filter(|_| safe_id(id))
        && let Ok(raw) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)
        && let Ok(raw) = String::from_utf8(raw)
        && !raw.is_empty()
        && out_id(&raw) == id
    {
        return raw;
    }
    id.to_string()
}

/// 工具参数：对象原样，字符串要能解析成对象，否则 `{}`。
fn args_of(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::Object(_)) => arguments.map(Value::to_string).unwrap_or_default(),
        Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
            Ok(value @ Value::Object(_)) => value.to_string(),
            _ => "{}".to_string(),
        },
        _ => "{}".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DevinCall {
    id: String,
    name: String,
    args: String,
}

#[derive(Debug, Clone, PartialEq)]
struct DevinMessage {
    role: u64,
    text: String,
    calls: Vec<DevinCall>,
    call_id: String,
    /// (base64, mime)
    images: Vec<(String, String)>,
}

impl DevinMessage {
    fn new(role: u64, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            calls: Vec::new(),
            call_id: String::new(),
            images: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct DevinTool {
    name: String,
    description: String,
    schema: Value,
}

/// `data:` URL 里的图片（只认 data URL，上游不收远程地址）。
fn data_image(part: &Value) -> Option<(String, String)> {
    if part.get("type").and_then(Value::as_str) != Some("image_url") {
        return None;
    }
    let url = part
        .pointer("/image_url/url")
        .or_else(|| part.get("image_url"))
        .and_then(Value::as_str)?;
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.trim_end_matches(";base64");
    let mime = if mime.is_empty() { "image/png" } else { mime };
    Some((data.to_string(), mime.to_string()))
}

fn images_of(content: Option<&Value>) -> Vec<(String, String)> {
    content
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(data_image)
        .collect()
}

/// 一条 assistant 紧跟在带工具调用的 assistant 后面（并行调用被拆成了两条）：合并。
fn join_split_calls(messages: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for message in messages {
        let is_assistant = message.get("role").and_then(Value::as_str) == Some("assistant");
        if is_assistant
            && let Some(previous) = out.last_mut()
            && previous.get("role").and_then(Value::as_str) == Some("assistant")
            && previous
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty())
        {
            let text = [
                content_text(previous.get("content")),
                content_text(message.get("content")),
            ]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
            let mut calls = previous
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            calls.extend(
                message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            previous["content"] = json!(text);
            previous["tool_calls"] = json!(calls);
            continue;
        }
        out.push(message.clone());
    }
    out
}

/// Chat 请求 → Devin 的消息与工具。
fn convert(request: &Value) -> (Vec<DevinMessage>, Vec<DevinTool>) {
    let raw = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let messages = join_split_calls(&raw);

    // 系统提示 + response_format 的要求 + 工具描述，统一放进第一条 user 消息。
    let mut instructions = system_texts(request);
    match request.pointer("/response_format/type").and_then(Value::as_str) {
        Some("json_schema") => match request.pointer("/response_format/json_schema/schema") {
            Some(schema) => instructions.push(format!(
                "Respond with a single JSON value and nothing else, matching this JSON schema:\n{schema}."
            )),
            None => instructions.push("Respond with a single JSON object and nothing else.".into()),
        },
        Some("json_object") => {
            instructions.push("Respond with a single JSON object and nothing else.".into())
        }
        _ => {}
    }

    let tools_off = request.get("tool_choice").and_then(Value::as_str) == Some("none");
    let mut tools = Vec::new();
    let mut described = Vec::new();
    if !tools_off {
        for tool in request
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if tool
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "function")
            {
                continue;
            }
            let Some(name) = tool
                .pointer("/function/name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let description = tool
                .pointer("/function/description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let schema = object_root(
                tool.pointer("/function/parameters")
                    .unwrap_or(&json!({ "type": "object", "properties": {} })),
            );
            let field = if description.is_empty() {
                name.to_string()
            } else {
                described.push(format!("<tool name=\"{name}\">\n{description}\n</tool>"));
                format!(
                    "Described under <tool name=\"{name}\"> in <tool_descriptions>, in the instructions."
                )
            };
            tools.push(DevinTool {
                name: name.to_string(),
                description: field,
                schema,
            });
        }
    }
    if !described.is_empty() {
        instructions.push(format!(
            "<tool_descriptions>\n{}\n</tool_descriptions>",
            described.join("\n")
        ));
    }

    let mut out: Vec<DevinMessage> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let flush = |out: &mut Vec<DevinMessage>, pending: &mut Vec<String>| {
        for id in pending.drain(..) {
            let mut message = DevinMessage::new(ROLE_TOOL, INTERRUPTED);
            message.call_id = id;
            out.push(message);
        }
    };
    for message in &messages {
        let content = message.get("content");
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {}
            "assistant" => {
                let calls: Vec<DevinCall> = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|call| DevinCall {
                        id: in_id(call.get("id").and_then(Value::as_str).unwrap_or_default()),
                        name: call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        args: args_of(call.pointer("/function/arguments")),
                    })
                    .collect();
                let text = content_text(content);
                if text.trim().is_empty() && calls.is_empty() {
                    continue;
                }
                flush(&mut out, &mut pending);
                pending = calls.iter().map(|call| call.id.clone()).collect();
                let mut converted = DevinMessage::new(ROLE_ASSISTANT, text);
                converted.calls = calls;
                out.push(converted);
            }
            "tool" => {
                let id = in_id(
                    message
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                let Some(position) = pending.iter().position(|item| *item == id) else {
                    continue;
                };
                pending.remove(position);
                let text = content_text(content);
                let mut converted = DevinMessage::new(
                    ROLE_TOOL,
                    if text.trim().is_empty() {
                        "(no output)".to_string()
                    } else {
                        text
                    },
                );
                converted.call_id = id;
                converted.images = images_of(content);
                out.push(converted);
            }
            _ => {
                let text = content_text(content);
                let images = images_of(content);
                if text.trim().is_empty() && images.is_empty() {
                    continue;
                }
                flush(&mut out, &mut pending);
                let mut converted = DevinMessage::new(ROLE_USER, text);
                converted.images = images;
                out.push(converted);
            }
        }
    }
    flush(&mut out, &mut pending);
    if out
        .last()
        .is_none_or(|last| last.role == ROLE_ASSISTANT && last.calls.is_empty())
    {
        out.push(DevinMessage::new(ROLE_USER, PROCEED));
    }

    if !instructions.is_empty() {
        let prefix = instructions.join("\n\n");
        match out.iter_mut().find(|message| message.role == ROLE_USER) {
            Some(first) if first.text.is_empty() => first.text = prefix,
            Some(first) => first.text = format!("{prefix}\n\n{}", first.text),
            None => out.insert(0, DevinMessage::new(ROLE_USER, prefix)),
        }
    }

    // 历史里调用过、这次没声明的工具也要声明，否则上游拒绝。
    let declared: HashSet<String> = tools.iter().map(|tool| tool.name.clone()).collect();
    let mut extra = HashSet::new();
    for message in &out {
        for call in &message.calls {
            if !call.name.is_empty()
                && !declared.contains(&call.name)
                && extra.insert(call.name.clone())
            {
                tools.push(DevinTool {
                    name: call.name.clone(),
                    description: "Tool".into(),
                    schema: json!({ "type": "object", "properties": {} }),
                });
            }
        }
    }
    (out, tools)
}

/// 采样参数不能是精确的 0（上游回「internal error」）。
fn sampling(value: Option<f64>, fallback: f64) -> f64 {
    match value {
        Some(value) if value == 0.0 => 1e-6,
        Some(value) => value,
        None => fallback,
    }
}

/// `GetChatMessageRequest` 的 protobuf。
fn encode_request(request: &Value, token: &str) -> Vec<u8> {
    let (messages, tools) = convert(request);
    let mut pb = Pb::default();
    pb.message(1, &metadata(token));
    for message in &messages {
        let mut item = Pb::default();
        item.string(1, &uuid::Uuid::new_v4().to_string())
            .uint(2, message.role);
        if !message.text.is_empty() {
            item.string(3, &message.text);
        }
        for call in &message.calls {
            let mut converted = Pb::default();
            converted
                .string(1, &call.id)
                .string(2, &call.name)
                .string(3, &call.args);
            item.message(6, &converted);
        }
        if !message.call_id.is_empty() {
            item.string(7, &message.call_id);
        }
        for (data, mime) in &message.images {
            let mut image = Pb::default();
            image.string(1, data).string(2, mime);
            item.message(10, &image);
        }
        pb.message(3, &item);
    }
    pb.uint(7, 5);
    let mut config = Pb::default();
    config
        .uint(1, 1)
        .uint(
            2,
            b_shared::max_tokens(request).unwrap_or(DEFAULT_MAX_TOKENS),
        )
        .uint(3, 400)
        .double(
            5,
            sampling(request.get("temperature").and_then(Value::as_f64), 1.0),
        )
        .uint(7, 40)
        .double(
            8,
            sampling(request.get("top_p").and_then(Value::as_f64), 0.95),
        );
    pb.message(8, &config);
    for tool in &tools {
        let mut converted = Pb::default();
        converted
            .string(1, &tool.name)
            .string(2, &tool.description)
            .string(3, &tool.schema.to_string());
        pb.message(10, &converted);
    }
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    pb.string(21, model);
    pb.0
}

/// 一帧 Connect envelope；大请求 gzip。返回 (帧, 是否 gzip)。
fn envelope(payload: &[u8]) -> (Vec<u8>, bool) {
    let (flag, body) = if payload.len() > GZIP_AT {
        (1_u8, gzip_stored(payload))
    } else {
        (0_u8, payload.to_vec())
    };
    let mut frame = Vec::with_capacity(body.len() + 5);
    frame.push(flag);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    (frame, flag == 1)
}

// ---------------------------------------------------------------------------
// 答复
// ---------------------------------------------------------------------------

/// Connect 错误（`{"code","message"}` 或 `{"error":{…}}`）→ 带状态码的失败。
fn failure(status: u16, body: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let error = value
        .get("error")
        .filter(|error| error.is_object())
        .unwrap_or(&value);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| body.chars().take(300).collect());
    let lower = message.to_ascii_lowercase();
    let status = if code == "unauthenticated" || status == 401 {
        return fail(401, format!("Devin 登录已失效，请重新登录：{message}"));
    } else if code == "resource_exhausted"
        || lower.contains("quota")
        || lower.contains("rate limit")
    {
        return fail(429, format!("Devin 额度已用完或被限流：{message}"));
    } else if lower.contains("content policy")
        || lower.contains("too long")
        || lower.contains("context length")
        || lower.contains("too many tokens")
        || code == "invalid_argument"
    {
        400
    } else if code == "permission_denied" {
        403
    } else if code == "unavailable" {
        503
    } else if !code.is_empty() {
        502
    } else if status >= 400 {
        status
    } else {
        502
    };
    fail(status, format!("Devin 返回 {status}：{message}"))
}

/// Connect 流 → Chat SSE。
struct DevinDecoder {
    buffer: Vec<u8>,
    sse: ChatSse,
    current: Option<String>,
    stop: u64,
    input: u64,
    output: u64,
    cache_write: u64,
    cache_read: u64,
    said: usize,
    done: bool,
}

impl DevinDecoder {
    fn new(model: &str) -> Self {
        Self {
            buffer: Vec::new(),
            sse: ChatSse::new(model),
            current: None,
            stop: 0,
            input: 0,
            output: 0,
            cache_write: 0,
            cache_read: 0,
            said: 0,
            done: false,
        }
    }

    fn message(&mut self, data: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        for (field, value) in pb_fields(data)? {
            match (field, value) {
                (3, PbValue::Bytes(bytes)) if !bytes.is_empty() => {
                    self.said += bytes.len();
                    out.extend(self.sse.text(&String::from_utf8_lossy(bytes)));
                }
                (9, PbValue::Bytes(bytes)) if !bytes.is_empty() => {
                    self.said += bytes.len();
                    out.extend(self.sse.reasoning(&String::from_utf8_lossy(bytes)));
                }
                (6, PbValue::Bytes(bytes)) => {
                    let mut id = String::new();
                    let mut name = String::new();
                    let mut args = String::new();
                    for (inner, value) in pb_fields(bytes)? {
                        match inner {
                            1 => id = pb_text(&value),
                            2 => name = pb_text(&value),
                            3 => args.push_str(&pb_text(&value)),
                            _ => {}
                        }
                    }
                    if !name.is_empty() {
                        let id = if id.is_empty() {
                            format!("call_{}", b_shared::random_hex(12))
                        } else {
                            out_id(&id)
                        };
                        out.extend(self.sse.tool_call(&id, &name, ""));
                        self.current = Some(id);
                    }
                    if !args.is_empty()
                        && let Some(id) = self.current.clone()
                    {
                        out.extend(self.sse.tool_arguments(&id, &args));
                    }
                }
                (5, PbValue::Varint(value)) => self.stop = value,
                (7, PbValue::Bytes(bytes)) => {
                    for (inner, value) in pb_fields(bytes)? {
                        let PbValue::Varint(number) = value else {
                            continue;
                        };
                        if number == 0 {
                            continue;
                        }
                        match inner {
                            2 => self.input = number,
                            3 => self.output = number,
                            4 => self.cache_write = number,
                            5 => self.cache_read = number,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> anyhow::Result<Vec<u8>> {
        if self.done {
            return Ok(Vec::new());
        }
        self.done = true;
        if self.said == 0 && !self.sse.has_tool_calls() {
            return Err(fail(502, "Devin 返回了空回复"));
        }
        let output = if self.output == 0 {
            (self.said as u64).div_ceil(4)
        } else {
            self.output
        };
        let usage = Usage {
            prompt_tokens: self.input + self.cache_read + self.cache_write,
            completion_tokens: output,
            cached_tokens: self.cache_read,
            reasoning_tokens: 0,
        };
        let reason = match self.stop {
            _ if self.sse.has_tool_calls() => None,
            3 => Some("length"),
            _ => None,
        };
        Ok(self.sse.finish(reason, Some(usage)))
    }
}

impl Decoder for DevinDecoder {
    fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while self.buffer.len() >= 5 && !self.done {
            let flags = self.buffer[0];
            let len = u32::from_be_bytes(self.buffer[1..5].try_into()?) as usize;
            if len > 64 << 20 {
                return Err(fail(502, "Devin 返回了格式错误的数据流"));
            }
            if self.buffer.len() < 5 + len {
                break;
            }
            let raw: Vec<u8> = self.buffer.drain(..5 + len).skip(5).collect();
            // Flag bit 0 is the Connect compression bit. Requests already use gzip.
            let data = if flags & 1 != 0 {
                gunzip(&raw).map_err(|error| {
                    fail(502, format!("Devin 返回了无法解压的压缩数据帧：{error}"))
                })?
            } else {
                raw
            };
            if flags & 2 != 0 {
                let text = String::from_utf8_lossy(&data);
                let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if value.get("error").is_some_and(|error| !error.is_null()) {
                    self.done = true;
                    return Err(failure(200, &text));
                }
                out.extend(self.finish()?);
                break;
            }
            out.extend(self.message(&data)?);
        }
        Ok(out)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        if !self.done && !self.buffer.is_empty() {
            self.done = true;
            // `push` already consumed every complete frame. A leftover that
            // starts with the compression bit is a partial gzip frame.
            if self.buffer[0] & 1 != 0 {
                return Err(fail(502, "Devin 返回了被截断的压缩数据帧"));
            }
            return Err(fail(502, "Devin 的数据流意外中断"));
        }
        self.finish()
    }
}

/// Inflate one gzip frame. Truncation, a bad CRC, or trailing bytes are errors.
fn gunzip(compressed: &[u8]) -> anyhow::Result<Vec<u8>> {
    use std::io::{Cursor, Read};

    let mut input = Cursor::new(compressed);
    let mut decoder = flate2::bufread::GzDecoder::new(&mut input);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .context("gzip 数据不完整或校验失败")?;
    if input.position() != compressed.len() as u64 {
        anyhow::bail!("gzip 帧尾部有多余字节");
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 凭据与登录
// ---------------------------------------------------------------------------

/// `credentials.toml` 的位置。
fn credentials_path() -> std::path::PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| b_shared::home().join("AppData").join("Roaming"));
        return base.join("devin").join("credentials.toml");
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| b_shared::home().join(".local").join("share"));
    base.join("devin").join("credentials.toml")
}

/// 解析 `credentials.toml`：返回 (token, 服务器地址)。
fn parse_credentials_toml(text: &str) -> anyhow::Result<(String, Option<String>)> {
    let value: toml::Value = toml::from_str(text).context("credentials.toml 格式不对")?;
    let token = value
        .get("windsurf_api_key")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .context("credentials.toml 里没有登录信息，请先运行 devin auth login")?;
    let server = value
        .get("api_server_url")
        .and_then(toml::Value::as_str)
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| url.starts_with("https://"));
    Ok((token.to_string(), server))
}

fn read_local() -> anyhow::Result<(String, Option<String>)> {
    let path = credentials_path();
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("没找到 {}，请先在终端运行 devin auth login", path.display()))?;
    parse_credentials_toml(&text)
}

/// token 里 `$` 后面的 JWT 的过期时间（毫秒）；没有时为 None。
fn token_expires_ms(token: &str) -> Option<u64> {
    oauth::jwt_expires_ms(token.rsplit('$').next().unwrap_or(token))
}

fn credentials(token: &str, server: &str, source: &str) -> Value {
    json!({
        "token": token,
        "server": server,
        "source": source,
        "expires_ms": token_expires_ms(token),
    })
}

fn server_of(credentials: &Value) -> String {
    credentials
        .get("server")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://"))
        .unwrap_or(SERVER)
        .trim_end_matches('/')
        .to_string()
}

fn token_of(credentials: &Value) -> anyhow::Result<String> {
    credentials
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Devin 凭据缺失，请重新登录"))
}

/// 用 GetUserStatus 校验 token，返回账号名（邮箱 / 名字）。
async fn account_name(http: &reqwest::Client, server: &str, token: &str) -> anyhow::Result<String> {
    let body = json!({ "metadata": {
        "ideName": "devin-cli",
        "ideVersion": CLI_VERSION,
        "extensionName": "devin-cli",
        "extensionVersion": CLI_VERSION,
        "apiKey": token,
        "locale": "en",
        "os": os_name(),
    }});
    let response = http
        .post(format!("{server}{USER_STATUS}"))
        .header("connect-protocol-version", "1")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .context("无法连接 Devin")?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(failure(status, &text));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let user = value.get("userStatus").cloned().unwrap_or(Value::Null);
    Ok(b_shared::str_at(
        &user,
        &[
            "email",
            "name",
            "/planStatus/planInfo/devinInfo/accountDisplayName",
        ],
    )
    .unwrap_or("Devin")
    .to_string())
}

#[async_trait::async_trait]
impl Subscription for Devin {
    fn id(&self) -> &'static str {
        "devin"
    }

    fn name(&self) -> &'static str {
        "Devin"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要 Devin 账号（免费额度只能用 SWE 系列模型，其它模型要 Pro / Teams 订阅）；可在浏览器登录，或先在终端运行 devin auth login 再导入。"
    }

    fn detect_local(&self) -> Option<String> {
        read_local().ok().map(|_| String::new())
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => {
                let listener = oauth::bind_callback(0).await?;
                let port = listener.local_addr()?.port();
                let redirect = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
                let (verifier, challenge) = oauth::pkce_pair();
                let state = oauth::random_state();
                let url = format!(
                    "{AUTHORIZE}?{}",
                    oauth::query(&[
                        ("redirect_uri", &redirect),
                        ("state", &state),
                        ("prompt", "select_account"),
                        ("code_challenge", &challenge),
                        ("code_challenge_method", "S256"),
                        ("cli_pkce_marker", "1"),
                    ])
                );
                let key = b_shared::park_listener(listener);
                Ok(LoginStart {
                    open_url: Some(url),
                    user_code: None,
                    message: "在浏览器里登录 Devin，完成后会自动回到 Codex++。".into(),
                    state: json!({
                        "method": "browser",
                        "listener": key,
                        "verifier": verifier,
                        "state": state,
                        "redirect": redirect,
                    }),
                })
            }
            LoginMethod::ImportLocal => {
                let (token, server) = read_local()?;
                Ok(LoginStart {
                    message: "已读取本机 devin CLI 的登录。".into(),
                    state: json!({
                        "method": "local",
                        "token": token,
                        "server": server.unwrap_or_else(|| SERVER.to_string()),
                    }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let token = input.trim().trim_start_matches("Basic ").trim();
                if token.is_empty() {
                    anyhow::bail!("请粘贴 Devin 的会话 token（devin-session-token$…）");
                }
                Ok(LoginStart {
                    message: "正在校验 token。".into(),
                    state: json!({ "method": "paste", "token": token, "server": SERVER }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::DeviceCode => anyhow::bail!("Devin 不支持设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let method = state
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (token, server, source) = if method == "browser" {
            let key = state
                .get("listener")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let listener =
                b_shared::take_listener(key).context("这次登录已经失效，请重新开始登录")?;
            let expected = state
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let params = oauth::wait_callback(&listener, CALLBACK_PATH, expected).await?;
            let code = params
                .get("code")
                .filter(|code| !code.is_empty())
                .context("Devin 回调里没有授权码")?;
            let answer = oauth::post_json(
                http,
                &format!("{SERVER}{EXCHANGE}"),
                &json!({
                    "code": code,
                    "code_verifier": state.get("verifier").and_then(Value::as_str).unwrap_or_default(),
                    "redirect_uri": state.get("redirect").and_then(Value::as_str).unwrap_or_default(),
                }),
                &[("connect-protocol-version", "1")],
            )
            .await?;
            let token = b_shared::str_at(&answer, &["sessionToken", "session_token"])
                .context("Devin 没有返回会话 token")?
                .to_string();
            (token, SERVER.to_string(), "browser")
        } else {
            let token = state
                .get("token")
                .and_then(Value::as_str)
                .context("缺少 token")?
                .to_string();
            let server = state
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or(SERVER)
                .to_string();
            (
                token,
                server,
                if method == "local" { "local" } else { "paste" },
            )
        };
        let account = account_name(http, &server, &token).await?;
        Ok(LoginDone {
            credentials: credentials(&token, &server, source),
            account,
        })
    }

    async fn refresh(
        &self,
        _http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        // Devin 的 token 不按时间过期，也没有刷新接口；导入本机登录的账号跟着 CLI 的登录走。
        if credentials.get("source").and_then(Value::as_str) != Some("local") {
            return Ok(None);
        }
        let Ok((token, server)) = read_local() else {
            return Ok(None);
        };
        if credentials.get("token").and_then(Value::as_str) == Some(token.as_str()) {
            return Ok(None);
        }
        let server = server.unwrap_or_else(|| server_of(credentials));
        Ok(Some(self::credentials(&token, &server, "local")))
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let token = token_of(credentials)?;
        let mut body = Pb::default();
        body.message(1, &metadata(&token));
        let response = http
            .post(format!("{}{MODEL_CONFIGS_RPC}", server_of(credentials)))
            .header("content-type", "application/proto")
            .header("connect-protocol-version", "1")
            .body(body.0)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await;
        let models = match response {
            Ok(response) if response.status().is_success() => response
                .bytes()
                .await
                .ok()
                .and_then(|bytes| parse_model_configs(&bytes).ok())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if models.is_empty() {
            return Ok(DEFAULT_MODELS
                .iter()
                .map(|model| model.to_string())
                .collect());
        }
        Ok(models)
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let token = token_of(credentials)?;
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (frame, gzipped) = envelope(&encode_request(&request, &token));
        let mut builder = http
            .post(format!("{}{CHAT_RPC}", server_of(credentials)))
            .header("content-type", "application/connect+proto")
            .header("connect-protocol-version", "1")
            .header("authorization", format!("Basic {token}"));
        if gzipped {
            builder = builder.header("connect-content-encoding", "gzip");
        }
        let response = builder
            .body(frame)
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 Devin：{error}")))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(status, &text));
        }
        Ok(decode_stream(response, DevinDecoder::new(&model)))
    }
}

/// `GetCliModelConfigs` 的答复 → 变体 id（跳过只在 Devin 自己 agent 里用的 adaptive / fusion）。
fn parse_model_configs(data: &[u8]) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for (field, value) in pb_fields(data)? {
        let (1, PbValue::Bytes(bytes)) = (field, value) else {
            continue;
        };
        let mut uid = String::new();
        let mut label = String::new();
        for (inner, value) in pb_fields(bytes)? {
            match inner {
                22 => uid = pb_text(&value),
                1 => label = pb_text(&value),
                _ => {}
            }
        }
        let lower = format!("{uid} {label}").to_ascii_lowercase();
        if uid.is_empty() || lower.contains("adaptive") || lower.contains("fusion") {
            continue;
        }
        if !out.contains(&uid) {
            out.push(uid);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::b_shared::run_decoder;
    use crate::gateway::subscriptions::chat::aggregate;

    fn field<'a>(fields: &'a [(u32, PbValue<'a>)], number: u32) -> Vec<&'a PbValue<'a>> {
        fields
            .iter()
            .filter(|(field, _)| *field == number)
            .map(|(_, value)| value)
            .collect()
    }

    fn sample_request() -> Value {
        json!({
            "model": "claude-opus-5-5-high",
            "temperature": 0,
            "messages": [
                { "role": "system", "content": "你是助手" },
                { "role": "user", "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "data:image/jpeg;base64,QUJD" } }
                ]},
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "dv_QmFzaDowI2E2NWI", "type": "function", "function": { "name": "Bash", "arguments": "{\"cmd\":\"ls\"}" } }
                ]},
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_b", "type": "function", "function": { "name": "Read", "arguments": "oops" } }
                ]},
                { "role": "tool", "tool_call_id": "dv_QmFzaDowI2E2NWI", "content": "a.txt" },
                { "role": "tool", "tool_call_id": "unknown", "content": "丢弃" },
                { "role": "assistant", "content": "好的" }
            ],
            "tools": [
                { "type": "function", "function": { "name": "Bash", "description": "Run a command", "parameters": {
                    "oneOf": [{ "type": "object", "properties": { "cmd": { "type": "string" } }, "required": ["cmd"] }]
                }}}
            ]
        })
    }

    #[test]
    fn tool_ids_round_trip() {
        let raw = "Bash:0#a65b";
        let shown = out_id(raw);
        assert!(shown.starts_with("dv_"));
        assert!(safe_id(&shown));
        assert_eq!(in_id(&shown), raw);
        assert_eq!(out_id("call_1"), "call_1");
        assert_eq!(in_id("call_1"), "call_1");
    }

    #[test]
    fn converts_messages_tools_and_pending_calls() {
        let (messages, tools) = convert(&sample_request());
        // 第一条 user 前面带系统提示和工具描述。
        assert_eq!(messages[0].role, ROLE_USER);
        assert!(
            messages[0]
                .text
                .starts_with("你是助手\n\n<tool_descriptions>")
        );
        assert!(
            messages[0]
                .text
                .contains("<tool name=\"Bash\">\nRun a command\n</tool>")
        );
        assert!(messages[0].text.ends_with("看图"));
        assert_eq!(
            messages[0].images,
            vec![("QUJD".into(), "image/jpeg".into())]
        );
        // 两条拆开的 assistant 合并成一条，id 还原成上游原样。
        assert_eq!(messages[1].role, ROLE_ASSISTANT);
        assert_eq!(messages[1].calls.len(), 2);
        assert_eq!(messages[1].calls[0].id, "Bash:0#a65b");
        assert_eq!(messages[1].calls[1].args, "{}");
        // 工具结果配上调用；没回答的补「被打断」；对不上的丢弃。
        assert_eq!(messages[2].role, ROLE_TOOL);
        assert_eq!(messages[2].call_id, "Bash:0#a65b");
        assert_eq!(messages[2].text, "a.txt");
        assert_eq!(messages[3].call_id, "call_b");
        assert_eq!(messages[3].text, INTERRUPTED);
        assert_eq!(messages[4].text, "好的");
        // 最后一条是不带调用的 assistant：补一条 user。
        assert_eq!(messages[5].role, ROLE_USER);
        assert_eq!(messages[5].text, PROCEED);
        // 工具：描述挪走、schema 根部展开；历史里用过的 Read 也要声明。
        assert_eq!(tools[0].name, "Bash");
        assert!(tools[0].description.starts_with("Described under"));
        assert_eq!(tools[0].schema["type"], "object");
        assert_eq!(tools[0].schema["required"], json!(["cmd"]));
        assert_eq!(tools[1].name, "Read");
    }

    #[test]
    fn encodes_request_fields() {
        let bytes = encode_request(&sample_request(), "devin-session-token$abc");
        let fields = pb_fields(&bytes).unwrap();
        let PbValue::Bytes(meta) = field(&fields, 1)[0] else {
            panic!("缺 metadata");
        };
        let meta = pb_fields(meta).unwrap();
        assert_eq!(pb_text(field(&meta, 3)[0]), "devin-session-token$abc");
        assert_eq!(pb_text(field(&meta, 12)[0]), "chisel");
        assert_eq!(field(&fields, 3).len(), 6);
        assert_eq!(*field(&fields, 7)[0], PbValue::Varint(5));
        let PbValue::Bytes(config) = field(&fields, 8)[0] else {
            panic!("缺 config");
        };
        let config = pb_fields(config).unwrap();
        assert_eq!(*field(&config, 2)[0], PbValue::Varint(DEFAULT_MAX_TOKENS));
        // temperature 0 → 1e-6。
        assert_eq!(*field(&config, 5)[0], PbValue::Fixed64(1e-6_f64.to_bits()));
        assert_eq!(field(&fields, 10).len(), 2);
        assert_eq!(pb_text(field(&fields, 21)[0]), "claude-opus-5-5-high");
        // 工具结果消息带 call id。
        let PbValue::Bytes(tool) = field(&fields, 3)[2] else {
            panic!();
        };
        let tool = pb_fields(tool).unwrap();
        assert_eq!(*field(&tool, 2)[0], PbValue::Varint(ROLE_TOOL));
        assert_eq!(pb_text(field(&tool, 7)[0]), "Bash:0#a65b");
    }

    #[test]
    fn envelope_gzips_large_bodies() {
        let (small, gz) = envelope(b"abc");
        assert!(!gz);
        assert_eq!(small, vec![0, 0, 0, 0, 3, b'a', b'b', b'c']);
        let (large, gz) = envelope(&vec![b'x'; GZIP_AT + 1]);
        assert!(gz);
        assert_eq!(large[0], 1);
        assert_eq!(&large[5..7], &[0x1f, 0x8b]);
    }

    fn frame(flags: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![flags];
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    #[test]
    fn decodes_stream_into_chat_sse() {
        let mut first = Pb::default();
        first.string(9, "思考").string(3, "你好");
        let mut call = Pb::default();
        call.string(1, "Bash:0#x")
            .string(2, "Bash")
            .string(3, "{\"cmd\":");
        let mut second = Pb::default();
        second.message(6, &call);
        let mut more = Pb::default();
        more.string(3, "\"ls\"}");
        let mut third = Pb::default();
        third.message(6, &more).uint(5, 10);
        let mut usage = Pb::default();
        usage.uint(2, 100).uint(3, 20).uint(5, 30);
        let mut fourth = Pb::default();
        fourth.message(7, &usage);
        let mut bytes = Vec::new();
        for message in [&first, &second, &third, &fourth] {
            bytes.extend(frame(0, &message.0));
        }
        bytes.extend(frame(2, b"{}"));
        // 故意切成小块喂进去。
        let parts: Vec<&[u8]> = bytes.chunks(7).collect();
        let (text, error) = run_decoder(DevinDecoder::new("m"), &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "你好");
        assert_eq!(message["reasoning_content"], "思考");
        assert_eq!(message["tool_calls"][0]["id"], out_id("Bash:0#x"));
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"cmd\":\"ls\"}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 130);
        assert_eq!(completion["usage"]["completion_tokens"], 20);
    }

    #[test]
    fn decoder_reports_errors_and_empty_replies() {
        let end = frame(
            2,
            br#"{"error":{"code":"resource_exhausted","message":"quota exceeded"}}"#,
        );
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&end]);
        let error = error.unwrap();
        assert_eq!(
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status,
            429
        );
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&frame(2, b"{}")]);
        assert!(error.unwrap().to_string().contains("空回复"));
        let status = |error: anyhow::Error| {
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status
        };
        assert_eq!(status(failure(401, "")), 401);
        assert_eq!(
            status(failure(200, r#"{"code":"unauthenticated","message":"x"}"#)),
            401
        );
        assert_eq!(
            status(failure(400, r#"{"code":"invalid_argument","message":"x"}"#)),
            400
        );
        assert_eq!(
            status(failure(500, r#"{"code":"unavailable","message":"x"}"#)),
            503
        );
    }

    fn gzip_bytes(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn status_of(error: anyhow::Error) -> (u16, String) {
        let message = error.to_string();
        let status = error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap()
            .status;
        (status, message)
    }

    #[test]
    fn decodes_gzip_compressed_frames() {
        let mut message = Pb::default();
        message.string(3, "压缩后的回复");
        let mut bytes = frame(1, &gzip_bytes(&message.0));
        bytes.extend(frame(2, b"{}"));
        // Split the frame so a partial chunk waits instead of failing as bad gzip.
        let parts: Vec<&[u8]> = bytes.chunks(5).collect();
        let (text, error) = run_decoder(DevinDecoder::new("m"), &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "m").unwrap();
        assert_eq!(
            completion["choices"][0]["message"]["content"],
            "压缩后的回复"
        );

        // Request bodies use stored deflate blocks; accept that gzip form too.
        let mut stored = frame(1, &gzip_stored(&message.0));
        stored.extend(frame(2, b"{}"));
        let (text, error) = run_decoder(DevinDecoder::new("m"), &[&stored]);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "m").unwrap();
        assert_eq!(
            completion["choices"][0]["message"]["content"],
            "压缩后的回复"
        );

        // End-of-stream frames can carry the compression bit as well.
        let end = frame(
            3,
            &gzip_bytes(br#"{"error":{"code":"unavailable","message":"down"}}"#),
        );
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&end]);
        let (status, message) = status_of(error.expect("compressed end frame"));
        assert_eq!(status, 503, "{message}");
    }

    #[test]
    fn rejects_invalid_or_truncated_compressed_frames() {
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&frame(1, b"not-gzip")]);
        let (status, message) = status_of(error.expect("invalid gzip"));
        assert_eq!(status, 502, "{message}");
        assert!(message.contains("无法解压"), "{message}");

        let mut truncated_member = gzip_bytes(b"hello");
        truncated_member.truncate(truncated_member.len().saturating_sub(8));
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&frame(1, &truncated_member)]);
        let (status, message) = status_of(error.expect("truncated gzip member"));
        assert_eq!(status, 502, "{message}");
        assert!(message.contains("无法解压"), "{message}");

        let mut trailing = gzip_bytes(b"hello");
        trailing.push(0xff);
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&frame(1, &trailing)]);
        let (status, message) = status_of(error.expect("trailing bytes"));
        assert_eq!(status, 502, "{message}");
        assert!(message.contains("多余字节"), "{message}");

        let mut partial = frame(1, &gzip_bytes(b"hello"));
        partial.truncate(8);
        let (_, error) = run_decoder(DevinDecoder::new("m"), &[&partial]);
        let (status, message) = status_of(error.expect("truncated frame"));
        assert_eq!(status, 502, "{message}");
        assert!(message.contains("被截断"), "{message}");
    }

    #[test]
    fn parses_local_credentials() {
        let text = r#"
windsurf_api_key = "devin-session-token$aaa.bbb.ccc"
api_server_url = "https://server.example.com/"
devin_webapp_host = "app.devin.ai"
"#;
        let (token, server) = parse_credentials_toml(text).unwrap();
        assert_eq!(token, "devin-session-token$aaa.bbb.ccc");
        assert_eq!(server.as_deref(), Some("https://server.example.com"));
        assert!(parse_credentials_toml("devin_webapp_host = \"x\"").is_err());
        let creds = credentials("devin-session-token$x", SERVER, "local");
        assert_eq!(creds["source"], "local");
        assert_eq!(server_of(&creds), SERVER);
        assert!(creds["expires_ms"].is_null());
    }

    #[test]
    fn parses_model_configs() {
        let mut first = Pb::default();
        first
            .string(1, "Claude Opus 5.5 High")
            .string(22, "claude-opus-5-5-high");
        let mut skipped = Pb::default();
        skipped.string(1, "Adaptive").string(22, "adaptive");
        let mut response = Pb::default();
        response.message(1, &first).message(1, &skipped);
        assert_eq!(
            parse_model_configs(&response.0).unwrap(),
            vec!["claude-opus-5-5-high".to_string()]
        );
    }
}
