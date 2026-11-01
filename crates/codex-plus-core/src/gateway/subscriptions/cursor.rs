//! Cursor 订阅：按 cursor-agent（Cursor 的命令行 agent）的方式调用 Cursor 的 agent 接口。
//!
//! 接口是 Connect + protobuf。这里没有 HTTP/2，走 Cursor 客户端在 HTTP/1.1 下的做法：
//!
//! ```text
//! RunSSE（server stream，下行）：AgentServerMessage 一帧一帧地来
//! BidiAppend（每条上行消息一次 unary）：AgentClientMessage 的十六进制 + 请求 id + 序号
//! ```
//!
//! 每次 Chat 请求开一个新的 Run：整段对话按 AI SDK 消息 JSON 编成 blob（以 SHA-256 为 id），
//! 服务端按需来要（KV get_blob）。调用方的工具作为 MCP 工具（命名空间 `tools`）交给模型，
//! 系统提示末尾附上工具目录；模型发起 MCP 调用就转成 Chat 的 tool_calls，本步的调用都到齐后
//! 关掉 Run，由调用方执行，下一轮请求带着工具结果重开。Cursor 自己的工具（读文件、跑命令等）
//! 一律拒绝，从不执行。protobuf 没有可用的库，这里手写最小的 varint / length-delimited 编解码。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::bail;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::gateway::subscriptions::chat::{ChatSse, Usage};
use crate::gateway::subscriptions::editor_kit::{
    self, Emitter, arguments_object, content_text, reasoning_text, text_of,
};
use crate::gateway::subscriptions::oauth::{expiring, jwt_claims, jwt_expires_ms, now_ms, query};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const API: &str = "https://api2.cursor.sh";
const WEBSITE: &str = "https://cursor.com";
/// 找不到本机 cursor-agent 时自报的版本。
const VERSION_FALLBACK: &str = "2026.09.23-86fc751";
/// Cursor 桌面客户端刷新登录用的 OAuth 客户端。
const OAUTH_CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";
/// 调用方工具所在的 MCP 命名空间。
const NS: &str = "tools";
const CALL: &str = "CallDynamicTool";
const NO_RESULT: &str = "Tool use was interrupted and did not produce a result.";
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;
/// 多久没有任何帧就算断了。
const QUIET: Duration = Duration::from_secs(60);
/// 同时在途的 BidiAppend 上限（只有 KV 回复可以并发）。
const APPEND_AT_ONCE: usize = 16;
const MAX_FRAME: usize = 64 * 1024 * 1024;

pub struct Cursor;

pub static CURSOR: Cursor = Cursor;

/* —————————————— protobuf —————————————— */

/// 最小的 protobuf 编码器（按字段号依次写）。
#[derive(Debug, Default, Clone)]
pub(crate) struct Pb(Vec<u8>);

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

impl Pb {
    pub fn new() -> Self {
        Self::default()
    }

    fn tag(&mut self, num: u64, wire: u64) {
        put_varint(&mut self.0, (num << 3) | wire);
    }

    pub fn varint(mut self, num: u64, value: u64) -> Self {
        self.tag(num, 0);
        put_varint(&mut self.0, value);
        self
    }

    pub fn bytes(mut self, num: u64, data: &[u8]) -> Self {
        self.tag(num, 2);
        put_varint(&mut self.0, data.len() as u64);
        self.0.extend_from_slice(data);
        self
    }

    pub fn str(self, num: u64, text: &str) -> Self {
        self.bytes(num, text.as_bytes())
    }

    pub fn msg(self, num: u64, message: Pb) -> Self {
        self.bytes(num, &message.0)
    }

    pub fn double(mut self, num: u64, value: f64) -> Self {
        self.tag(num, 1);
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }

    pub fn done(self) -> Vec<u8> {
        self.0
    }
}

/// 读出来的一个字段。varint / 定长的值在 `n`，length-delimited 的内容在 `data`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Field<'a> {
    pub num: u64,
    pub wire: u8,
    pub n: u64,
    pub data: &'a [u8],
}

fn get_varint(bytes: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    let mut shift = 0;
    while *at < bytes.len() && shift < 64 {
        let byte = bytes[*at];
        *at += 1;
        value |= u64::from(byte & 0x7F) << shift;
        if byte < 0x80 {
            return Some(value);
        }
        shift += 7;
    }
    None
}

/// 读一条消息的全部字段；尾部坏了就丢掉坏的部分。
pub(crate) fn fields(bytes: &[u8]) -> Vec<Field<'_>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(key) = get_varint(bytes, &mut at) else {
            break;
        };
        let mut field = Field {
            num: key >> 3,
            wire: (key & 7) as u8,
            n: 0,
            data: &[],
        };
        match field.wire {
            0 => match get_varint(bytes, &mut at) {
                Some(value) => field.n = value,
                None => break,
            },
            1 => {
                if at + 8 > bytes.len() {
                    break;
                }
                field.data = &bytes[at..at + 8];
                field.n = u64::from_le_bytes(field.data.try_into().unwrap_or([0; 8]));
                at += 8;
            }
            2 => {
                let Some(length) = get_varint(bytes, &mut at) else {
                    break;
                };
                let length = length as usize;
                if at + length > bytes.len() {
                    break;
                }
                field.data = &bytes[at..at + length];
                at += length;
            }
            5 => {
                if at + 4 > bytes.len() {
                    break;
                }
                field.data = &bytes[at..at + 4];
                field.n = u64::from(u32::from_le_bytes(field.data.try_into().unwrap_or([0; 4])));
                at += 4;
            }
            _ => break,
        }
        out.push(field);
    }
    out
}

fn field<'a>(fields: &[Field<'a>], num: u64) -> Option<Field<'a>> {
    fields.iter().find(|field| field.num == num).copied()
}

fn str_of(fields: &[Field<'_>], num: u64) -> String {
    field(fields, num)
        .map(|field| String::from_utf8_lossy(field.data).to_string())
        .unwrap_or_default()
}

fn num_of(fields: &[Field<'_>], num: u64) -> u64 {
    field(fields, num).map(|field| field.n).unwrap_or(0)
}

/// JSON → google.protobuf.Value（对象的键按字典序）。
pub(crate) fn pb_value(value: &Value) -> Pb {
    match value {
        Value::Null => Pb::new().varint(1, 0),
        Value::Number(number) => Pb::new().double(2, number.as_f64().unwrap_or(0.0)),
        Value::String(text) => Pb::new().str(3, text),
        Value::Bool(flag) => Pb::new().varint(4, u64::from(*flag)),
        Value::Array(items) => {
            let list = items
                .iter()
                .fold(Pb::new(), |list, item| list.msg(1, pb_value(item)));
            Pb::new().msg(6, list)
        }
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let structure = keys.into_iter().fold(Pb::new(), |structure, key| {
                structure.msg(1, Pb::new().str(1, key).msg(2, pb_value(&object[key])))
            });
            Pb::new().msg(5, structure)
        }
    }
}

/// google.protobuf.Value → JSON。整数值的 double 还原成整数。
pub(crate) fn pb_any(bytes: &[u8]) -> Value {
    if let Some(item) = fields(bytes).first() {
        return match item.num {
            1 => Value::Null,
            2 => {
                let value = f64::from_le_bytes(item.data.try_into().unwrap_or([0; 8]));
                if value.fract() == 0.0 && value.abs() < 9.0e15 {
                    json!(value as i64)
                } else {
                    json!(value)
                }
            }
            3 => json!(String::from_utf8_lossy(item.data)),
            4 => json!(item.n != 0),
            5 => {
                let mut object = serde_json::Map::new();
                for entry in fields(item.data).iter().filter(|entry| entry.num == 1) {
                    let pair = fields(entry.data);
                    let value = field(&pair, 2)
                        .map(|value| pb_any(value.data))
                        .unwrap_or(Value::Null);
                    object.insert(str_of(&pair, 1), value);
                }
                Value::Object(object)
            }
            6 => Value::Array(
                fields(item.data)
                    .iter()
                    .filter(|entry| entry.num == 1)
                    .map(|entry| pb_any(entry.data))
                    .collect(),
            ),
            _ => Value::Null,
        };
    }
    Value::Null
}

/* —————————————— Connect 信封 —————————————— */

/// 一帧：`[flags][长度 4 字节大端][内容]`。
pub(crate) fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + 5);
    out.push(0);
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// 读到的一帧：结束帧（flags & 2）的内容是 JSON。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectFrame {
    pub end: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub(crate) struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<ConnectFrame>> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while self.buffer.len() >= 5 {
            let flags = self.buffer[0];
            let length = u32::from_be_bytes([
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
                self.buffer[4],
            ]) as usize;
            if length > MAX_FRAME {
                bail!("Cursor 的帧太大（{length} 字节），流已损坏");
            }
            if self.buffer.len() < 5 + length {
                break;
            }
            let data: Vec<u8> = self.buffer.drain(..5 + length).skip(5).collect();
            if flags & 1 != 0 {
                bail!("Cursor 返回了压缩帧（请求里没有声明支持压缩）");
            }
            out.push(ConnectFrame {
                end: flags & 2 != 0,
                data,
            });
        }
        Ok(out)
    }

    pub fn has_partial(&self) -> bool {
        !self.buffer.is_empty()
    }
}

/* —————————————— 失败 —————————————— */

/// Cursor 的 Connect 错误 → (状态码, 说明)。错误 JSON 形如
/// `{"code","message","details":[{"debug":{"error","details":{"title","detail"}}}]}`，流结束帧里包在 `error` 下。
pub(crate) fn failure(status: u16, text: &str) -> (u16, String) {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return status_of(status, "", text.trim());
    };
    let error = value
        .get("error")
        .filter(|error| error.is_object())
        .unwrap_or(&value);
    let mut message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    for detail in error
        .get("details")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let title = detail
            .pointer("/debug/details/title")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let body = detail
            .pointer("/debug/details/detail")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let joined = format!("{title}: {body}");
        let joined = joined.trim_matches(|ch: char| ch == ':' || ch.is_whitespace());
        if !joined.is_empty() {
            message = joined.to_string();
        }
    }
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if message.is_empty() || message == "Error" {
        message = code.to_string();
    }
    if code.is_empty() && (200..300).contains(&status) {
        return (status, message);
    }
    status_of(status, code, &message)
}

fn status_of(status: u16, code: &str, message: &str) -> (u16, String) {
    let message = if message.is_empty() {
        reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|code| code.canonical_reason())
            .unwrap_or("error")
            .to_string()
    } else {
        message.to_string()
    };
    let lower = message.to_ascii_lowercase();
    if lower.contains("region") {
        return (
            403,
            format!("{message}（Cursor 只在部分区域为你的团队提供服务，重新登录也没用）"),
        );
    }
    if code == "permission_denied" {
        return (403, message);
    }
    if code == "unauthenticated" || status == 401 || lower.contains("expired") {
        return (401, format!("{message}（请重新登录 Cursor）"));
    }
    if code == "resource_exhausted"
        || lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("usage limit")
    {
        return (429, format!("额度或频率受限：{message}"));
    }
    if lower.contains("too long")
        || lower.contains("context length")
        || lower.contains("too many tokens")
    {
        return (400, format!("输入超过模型上下文：{message}"));
    }
    match code {
        "invalid_argument" => (400, message),
        "unavailable" => (503, message),
        "" => (
            if (400..600).contains(&status) {
                status
            } else {
                502
            },
            message,
        ),
        _ => (502, message),
    }
}

/// 要求 Max Mode 的拒绝。
fn max_mode_required(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("max mode required")
        || lower.contains("max_mode_required")
        || lower.contains("requires max mode")
}

/* —————————————— 对话 → blob —————————————— */

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tool {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub schema_text: String,
}

/// 能交给模型的工具：`tool_choice: none` 时没有，指定了一个时只给那一个。
pub(crate) fn tools_of(request: &Value) -> Vec<Tool> {
    let choice = request.get("tool_choice");
    if choice.and_then(Value::as_str) == Some("none") {
        return Vec::new();
    }
    let only = choice
        .and_then(|choice| choice.pointer("/function/name"))
        .and_then(Value::as_str);
    request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .unwrap_or("function")
                == "function"
        })
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name").and_then(Value::as_str)?.to_string();
            if only.is_some_and(|only| only != name) {
                return None;
            }
            let schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            Some(Tool {
                name,
                description: function
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                schema_text: schema.to_string(),
                schema,
            })
        })
        .collect()
}

/// 系统提示末尾的工具目录（不列出来模型会说自己没有这个工具）。
fn catalog(tools: &[Tool]) -> String {
    if tools.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "\n\n<dynamic_tool_catalog>\nThe MCP namespace \"{NS}\" has the tools below. They are reached only through `{CALL}`, never as tools of their own. Each entry is the exact call to make; its arguments must match the schema given. The schemas are complete, so there is no need to call `GetDynamicTools` first.\n"
    );
    for tool in tools {
        text.push_str(&format!(
            "<call>{CALL}({{\"namespace\":\"{NS}\",\"toolName\":\"{}\",\"arguments\":{{...}}}})\n{}\narguments schema: {}\n</call>\n",
            tool.name, tool.description, tool.schema_text
        ));
    }
    text + "</dynamic_tool_catalog>"
}

/// 发给调用方的调用 id 里换行写成 `__`，回传时换回来。
fn cursor_call_id(id: &str) -> String {
    if id.starts_with("call_") {
        id.replace("__fc_", "\nfc_")
    } else {
        id.to_string()
    }
}

fn tool_result(id: &str, text: &str, error: bool) -> Value {
    let result = serde_json::from_str::<Value>(text).unwrap_or_else(|_| json!(text));
    let mut part = json!({
        "type": "tool-result",
        "toolCallId": cursor_call_id(id),
        "toolName": CALL,
        "result": result,
        "experimental_content": [{ "type": "text", "text": text }],
    });
    if error {
        part["isError"] = json!(true);
    }
    part
}

/// assistant 带工具调用后紧跟的 assistant 并进前一条（并行调用被拆成两条时）。
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
            let join = |left: String, right: String| {
                [left, right]
                    .into_iter()
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            };
            let content = join(content_text(previous), content_text(message));
            let reasoning = join(reasoning_text(previous), reasoning_text(message));
            let mut calls = previous["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            calls.extend(
                message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            *previous = json!({
                "role": "assistant",
                "content": content,
                "reasoning_content": reasoning,
                "tool_calls": calls,
            });
            continue;
        }
        out.push(message.clone());
    }
    out
}

/// 对话：AI SDK 消息 JSON 的列表、最后一条用户消息的文字、提示长度的估算（token）。
pub(crate) struct Conversation {
    pub messages: Vec<Vec<u8>>,
    pub last: String,
    pub estimate: u64,
}

pub(crate) fn conversation(request: &Value, tools: &[Tool]) -> Conversation {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut size = 0_usize;
    let mut last = ".".to_string();
    let add = |out: &mut Vec<Vec<u8>>, value: Value| out.push(value.to_string().into_bytes());
    let system: Vec<String> = messages
        .iter()
        .filter(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            )
        })
        .map(content_text)
        .filter(|text| !text.is_empty())
        .collect();
    let system = system.join("\n\n") + &catalog(tools);
    size += system.len();
    if !system.is_empty() {
        add(&mut out, json!({ "role": "system", "content": system }));
    }
    let mut pending: Vec<String> = Vec::new();
    let mut results: Vec<Value> = Vec::new();
    let answer = |out: &mut Vec<Vec<u8>>, pending: &mut Vec<String>, results: &mut Vec<Value>| {
        for id in pending.drain(..) {
            results.push(tool_result(&id, NO_RESULT, true));
        }
        if !results.is_empty() {
            out.push(
                json!({ "role": "tool", "content": std::mem::take(results) })
                    .to_string()
                    .into_bytes(),
            );
        }
    };
    for message in join_split_calls(&messages) {
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {}
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(position) = pending.iter().position(|item| item == id) {
                    pending.remove(position);
                    let text = content_text(&message);
                    size += text.len();
                    results.push(tool_result(id, &text, false));
                }
            }
            "assistant" => {
                answer(&mut out, &mut pending, &mut results);
                let mut content = Vec::new();
                let reasoning = reasoning_text(&message);
                if !reasoning.is_empty() {
                    content.push(json!({ "type": "reasoning", "text": reasoning }));
                }
                let text = content_text(&message);
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                size += reasoning.len() + text.len();
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if call
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind != "function")
                    {
                        continue;
                    }
                    let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let arguments = call.pointer("/function/arguments");
                    size +=
                        arguments.map(|value| value.to_string().len()).unwrap_or(0) + name.len();
                    content.push(json!({
                        "type": "tool-call",
                        "toolCallId": cursor_call_id(id),
                        "toolName": CALL,
                        "args": { "namespace": NS, "toolName": name, "arguments": arguments_object(arguments) },
                    }));
                    pending.push(id.to_string());
                }
                if !content.is_empty() {
                    add(&mut out, json!({ "role": "assistant", "content": content }));
                }
            }
            _ => {
                let parts: Vec<Value> = match message.get("content") {
                    Some(Value::String(text)) => vec![json!({ "type": "text", "text": text })],
                    Some(Value::Array(parts)) => parts.clone(),
                    _ => Vec::new(),
                };
                let mut content = Vec::new();
                let mut first = String::new();
                for part in &parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = part.get("text").and_then(Value::as_str).unwrap_or_default();
                            if text.is_empty() {
                                continue;
                            }
                            size += text.len();
                            if first.is_empty() {
                                first = text.to_string();
                            }
                            content.push(json!({ "type": "text", "text": text }));
                        }
                        Some("image_url") => {
                            let url = part
                                .pointer("/image_url/url")
                                .or_else(|| part.get("image_url"))
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let data = url
                                .strip_prefix("data:")
                                .and_then(|rest| rest.split_once(";base64,"))
                                .and_then(|(mime, data)| {
                                    use base64::Engine;
                                    base64::engine::general_purpose::STANDARD
                                        .decode(data)
                                        .ok()
                                        .map(|bytes| (mime.to_string(), bytes))
                                });
                            match data {
                                Some((mime, bytes)) => content.push(json!({
                                    "type": "image",
                                    "mimeType": mime,
                                    "image": { "__type": "Uint8Array", "hex": editor_kit::hex(&bytes) },
                                })),
                                None => content.push(json!({ "type": "text", "text": format!("[attachment image: {url}]") })),
                            }
                        }
                        Some("file") => {
                            let name = part
                                .pointer("/file/filename")
                                .and_then(Value::as_str)
                                .unwrap_or("file");
                            content.push(
                                json!({ "type": "text", "text": format!("[attachment {name}]") }),
                            );
                        }
                        _ => {}
                    }
                }
                answer(&mut out, &mut pending, &mut results);
                if !first.is_empty() {
                    last = first;
                }
                if !content.is_empty() {
                    add(&mut out, json!({ "role": "user", "content": content }));
                }
            }
        }
    }
    answer(&mut out, &mut pending, &mut results);
    for tool in tools {
        size += tool.name.len() + tool.description.len() + tool.schema_text.len();
    }
    Conversation {
        messages: out,
        last,
        estimate: (size / 4) as u64,
    }
}

/* —————————————— AgentRunRequest —————————————— */

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

fn env_message() -> Pb {
    Pb::new()
        .str(1, os_name())
        .str(2, &std::env::temp_dir().to_string_lossy())
        .str(10, "UTC")
}

/// McpToolDefinition。
fn tool_def(tool: &Tool) -> Pb {
    let mut definition = Pb::new().str(1, &tool.name);
    if !tool.description.is_empty() {
        definition = definition.str(2, &tool.description);
    }
    definition
        .msg(3, pb_value(&tool.schema))
        .str(4, NS)
        .str(5, &tool.name)
        .str(6, &tool.schema_text)
}

/// 第一条上行消息（AgentClientMessage.run_request）和服务端可能来要的 blob。
pub(crate) struct RunRequest {
    pub first: Vec<u8>,
    pub blobs: HashMap<[u8; 32], Vec<u8>>,
}

pub(crate) fn build_run(
    conversation: &Conversation,
    tools: &[Tool],
    model: &str,
    conversation_id: &str,
    max_mode: bool,
) -> RunRequest {
    let mut blobs = HashMap::new();
    let mut put = |bytes: Vec<u8>| {
        let id = editor_kit::sha256(&bytes);
        blobs.insert(id, bytes);
        id
    };
    let mut state = Pb::new();
    for message in &conversation.messages {
        let id = put(message.clone());
        state = state.bytes(1, &id);
    }
    let message_id = uuid::Uuid::new_v4().to_string();
    // UserMessage：text、message id、mode = agent。
    let user = Pb::new()
        .str(1, &conversation.last)
        .str(2, &message_id)
        .varint(4, 1)
        .done();
    let user_id = put(user);
    let turn = Pb::new()
        .msg(1, Pb::new().bytes(1, &user_id).str(10, &message_id))
        .done();
    let turn_id = put(turn);
    let state = state.bytes(8, &turn_id).varint(10, 1).str(22, "cli");
    let mut context = Pb::new().msg(4, env_message());
    let mut mcp = Pb::new();
    for tool in tools {
        context = context.msg(7, tool_def(tool));
        mcp = mcp.msg(1, tool_def(tool));
    }
    let action = Pb::new().msg(2, Pb::new().msg(2, context));
    let mut details = Pb::new().str(1, model).str(3, model).str(4, model);
    let mut requested = Pb::new().str(1, model);
    if max_mode {
        details = details.varint(7, 1);
        requested = requested.varint(2, 1);
    }
    let run = Pb::new()
        .msg(1, state)
        .msg(2, action)
        .msg(3, details)
        .msg(4, mcp)
        .str(5, conversation_id)
        .msg(9, requested)
        .varint(19, 1);
    RunRequest {
        first: Pb::new().msg(1, run).done(),
        blobs,
    }
}

/// 会话 id：按 `prompt_cache_key` 固定（上游按它路由到缓存了提示的机器），没有就随机。
fn conversation_id(request: &Value) -> String {
    let key = request
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty());
    let Some(key) = key else {
        return uuid::Uuid::new_v4().to_string();
    };
    let first_user = request
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages
                .iter()
                .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        })
        .map(Value::to_string)
        .unwrap_or_default();
    let digest = editor_kit::sha256(format!("cursor conversation\0{key}\0{first_user}").as_bytes());
    let mut bytes: [u8; 16] = digest[..16].try_into().unwrap_or([0; 16]);
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/* —————————————— 上行消息 —————————————— */

fn close_exec(id: u64) -> Vec<u8> {
    Pb::new()
        .msg(5, Pb::new().msg(1, Pb::new().varint(1, id)))
        .done()
}

fn refuse_exec(id: u64) -> Vec<u8> {
    Pb::new()
        .msg(
            5,
            Pb::new().msg(2, Pb::new().varint(1, id).str(2, "not available")),
        )
        .done()
}

fn heartbeat() -> Vec<u8> {
    Pb::new().bytes(7, &[]).done()
}

/// BidiAppend 的请求体：消息的十六进制、请求 id、序号。
pub(crate) fn append_body(message: &[u8], request_id: &str, seq: u64) -> Vec<u8> {
    Pb::new()
        .str(1, &editor_kit::hex(message))
        .msg(2, Pb::new().str(1, request_id))
        .varint(3, seq)
        .done()
}

/// 一条 MCP 调用。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct McpCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// exec_server_message：调用方工具的调用返回出来；查工具列表、查环境的就地回答；其余（Cursor 自己的
/// 工具）一律拒绝。`replies` 收集要发回的上行消息。返回 (调用, 是否算本步「已答」的一个)。
pub(crate) fn exec(
    message: &[u8],
    tools: &[Tool],
    replies: &mut Vec<Vec<u8>>,
) -> (Option<McpCall>, bool) {
    let items = fields(message);
    let id = num_of(&items, 1);
    let exec_id = str_of(&items, 15);
    let answer = |replies: &mut Vec<Vec<u8>>, num: u64, result: Pb| {
        replies.push(
            Pb::new()
                .msg(
                    2,
                    Pb::new().varint(1, id).str(15, &exec_id).msg(num, result),
                )
                .done(),
        );
        replies.push(close_exec(id));
    };
    let refuse = |replies: &mut Vec<Vec<u8>>| {
        replies.push(refuse_exec(id));
        replies.push(close_exec(id));
    };
    for item in &items {
        match item.num {
            11 => {
                let args = fields(item.data);
                let full = str_of(&args, 1);
                let name = Some(str_of(&args, 5))
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| {
                        full.strip_prefix(&format!("{NS}-"))
                            .unwrap_or(&full)
                            .to_string()
                    });
                let provider = str_of(&args, 4);
                let foreign = (!provider.is_empty() && provider != NS)
                    || num_of(&args, 7) != 0
                    || !tools.iter().any(|tool| tool.name == name);
                if foreign {
                    refuse(replies);
                    return (None, true);
                }
                let mut object = serde_json::Map::new();
                for entry in args.iter().filter(|entry| entry.num == 2) {
                    let pair = fields(entry.data);
                    let value = field(&pair, 2)
                        .map(|value| pb_any(value.data))
                        .unwrap_or(Value::Null);
                    object.insert(str_of(&pair, 1), value);
                }
                let call_id = str_of(&args, 3).replace('\n', "__");
                let call_id = if call_id.is_empty() {
                    editor_kit::new_call_id()
                } else {
                    call_id
                };
                return (
                    Some(McpCall {
                        id: call_id,
                        name,
                        arguments: Value::Object(object).to_string(),
                    }),
                    false,
                );
            }
            36 => {
                let mut server = Pb::new().str(1, NS).str(2, NS).str(7, "connected");
                for tool in tools {
                    server = server.msg(5, tool_def(tool));
                }
                answer(replies, 36, Pb::new().msg(1, Pb::new().msg(1, server)));
                return (None, true);
            }
            10 => {
                answer(
                    replies,
                    10,
                    Pb::new().msg(1, Pb::new().msg(1, Pb::new().msg(4, env_message()))),
                );
                return (None, false);
            }
            _ => {}
        }
    }
    refuse(replies);
    (None, true)
}

/// kv_server_message：要 blob 就给，存 blob 就应一声。
pub(crate) fn kv(message: &[u8], blobs: &HashMap<[u8; 32], Vec<u8>>, replies: &mut Vec<Vec<u8>>) {
    let items = fields(message);
    let id = num_of(&items, 1);
    for item in &items {
        match item.num {
            2 => {
                let wanted = field(&fields(item.data), 1)
                    .map(|field| field.data.to_vec())
                    .unwrap_or_default();
                let blob = <[u8; 32]>::try_from(wanted.as_slice())
                    .ok()
                    .and_then(|key| blobs.get(&key));
                let result = match blob {
                    Some(blob) => Pb::new().bytes(1, blob),
                    None => Pb::new().msg(2, Pb::new().str(1, "blob not found")),
                };
                replies.push(
                    Pb::new()
                        .msg(3, Pb::new().varint(1, id).msg(2, result))
                        .done(),
                );
            }
            3 => replies.push(
                Pb::new()
                    .msg(3, Pb::new().varint(1, id).bytes(3, &[]))
                    .done(),
            ),
            _ => {}
        }
    }
}

/* —————————————— 下行事件 → Chat SSE —————————————— */

/// 读下行消息时的状态。
pub(crate) struct RunState {
    tools: Vec<Tool>,
    blobs: HashMap<[u8; 32], Vec<u8>>,
    estimate: u64,
    calls: u64,
    answered: u64,
    listed: u64,
    said: u64,
    wrote: u64,
    usage: Option<Usage>,
}

/// 处理一帧的结果。
#[derive(Debug, PartialEq)]
pub(crate) enum Step {
    /// 继续读。
    Going,
    /// 本轮结束（已经写了 finish）。
    Done,
}

impl RunState {
    pub fn new(tools: Vec<Tool>, blobs: HashMap<[u8; 32], Vec<u8>>, estimate: u64) -> Self {
        Self {
            tools,
            blobs,
            estimate,
            calls: 0,
            answered: 0,
            listed: 0,
            said: 0,
            wrote: 0,
            usage: None,
        }
    }

    /// 本步的调用都到齐了（全被就地答掉的不算，Cursor 那边会接着走）。
    fn step_done(&mut self) -> bool {
        if self.listed == 0 || self.calls + self.answered < self.listed {
            return false;
        }
        if self.calls > 0 {
            return true;
        }
        self.listed = 0;
        self.answered = 0;
        false
    }

    fn finish(&self, sse: &mut ChatSse) -> Vec<u8> {
        let usage = self.usage.unwrap_or(Usage {
            prompt_tokens: self.estimate,
            completion_tokens: self.said.div_ceil(4),
            ..Usage::default()
        });
        let reason = if self.calls > 0 { "tool_calls" } else { "stop" };
        sse.finish(Some(reason), Some(usage))
    }

    fn empty_reply() -> anyhow::Error {
        fail(
            502,
            "Cursor 没有给出任何回复（没有文字也没有工具调用），请重试",
        )
    }

    /// 一帧下行消息：输出写进 `out`，要回的上行消息写进 `replies`。
    pub fn push(
        &mut self,
        sse: &mut ChatSse,
        frame: &ConnectFrame,
        out: &mut Vec<u8>,
        replies: &mut Vec<Vec<u8>>,
    ) -> anyhow::Result<Step> {
        if frame.end {
            let (status, message) = failure(200, &String::from_utf8_lossy(&frame.data));
            if !(200..300).contains(&status) {
                return Err(fail(status, format!("Cursor：{message}")));
            }
            if self.wrote == 0 && self.calls == 0 {
                return Err(Self::empty_reply());
            }
            out.extend(self.finish(sse));
            return Ok(Step::Done);
        }
        for message in fields(&frame.data) {
            match message.num {
                1 => {
                    for update in fields(message.data) {
                        let inner = fields(update.data);
                        match update.num {
                            1 => {
                                let text = str_of(&inner, 1);
                                self.said += text.chars().count() as u64;
                                self.wrote += text.chars().count() as u64;
                                out.extend(sse.text(&text));
                            }
                            4 => {
                                let text = str_of(&inner, 1);
                                self.said += text.chars().count() as u64;
                                out.extend(sse.reasoning(&text));
                            }
                            14 => {
                                let input = num_of(&inner, 1);
                                let cache_read = num_of(&inner, 3);
                                let cache_write = num_of(&inner, 4);
                                self.usage = Some(Usage {
                                    prompt_tokens: input.max(cache_read + cache_write),
                                    completion_tokens: num_of(&inner, 2),
                                    cached_tokens: cache_read,
                                    reasoning_tokens: num_of(&inner, 5),
                                });
                                if self.calls == 0 {
                                    if self.wrote == 0 {
                                        return Err(Self::empty_reply());
                                    }
                                    out.extend(self.finish(sse));
                                    return Ok(Step::Done);
                                }
                            }
                            27 => {
                                self.listed = num_of(&inner, 1);
                                if self.step_done() {
                                    out.extend(self.finish(sse));
                                    return Ok(Step::Done);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                2 => {
                    let (call, answered) = exec(message.data, &self.tools, replies);
                    if let Some(call) = call {
                        self.calls += 1;
                        self.said += call.arguments.len() as u64;
                        out.extend(sse.tool_call(&call.id, &call.name, &call.arguments));
                    } else if answered {
                        self.answered += 1;
                    }
                    if self.step_done() {
                        out.extend(self.finish(sse));
                        return Ok(Step::Done);
                    }
                }
                4 => kv(message.data, &self.blobs, replies),
                _ => {}
            }
        }
        Ok(Step::Going)
    }

    /// 下行流结束了还没收尾：有调用就按调用收尾，否则算断了。
    pub fn eof(&self, sse: &mut ChatSse) -> anyhow::Result<Vec<u8>> {
        if self.calls > 0 {
            return Ok(self.finish(sse));
        }
        Err(fail(502, "Cursor 的回复中途断开（EOF）"))
    }
}

/* —————————————— 版本、请求头 —————————————— */

fn is_version(name: &str) -> bool {
    let Some((date, hash)) = name.split_once('-') else {
        return false;
    };
    let parts: Vec<&str> = date.split('.').collect();
    parts.len() == 3
        && [4, 2, 2]
            .iter()
            .zip(&parts)
            .all(|(len, part)| part.len() == *len && part.chars().all(|ch| ch.is_ascii_digit()))
        && !hash.is_empty()
        && hash
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
}

/// 自报的客户端版本 `cli-<版本>`：本机装了 cursor-agent 用它的版本（服务端会拒绝过旧的版本）。
fn client_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION.get_or_init(|| {
        let mut dirs = vec![
            editor_kit::home()
                .join(".local")
                .join("share")
                .join("cursor-agent")
                .join("versions"),
        ];
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                std::path::PathBuf::from(local)
                    .join("cursor-agent")
                    .join("versions"),
            );
        }
        let best = dirs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_version(name))
            .max();
        format!("cli-{}", best.as_deref().unwrap_or(VERSION_FALLBACK))
    })
}

/// cursor-agent 的 cli-config.json 里登录的团队（账号对得上时）。
fn team_id(token: &str) -> Option<String> {
    let dir = std::env::var_os("CURSOR_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .filter(|dir| !dir.is_empty())
                .map(|dir| std::path::PathBuf::from(dir).join("cursor"))
        })
        .unwrap_or_else(|| editor_kit::home().join(".cursor"));
    let config = editor_kit::read_json_file(&dir.join("cli-config.json")).ok()?;
    let auth = config.get("authInfo")?;
    let subject = jwt_claims(token)
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string)?;
    if auth.get("authId").and_then(Value::as_str) != Some(subject.as_str()) {
        return None;
    }
    match auth.get("activeTeamId")? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn common_headers(request: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    let mut request = request
        .bearer_auth(token)
        .header("x-cursor-client-version", client_version())
        .header("x-cursor-client-type", "cli")
        .header("x-ghost-mode", "true");
    if let Some(team) = team_id(token) {
        request = request.header("x-cursor-team-id", team);
    }
    request
}

/// JSON 的 Connect unary 调用。
async fn unary(
    http: &reqwest::Client,
    path: &str,
    token: &str,
    body: &Value,
) -> anyhow::Result<Value> {
    let response = common_headers(http.post(format!("{API}/{path}")), token)
        .header("content-type", "application/json")
        .header("connect-protocol-version", "1")
        .json(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Cursor：{error}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let (status, message) = failure(status, &text);
        return Err(fail(status, format!("Cursor {path}：{message}")));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/* —————————————— 凭据 —————————————— */

fn access_token(credentials: &Value) -> anyhow::Result<String> {
    text_of(credentials, "accessToken")
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Cursor 凭据里没有 access token，请重新登录"))
}

/// token 的过期时间：JWT 的 exp，读不到按 30 天。
fn expiry_of(token: &str) -> u64 {
    jwt_expires_ms(token).unwrap_or_else(|| now_ms() + 30 * 24 * 3600 * 1000)
}

fn credentials_from(access: &str, refresh: &str, source: &str) -> Value {
    json!({
        "accessToken": access,
        "refreshToken": refresh,
        "expiresAt": expiry_of(access),
        "source": source,
    })
}

/// cursor-agent 的 auth.json 路径。
fn agent_auth_file() -> std::path::PathBuf {
    let home = editor_kit::home();
    match std::env::consts::OS {
        "windows" => std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("Cursor")
            .join("auth.json"),
        "macos" => home.join(".cursor").join("auth.json"),
        _ => std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("cursor")
            .join("auth.json"),
    }
}

/// Cursor 编辑器的 state.vscdb 路径。
fn ide_state_db() -> std::path::PathBuf {
    let home = editor_kit::home();
    let base = match std::env::consts::OS {
        "macos" => home.join("Library").join("Application Support"),
        "windows" => std::env::var_os("APPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming")),
        _ => std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home.join(".config")),
    };
    base.join("Cursor")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb")
}

/// cursor-agent 的 auth.json → (access, refresh)。
fn parse_agent_auth(value: &Value) -> Option<(String, String)> {
    let access = text_of(value, "accessToken")?;
    Some((
        access.to_string(),
        text_of(value, "refreshToken")
            .unwrap_or_default()
            .to_string(),
    ))
}

/// cursor-agent 的登录：macOS 先读钥匙串，再读 auth.json。
fn read_agent() -> Option<Value> {
    if cfg!(target_os = "macos")
        && let Some(access) = editor_kit::command_output(
            "/usr/bin/security",
            &[
                "find-generic-password",
                "-s",
                "cursor-access-token",
                "-a",
                "cursor-user",
                "-w",
            ],
        )
    {
        let refresh = editor_kit::command_output(
            "/usr/bin/security",
            &[
                "find-generic-password",
                "-s",
                "cursor-refresh-token",
                "-a",
                "cursor-user",
                "-w",
            ],
        )
        .unwrap_or_default();
        return Some(credentials_from(&access, &refresh, "agent"));
    }
    let value = editor_kit::read_json_file(&agent_auth_file()).ok()?;
    let (access, refresh) = parse_agent_auth(&value)?;
    Some(credentials_from(&access, &refresh, "agent"))
}

/// Cursor 编辑器的登录（state.vscdb 的 ItemTable）。
fn read_ide() -> Option<Value> {
    let db = rusqlite::Connection::open_with_flags(
        ide_state_db(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let get = |key: &str| -> Option<String> {
        db.query_row("SELECT value FROM ItemTable WHERE key = ?1", [key], |row| {
            row.get::<_, String>(0)
        })
        .ok()
        .map(|value| value.trim().trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
    };
    let access = get("cursorAuth/accessToken")?;
    let refresh = get("cursorAuth/refreshToken").unwrap_or_default();
    let mut credentials = credentials_from(&access, &refresh, "ide");
    if let Some(email) = get("cursorAuth/cachedEmail") {
        credentials["account"] = json!(email);
    }
    Some(credentials)
}

/// 用 refresh token 换新的 access token（Cursor 客户端的刷新方式）。
async fn refresh_with_token(http: &reqwest::Client, refresh: &str) -> anyhow::Result<String> {
    let response = http
        .post(format!("{API}/oauth/token"))
        .header("content-type", "application/json")
        .json(&json!({
            "grant_type": "refresh_token",
            "client_id": OAUTH_CLIENT_ID,
            "refresh_token": refresh,
        }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Cursor：{error}")))?;
    let status = response.status().as_u16();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if value.get("shouldLogout").and_then(Value::as_bool) == Some(true) {
        bail!(fail(401, "Cursor 要求重新登录"));
    }
    match text_of(&value, "access_token").or_else(|| text_of(&value, "accessToken")) {
        Some(token) if (200..300).contains(&status) => Ok(token.to_string()),
        _ => Err(fail(
            if matches!(status, 400 | 401 | 403) {
                401
            } else {
                502
            },
            format!("刷新 Cursor 登录失败（{status}）"),
        )),
    }
}

/// Cursor API Key 换 access token。
async fn exchange_api_key(http: &reqwest::Client, key: &str) -> anyhow::Result<String> {
    let response = common_headers(http.post(format!("{API}/auth/exchange_user_api_key")), key)
        .header("content-type", "application/json")
        .body("{}")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Cursor：{error}")))?;
    let status = response.status().as_u16();
    if status >= 500 {
        return Err(fail(502, format!("Cursor 换取 token 失败（{status}）")));
    }
    let value: Value = response.json().await.unwrap_or(Value::Null);
    text_of(&value, "accessToken")
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Cursor 不认这个 API Key"))
}

/// 账号邮箱和套餐。
async fn who_is(http: &reqwest::Client, token: &str) -> (String, String) {
    let email = unary(
        http,
        "aiserver.v1.DashboardService/GetMe",
        token,
        &json!({}),
    )
    .await
    .ok()
    .and_then(|value| text_of(&value, "email").map(str::to_string))
    .unwrap_or_default();
    let plan = unary(
        http,
        "aiserver.v1.DashboardService/GetPlanInfo",
        token,
        &json!({}),
    )
    .await
    .ok()
    .and_then(|value| {
        value
            .pointer("/planInfo/planName")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
    .unwrap_or_default();
    (email, plan)
}

async fn finish_credentials(
    http: &reqwest::Client,
    mut credentials: Value,
) -> anyhow::Result<LoginDone> {
    let token = access_token(&credentials)?;
    let (email, plan) = who_is(http, &token).await;
    let account = if email.is_empty() {
        text_of(&credentials, "account")
            .unwrap_or("Cursor")
            .to_string()
    } else {
        email
    };
    credentials["account"] = json!(account);
    if !plan.is_empty() {
        credentials["plan"] = json!(plan);
    }
    Ok(LoginDone {
        credentials,
        account,
    })
}

/// 浏览器登录：轮询 `/auth/poll` 直到用户在网页上登录完。
async fn poll_browser(
    http: &reqwest::Client,
    uuid: &str,
    verifier: &str,
) -> anyhow::Result<(String, String)> {
    let mut errors = 0;
    for round in 0..150 {
        let wait = Duration::from_millis((1000.0 * 1.2_f64.powi(round)).min(10_000.0) as u64);
        let response = http
            .get(format!(
                "{API}/auth/poll?{}",
                query(&[("uuid", uuid), ("verifier", verifier)])
            ))
            .header("content-type", "application/json")
            .header("x-cursor-client-version", client_version())
            .header("x-cursor-client-type", "cli")
            .timeout(Duration::from_secs(15))
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                errors += 1;
                if errors >= 3 {
                    bail!("连不上 Cursor：{error}");
                }
                tokio::time::sleep(wait).await;
                continue;
            }
        };
        let status = response.status().as_u16();
        if status == 404 {
            errors = 0;
            tokio::time::sleep(wait).await;
            continue;
        }
        if !(200..300).contains(&status) {
            errors += 1;
            if status == 403 || errors >= 3 {
                bail!("Cursor 拒绝了这次登录（{status}）");
            }
            tokio::time::sleep(wait).await;
            continue;
        }
        let value: Value = response.json().await.unwrap_or(Value::Null);
        let access = text_of(&value, "accessToken")
            .ok_or_else(|| anyhow::anyhow!("Cursor 没有返回 token"))?;
        return Ok((
            access.to_string(),
            text_of(&value, "refreshToken")
                .unwrap_or_default()
                .to_string(),
        ));
    }
    bail!("登录超时：没有在浏览器里完成 Cursor 登录")
}

/* —————————————— 一次 Run —————————————— */

/// 只记得「这个模型只能用 Max Mode」。
fn max_only() -> &'static Mutex<HashSet<String>> {
    static MAX_ONLY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    MAX_ONLY.get_or_init(Default::default)
}

/// 一次 Run 的结局。
enum Outcome {
    /// 已经写完（或写了错误）。
    Finished,
    /// 还没写任何东西就被要求开 Max Mode。
    NeedMaxMode,
}

/// 上行：按序号发 BidiAppend。KV 回复可以并发（最多 16 个），其余的等前面的都答完再单独发。
async fn uplink(
    http: reqwest::Client,
    token: String,
    request_id: String,
    mut messages: mpsc::UnboundedReceiver<Vec<u8>>,
    errors: mpsc::UnboundedSender<anyhow::Error>,
) {
    let mut seq = 0_u64;
    let mut inflight = tokio::task::JoinSet::new();
    while let Some(message) = messages.recv().await {
        let body = append_body(&message, &request_id, seq);
        seq += 1;
        let overlaps = message.first() == Some(&0x1a);
        if !overlaps {
            while let Some(done) = inflight.join_next().await {
                if let Ok(Err(error)) = done {
                    let _ = errors.send(error);
                    return;
                }
            }
        } else if inflight.len() >= APPEND_AT_ONCE
            && let Some(Ok(Err(error))) = inflight.join_next().await
        {
            let _ = errors.send(error);
            return;
        }
        let send = append(http.clone(), token.clone(), request_id.clone(), body);
        if overlaps {
            inflight.spawn(send);
        } else if let Err(error) = send.await {
            let _ = errors.send(error);
            return;
        }
        // 顺手收掉已经完成的。
        while let Some(done) = inflight.try_join_next() {
            if let Ok(Err(error)) = done {
                let _ = errors.send(error);
                return;
            }
        }
    }
}

async fn append(
    http: reqwest::Client,
    token: String,
    request_id: String,
    body: Vec<u8>,
) -> anyhow::Result<()> {
    let timeout = Duration::from_secs(60 + body.len().div_ceil(65_536) as u64);
    let response = common_headers(
        http.post(format!("{API}/aiserver.v1.BidiService/BidiAppend")),
        &token,
    )
    .header("x-request-id", &request_id)
    .header(
        "x-cursor-agent-allowed-tools",
        "mcp_tool_call,get_mcp_tools_tool_call",
    )
    .header("accept-encoding", "identity")
    .header("x-cursor-streaming", "true")
    .header("content-type", "application/proto")
    .header("connect-protocol-version", "1")
    .body(body)
    .timeout(timeout)
    .send()
    .await
    .map_err(|error| fail(503, format!("Cursor BidiAppend：{error}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let (status, message) = failure(status, &text);
        return Err(fail(status, format!("Cursor：{message}")));
    }
    Ok(())
}

/// 后台任务结束时把上行和心跳一起停掉。
struct AbortOnDrop(Vec<tokio::task::JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for handle in &self.0 {
            handle.abort();
        }
    }
}

async fn run_once(
    out: &mut Emitter,
    http: &reqwest::Client,
    token: &str,
    request: &Value,
    model_id: &str,
    max_mode: bool,
) -> Outcome {
    let tools = tools_of(request);
    let conversation = conversation(request, &tools);
    let run = build_run(
        &conversation,
        &tools,
        model_id,
        &conversation_id(request),
        max_mode,
    );
    let request_id = uuid::Uuid::new_v4().to_string();
    let (up_tx, up_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let (error_tx, mut error_rx) = mpsc::unbounded_channel::<anyhow::Error>();
    let uplink_task = tokio::spawn(uplink(
        http.clone(),
        token.to_string(),
        request_id.clone(),
        up_rx,
        error_tx,
    ));
    let beat_tx = up_tx.clone();
    let heartbeat_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;
        loop {
            interval.tick().await;
            if beat_tx.send(heartbeat()).is_err() {
                return;
            }
        }
    });
    // 先把下行流发出去，再发第一条上行消息（不必等下行的响应头）。
    let downlink = common_headers(
        http.post(format!("{API}/agent.v1.AgentService/RunSSE")),
        token,
    )
    .header("x-request-id", &request_id)
    .header(
        "x-cursor-agent-allowed-tools",
        "mcp_tool_call,get_mcp_tools_tool_call",
    )
    .header("accept-encoding", "identity")
    .header("x-cursor-streaming", "true")
    .header("content-type", "application/connect+proto")
    .header("connect-protocol-version", "1")
    .body(frame(&Pb::new().str(1, &request_id).done()))
    .send();
    let downlink = tokio::spawn(downlink);
    let _guard = AbortOnDrop(vec![uplink_task, heartbeat_task]);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = up_tx.send(run.first.clone());
    let response = match downlink.await {
        Ok(response) => response.map_err(anyhow::Error::from),
        Err(error) => Err(anyhow::Error::from(error)),
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            out.fail(fail(503, format!("无法连接 Cursor：{error}")))
                .await;
            return Outcome::Finished;
        }
    };
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let text = response.text().await.unwrap_or_default();
        let (status, message) = failure(status, &text);
        if !max_mode && max_mode_required(&message) {
            return Outcome::NeedMaxMode;
        }
        out.fail(fail(status, format!("Cursor：{message}"))).await;
        return Outcome::Finished;
    }
    let mut state = RunState::new(tools, run.blobs, conversation.estimate);
    let mut reader = FrameReader::default();
    let mut bytes = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            chunk = tokio::time::timeout(QUIET, bytes.next()) => chunk,
            Some(error) = error_rx.recv() => {
                let message = format!("{error:#}");
                if !max_mode && !out.started() && max_mode_required(&message) {
                    return Outcome::NeedMaxMode;
                }
                out.fail(error).await;
                return Outcome::Finished;
            }
        };
        let chunk = match chunk {
            Err(_) => {
                out.fail(fail(502, "Cursor 超过 60 秒没有回应")).await;
                return Outcome::Finished;
            }
            Ok(None) => {
                let result = if reader.has_partial() {
                    Err(fail(502, "Cursor 的回复在帧中间断开"))
                } else {
                    state.eof(&mut out.sse)
                };
                match result {
                    Ok(bytes) => {
                        out.send(bytes).await;
                    }
                    Err(error) => out.fail(error).await,
                }
                return Outcome::Finished;
            }
            Ok(Some(Err(error))) => {
                match state.eof(&mut out.sse) {
                    Ok(bytes) => {
                        out.send(bytes).await;
                    }
                    Err(_) => {
                        out.fail(fail(502, format!("Cursor 的回复中途断开：{error}")))
                            .await
                    }
                }
                return Outcome::Finished;
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        let frames = match reader.push(&chunk) {
            Ok(frames) => frames,
            Err(error) => {
                out.fail(fail(502, format!("{error:#}"))).await;
                return Outcome::Finished;
            }
        };
        for frame in frames {
            let mut bytes = Vec::new();
            let mut replies = Vec::new();
            let step = state.push(&mut out.sse, &frame, &mut bytes, &mut replies);
            for reply in replies {
                let _ = up_tx.send(reply);
            }
            match step {
                Ok(step) => {
                    if !out.send(bytes).await {
                        return Outcome::Finished;
                    }
                    if step == Step::Done {
                        return Outcome::Finished;
                    }
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    if !max_mode && !out.started() && max_mode_required(&message) {
                        return Outcome::NeedMaxMode;
                    }
                    out.fail(error).await;
                    return Outcome::Finished;
                }
            }
        }
    }
}

/// 请求里的模型名 → Cursor 的 model_id（Auto 是 `default`）。
fn model_id(model: &str) -> String {
    if model.is_empty() || model == "auto" {
        "default".to_string()
    } else {
        model.to_string()
    }
}

/// GetUsableModels 的答复 → 模型列表（`default` 显示成 `auto`，放第一个）。
fn parse_models(value: &Value) -> Vec<String> {
    let mut out = vec!["auto".to_string()];
    for model in value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = text_of(model, "modelId") else {
            continue;
        };
        let id = if id == "default" { "auto" } else { id };
        if !out.iter().any(|item| item == id) {
            out.push(id.to_string());
        }
    }
    out
}

/* —————————————— Subscription —————————————— */

#[async_trait::async_trait]
impl Subscription for Cursor {
    fn id(&self) -> &'static str {
        "cursor"
    }

    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要 Cursor 账号（Pro / Business 等付费套餐才有足量的高级模型）：可在浏览器里登录，或先在 Cursor / cursor-agent 里登录后导入，也可粘贴 Cursor API Key。"
    }

    fn detect_local(&self) -> Option<String> {
        // 不读钥匙串：先看编辑器的 state.vscdb，再看 cursor-agent 的 auth.json。
        if let Some(credentials) = read_ide() {
            return Some(text_of(&credentials, "account").unwrap_or_default().to_string());
        }
        let value = editor_kit::read_json_file(&agent_auth_file()).ok()?;
        parse_agent_auth(&value).map(|_| String::new())
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => {
                let (verifier, challenge) = editor_kit::pkce(32);
                let uuid = uuid::Uuid::new_v4().to_string();
                let url = format!(
                    "{WEBSITE}/loginDeepControl?{}",
                    query(&[
                        ("challenge", &challenge),
                        ("uuid", &uuid),
                        ("mode", "login"),
                        ("redirectTarget", "cli")
                    ])
                );
                Ok(LoginStart {
                    open_url: Some(url),
                    message: "请在浏览器里登录 Cursor，登录完会自动继续。".to_string(),
                    state: json!({ "method": "browser", "uuid": uuid, "verifier": verifier }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::ImportLocal => {
                let credentials = read_agent().or_else(read_ide).ok_or_else(|| {
                    anyhow::anyhow!("没找到本机 Cursor 的登录：请先在 Cursor 编辑器或 cursor-agent（cursor-agent login）里登录")
                })?;
                Ok(LoginStart {
                    message: "已读取本机 Cursor 的登录。".to_string(),
                    state: json!({ "method": "import", "credentials": credentials }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let text = input.trim();
                if text.is_empty() {
                    bail!("请粘贴 Cursor 的 access token 或 API Key");
                }
                Ok(LoginStart {
                    message: "正在校验。".to_string(),
                    state: json!({ "method": "paste", "input": text }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::DeviceCode => bail!("Cursor 不支持设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let credentials = match text_of(state, "method").unwrap_or_default() {
            "browser" => {
                let (access, refresh) = poll_browser(
                    http,
                    text_of(state, "uuid").unwrap_or_default(),
                    text_of(state, "verifier").unwrap_or_default(),
                )
                .await?;
                credentials_from(&access, &refresh, "browser")
            }
            "import" => state.get("credentials").cloned().unwrap_or(Value::Null),
            "paste" => {
                let text = text_of(state, "input").unwrap_or_default();
                if text.split('.').count() == 3 {
                    credentials_from(text, "", "paste")
                } else {
                    let access = exchange_api_key(http, text).await?;
                    let mut credentials = credentials_from(&access, "", "apikey");
                    credentials["apiKey"] = json!(text);
                    credentials
                }
            }
            other => bail!("不认识的登录状态：{other}"),
        };
        finish_credentials(http, credentials).await
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !expiring(credentials, "expiresAt", REFRESH_AHEAD_MS) {
            return Ok(None);
        }
        let mut updated = credentials.clone();
        // 导入的：官方客户端可能已经自己续过了。
        let local = match text_of(credentials, "source") {
            Some("agent") => read_agent(),
            Some("ide") => read_ide(),
            _ => None,
        };
        if let Some(local) = local.filter(|local| !expiring(local, "expiresAt", REFRESH_AHEAD_MS)) {
            for key in ["accessToken", "refreshToken", "expiresAt"] {
                updated[key] = local[key].clone();
            }
            return Ok(Some(updated));
        }
        let fresh = if let Some(key) = text_of(credentials, "apiKey") {
            exchange_api_key(http, key).await
        } else if let Some(refresh) = text_of(credentials, "refreshToken") {
            refresh_with_token(http, refresh).await
        } else {
            Err(fail(401, "Cursor 的登录已过期，请重新登录"))
        };
        match fresh {
            Ok(access) => {
                updated["accessToken"] = json!(access);
                updated["expiresAt"] = json!(expiry_of(&access));
                Ok(Some(updated))
            }
            // 还没真过期就先用着，过期了才算失效。
            Err(_) if !expiring(credentials, "expiresAt", 0) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let token = access_token(credentials)?;
        match unary(
            http,
            "agent.v1.AgentService/GetUsableModels",
            &token,
            &json!({}),
        )
        .await
        {
            Ok(value) => Ok(parse_models(&value)),
            Err(_) => Ok(vec!["auto".to_string()]),
        }
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let token = access_token(credentials)?;
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string();
        let id = model_id(&model);
        let http = http.clone();
        Ok(editor_kit::spawn_chat(&model, move |mut out| async move {
            let mut max_mode = max_only()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&id);
            loop {
                match run_once(&mut out, &http, &token, &request, &id, max_mode).await {
                    Outcome::Finished => return,
                    Outcome::NeedMaxMode => {
                        max_only()
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert(id.clone());
                        max_mode = true;
                    }
                }
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::SubscriptionError;
    use crate::gateway::subscriptions::chat::aggregate;

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    fn status(error: &anyhow::Error) -> u16 {
        error
            .downcast_ref::<SubscriptionError>()
            .map(|error| error.status)
            .unwrap_or(0)
    }

    #[test]
    fn varints_round_trip() {
        for value in [0_u64, 1, 127, 128, 300, 16_384, u32::MAX as u64, u64::MAX] {
            let bytes = Pb::new().varint(5, value).done();
            let decoded = fields(&bytes);
            assert_eq!(decoded.len(), 1);
            assert_eq!(
                (decoded[0].num, decoded[0].wire, decoded[0].n),
                (5, 0, value)
            );
        }
        let bytes = Pb::new()
            .str(1, "hi")
            .msg(2, Pb::new().varint(1, 7))
            .double(3, 1.5)
            .done();
        let decoded = fields(&bytes);
        assert_eq!(str_of(&decoded, 1), "hi");
        assert_eq!(num_of(&fields(field(&decoded, 2).unwrap().data), 1), 7);
        assert_eq!(
            f64::from_le_bytes(field(&decoded, 3).unwrap().data.try_into().unwrap()),
            1.5
        );
        // 截断的尾巴被丢掉，前面的字段保留。
        let mut broken = bytes.clone();
        broken.truncate(bytes.len() - 3);
        assert_eq!(fields(&broken).len(), 2);
    }

    #[test]
    fn protobuf_values_round_trip() {
        let value = json!({ "b": [1, 2.5, "x", true, null], "a": { "nested": "y" }, "n": -3 });
        let encoded = pb_value(&value).done();
        assert_eq!(pb_any(&encoded), value);
    }

    #[test]
    fn client_messages_match_reference_bytes() {
        assert_eq!(editor_kit::hex(&frame(&heartbeat())), "00000000023a00");
        let request_id = "00000000-0000-4000-8000-000000000000";
        assert_eq!(
            editor_kit::hex(&append_body(&heartbeat(), request_id, 3)),
            "0a043361303012260a2430303030303030302d303030302d343030302d383030302d3030303030303030303030301803"
        );
        assert_eq!(
            editor_kit::hex(&frame(&Pb::new().str(1, request_id).done())),
            "00000000260a2430303030303030302d303030302d343030302d383030302d303030303030303030303030"
        );
        assert_eq!(editor_kit::hex(&close_exec(1)), "2a040a020801");
        assert_eq!(
            editor_kit::hex(&refuse_exec(2)),
            "2a1312110802120d6e6f7420617661696c61626c65"
        );
    }

    fn read_tool() -> Tool {
        let schema = json!({ "type": "object", "properties": { "path": { "type": "string" } } });
        Tool {
            name: "read".into(),
            description: "Read a file".into(),
            schema_text: schema.to_string(),
            schema,
        }
    }

    #[test]
    fn exec_returns_mcp_calls_and_refuses_cursor_tools() {
        // 参考资料里的 mcp 调用样例：id 1、exec-1、read{path:"a"}、call_1。
        let frames = FrameReader::default()
            .push(&unhex("0000000043124108017a06657865632d315a350a0a746f6f6c732d72656164120b0a047061746812031a01611a0663616c6c5f312205746f6f6c732a04726561644a05746f6f6c73"))
            .unwrap();
        let message = fields(&frames[0].data);
        let exec_message = field(&message, 2).unwrap();
        let mut replies = Vec::new();
        let (call, answered) = exec(exec_message.data, &[read_tool()], &mut replies);
        assert_eq!(
            call,
            Some(McpCall {
                id: "call_1".into(),
                name: "read".into(),
                arguments: "{\"path\":\"a\"}".into()
            })
        );
        assert!(!answered);
        assert!(replies.is_empty());

        // 调用方没有这个工具：拒绝并关闭。
        let (call, answered) = exec(exec_message.data, &[], &mut replies);
        assert!(call.is_none() && answered);
        assert_eq!(
            editor_kit::hex(&replies[0]),
            "2a1312110802120d6e6f7420617661696c61626c65".replace("0802", "0801")
        );
        assert_eq!(editor_kit::hex(&replies[1]), "2a040a020801");

        // Cursor 自己的 shell 工具（字段 14）：拒绝。
        let shell = Pb::new()
            .varint(1, 9)
            .str(15, "e9")
            .msg(14, Pb::new().str(1, "rm -rf /"))
            .done();
        let mut replies = Vec::new();
        let (call, answered) = exec(&shell, &[read_tool()], &mut replies);
        assert!(call.is_none() && answered);
        assert_eq!(fields(&replies[0])[0].num, 5);

        // 查工具列表：回答带上工具定义。
        let state = Pb::new()
            .varint(1, 4)
            .str(15, "e4")
            .msg(36, Pb::new())
            .done();
        let mut replies = Vec::new();
        let (_, answered) = exec(&state, &[read_tool()], &mut replies);
        assert!(answered);
        let reply = fields(&replies[0]);
        let inner = fields(field(&reply, 2).unwrap().data);
        assert_eq!(num_of(&inner, 1), 4);
        assert_eq!(str_of(&inner, 15), "e4");
        assert!(field(&inner, 36).is_some());

        // 查环境：回答但不算本步的调用。
        let context = Pb::new().varint(1, 5).msg(10, Pb::new()).done();
        let mut replies = Vec::new();
        let (_, answered) = exec(&context, &[], &mut replies);
        assert!(!answered);
        assert_eq!(replies.len(), 2);
    }

    #[test]
    fn kv_serves_blobs() {
        let mut blobs = HashMap::new();
        blobs.insert([0xab_u8; 32], b"{}".to_vec());
        let frames = FrameReader::default()
            .push(&unhex("00000000282226080512220a20abababababababababababababababababababababababababababababababab"))
            .unwrap();
        let message = fields(&frames[0].data);
        let mut replies = Vec::new();
        kv(field(&message, 4).unwrap().data, &blobs, &mut replies);
        assert_eq!(editor_kit::hex(&replies[0]), "1a08080512040a027b7d");
        let mut replies = Vec::new();
        kv(
            &Pb::new().varint(1, 6).msg(3, Pb::new().str(1, "x")).done(),
            &blobs,
            &mut replies,
        );
        assert_eq!(editor_kit::hex(&replies[0]), "1a0408061a00");
        let mut replies = Vec::new();
        kv(
            &Pb::new()
                .varint(1, 7)
                .msg(2, Pb::new().bytes(1, &[1; 32]))
                .done(),
            &blobs,
            &mut replies,
        );
        assert!(String::from_utf8_lossy(&replies[0]).contains("blob not found"));
    }

    #[test]
    fn conversation_turns_into_blobs() {
        let request = json!({
            "model": "auto",
            "messages": [
                { "role": "system", "content": "be brief" },
                { "role": "user", "content": [
                    { "type": "text", "text": "read a" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUJD" } },
                ] },
                { "role": "assistant", "content": "", "reasoning_content": "hmm", "tool_calls": [
                    { "id": "call_1__fc_2", "type": "function", "function": { "name": "read", "arguments": "{\"path\":\"a\"}" } },
                ] },
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "call_3", "type": "function", "function": { "name": "read", "arguments": "{\"path\":\"b\"}" } },
                ] },
                { "role": "tool", "tool_call_id": "call_1__fc_2", "content": "{\"ok\":true}" },
                { "role": "user", "content": "thanks" },
            ],
            "tools": [{ "type": "function", "function": { "name": "read", "description": "Read a file", "parameters": { "type": "object", "properties": { "path": { "type": "string" } } } } }],
        });
        let tools = tools_of(&request);
        assert_eq!(tools, vec![read_tool()]);
        let conversation = conversation(&request, &tools);
        let messages: Vec<Value> = conversation
            .messages
            .iter()
            .map(|bytes| serde_json::from_slice(bytes).unwrap())
            .collect();
        assert_eq!(conversation.last, "thanks");
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[0]["role"], "system");
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.starts_with("be brief\n\n<dynamic_tool_catalog>"));
        assert!(system.contains("\"toolName\":\"read\""));
        assert_eq!(messages[1]["content"][1]["image"]["hex"], "414243");
        // 两条 assistant 合并成一条，两个调用。
        let calls = messages[2]["content"].as_array().unwrap();
        assert_eq!(calls[0]["type"], "reasoning");
        assert_eq!(calls[1]["toolCallId"], "call_1\nfc_2");
        assert_eq!(calls[1]["args"]["arguments"]["path"], "a");
        assert_eq!(calls[2]["toolCallId"], "call_3");
        // 结果：一个真结果（JSON 解析过）、一个补的「被中断」。
        let results = messages[3]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["result"]["ok"], true);
        assert_eq!(results[1]["toolCallId"], "call_3");
        assert_eq!(results[1]["isError"], true);
        assert_eq!(messages[4]["content"][0]["text"], "thanks");

        let run = build_run(&conversation, &tools, "default", "conv-1", false);
        // 每条消息一个 blob，加上 UserMessage 和 turn。
        assert_eq!(run.blobs.len(), conversation.messages.len() + 2);
        let client = fields(&run.first);
        let request_fields = fields(field(&client, 1).unwrap().data);
        let state = fields(field(&request_fields, 1).unwrap().data);
        assert_eq!(state.iter().filter(|item| item.num == 1).count(), 5);
        assert!(
            run.blobs
                .contains_key(<&[u8; 32]>::try_from(field(&state, 1).unwrap().data).unwrap())
        );
        assert_eq!(str_of(&state, 22), "cli");
        assert_eq!(str_of(&request_fields, 5), "conv-1");
        let details = fields(field(&request_fields, 3).unwrap().data);
        assert_eq!(str_of(&details, 1), "default");
        assert!(field(&details, 7).is_none());
        let mcp = fields(field(&request_fields, 4).unwrap().data);
        let definition = fields(field(&mcp, 1).unwrap().data);
        assert_eq!(str_of(&definition, 4), NS);
        assert_eq!(
            pb_any(field(&definition, 3).unwrap().data),
            read_tool().schema
        );
        assert_eq!(num_of(&request_fields, 19), 1);

        let max = build_run(&conversation, &tools, "m", "c", true);
        let request_fields = fields(field(&fields(&max.first), 1).unwrap().data);
        assert_eq!(
            num_of(&fields(field(&request_fields, 9).unwrap().data), 2),
            1
        );

        // tool_choice none：不给工具，系统提示里也没有目录。
        let mut none = request.clone();
        none["tool_choice"] = json!("none");
        assert!(tools_of(&none).is_empty());
    }

    #[test]
    fn conversation_ids_are_stable_per_key() {
        let request =
            json!({ "prompt_cache_key": "k", "messages": [{ "role": "user", "content": "hi" }] });
        let id = conversation_id(&request);
        assert_eq!(id, conversation_id(&request));
        assert_eq!(&id[14..15], "4");
        assert_ne!(
            conversation_id(&json!({ "messages": [] })),
            conversation_id(&json!({ "messages": [] }))
        );
    }

    fn update(num: u64, inner: Pb) -> ConnectFrame {
        ConnectFrame {
            end: false,
            data: Pb::new().msg(1, Pb::new().msg(num, inner)).done(),
        }
    }

    #[test]
    fn server_stream_becomes_chat_sse() {
        let mut state = RunState::new(vec![read_tool()], HashMap::new(), 10);
        let mut sse = ChatSse::new("auto");
        let mut out = Vec::new();
        let mut replies = Vec::new();
        // 参考资料里的字节：text "hi"、thinking "ok"。
        let mut reader = FrameReader::default();
        let mut frames = reader
            .push(&unhex(
                "00000000080a0622040a026f6b00000000080a060a040a026869",
            ))
            .unwrap();
        frames.push(update(1, Pb::new().str(1, " there")));
        for frame in &frames {
            assert_eq!(
                state.push(&mut sse, frame, &mut out, &mut replies).unwrap(),
                Step::Going
            );
        }
        let ended = reader
            .push(&unhex("000000000c0a0a7208080a100518022801"))
            .unwrap();
        assert_eq!(
            state
                .push(&mut sse, &ended[0], &mut out, &mut replies)
                .unwrap(),
            Step::Done
        );
        let completion = aggregate(&String::from_utf8(out).unwrap(), "auto").unwrap();
        assert_eq!(completion["choices"][0]["message"]["content"], "hi there");
        assert_eq!(
            completion["choices"][0]["message"]["reasoning_content"],
            "ok"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "stop");
        assert_eq!(completion["usage"]["prompt_tokens"], 10);
        assert_eq!(completion["usage"]["completion_tokens"], 5);
        assert_eq!(
            completion["usage"]["prompt_tokens_details"]["cached_tokens"],
            2
        );
    }

    #[test]
    fn tool_step_ends_when_all_calls_arrive() {
        let mut state = RunState::new(vec![read_tool()], HashMap::new(), 10);
        let mut sse = ChatSse::new("auto");
        let mut out = Vec::new();
        let mut replies = Vec::new();
        // 先来一个 Cursor 自己的工具（被拒，算已答）、再来一个 MCP 调用，本步共 2 个。
        let shell = ConnectFrame {
            end: false,
            data: Pb::new()
                .msg(2, Pb::new().varint(1, 1).msg(14, Pb::new()))
                .done(),
        };
        assert_eq!(
            state
                .push(&mut sse, &shell, &mut out, &mut replies)
                .unwrap(),
            Step::Going
        );
        let count = update(27, Pb::new().varint(1, 2));
        assert_eq!(
            state
                .push(&mut sse, &count, &mut out, &mut replies)
                .unwrap(),
            Step::Going
        );
        let call = ConnectFrame {
            end: false,
            data: Pb::new()
                .msg(
                    2,
                    Pb::new().varint(1, 2).str(15, "e2").msg(
                        11,
                        Pb::new()
                            .str(1, "tools-read")
                            .msg(2, Pb::new().str(1, "path").msg(2, pb_value(&json!("a"))))
                            .str(3, "call_x\nfc_y"),
                    ),
                )
                .done(),
        };
        assert_eq!(
            state.push(&mut sse, &call, &mut out, &mut replies).unwrap(),
            Step::Done
        );
        assert_eq!(replies.len(), 2, "拒绝 + 关闭");
        let completion = aggregate(&String::from_utf8(out).unwrap(), "auto").unwrap();
        let calls = &completion["choices"][0]["message"]["tool_calls"];
        assert_eq!(calls[0]["id"], "call_x__fc_y");
        assert_eq!(calls[0]["function"]["name"], "read");
        assert_eq!(calls[0]["function"]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn end_frames_and_empty_replies_fail_with_status() {
        let mut sse = ChatSse::new("m");
        let mut out = Vec::new();
        let mut replies = Vec::new();
        let mut state = RunState::new(Vec::new(), HashMap::new(), 0);
        let end = ConnectFrame { end: true, data: br#"{"error":{"code":"resource_exhausted","message":"You've hit your usage limit"}}"#.to_vec() };
        assert_eq!(
            status(
                &state
                    .push(&mut sse, &end, &mut out, &mut replies)
                    .unwrap_err()
            ),
            429
        );
        let empty = FrameReader::default()
            .push(&unhex("02000000027b7d"))
            .unwrap();
        assert!(empty[0].end);
        assert_eq!(
            status(
                &state
                    .push(&mut sse, &empty[0], &mut out, &mut replies)
                    .unwrap_err()
            ),
            502
        );
        assert_eq!(status(&state.eof(&mut sse).unwrap_err()), 502);
    }

    #[test]
    fn failures_map_to_statuses() {
        let unauthenticated = r#"{"code":"unauthenticated","message":"Error","details":[{"debug":{"error":"ERROR_NOT_LOGGED_IN","details":{"title":"Authentication error","detail":"If you are logged in, try logging out and back in."}}}]}"#;
        let (code, message) = failure(401, unauthenticated);
        assert_eq!(code, 401);
        assert!(message.starts_with("Authentication error: If you are logged in"));
        assert_eq!(
            failure(400, r#"{"code":"permission_denied","message":"no"}"#).0,
            403
        );
        assert_eq!(
            failure(
                400,
                r#"{"code":"invalid_argument","message":"prompt is too long"}"#
            )
            .0,
            400
        );
        assert_eq!(
            failure(503, r#"{"code":"unavailable","message":"busy"}"#).0,
            503
        );
        assert_eq!(failure(500, "plain text").0, 500);
        assert_eq!(failure(200, "{}"), (200, String::new()));
        assert!(max_mode_required(
            "Max Mode Required: The model \"x\" requires Max Mode to be enabled."
        ));
        assert!(!max_mode_required("plain"));
    }

    #[test]
    fn frames_split_anywhere() {
        let bytes = [frame(b"abc"), frame(b""), {
            let mut end = frame(b"{}");
            end[0] = 2;
            end
        }]
        .concat();
        let mut reader = FrameReader::default();
        let mut frames = Vec::new();
        for byte in &bytes {
            frames.extend(reader.push(&[*byte]).unwrap());
        }
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"abc");
        assert!(frames[2].end);
        assert!(!reader.has_partial());
        let mut compressed = frame(b"x");
        compressed[0] = 1;
        assert!(FrameReader::default().push(&compressed).is_err());
    }

    #[test]
    fn local_logins_parse_and_expire() {
        let token = format!(
            "x.{}.y",
            editor_kit::b64url(
                json!({ "sub": "auth0|u", "exp": 4_102_444_800_u64 })
                    .to_string()
                    .as_bytes()
            )
        );
        let (access, refresh) =
            parse_agent_auth(&json!({ "accessToken": token, "refreshToken": "r" })).unwrap();
        let credentials = credentials_from(&access, &refresh, "agent");
        assert_eq!(credentials["expiresAt"], 4_102_444_800_000_u64);
        assert!(!expiring(&credentials, "expiresAt", REFRESH_AHEAD_MS));
        let old = format!(
            "x.{}.y",
            editor_kit::b64url(json!({ "exp": 1 }).to_string().as_bytes())
        );
        assert!(expiring(
            &credentials_from(&old, "", "paste"),
            "expiresAt",
            REFRESH_AHEAD_MS
        ));
        assert!(parse_agent_auth(&json!({})).is_none());
        assert!(is_version("2026.09.23-86fc751"));
        assert!(!is_version("2026.9.23-86fc751"));
        assert!(client_version().starts_with("cli-"));
    }

    #[test]
    fn models_map_default_to_auto() {
        let value = json!({ "models": [{ "modelId": "default" }, { "modelId": "claude-opus-5-5-high" }, { "modelId": "gpt-5.5" }] });
        assert_eq!(
            parse_models(&value),
            vec!["auto", "claude-opus-5-5-high", "gpt-5.5"]
        );
        assert_eq!(parse_models(&json!({})), vec!["auto"]);
        assert_eq!(model_id("auto"), "default");
        assert_eq!(model_id("gpt-5.5"), "gpt-5.5");
    }
}
