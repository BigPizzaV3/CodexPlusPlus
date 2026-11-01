//! 编辑器与 IDE 订阅（Cursor、Kiro、Zed、Command Code）共用的小工具：
//!
//! - 随机字节、base64url、SHA-256 十六进制；
//! - 跨 `begin_login` / `finish_login` 暂存登录现场（本机监听、私钥等不能放进界面传回的 JSON）；
//! - 极简的本机 HTTP 回调服务（有的登录回调是 POST 表单、没有 code/state，框架的回调等不了）；
//! - 后台任务产出 Chat SSE 的流（第一块之前出错时把带状态码的错误交给适配入口）；
//! - Anthropic Messages / OpenAI Chat 流事件 → [`ChatSse`] 的转换，Chat 请求 → Anthropic 请求。

use std::any::Any;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::gateway::subscriptions::chat::{ChatSse, Usage, message_text};
use crate::gateway::subscriptions::{ChatStream, fail};

/* —————————————— 杂项 —————————————— */

/// 用户主目录。
pub(crate) fn home() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 随机字节：每 32 字节取三个 v4 UUID（各 122 位系统随机数）做 SHA-256。
pub(crate) fn random_bytes(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter = 0_u64;
    while out.len() < len {
        let mut hasher = Sha256::new();
        hasher.update(counter.to_le_bytes());
        for _ in 0..3 {
            hasher.update(uuid::Uuid::new_v4().as_bytes());
        }
        out.extend_from_slice(&hasher.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

/// base64url，不带填充。
pub(crate) fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// PKCE：(verifier, S256 challenge)，verifier 是 `bytes` 个随机字节的 base64url。
pub(crate) fn pkce(bytes: usize) -> (String, String) {
    let verifier = b64url(&random_bytes(bytes));
    let challenge = b64url(&sha256(verifier.as_bytes()));
    (verifier, challenge)
}

/// JSON 里的字符串字段（去首尾空白，空串当没有）。
pub(crate) fn text_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// 凭据里的毫秒时间戳。
pub(crate) fn millis_of(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/* —————————————— 登录现场暂存 —————————————— */

type Parked = (String, Box<dyn Any + Send>);

fn parked() -> &'static Mutex<HashMap<String, Parked>> {
    static PARKED: OnceLock<Mutex<HashMap<String, Parked>>> = OnceLock::new();
    PARKED.get_or_init(Default::default)
}

/// 暂存一家订阅的登录现场，返回取回用的 id。同一家只留最新一次（旧的监听随之关闭）。
pub(crate) fn park<T: Send + 'static>(vendor: &str, value: T) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    parked()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(vendor.to_string(), (id.clone(), Box::new(value)));
    id
}

/// 取回 [`park`] 暂存的现场；id 不对（已被新的登录顶掉）或类型不对时为 None。
pub(crate) fn unpark<T: 'static>(vendor: &str, id: &str) -> Option<T> {
    let mut map = parked()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if map.get(vendor).map(|(parked_id, _)| parked_id.as_str()) != Some(id) {
        return None;
    }
    let (_, value) = map.remove(vendor)?;
    value.downcast::<T>().ok().map(|value| *value)
}

/* —————————————— 本机回调服务 —————————————— */

/// 浏览器发到本机回调端口的一个请求。
#[derive(Debug, Default, Clone)]
pub(crate) struct LocalRequest {
    pub method: String,
    pub path: String,
    pub query: BTreeMap<String, String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl LocalRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// 请求体里的字段：表单（`a=1&b=2`）或 JSON 对象（只取字符串值）。
    pub fn form(&self) -> BTreeMap<String, String> {
        let text = String::from_utf8_lossy(&self.body);
        let json_body = self
            .header("content-type")
            .is_some_and(|value| value.to_ascii_lowercase().contains("json"))
            || text.trim_start().starts_with('{');
        if json_body {
            return serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|value| value.as_object().cloned())
                .map(|object| {
                    object
                        .into_iter()
                        .filter_map(|(key, value)| match value {
                            Value::String(text) => Some((key, text)),
                            Value::Number(number) => Some((key, number.to_string())),
                            Value::Bool(flag) => Some((key, flag.to_string())),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
        }
        parse_query(&text)
    }
}

/// `%XX` 解码（`+` 当空格）。
pub(crate) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let digit = |byte: u8| (byte as char).to_digit(16);
                match (digit(bytes[index + 1]), digit(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        out.push((high * 16 + low) as u8);
                        index += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 查询串 → 键值（重复的键取第一个）。
pub(crate) fn parse_query(query: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.entry(percent_decode(key))
            .or_insert_with(|| percent_decode(value));
    }
    out
}

/// 解析一段完整的 HTTP 请求头（到空行为止）。
fn parse_head(head: &str) -> Option<LocalRequest> {
    let mut lines = head.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();
    Some(LocalRequest {
        method,
        path: path.to_string(),
        query: parse_query(query),
        headers,
        body: Vec::new(),
    })
}

/// 等下一个请求：读完请求头和 `content-length` 的请求体。浏览器预连接却不发请求的连接丢掉。
pub(crate) async fn next_request(
    listener: &tokio::net::TcpListener,
) -> anyhow::Result<(tokio::net::TcpStream, LocalRequest)> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let read = tokio::time::timeout(Duration::from_secs(10), async {
            let mut buffer = Vec::new();
            let mut chunk = [0_u8; 8192];
            loop {
                let count = stream.read(&mut chunk).await?;
                if count == 0 {
                    return anyhow::Ok(None);
                }
                buffer.extend_from_slice(&chunk[..count]);
                let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
                    if buffer.len() > 64 * 1024 {
                        return Ok(None);
                    }
                    continue;
                };
                let head = String::from_utf8_lossy(&buffer[..end]).to_string();
                let Some(mut request) = parse_head(&head) else {
                    return Ok(None);
                };
                let length = request
                    .header("content-length")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0)
                    .min(1024 * 1024);
                let mut body = buffer[end + 4..].to_vec();
                while body.len() < length {
                    let count = stream.read(&mut chunk).await?;
                    if count == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..count]);
                }
                body.truncate(length);
                request.body = body;
                return Ok(Some(request));
            }
        })
        .await;
        if let Ok(Ok(Some(request))) = read {
            return Ok((stream, request));
        }
    }
}

/// 回一个完整答复并关闭连接。
pub(crate) async fn respond(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    headers: &[(&str, &str)],
    content_type: &str,
    body: &[u8],
) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\ncache-control: no-store\r\n",
        body.len()
    );
    if !content_type.is_empty() {
        head.push_str(&format!("content-type: {content_type}\r\n"));
    }
    for (key, value) in headers {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}

/// 回给浏览器的一句话页面。
pub(crate) fn page(text: &str) -> Vec<u8> {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Codex++</title><body style=\"font-family:system-ui;padding:48px;text-align:center\"><h2>{text}</h2></body>"
    )
    .into_bytes()
}

/// 在 `ports` 里找第一个能监听的本机端口（`0` 表示让系统分配）。
pub(crate) async fn listen_first(ports: &[u16]) -> anyhow::Result<tokio::net::TcpListener> {
    for port in ports {
        if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", *port)).await {
            return Ok(listener);
        }
    }
    bail!("本机登录回调端口都被占用了，请关掉官方客户端正在进行的登录后再试")
}

/// 读本机命令的标准输出（导入钥匙串里的登录等）；失败时为 None。
pub(crate) fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/* —————————————— 流 —————————————— */

/// 后台任务往 Chat SSE 流里写东西。还没写出任何内容时出错，错误（带状态码）原样交给适配入口，
/// 由它回真实的 HTTP 状态；写出内容之后出错，改发一个 SSE 错误事件。
pub(crate) struct Emitter {
    tx: mpsc::Sender<anyhow::Result<Vec<u8>>>,
    pub sse: ChatSse,
    started: bool,
}

impl Emitter {
    /// 写一块；接收方已经走了（客户端断开）时返回 false。
    pub async fn send(&mut self, bytes: Vec<u8>) -> bool {
        if bytes.is_empty() {
            return !self.tx.is_closed();
        }
        self.started = true;
        self.tx.send(Ok(bytes)).await.is_ok()
    }

    pub fn started(&self) -> bool {
        self.started
    }

    /// 失败收尾。
    pub async fn fail(&mut self, error: anyhow::Error) {
        if self.sse.is_finished() {
            return;
        }
        if self.started {
            let bytes = self.sse.error(&format!("{error:#}"));
            let _ = self.tx.send(Ok(bytes)).await;
        } else {
            let _ = self.tx.send(Err(error)).await;
        }
    }
}

/// 起一个后台任务产出 Chat SSE；任务拿到的 [`Emitter`] 已经按 `model` 建好 [`ChatSse`]。
pub(crate) fn spawn_chat<F, Fut>(model: &str, work: F) -> ChatStream
where
    F: FnOnce(Emitter) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let (tx, rx) = mpsc::channel(64);
    let emitter = Emitter {
        tx,
        sse: ChatSse::new(model),
        started: false,
    };
    tokio::spawn(work(emitter));
    Box::pin(futures_util::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    }))
}

/// 按行切字节流（NDJSON）。
#[derive(Debug, Default)]
pub(crate) struct LineReader {
    buffer: Vec<u8>,
}

impl LineReader {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if !line.is_empty() {
                out.push(line);
            }
        }
        out
    }

    pub fn finish(&mut self) -> Option<String> {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.buffer))
            .trim()
            .to_string();
        (!rest.is_empty()).then_some(rest)
    }
}

/* —————————————— Chat 请求的小工具 —————————————— */

/// Chat 请求里的 `reasoning_effort`（也认 `reasoning.effort`）；`none` / `off` 原样返回。
pub(crate) fn reasoning_effort(request: &Value) -> Option<String> {
    request
        .get("reasoning_effort")
        .or_else(|| request.pointer("/reasoning/effort"))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
}

/// 工具调用参数（字符串或对象）→ JSON 对象；解析不了时为空对象。
pub(crate) fn arguments_object(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::Object(_)) => arguments.cloned().unwrap_or_else(|| json!({})),
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({})),
        _ => json!({}),
    }
}

/// 消息的文本（字符串或 text 片段拼起来）。
pub(crate) fn content_text(message: &Value) -> String {
    message_text(message)
}

/// assistant 消息里回放的推理文本。
pub(crate) fn reasoning_text(message: &Value) -> String {
    ["reasoning_content", "reasoning"]
        .iter()
        .filter_map(|key| message.get(*key).and_then(Value::as_str))
        .find(|text| !text.trim().is_empty())
        .unwrap_or_default()
        .to_string()
}

/// tool 消息的 tool_call_id → 名字（从前面 assistant 的 tool_calls 查）。
pub(crate) fn tool_names(messages: &[Value]) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for message in messages {
        for call in message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let (Some(id), Some(name)) = (
                call.get("id").and_then(Value::as_str),
                call.pointer("/function/name").and_then(Value::as_str),
            ) {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }
    names
}

/// 生成一个工具调用 id（`call_` + 24 位十六进制）。
pub(crate) fn new_call_id() -> String {
    format!("call_{}", hex(&random_bytes(12)))
}

/* —————————————— Chat → Anthropic Messages —————————————— */

/// Chat 请求 → Anthropic Messages 请求体：复用网关已有的转换，再补订阅上游要的几处：
/// 每个 `tool_result` 带 `is_error`；`reasoning_effort` 换成思考预算（工具循环中途不开，
/// 否则上游要求回放带签名的思考块）。`stream` 按 `stream` 参数设置或删掉。
pub(crate) fn anthropic_request(request: &Value, stream: bool) -> anyhow::Result<Value> {
    let options = crate::anthropic_bridge::AnthropicRequestOptions {
        max_tokens: 32_000,
        prompt_cache: false,
        ..Default::default()
    };
    let mut body =
        crate::anthropic_bridge::chat_completions_to_anthropic_messages(request.clone(), &options)?;
    for message in body
        .get_mut("messages")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        for block in message
            .get_mut("content")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if block.get("type").and_then(Value::as_str) == Some("tool_result")
                && block.get("is_error").is_none()
            {
                block["is_error"] = json!(false);
            }
        }
    }
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let budget = reasoning_effort(request)
        .and_then(|effort| crate::anthropic_bridge::effort_to_thinking_budget(&effort));
    if let Some(budget) = budget.filter(|_| !in_tool_loop(&messages)) {
        body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        let max_tokens = body.get("max_tokens").and_then(Value::as_u64).unwrap_or(0);
        if max_tokens <= budget {
            body["max_tokens"] = json!(budget + 8192);
        }
        // 开思考时上游只认默认温度。
        if let Some(object) = body.as_object_mut() {
            object.remove("temperature");
            object.remove("top_p");
        }
    }
    if let Some(object) = body.as_object_mut() {
        if stream {
            object.insert("stream".to_string(), json!(true));
        } else {
            object.remove("stream");
        }
    }
    Ok(body)
}

/// 最后一条用户消息之后已经有带工具调用的 assistant（正在跑工具循环）。
fn in_tool_loop(messages: &[Value]) -> bool {
    messages
        .iter()
        .rev()
        .take_while(|message| message.get("role").and_then(Value::as_str) != Some("user"))
        .any(|message| {
            message
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty())
        })
}

/// Anthropic 的错误类型 → 状态码。
pub(crate) fn anthropic_error_status(kind: &str) -> u16 {
    match kind {
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "rate_limit_error" => 429,
        "overloaded_error" => 529,
        "invalid_request_error" | "request_too_large" => 400,
        "billing_error" => 402,
        _ => 502,
    }
}

/// Anthropic Messages 流事件 → Chat SSE。
#[derive(Debug, Default)]
pub(crate) struct AnthropicEvents {
    /// 内容块序号 → 工具调用 id。
    tools: HashMap<u64, String>,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    stop: Option<&'static str>,
    pub ended: bool,
}

impl AnthropicEvents {
    /// 处理一个事件（`data` 的 JSON）；上游报错时返回带状态码的错误。
    pub fn push(&mut self, sse: &mut ChatSse, event: &Value) -> anyhow::Result<Vec<u8>> {
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "message_start" => {
                if let Some(usage) = event.pointer("/message/usage") {
                    self.read_usage(usage);
                }
                Ok(Vec::new())
            }
            "content_block_start" => {
                let block = event.get("content_block").cloned().unwrap_or(Value::Null);
                match block.get("type").and_then(Value::as_str) {
                    Some("tool_use") | Some("server_tool_use") => {
                        let id = text_of(&block, "id")
                            .map(str::to_string)
                            .unwrap_or_else(new_call_id);
                        let name = text_of(&block, "name").unwrap_or_default().to_string();
                        self.tools.insert(index, id.clone());
                        // 有的上游把完整参数放在 start 里。
                        let arguments = block
                            .get("input")
                            .filter(|input| {
                                input.as_object().is_some_and(|object| !object.is_empty())
                            })
                            .map(Value::to_string)
                            .unwrap_or_default();
                        Ok(sse.tool_call(&id, &name, &arguments))
                    }
                    Some("text") => Ok(sse.text(text_of(&block, "text").unwrap_or_default())),
                    Some("thinking") => {
                        Ok(sse.reasoning(text_of(&block, "thinking").unwrap_or_default()))
                    }
                    _ => Ok(Vec::new()),
                }
            }
            "content_block_delta" => {
                let delta = event.get("delta").cloned().unwrap_or(Value::Null);
                let text = |key: &str| {
                    delta
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string()
                };
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => Ok(sse.text(&text("text"))),
                    Some("thinking_delta") => Ok(sse.reasoning(&text("thinking"))),
                    Some("input_json_delta") => match self.tools.get(&index) {
                        Some(id) => Ok(sse.tool_arguments(&id.clone(), &text("partial_json"))),
                        None => Ok(Vec::new()),
                    },
                    _ => Ok(Vec::new()),
                }
            }
            "message_delta" => {
                if let Some(usage) = event.get("usage") {
                    self.read_usage(usage);
                }
                if let Some(reason) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop = Some(match reason {
                        "max_tokens" | "model_context_window_exceeded" => "length",
                        "tool_use" => "tool_calls",
                        "refusal" => "content_filter",
                        _ => "stop",
                    });
                }
                Ok(Vec::new())
            }
            "message_stop" => Ok(self.finish(sse)),
            "error" => {
                let kind = event
                    .pointer("/error/type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let message = event
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("上游返回错误");
                Err(fail(anthropic_error_status(kind), message.to_string()))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn read_usage(&mut self, usage: &Value) {
        let get = |key: &str| usage.get(key).and_then(Value::as_u64);
        if let Some(value) = get("input_tokens") {
            self.input = value;
        }
        if let Some(value) = get("cache_read_input_tokens") {
            self.cache_read = value;
        }
        if let Some(value) = get("cache_creation_input_tokens") {
            self.cache_write = value;
        }
        if let Some(value) = get("output_tokens") {
            self.output = value;
        }
    }

    pub fn usage(&self) -> Usage {
        Usage {
            prompt_tokens: self.input + self.cache_read + self.cache_write,
            completion_tokens: self.output,
            cached_tokens: self.cache_read,
            reasoning_tokens: 0,
        }
    }

    /// 收尾（`message_stop`，或上游没发它就断了但已经有内容）。
    pub fn finish(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        self.ended = true;
        let stop = match self.stop {
            Some("stop") if sse.has_tool_calls() => Some("tool_calls"),
            other => other,
        };
        sse.finish(stop, Some(self.usage()))
    }
}

/* —————————————— OpenAI Chat 流 → Chat SSE —————————————— */

/// 上游本身是 Chat Completions 流（但要重新编号、统一收尾）时用：chunk → [`ChatSse`]。
#[derive(Debug, Default)]
pub(crate) struct ChatChunks {
    /// 工具调用序号 → id。
    tools: HashMap<u64, String>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<String>,
}

impl ChatChunks {
    pub fn push(&mut self, sse: &mut ChatSse, chunk: &Value) -> anyhow::Result<Vec<u8>> {
        if let Some(error) = chunk.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            let status = error
                .get("code")
                .and_then(Value::as_u64)
                .filter(|code| (400..600).contains(code))
                .unwrap_or(502) as u16;
            return Err(fail(status, message));
        }
        if let Some(usage) = chunk.get("usage").filter(|usage| usage.is_object()) {
            let get = |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
            self.usage = Some(Usage {
                prompt_tokens: get("/prompt_tokens"),
                completion_tokens: get("/completion_tokens"),
                cached_tokens: get("/prompt_tokens_details/cached_tokens"),
                reasoning_tokens: get("/completion_tokens_details/reasoning_tokens"),
            });
        }
        let mut out = Vec::new();
        let Some(choice) = chunk.get("choices").and_then(|choices| choices.get(0)) else {
            return Ok(out);
        };
        let delta = choice.get("delta").cloned().unwrap_or(Value::Null);
        for key in ["reasoning_content", "reasoning"] {
            if let Some(text) = delta.get(key).and_then(Value::as_str) {
                out.extend(sse.reasoning(text));
                break;
            }
        }
        if let Some(text) = delta.get("content").and_then(Value::as_str) {
            out.extend(sse.text(text));
        }
        for call in delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match self.tools.get(&index).cloned() {
                Some(id) => out.extend(sse.tool_arguments(&id, arguments)),
                None => {
                    let id = text_of(call, "id")
                        .map(str::to_string)
                        .unwrap_or_else(new_call_id);
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    self.tools.insert(index, id.clone());
                    out.extend(sse.tool_call(&id, name, arguments));
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        Ok(out)
    }

    pub fn finish(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        let reason = self.finish_reason.clone();
        sse.finish(reason.as_deref(), self.usage)
    }
}

/// 读本机 JSON 文件（不存在时报清楚的错误）。
pub(crate) fn read_json_file(path: &std::path::Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("没找到 {}，请先在官方客户端里登录", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} 不是合法的 JSON", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::chat::aggregate;

    #[test]
    fn query_and_form_parse() {
        let query = parse_query("a=1&b=x%2Fy+z&a=2&c");
        assert_eq!(query["a"], "1");
        assert_eq!(query["b"], "x/y z");
        assert_eq!(query["c"], "");
        let request = LocalRequest {
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: br#"{"apiKey":"k","n":3,"x":{}}"#.to_vec(),
            ..LocalRequest::default()
        };
        let form = request.form();
        assert_eq!(form["apiKey"], "k");
        assert_eq!(form["n"], "3");
        assert!(!form.contains_key("x"));
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%E4%BD%A0"), "你");
    }

    #[test]
    fn head_parses_method_path_and_headers() {
        let request =
            parse_head("POST /callback?state=s HTTP/1.1\r\nHost: x\r\nContent-Length: 3").unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/callback");
        assert_eq!(request.query["state"], "s");
        assert_eq!(request.header("content-length"), Some("3"));
    }

    #[test]
    fn park_keeps_latest_per_vendor() {
        let first = park("test-vendor", 1_u32);
        let second = park("test-vendor", 2_u32);
        assert_eq!(unpark::<u32>("test-vendor", &first), None);
        assert_eq!(unpark::<u32>("test-vendor", &second), Some(2));
        assert_eq!(unpark::<u32>("test-vendor", &second), None);
    }

    #[test]
    fn random_bytes_differ() {
        let a = random_bytes(48);
        let b = random_bytes(48);
        assert_eq!(a.len(), 48);
        assert_ne!(a, b);
        let (verifier, challenge) = pkce(32);
        assert_eq!(challenge, b64url(&sha256(verifier.as_bytes())));
    }

    #[test]
    fn anthropic_request_adds_is_error_and_thinking() {
        let request = json!({
            "model": "claude-x",
            "stream": true,
            "reasoning_effort": "high",
            "temperature": 0.5,
            "messages": [
                { "role": "system", "content": "be brief" },
                { "role": "user", "content": "hi" },
            ],
        });
        let body = anthropic_request(&request, false).unwrap();
        assert!(body.get("stream").is_none());
        assert_eq!(body["thinking"]["type"], "enabled");
        assert!(
            body["max_tokens"].as_u64().unwrap()
                > body["thinking"]["budget_tokens"].as_u64().unwrap()
        );
        assert!(body.get("temperature").is_none());
        assert_eq!(body["system"], "be brief");

        let looping = json!({
            "model": "claude-x",
            "reasoning_effort": "high",
            "messages": [
                { "role": "user", "content": "read a" },
                { "role": "assistant", "content": "", "tool_calls": [{ "id": "t1", "type": "function", "function": { "name": "read", "arguments": "{\"p\":\"a\"}" } }] },
                { "role": "tool", "tool_call_id": "t1", "content": "A" },
            ],
            "tools": [{ "type": "function", "function": { "name": "read", "parameters": { "type": "object" } } }],
        });
        let body = anthropic_request(&looping, true).unwrap();
        assert_eq!(body["stream"], true);
        assert!(body.get("thinking").is_none(), "工具循环中途不开思考");
        let text = body.to_string();
        assert!(text.contains("\"is_error\":false"), "{text}");
        assert!(text.contains("\"tool_use_id\":\"t1\""));
    }

    #[test]
    fn anthropic_events_become_chat_sse() {
        let events = [
            json!({"type":"message_start","message":{"usage":{"input_tokens":5,"cache_read_input_tokens":2,"output_tokens":0}}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"嗯"}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"hi"}}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"p\":"}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"1}"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}),
            json!({"type":"message_stop"}),
        ];
        let mut sse = ChatSse::new("m");
        let mut state = AnthropicEvents::default();
        let mut out = Vec::new();
        for event in &events {
            out.extend(state.push(&mut sse, event).unwrap());
        }
        assert!(state.ended);
        let completion = aggregate(&String::from_utf8(out).unwrap(), "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "hi");
        assert_eq!(message["reasoning_content"], "嗯");
        assert_eq!(message["tool_calls"][0]["id"], "toolu_1");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"p\":1}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 7);
        assert_eq!(completion["usage"]["completion_tokens"], 7);

        let mut sse = ChatSse::new("m");
        let error = AnthropicEvents::default()
            .push(
                &mut sse,
                &json!({"type":"error","error":{"type":"overloaded_error","message":"busy"}}),
            )
            .unwrap_err();
        let error = error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap();
        assert_eq!(error.status, 529);
    }

    #[test]
    fn chat_chunks_are_renumbered() {
        let chunks = [
            json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Hel"}}]}),
            json!({"choices":[{"index":0,"delta":{"content":"lo","reasoning_content":"r"}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"f","arguments":"{\"a\""}}]}}]}),
            json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":":1}"}}]}}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":5,"completion_tokens":3}}),
        ];
        let mut sse = ChatSse::new("m");
        let mut state = ChatChunks::default();
        let mut out = Vec::new();
        for chunk in &chunks {
            out.extend(state.push(&mut sse, chunk).unwrap());
        }
        out.extend(state.finish(&mut sse));
        let completion = aggregate(&String::from_utf8(out).unwrap(), "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "Hello");
        assert_eq!(message["reasoning_content"], "r");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"a\":1}"
        );
        assert_eq!(completion["usage"]["total_tokens"], 8);
    }

    #[test]
    fn line_reader_splits() {
        let mut reader = LineReader::default();
        assert_eq!(
            reader.push(b"{\"a\":1}\n{\"b\""),
            vec!["{\"a\":1}".to_string()]
        );
        assert_eq!(reader.push(b":2}\n\n"), vec!["{\"b\":2}".to_string()]);
        assert_eq!(reader.push(b"tail"), Vec::<String>::new());
        assert_eq!(reader.finish(), Some("tail".to_string()));
    }

    #[test]
    fn arguments_parse() {
        assert_eq!(arguments_object(Some(&json!("{\"a\":1}"))), json!({"a":1}));
        assert_eq!(arguments_object(Some(&json!("[1]"))), json!({}));
        assert_eq!(arguments_object(Some(&json!({"b":2}))), json!({"b":2}));
        assert_eq!(arguments_object(None), json!({}));
    }
}
