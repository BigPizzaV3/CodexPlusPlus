//! catalog_b 各家订阅共用的小工具：
//!
//! - 把上游的流（SSE / 自定义帧）逐块解码成 Chat SSE 的 [`decode_stream`]；
//! - 三种常见上游事件 → [`ChatSse`] 的转换：OpenAI Chat 分片（[`ChatRelay`]）、
//!   Anthropic Messages 事件（[`AnthropicRelay`]）、OpenAI Responses 事件（[`ResponsesRelay`]）；
//! - 浏览器登录时跨 `begin_login` / `finish_login` 暂存本机回调监听；
//! - 工具 schema 根部展开、无压缩 gzip、时间解析等零碎工具。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};

use futures_util::{Stream, StreamExt};
use serde_json::{Map, Value, json};

use crate::gateway::subscriptions::chat::{ChatSse, SseReader, Usage};
use crate::gateway::subscriptions::{ChatStream, fail};

// ---------------------------------------------------------------------------
// 杂项
// ---------------------------------------------------------------------------

/// 用户主目录。
pub fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 依次取几个字段（支持 `/a/b` 指针），返回第一个非空字符串。
pub fn str_at<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        let found = if key.starts_with('/') {
            value.pointer(key)
        } else {
            value.get(*key)
        };
        found
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    })
}

/// 依次取几个字段，返回第一个数字（数字或纯数字字符串）。
pub fn u64_at(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        let found = if key.starts_with('/') {
            value.pointer(key)
        } else {
            value.get(*key)
        }?;
        match found {
            Value::Number(number) => number
                .as_u64()
                .or_else(|| number.as_f64().filter(|v| *v >= 0.0).map(|v| v as u64)),
            Value::String(text) => text.trim().parse::<u64>().ok(),
            _ => None,
        }
    })
}

/// 小写十六进制随机串（`bytes` 个字节）。
pub fn random_hex(bytes: usize) -> String {
    let mut out = String::new();
    while out.len() < bytes * 2 {
        out.push_str(&uuid::Uuid::new_v4().simple().to_string());
    }
    out.truncate(bytes * 2);
    out
}

/// 随机字节。
pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 16);
    while out.len() < len {
        out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    out.truncate(len);
    out
}

/// SHA-256 十六进制。
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 把种子哈希成 UUID 形状（8-4-4-4-12），同一种子总得到同一个值。
pub fn uuid_from_seed(seed: &str) -> String {
    let hex = sha256_hex(seed.as_bytes());
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// 解析 ISO 8601 / RFC 3339 时间（`2026-10-30T08:00:00Z`、`2026-10-30 08:00:00+08:00`、
/// 带毫秒都行），返回毫秒时间戳；不认识的格式返回 None。
pub fn parse_iso_ms(text: &str) -> Option<u64> {
    let text = text.trim();
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| -> Option<i64> { text.get(range)?.parse().ok() };
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if bytes[4] != b'-' || bytes[7] != b'-' || !matches!(bytes[10], b'T' | b't' | b' ') {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0_i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
        rest = &fraction[digits.len()..];
        let padded = format!("{digits:0<3}");
        millis = padded[..3].parse().unwrap_or(0);
    }
    let offset_minutes = match rest {
        "" | "Z" | "z" => 0,
        zone => {
            let sign = match zone.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            let zone = zone[1..].replace(':', "");
            let hours: i64 = zone.get(0..2)?.parse().ok()?;
            let minutes: i64 = zone.get(2..4).unwrap_or("00").parse().ok()?;
            sign * (hours * 60 + minutes)
        }
    };
    // 公历日期 → 自 1970-01-01 的天数（Howard Hinnant 的算法）。
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset_minutes * 60;
    let ms = seconds * 1000 + millis;
    (ms >= 0).then_some(ms as u64)
}

/// 「秒或毫秒」的时间戳统一成毫秒（小于 1e12 视为秒）。
pub fn to_ms(value: u64) -> u64 {
    if value < 1_000_000_000_000 {
        value * 1000
    } else {
        value
    }
}

// ---------------------------------------------------------------------------
// 浏览器登录：暂存本机回调监听
// ---------------------------------------------------------------------------

fn listeners() -> &'static Mutex<HashMap<String, tokio::net::TcpListener>> {
    static LISTENERS: OnceLock<Mutex<HashMap<String, tokio::net::TcpListener>>> = OnceLock::new();
    LISTENERS.get_or_init(Default::default)
}

/// `begin_login` 里建好的回调监听先存起来（`LoginStart.state` 只能放 JSON），返回取用的键。
pub fn park_listener(listener: tokio::net::TcpListener) -> String {
    let key = uuid::Uuid::new_v4().simple().to_string();
    listeners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key.clone(), listener);
    key
}

/// `finish_login` 取回监听；已经取过或进程重启过时为 None。
pub fn take_listener(key: &str) -> Option<tokio::net::TcpListener> {
    listeners()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(key)
}

// ---------------------------------------------------------------------------
// gzip（只用存储块，不压缩）：上游要求请求体带 gzip 编码时用
// ---------------------------------------------------------------------------

/// CRC-32（IEEE）。
pub fn crc32(data: &[u8]) -> u32 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0_u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut crc = index as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
            *slot = crc;
        }
        table
    });
    let mut crc = 0xFFFF_FFFF_u32;
    for byte in data {
        crc = table[((crc ^ *byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

/// 合法的 gzip 流（deflate 存储块，不做压缩）：没有压缩库可用，服务端照样能解。
pub fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

// ---------------------------------------------------------------------------
// 工具 schema：根部 anyOf / oneOf / allOf 展开成 object
// ---------------------------------------------------------------------------

/// 有些上游拒收根部是 `anyOf` / `oneOf` / `allOf` 或没写 `type` 的工具参数：
/// 合并成一个 object（`allOf` 的 required 取并集，`anyOf` / `oneOf` 取交集；同名属性定义不同时写成 anyOf）。
pub fn object_root(schema: &Value) -> Value {
    let Some(object) = schema.as_object() else {
        return json!({ "type": "object", "properties": {} });
    };
    let has_union = ["anyOf", "oneOf", "allOf"]
        .iter()
        .any(|key| object.contains_key(*key));
    if !has_union && object.get("type").and_then(Value::as_str) == Some("object") {
        return schema.clone();
    }
    let mut properties = Map::new();
    let mut required: Option<Vec<String>> = None;
    collect_object(schema, schema, 0, &mut properties, &mut required);
    let mut out = Map::new();
    for (key, value) in object {
        if !matches!(
            key.as_str(),
            "anyOf" | "oneOf" | "allOf" | "type" | "properties" | "required"
        ) {
            out.insert(key.clone(), value.clone());
        }
    }
    out.insert("type".into(), json!("object"));
    out.insert("properties".into(), Value::Object(properties));
    if let Some(required) = required.filter(|list| !list.is_empty()) {
        out.insert("required".into(), json!(required));
    }
    Value::Object(out)
}

fn resolve_ref<'a>(root: &'a Value, schema: &'a Value) -> &'a Value {
    let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
        return schema;
    };
    reference
        .strip_prefix('#')
        .and_then(|pointer| root.pointer(pointer))
        .unwrap_or(schema)
}

fn merge_property(properties: &mut Map<String, Value>, key: &str, value: &Value) {
    match properties.get_mut(key) {
        None => {
            properties.insert(key.to_string(), value.clone());
        }
        Some(existing) if existing == value => {}
        Some(existing) => {
            let mut variants = existing
                .get("anyOf")
                .and_then(Value::as_array)
                .filter(|_| existing.as_object().is_some_and(|map| map.len() == 1))
                .cloned()
                .unwrap_or_else(|| vec![existing.clone()]);
            if !variants.contains(value) {
                variants.push(value.clone());
            }
            *existing = json!({ "anyOf": variants });
        }
    }
}

fn collect_object(
    root: &Value,
    schema: &Value,
    depth: usize,
    properties: &mut Map<String, Value>,
    required: &mut Option<Vec<String>>,
) {
    if depth > 8 {
        return;
    }
    let schema = resolve_ref(root, schema);
    if let Some(own) = schema.get("properties").and_then(Value::as_object) {
        for (key, value) in own {
            merge_property(properties, key, value);
        }
    }
    let own_required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if !own_required.is_empty() || schema.get("properties").is_some() {
        merge_required(required, own_required);
    }
    if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
        for branch in all {
            let mut branch_required = None;
            collect_object(root, branch, depth + 1, properties, &mut branch_required);
            if let Some(list) = branch_required {
                // allOf：各分支的 required 都要满足 → 并集。
                let merged = required.get_or_insert_with(Vec::new);
                for item in list {
                    if !merged.contains(&item) {
                        merged.push(item);
                    }
                }
            }
        }
    }
    for key in ["anyOf", "oneOf"] {
        let Some(branches) = schema.get(key).and_then(Value::as_array) else {
            continue;
        };
        let mut common: Option<Vec<String>> = None;
        for branch in branches {
            let target = resolve_ref(root, branch);
            let is_object = target.get("type").and_then(Value::as_str) == Some("object")
                || target.get("properties").is_some()
                || target.get("anyOf").is_some()
                || target.get("oneOf").is_some()
                || target.get("allOf").is_some();
            if !is_object {
                continue;
            }
            let mut branch_required = Some(Vec::new());
            collect_object(root, target, depth + 1, properties, &mut branch_required);
            let list = branch_required.unwrap_or_default();
            common = Some(match common {
                None => list,
                Some(previous) => previous
                    .into_iter()
                    .filter(|item| list.contains(item))
                    .collect(),
            });
        }
        if let Some(list) = common {
            // anyOf / oneOf：只有每个分支都要求的才算必填 → 交集。
            let merged = required.get_or_insert_with(Vec::new);
            for item in list {
                if !merged.contains(&item) {
                    merged.push(item);
                }
            }
        }
    }
}

fn merge_required(required: &mut Option<Vec<String>>, list: Vec<String>) {
    let merged = required.get_or_insert_with(Vec::new);
    for item in list {
        if !merged.contains(&item) {
            merged.push(item);
        }
    }
}

// ---------------------------------------------------------------------------
// 上游流 → Chat SSE
// ---------------------------------------------------------------------------

/// 把上游字节流解码成 Chat SSE 的解码器。`push` 喂进一段字节，`end` 在上游结束时调用。
/// 返回的空字节会被跳过（第一块有内容或出错时网关才回状态码）。
pub trait Decoder: Send + 'static {
    fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>>;
    fn end(&mut self) -> anyhow::Result<Vec<u8>>;
}

/// 按 SSE 事件处理的上游。
pub trait SseHandler: Send + 'static {
    fn event(&mut self, event: &str, data: &str) -> anyhow::Result<Vec<u8>>;
    fn end(&mut self) -> anyhow::Result<Vec<u8>>;
}

/// SSE 切行 + [`SseHandler`]。
pub struct Sse<H> {
    reader: SseReader,
    pub handler: H,
}

impl<H: SseHandler> Sse<H> {
    pub fn new(handler: H) -> Self {
        Self {
            reader: SseReader::default(),
            handler,
        }
    }
}

impl<H: SseHandler> Decoder for Sse<H> {
    fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        for (event, data) in self.reader.push(bytes) {
            out.extend(self.handler.event(&event, &data)?);
        }
        Ok(out)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        if let Some((event, data)) = self.reader.finish() {
            out.extend(self.handler.event(&event, &data)?);
        }
        out.extend(self.handler.end()?);
        Ok(out)
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Vec<u8>>> + Send>>;

/// 上游答复 → Chat SSE 流。
pub fn decode_stream<D: Decoder>(response: reqwest::Response, decoder: D) -> ChatStream {
    let body: ByteStream = Box::pin(
        response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec())),
    );
    decode_byte_stream(body, decoder)
}

fn decode_byte_stream<D: Decoder>(body: ByteStream, decoder: D) -> ChatStream {
    Box::pin(futures_util::stream::unfold(
        (body, decoder, false),
        |(mut body, mut decoder, done)| async move {
            if done {
                return None;
            }
            loop {
                match body.next().await {
                    Some(Ok(bytes)) => match decoder.push(&bytes) {
                        Ok(out) if out.is_empty() => continue,
                        Ok(out) => return Some((Ok(out), (body, decoder, false))),
                        Err(error) => return Some((Err(error), (body, decoder, true))),
                    },
                    Some(Err(error)) => {
                        return Some((
                            Err(fail(502, format!("上游连接中断：{error}"))),
                            (body, decoder, true),
                        ));
                    }
                    None => {
                        return match decoder.end() {
                            Ok(out) if out.is_empty() => None,
                            Ok(out) => Some((Ok(out), (body, decoder, true))),
                            Err(error) => Some((Err(error), (body, decoder, true))),
                        };
                    }
                }
            }
        },
    ))
}

/// 测试用：把几段字节喂给解码器，返回拼出的 SSE 文本和中途的错误。
#[cfg(test)]
pub fn run_decoder<D: Decoder>(
    mut decoder: D,
    chunks: &[&[u8]],
) -> (String, Option<anyhow::Error>) {
    let mut out = Vec::new();
    for chunk in chunks {
        match decoder.push(chunk) {
            Ok(bytes) => out.extend(bytes),
            Err(error) => return (String::from_utf8_lossy(&out).to_string(), Some(error)),
        }
    }
    match decoder.end() {
        Ok(bytes) => out.extend(bytes),
        Err(error) => return (String::from_utf8_lossy(&out).to_string(), Some(error)),
    }
    (String::from_utf8_lossy(&out).to_string(), None)
}

/// OpenAI 格式的 usage → [`Usage`]。
pub fn openai_usage(usage: &Value) -> Usage {
    let number = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
    Usage {
        prompt_tokens: number("/prompt_tokens"),
        completion_tokens: number("/completion_tokens"),
        cached_tokens: number("/prompt_tokens_details/cached_tokens"),
        reasoning_tokens: number("/completion_tokens_details/reasoning_tokens"),
    }
}

/// 把 OpenAI Chat 分片重新拼成 [`ChatSse`]：统一推理字段名（`reasoning_content` / `reasoning`），
/// 工具调用按序号对上 id（id 没给时生成），可以把工具名映射回调用方的原名。
#[derive(Debug)]
pub struct ChatRelay {
    pub sse: ChatSse,
    /// 上游工具名 → 调用方原名。
    pub rename: HashMap<String, String>,
    tools: HashMap<u64, RelayTool>,
    finish_reason: Option<String>,
    usage: Option<Usage>,
    said: bool,
}

#[derive(Debug, Default)]
struct RelayTool {
    id: String,
    name: String,
    pending: String,
    started: bool,
}

impl ChatRelay {
    pub fn new(model: &str) -> Self {
        Self {
            sse: ChatSse::new(model),
            rename: HashMap::new(),
            tools: HashMap::new(),
            finish_reason: None,
            usage: None,
            said: false,
        }
    }

    /// 有没有输出过任何内容（文本、推理、工具调用）。
    pub fn said(&self) -> bool {
        self.said
    }

    pub fn text(&mut self, text: &str) -> Vec<u8> {
        if !text.is_empty() {
            self.said = true;
        }
        self.sse.text(text)
    }

    /// 直接发一个完整的工具调用（从文本里解析出来的那种）。
    pub fn whole_tool_call(&mut self, name: &str, arguments: &str) -> Vec<u8> {
        self.said = true;
        let id = format!("call_{}", uuid::Uuid::new_v4().simple());
        self.sse.tool_call(&id, name, arguments)
    }

    /// 一个分片（`chat.completion.chunk`）。分片里带 `error` 时报错。
    pub fn chunk(&mut self, chunk: &Value) -> anyhow::Result<Vec<u8>> {
        self.chunk_with(chunk, |relay, text| relay.text(text))
    }

    /// 同 [`Self::chunk`]，正文交给 `on_text`（要从正文里再切工具调用的上游用）。
    pub fn chunk_with(
        &mut self,
        chunk: &Value,
        mut on_text: impl FnMut(&mut Self, &str) -> Vec<u8>,
    ) -> anyhow::Result<Vec<u8>> {
        if let Some(error) = chunk.get("error").filter(|value| !value.is_null()) {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| error.as_str())
                .unwrap_or("上游返回错误");
            let status = error
                .get("code")
                .and_then(Value::as_u64)
                .filter(|code| (400..600).contains(code))
                .unwrap_or(502) as u16;
            return Err(fail(status, message.to_string()));
        }
        if let Some(usage) = chunk.get("usage").filter(|value| value.is_object()) {
            self.usage = Some(openai_usage(usage));
        }
        let mut out = Vec::new();
        let Some(choice) = chunk.get("choices").and_then(|choices| choices.get(0)) else {
            return Ok(out);
        };
        let delta = choice
            .get("delta")
            .or_else(|| choice.get("message"))
            .cloned()
            .unwrap_or(Value::Null);
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    self.said = true;
                    out.extend(self.sse.reasoning(text));
                }
                break;
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            out.extend(on_text(self, text));
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.extend(self.tool_delta(call));
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        Ok(out)
    }

    fn tool_delta(&mut self, call: &Value) -> Vec<u8> {
        let index = call
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or(self.tools.len() as u64);
        let slot = self.tools.entry(index).or_default();
        if slot.id.is_empty()
            && let Some(id) = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
        {
            slot.id = id.to_string();
        }
        if let Some(name) = call
            .pointer("/function/name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            slot.name = name.to_string();
        }
        if let Some(arguments) = call.pointer("/function/arguments") {
            match arguments {
                Value::String(text) => slot.pending.push_str(text),
                Value::Object(_) => slot.pending.push_str(&arguments.to_string()),
                _ => {}
            }
        }
        if slot.name.is_empty() {
            return Vec::new();
        }
        self.said = true;
        if slot.id.is_empty() {
            slot.id = format!("call_{}", uuid::Uuid::new_v4().simple());
        }
        let pending = std::mem::take(&mut slot.pending);
        if slot.started {
            let id = slot.id.clone();
            return self.sse.tool_arguments(&id, &pending);
        }
        slot.started = true;
        let id = slot.id.clone();
        let name = self
            .rename
            .get(&slot.name)
            .cloned()
            .unwrap_or_else(|| slot.name.clone());
        self.sse.tool_call(&id, &name, &pending)
    }

    /// 还没拿到名字的工具调用（流结束时仍是这样说明上游答复残缺）。
    pub fn has_nameless_tool(&self) -> bool {
        self.tools.values().any(|tool| !tool.started)
    }

    /// 收尾：按上游的结束原因和 usage 发结束块和 `[DONE]`。
    pub fn finish(&mut self) -> Vec<u8> {
        let reason = match self.finish_reason.as_deref() {
            _ if self.sse.has_tool_calls() => None,
            Some("length") => Some("length"),
            Some("content_filter") => Some("content_filter"),
            _ => None,
        };
        let usage = self.usage;
        self.sse.finish(reason, usage)
    }
}

/// 上游就是 OpenAI Chat SSE：每个 `data:` 是一个分片，`[DONE]` 结束。
pub struct OpenAiChatSse {
    pub relay: ChatRelay,
    pub vendor: &'static str,
}

impl SseHandler for OpenAiChatSse {
    fn event(&mut self, _event: &str, data: &str) -> anyhow::Result<Vec<u8>> {
        let data = data.trim();
        if data.is_empty() || self.relay.sse.is_finished() {
            return Ok(Vec::new());
        }
        if data == "[DONE]" {
            return Ok(self.relay.finish());
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|_| fail(502, format!("{} 返回了无法解析的数据", self.vendor)))?;
        self.relay.chunk(&chunk)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        if self.relay.sse.is_finished() {
            return Ok(Vec::new());
        }
        if !self.relay.said() {
            return Err(fail(502, format!("{} 返回了空回复", self.vendor)));
        }
        Ok(self.relay.finish())
    }
}

/// Anthropic 错误类型 → HTTP 状态码。
pub fn anthropic_error_status(kind: &str) -> u16 {
    match kind {
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "overloaded_error" => 503,
        "invalid_request_error" => 400,
        _ => 502,
    }
}

#[derive(Debug)]
enum AnthropicBlock {
    Other,
    Tool {
        id: String,
        /// 请求时把参数包成了 `{"arguments": …}`：攒齐后解包再发。
        wrapped: bool,
        buffer: String,
    },
}

/// Anthropic Messages 的流事件 → [`ChatSse`]。
#[derive(Debug)]
pub struct AnthropicRelay {
    pub sse: ChatSse,
    /// 请求时参数被包了一层 `arguments` 的工具名。
    pub wrapped: HashSet<String>,
    /// 上游工具名 → 调用方原名。
    pub rename: HashMap<String, String>,
    blocks: HashMap<u64, AnthropicBlock>,
    input_tokens: u64,
    cache_read: u64,
    cache_write: u64,
    output_tokens: u64,
    stop: Option<String>,
    said: bool,
    vendor: &'static str,
}

impl AnthropicRelay {
    pub fn new(model: &str, vendor: &'static str) -> Self {
        Self {
            sse: ChatSse::new(model),
            wrapped: HashSet::new(),
            rename: HashMap::new(),
            blocks: HashMap::new(),
            input_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            output_tokens: 0,
            stop: None,
            said: false,
            vendor,
        }
    }

    fn usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.input_tokens + self.cache_read + self.cache_write,
            completion_tokens: self.output_tokens,
            cached_tokens: self.cache_read,
            reasoning_tokens: 0,
        }
    }

    fn read_usage(&mut self, usage: &Value) {
        let number = |key: &str| usage.get(key).and_then(Value::as_u64);
        if let Some(value) = number("input_tokens") {
            self.input_tokens = value;
        }
        if let Some(value) = number("cache_read_input_tokens") {
            self.cache_read = value;
        }
        if let Some(value) = number("cache_creation_input_tokens") {
            self.cache_write = value;
        }
        if let Some(value) = number("output_tokens") {
            self.output_tokens = value;
        }
    }

    fn finish(&mut self) -> Vec<u8> {
        let reason = match self.stop.as_deref() {
            Some("max_tokens") => Some("length"),
            Some("refusal") => Some("content_filter"),
            _ => None,
        };
        let usage = self.usage();
        self.sse.finish(reason, Some(usage))
    }

    /// 一个事件（`data` 是 JSON）。
    pub fn event(&mut self, data: &Value) -> anyhow::Result<Vec<u8>> {
        let kind = data.get("type").and_then(Value::as_str).unwrap_or_default();
        let index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
        let mut out = Vec::new();
        match kind {
            "message_start" => {
                if let Some(usage) = data.pointer("/message/usage") {
                    self.read_usage(usage);
                }
            }
            "content_block_start" => {
                let block = data.get("content_block").cloned().unwrap_or(Value::Null);
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") => {
                        let id = block
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
                        let name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let wrapped = self.wrapped.contains(name);
                        let shown = self.rename.get(name).map(String::as_str).unwrap_or(name);
                        self.said = true;
                        out.extend(self.sse.tool_call(&id, shown, ""));
                        self.blocks.insert(
                            index,
                            AnthropicBlock::Tool {
                                id,
                                wrapped,
                                buffer: String::new(),
                            },
                        );
                    }
                    Some("text") => {
                        let text = block
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !text.is_empty() {
                            self.said = true;
                        }
                        out.extend(self.sse.text(text));
                        self.blocks.insert(index, AnthropicBlock::Other);
                    }
                    Some("thinking") => {
                        let text = block
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        out.extend(self.sse.reasoning(text));
                        self.blocks.insert(index, AnthropicBlock::Other);
                    }
                    _ => {
                        self.blocks.insert(index, AnthropicBlock::Other);
                    }
                }
            }
            "content_block_delta" => {
                let delta = data.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let text = delta
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !text.is_empty() {
                            self.said = true;
                        }
                        out.extend(self.sse.text(text));
                    }
                    Some("thinking_delta") => {
                        let text = delta
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if !text.is_empty() {
                            self.said = true;
                        }
                        out.extend(self.sse.reasoning(text));
                    }
                    Some("input_json_delta") => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(AnthropicBlock::Tool {
                            id,
                            wrapped,
                            buffer,
                        }) = self.blocks.get_mut(&index)
                        {
                            if *wrapped {
                                buffer.push_str(partial);
                            } else {
                                let id = id.clone();
                                out.extend(self.sse.tool_arguments(&id, partial));
                            }
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(AnthropicBlock::Tool {
                    id,
                    wrapped: true,
                    buffer,
                }) = self.blocks.remove(&index)
                {
                    let parsed: Value = serde_json::from_str(if buffer.trim().is_empty() {
                        "{}"
                    } else {
                        &buffer
                    })
                    .map_err(|_| fail(502, format!("{} 返回的工具参数不是 JSON", self.vendor)))?;
                    let inner = parsed.get("arguments").cloned().unwrap_or(parsed);
                    out.extend(self.sse.tool_arguments(&id, &inner.to_string()));
                }
            }
            "message_delta" => {
                if let Some(reason) = data.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = Some(reason.to_string());
                }
                if let Some(usage) = data.get("usage") {
                    self.read_usage(usage);
                }
            }
            "message_stop" => out.extend(self.finish()),
            "error" => {
                let error = data.get("error").cloned().unwrap_or(Value::Null);
                let error_kind = error
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("上游返回错误");
                return Err(fail(
                    anthropic_error_status(error_kind),
                    format!("{}：{message}", self.vendor),
                ));
            }
            _ => {}
        }
        Ok(out)
    }

    pub fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        if self.sse.is_finished() {
            return Ok(Vec::new());
        }
        if !self.said {
            return Err(fail(502, format!("{} 返回了空回复", self.vendor)));
        }
        Ok(self.finish())
    }
}

impl SseHandler for AnthropicRelay {
    fn event(&mut self, _event: &str, data: &str) -> anyhow::Result<Vec<u8>> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" || self.sse.is_finished() {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|_| fail(502, format!("{} 返回了无法解析的数据", self.vendor)))?;
        AnthropicRelay::event(self, &value)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        AnthropicRelay::end(self)
    }
}

/// OpenAI Responses 的流事件 → [`ChatSse`]。
#[derive(Debug)]
pub struct ResponsesRelay {
    pub sse: ChatSse,
    /// 输出项 id → (call_id, 是否已经收到过参数增量)。
    items: HashMap<String, (String, bool)>,
    said: bool,
    vendor: &'static str,
}

impl ResponsesRelay {
    pub fn new(model: &str, vendor: &'static str) -> Self {
        Self {
            sse: ChatSse::new(model),
            items: HashMap::new(),
            said: false,
            vendor,
        }
    }

    fn usage(response: &Value) -> Option<Usage> {
        let usage = response.get("usage").filter(|value| value.is_object())?;
        let number = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
        Some(Usage {
            prompt_tokens: number("/input_tokens"),
            completion_tokens: number("/output_tokens"),
            cached_tokens: number("/input_tokens_details/cached_tokens"),
            reasoning_tokens: number("/output_tokens_details/reasoning_tokens"),
        })
    }

    pub fn event(&mut self, data: &Value) -> anyhow::Result<Vec<u8>> {
        let kind = data.get("type").and_then(Value::as_str).unwrap_or_default();
        let mut out = Vec::new();
        match kind {
            "response.output_text.delta" | "response.refusal.delta" => {
                let text = data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.said = true;
                }
                out.extend(self.sse.text(text));
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let text = data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !text.is_empty() {
                    self.said = true;
                }
                out.extend(self.sse.reasoning(text));
            }
            "response.output_item.added" => {
                let item = data.get("item").cloned().unwrap_or(Value::Null);
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let call_id = str_at(&item, &["call_id", "id"])
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()));
                    let item_id = str_at(&item, &["id"]).unwrap_or(&call_id).to_string();
                    let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    self.said = true;
                    out.extend(self.sse.tool_call(&call_id, name, arguments));
                    self.items.insert(item_id, (call_id, !arguments.is_empty()));
                }
            }
            "response.function_call_arguments.delta" => {
                let item_id = data
                    .get("item_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let delta = data
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some((call_id, streamed)) = self.items.get_mut(item_id) {
                    *streamed = true;
                    let call_id = call_id.clone();
                    out.extend(self.sse.tool_arguments(&call_id, delta));
                }
            }
            "response.output_item.done" => {
                let item = data.get("item").cloned().unwrap_or(Value::Null);
                let item_id = str_at(&item, &["id", "call_id"])
                    .unwrap_or_default()
                    .to_string();
                if let Some((call_id, false)) = self.items.get(&item_id).cloned() {
                    // 没收到增量：参数一次性给在完成事件里。
                    let arguments = item
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    out.extend(self.sse.tool_arguments(&call_id, arguments));
                    self.items.insert(item_id, (call_id, true));
                }
            }
            "response.completed" | "response.incomplete" => {
                let response = data.get("response").cloned().unwrap_or(Value::Null);
                let reason = (kind == "response.incomplete"
                    && response
                        .pointer("/incomplete_details/reason")
                        .and_then(Value::as_str)
                        == Some("max_output_tokens"))
                .then_some("length");
                out.extend(self.sse.finish(reason, Self::usage(&response)));
            }
            "response.failed" | "error" => {
                let message = data
                    .pointer("/response/error/message")
                    .or_else(|| data.pointer("/error/message"))
                    .or_else(|| data.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("上游返回错误");
                let code = data
                    .pointer("/response/error/code")
                    .or_else(|| data.pointer("/error/code"))
                    .or_else(|| data.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let status = match code {
                    "rate_limit_exceeded" | "insufficient_quota" => 429,
                    "invalid_prompt" | "context_length_exceeded" => 400,
                    _ => 502,
                };
                return Err(fail(status, format!("{}：{message}", self.vendor)));
            }
            _ => {}
        }
        Ok(out)
    }
}

impl SseHandler for ResponsesRelay {
    fn event(&mut self, _event: &str, data: &str) -> anyhow::Result<Vec<u8>> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" || self.sse.is_finished() {
            return Ok(Vec::new());
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|_| fail(502, format!("{} 返回了无法解析的数据", self.vendor)))?;
        ResponsesRelay::event(self, &value)
    }

    fn end(&mut self) -> anyhow::Result<Vec<u8>> {
        if self.sse.is_finished() {
            return Ok(Vec::new());
        }
        if !self.said {
            return Err(fail(502, format!("{} 返回了空回复", self.vendor)));
        }
        Ok(self.sse.finish(None, None))
    }
}

// ---------------------------------------------------------------------------
// Chat 请求里常用的读法
// ---------------------------------------------------------------------------

/// 消息的纯文本（字符串或 text 片段拼接）。
pub fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                _ => part.get("text").and_then(Value::as_str),
            })
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// 请求里的 system / developer 文本（按顺序）。
pub fn system_texts(request: &Value) -> Vec<String> {
    request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            )
        })
        .map(|message| content_text(message.get("content")))
        .filter(|text| !text.trim().is_empty())
        .collect()
}

/// 请求的推理等级（`reasoning_effort` 或 `reasoning.effort`）。
pub fn reasoning_effort(request: &Value) -> Option<String> {
    str_at(request, &["reasoning_effort", "/reasoning/effort"]).map(str::to_ascii_lowercase)
}

/// 请求的输出上限。
pub fn max_tokens(request: &Value) -> Option<u64> {
    u64_at(request, &["max_completion_tokens", "max_tokens"]).filter(|value| *value > 0)
}

/// 上游 JSON 错误体里的错误信息（常见几种位置）。
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| {
            str_at(
                &value,
                &[
                    "/error/message",
                    "message",
                    "msg",
                    "error",
                    "/error/msg",
                    "detail",
                ],
            )
            .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::chat::aggregate;

    #[test]
    fn parses_iso_times() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("1970-01-02T00:00:00.5Z"), Some(86_400_500));
        assert_eq!(
            parse_iso_ms("2026-10-30T08:00:00+08:00"),
            parse_iso_ms("2026-10-30T00:00:00Z")
        );
        assert_eq!(parse_iso_ms("2000-03-01 00:00:00"), Some(951_868_800_000));
        assert_eq!(parse_iso_ms("昨天"), None);
        assert_eq!(to_ms(1_700_000_000), 1_700_000_000_000);
        assert_eq!(to_ms(1_700_000_000_000), 1_700_000_000_000);
    }

    #[test]
    fn crc_and_gzip_are_well_formed() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let data = vec![b'x'; 70_000];
        let gz = gzip_stored(&data);
        assert_eq!(&gz[..3], &[0x1f, 0x8b, 8]);
        // 两个存储块：65535 + 4465。
        assert_eq!(gz[10], 0);
        let second = 10 + 5 + 65_535;
        assert_eq!(gz[second], 1);
        assert_eq!(u16::from_le_bytes([gz[second + 1], gz[second + 2]]), 4465);
        let tail = &gz[gz.len() - 8..];
        assert_eq!(u32::from_le_bytes(tail[4..8].try_into().unwrap()), 70_000);
        assert_eq!(gzip_stored(b"").len(), 10 + 5 + 8);
    }

    #[test]
    fn object_root_flattens_unions() {
        let schema = json!({
            "anyOf": [
                { "type": "object", "properties": { "a": { "type": "string" }, "c": { "type": "string" } }, "required": ["a", "c"] },
                { "type": "object", "properties": { "b": { "type": "number" }, "c": { "type": "number" } }, "required": ["b", "c"] }
            ],
            "description": "x"
        });
        let flat = object_root(&schema);
        assert_eq!(flat["type"], "object");
        assert_eq!(flat["description"], "x");
        assert!(flat["properties"]["a"].is_object());
        assert!(flat["properties"]["b"].is_object());
        assert_eq!(
            flat["properties"]["c"]["anyOf"].as_array().unwrap().len(),
            2
        );
        assert_eq!(flat["required"], json!(["c"]));

        let all = json!({ "allOf": [
            { "properties": { "a": {} }, "required": ["a"] },
            { "$ref": "#/$defs/B" }
        ], "$defs": { "B": { "type": "object", "properties": { "b": {} }, "required": ["b"] } } });
        let flat = object_root(&all);
        assert_eq!(flat["required"], json!(["a", "b"]));
        // 普通 object 不动。
        let plain = json!({ "type": "object", "properties": { "x": {} } });
        assert_eq!(object_root(&plain), plain);
    }

    #[test]
    fn chat_relay_maps_tools_reasoning_and_usage() {
        let mut relay = ChatRelay::new("m");
        relay.rename.insert("read_file".into(), "read.file".into());
        let handler = Sse::new(OpenAiChatSse {
            relay,
            vendor: "测试",
        });
        let chunks = [
            r#"data: {"choices":[{"delta":{"reasoning":"想"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"好"}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"p\""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":":1}"}}]}}]}"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":1}}}"#,
            "data: [DONE]",
        ]
        .map(|line| format!("{line}\n\n"));
        let parts: Vec<&[u8]> = chunks.iter().map(|line| line.as_bytes()).collect();
        let (text, error) = run_decoder(handler, &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["reasoning_content"], "想");
        assert_eq!(message["content"], "好");
        assert_eq!(message["tool_calls"][0]["id"], "c1");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "read.file");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"p\":1}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 5);
        assert_eq!(
            completion["usage"]["prompt_tokens_details"]["cached_tokens"],
            1
        );
    }

    #[test]
    fn chat_relay_reports_empty_and_error() {
        let (_, error) = run_decoder(
            Sse::new(OpenAiChatSse {
                relay: ChatRelay::new("m"),
                vendor: "测试",
            }),
            &[b"data: [DONE]\n\n"],
        );
        // [DONE] 也算正常结束（只是没有内容）。
        assert!(error.is_none());
        let (_, error) = run_decoder(
            Sse::new(OpenAiChatSse {
                relay: ChatRelay::new("m"),
                vendor: "测试",
            }),
            &[b": keep-alive\n\n"],
        );
        assert!(error.unwrap().to_string().contains("空回复"));
        let (_, error) = run_decoder(
            Sse::new(OpenAiChatSse {
                relay: ChatRelay::new("m"),
                vendor: "测试",
            }),
            &[b"data: {\"error\":{\"message\":\"quota\",\"code\":429}}\n\n"],
        );
        let error = error.unwrap();
        assert_eq!(
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status,
            429
        );
    }

    fn anthropic_stream() -> Vec<String> {
        [
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":4}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"嗯"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"看看"}}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"edit","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"arguments\":{\"a\""}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":":1}}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"content_block_start","index":3,"content_block":{"type":"tool_use","id":"toolu_2","name":"ls","input":{}}}),
            json!({"type":"content_block_delta","index":3,"delta":{"type":"input_json_delta","partial_json":"{\"d\":\".\"}"}}),
            json!({"type":"content_block_stop","index":3}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
            json!({"type":"message_stop"}),
        ]
        .iter()
        .map(|event| format!("event: x\ndata: {event}\n\n"))
        .collect()
    }

    #[test]
    fn anthropic_relay_converts_and_unwraps() {
        let mut relay = AnthropicRelay::new("claude", "测试");
        relay.wrapped.insert("edit".into());
        let events = anthropic_stream();
        let parts: Vec<&[u8]> = events.iter().map(|line| line.as_bytes()).collect();
        let (text, error) = run_decoder(Sse::new(relay), &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "claude").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["reasoning_content"], "嗯");
        assert_eq!(message["content"], "看看");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "edit");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
        assert_eq!(
            message["tool_calls"][1]["function"]["arguments"],
            "{\"d\":\".\"}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 14);
        assert_eq!(completion["usage"]["completion_tokens"], 7);
    }

    #[test]
    fn anthropic_relay_maps_errors_and_length() {
        let error_event = "data: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow\"}}\n\n";
        let (_, error) = run_decoder(
            Sse::new(AnthropicRelay::new("c", "测试")),
            &[error_event.as_bytes()],
        );
        let error = error.unwrap();
        assert_eq!(
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status,
            429
        );
        let events = [
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"a"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":1}}),
        ]
        .map(|event| format!("data: {event}\n\n"));
        let parts: Vec<&[u8]> = events.iter().map(|line| line.as_bytes()).collect();
        // 没有 message_stop 也在流结束时收尾。
        let (text, error) = run_decoder(Sse::new(AnthropicRelay::new("c", "测试")), &parts);
        assert!(error.is_none());
        let completion = aggregate(&text, "c").unwrap();
        assert_eq!(completion["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn responses_relay_converts() {
        let events = [
            json!({"type":"response.reasoning_summary_text.delta","delta":"想"}),
            json!({"type":"response.output_text.delta","delta":"好"}),
            json!({"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"run","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"x\":"}),
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"2}"}),
            json!({"type":"response.output_item.added","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"ls","arguments":""}}),
            json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"ls","arguments":"{}"}}),
            json!({"type":"response.completed","response":{"usage":{"input_tokens":3,"output_tokens":4,"output_tokens_details":{"reasoning_tokens":1}}}}),
        ]
        .map(|event| format!("data: {event}\n\n"));
        let parts: Vec<&[u8]> = events.iter().map(|line| line.as_bytes()).collect();
        let (text, error) = run_decoder(Sse::new(ResponsesRelay::new("gpt", "测试")), &parts);
        assert!(error.is_none(), "{error:?}");
        let completion = aggregate(&text, "gpt").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["reasoning_content"], "想");
        assert_eq!(message["content"], "好");
        assert_eq!(message["tool_calls"][0]["id"], "call_1");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"x\":2}"
        );
        assert_eq!(message["tool_calls"][1]["function"]["arguments"], "{}");
        assert_eq!(
            completion["usage"]["completion_tokens_details"]["reasoning_tokens"],
            1
        );
        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"rate_limit_exceeded\",\"message\":\"slow\"}}}\n\n";
        let (_, error) = run_decoder(
            Sse::new(ResponsesRelay::new("gpt", "测试")),
            &[failed.as_bytes()],
        );
        assert!(error.unwrap().to_string().contains("slow"));
    }

    #[test]
    fn listeners_park_and_take() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let key = park_listener(listener);
            assert!(take_listener(&key).is_some());
            assert!(take_listener(&key).is_none());
        });
    }

    #[test]
    fn helpers_read_requests() {
        let request = json!({
            "reasoning_effort": "HIGH",
            "max_completion_tokens": 100,
            "messages": [
                { "role": "system", "content": "a" },
                { "role": "developer", "content": [{ "type": "text", "text": "b" }] },
                { "role": "user", "content": "c" }
            ]
        });
        assert_eq!(system_texts(&request), vec!["a", "b"]);
        assert_eq!(reasoning_effort(&request).as_deref(), Some("high"));
        assert_eq!(max_tokens(&request), Some(100));
        assert_eq!(uuid_from_seed("x"), uuid_from_seed("x"));
        assert_eq!(uuid_from_seed("x").len(), 36);
        assert_eq!(random_hex(16).len(), 32);
        assert_eq!(error_message("{\"error\":{\"message\":\"m\"}}"), "m");
    }
}
