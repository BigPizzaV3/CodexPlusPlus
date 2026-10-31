//! 供应商池的「测试」：给一家供应商发一条很短的请求，看它能不能答、多快答。
//!
//! 走网关真实请求的同一条路：账号类供应商先换 token（[`accounts::prepare_relay`]）、
//! OpenCode 按模型换协议和地址，再交给候选循环用的 [`pipeline::attempt_once`]——
//! ChatGPT 后端的请求整理、Responses → Chat / Anthropic 的协议转换、供应商自己的代理（含直连）
//! 和认证都与真实请求一致，这里不另拼一套协议。
//!
//! 和真实请求不同的地方：不占限速名额（RPM / 并发）、不记 Key 冷却 / 停用 / 熔断、不记测速、
//! 不改会话固定、不写路由动态、不记额度；也不经过网关的 HTTP 入口，
//! 所以请求日志和用量统计里都没有这次测试。

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::gateway::accounts;
use crate::gateway::apply;
use crate::gateway::key_pool::{self, FailureClass, KeyLimits, KeyRef};
use crate::gateway::opencode;
use crate::gateway::pipeline::{self, GatewayRequest};
use crate::gateway::route::{self, GatewayCandidate};
use crate::gateway::store::GatewaySettings;
use crate::protocol_proxy::{ProxySessionHeaders, UpstreamProxyResponse, UpstreamWireApi};
use crate::settings::{BackendSettings, RelayProfile, RelayProtocol};

/// 一次测试最多等多久（含换 token、等响应头和读完回复）。
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(60);
/// 让模型只回一个词：输出很短，花不了几个 token。
const PROBE_PROMPT: &str = "Reply with the single word: OK";
/// 输出上限。Responses 接口的下限是 16；推理模型会先花掉一些在思考上，留一点余量。
const PROBE_MAX_OUTPUT_TOKENS: u64 = 32;
/// 回复、错误原文给界面看的长度上限（字符）。
const REPLY_LIMIT: usize = 200;
const ERROR_LIMIT: usize = 600;
/// 最多读多少响应字节：测试的回复本来就很短，读到这么多还没完就不读了。
const BODY_LIMIT: usize = 512 * 1024;

/// 一次测试的结果。Key 原文不在里面，只有标识、名称和末 4 位。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub ok: bool,
    /// 失败的大类，界面按它给排查提示；成功时为空。
    /// `auth` / `not_found` / `rate_limited` / `quota` / `timeout` / `connect` / `server` /
    /// `client` / `stream` / `other`。
    pub category: String,
    /// 上游的 HTTP 状态码；请求没发出去或没收到响应时为空。
    pub http_status: Option<u16>,
    /// 发给上游的模型名。
    pub model: String,
    /// 实际和上游说的协议：`responses` / `chat` / `anthropic`。
    pub wire_api: String,
    /// 从开始测试到收到第一段输出（文字或思考）的毫秒数；没收到输出时为空。
    pub first_token_ms: Option<u64>,
    /// 从开始测试到读完回复（或失败、超时）的毫秒数。
    pub total_ms: u64,
    /// 回复的文字（截断）。
    pub reply: String,
    pub reply_truncated: bool,
    /// 失败时上游的错误原文或本地错误（截断）。
    pub error: String,
    /// 本次用的 Key 的标识（同供应商详情里 Key 列表的标识）；不用 Key 的供应商为空。
    pub key_id: String,
    /// 本次用的 Key 的名称（有多个 Key 时才有）。
    pub key_name: String,
    /// 本次用的 Key 的末 4 位（Key 太短时不给）。
    pub key_tail: String,
}

/// 测试池里的一家供应商：`model` 为空时用它的默认模型；`key_id` 给了就只用这个 Key。
///
/// `Err` 只表示没法测（供应商或 Key 不在了、没有模型）；上游答不上来、超时等都在结果里。
pub async fn probe_provider(
    settings: &BackendSettings,
    gateway: &GatewaySettings,
    relay_id: &str,
    model: &str,
    key_id: Option<&str>,
) -> anyhow::Result<ProbeResult> {
    probe_provider_with_timeout(settings, gateway, relay_id, model, key_id, PROBE_TIMEOUT).await
}

/// 同 [`probe_provider`]，超时由调用方给。
pub async fn probe_provider_with_timeout(
    settings: &BackendSettings,
    gateway: &GatewaySettings,
    relay_id: &str,
    model: &str,
    key_id: Option<&str>,
    timeout: Duration,
) -> anyhow::Result<ProbeResult> {
    let provider = gateway
        .provider(relay_id)
        .with_context(|| format!("供应商不存在：{relay_id}"))?;
    let mut relay = provider.relay.clone();
    let model = pick_model(&relay, model)?;
    let used_key = select_key(&mut relay, key_id)?;
    let mut result = ProbeResult {
        model: model.clone(),
        wire_api: protocol_label(&relay).to_string(),
        ..ProbeResult::default()
    };
    if !used_key.is_empty() {
        result.key_id = used_key;
        if let Some((name, tail)) = crate::relay_config::relay_test_key_identity(&relay) {
            result.key_name = name.unwrap_or_default();
            result.key_tail = tail;
        }
    }
    let started = Instant::now();
    let finished =
        tokio::time::timeout(timeout, run(settings, &relay, &model, started, &mut result)).await;
    if finished.is_err() {
        result.ok = false;
        result.category = "timeout".to_string();
        result.error = format!("{} 秒内没有完成，已放弃这次测试", timeout.as_secs());
    }
    result.total_ms = elapsed_ms(started);
    Ok(result)
}

/// 测哪个模型：没指定时用默认模型（模型列表的第一个能聊天的）。
fn pick_model(relay: &RelayProfile, model: &str) -> anyhow::Result<String> {
    let model = model.trim();
    if !model.is_empty() {
        return Ok(model.to_string());
    }
    match route::chat_models(relay).into_iter().next() {
        Some(model) => Ok(model),
        None => bail!("这个供应商还没有模型，先在「模型」一行添加"),
    }
}

/// 把 `relay` 换成这次要用的 Key，返回它的标识（不用 Key 的供应商返回空串）。
///
/// 指定了标识就只用那个 Key；没指定时和网关排第一的一样：当前选中的 Key
/// （不看冷却、不推进轮询，测试本身就是为了看它现在能不能用）。
fn select_key(relay: &mut RelayProfile, key_id: Option<&str>) -> anyhow::Result<String> {
    let keys = apply::provider_keys(relay);
    if let Some(id) = key_id.map(str::trim).filter(|id| !id.is_empty()) {
        if keys.is_empty() {
            bail!("这个供应商不用 API Key，不能按 Key 测试");
        }
        if !keys.iter().any(|key| key.id == id) {
            bail!("这个 Key 已经不在了，刷新后再试");
        }
        // 旧数据只有 `api_key`、没有命名 Key 时，唯一的那个 Key（标识 default）就是它本身。
        if let Some(entry) = relay.api_keys.iter().find(|entry| entry.id == id) {
            relay.api_key = entry.api_key.trim().to_string();
            relay.active_api_key_id = id.to_string();
        }
        return Ok(id.to_string());
    }
    if let Some(first) = key_pool::expand_keys(relay).into_iter().next() {
        *relay = first.relay;
    }
    if keys.is_empty() {
        return Ok(String::new());
    }
    let current = relay
        .api_keys
        .iter()
        .find(|entry| {
            !relay.api_key.trim().is_empty() && entry.api_key.trim() == relay.api_key.trim()
        })
        .map(|entry| entry.id.clone());
    match current {
        Some(id) => {
            relay.active_api_key_id = id.clone();
            Ok(id)
        }
        None => Ok(keys[0].id.clone()),
    }
}

async fn run(
    settings: &BackendSettings,
    relay: &RelayProfile,
    model: &str,
    started: Instant,
    result: &mut ProbeResult,
) {
    // 账号类供应商（Copilot、Grok、ChatGPT API）先换好短期 token，和真实请求一样。
    let mut prepared = match accounts::prepare_relay(relay).await {
        Ok(prepared) => prepared,
        Err(error) => {
            fail(result, "auth", &crate::protocol_proxy::error_chain(&error));
            return;
        }
    };
    // OpenCode Go / Zen：按模型换成它所在的协议和地址。
    if let Err(reason) = opencode::prepare(&mut prepared, model, None) {
        fail(result, "not_found", &reason);
        return;
    }
    result.wire_api = protocol_label(&prepared).to_string();
    let candidate = GatewayCandidate {
        key: KeyRef {
            relay_id: prepared.id.clone(),
            key_id: result.key_id.clone(),
            fingerprint: 0,
        },
        relay: prepared,
        upstream_model: model.to_string(),
        window: None,
        limits: KeyLimits::default(),
    };
    let request = GatewayRequest {
        responses_body: probe_body(model),
        anthropic: None,
        user_agent: None,
        session_headers: ProxySessionHeaders::default(),
        live_request_id: None,
        // 不做「请求避震」重试：测试要看上游第一次的真实答复。
        auto_retry: false,
        is_stream: true,
        upstream_path: "/responses",
        client: None,
        queue_timeout: Duration::ZERO,
    };
    match pipeline::attempt_once(settings, &candidate, &request).await {
        Err(error) => {
            fail(
                result,
                transport_category(&error),
                &crate::protocol_proxy::error_chain(&error),
            );
        }
        Ok((mut response, class)) => {
            result.http_status = Some(response.status_code);
            result.wire_api = wire_label(response.wire_api).to_string();
            if class.is_some() || !response.is_success() {
                let body = response.read_body().await.unwrap_or_default();
                let category = failure_category(response.status_code, class.as_ref(), &body);
                let text = String::from_utf8_lossy(&body);
                let text = if text.trim().is_empty() {
                    format!("上游返回 HTTP {}，没有错误内容", response.status_code)
                } else {
                    text.trim().to_string()
                };
                fail(result, category, &text);
                return;
            }
            read_reply(&mut response, started, result).await;
        }
    }
}

/// 测试请求体（Responses 格式，网关按供应商协议转换）：一句话、流式、输出上限很小。
fn probe_body(model: &str) -> Value {
    json!({
        "model": model,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": PROBE_PROMPT }]
        }],
        "max_output_tokens": PROBE_MAX_OUTPUT_TOKENS,
        "stream": true
    })
}

/// 读完成功的回复：记首字时间、拼出回复文字；流里报错也算失败。
async fn read_reply(
    response: &mut UpstreamProxyResponse,
    started: Instant,
    result: &mut ProbeResult,
) {
    let mut text = StreamText::default();
    let mut first: Option<u64> = None;
    let prefix = std::mem::take(&mut response.stream_prefix);
    if let Some(body) = response.buffered_body.take() {
        text.feed(&prefix);
        text.feed(&body);
    } else if let Some(mut inner) = response.response.take() {
        text.feed(&prefix);
        if text.saw_output {
            first = Some(elapsed_ms(started));
        }
        while text.raw.len() < BODY_LIMIT {
            match inner.chunk().await {
                Ok(Some(chunk)) => {
                    text.feed(&chunk);
                    if first.is_none() && text.saw_output {
                        first = Some(elapsed_ms(started));
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let error = anyhow::Error::new(error);
                    fail(
                        result,
                        "stream",
                        &format!(
                            "回复读到一半断开：{}",
                            crate::protocol_proxy::error_chain(&error)
                        ),
                    );
                    result.first_token_ms = first;
                    return;
                }
            }
        }
    }
    text.finish();
    // 上游没按流式答（整段 JSON）：按网关的非流式转换读出回复。
    if !text.saw_event {
        let raw = std::mem::take(&mut text.raw);
        match crate::launcher::non_stream_body_to_responses(response.wire_api, &raw, None) {
            Ok(value) => {
                if let Some(message) = error_message(&value) {
                    text.error = Some(message);
                } else {
                    text.reply = output_text(&value);
                    text.saw_output = !text.reply.is_empty();
                }
            }
            Err(_) => {
                let raw = String::from_utf8_lossy(&raw).trim().to_string();
                text.error = Some(if raw.is_empty() {
                    "上游返回了成功状态，但没有任何内容".to_string()
                } else {
                    format!("上游的回复无法解析：{raw}")
                });
            }
        }
        if text.saw_output && first.is_none() {
            first = Some(elapsed_ms(started));
        }
    }
    result.first_token_ms = first;
    if let Some(error) = text.error {
        fail(result, stream_error_category(&error), &error);
        return;
    }
    result.ok = true;
    result.category.clear();
    let (reply, truncated) = truncate(text.reply.trim(), REPLY_LIMIT);
    result.reply = reply;
    result.reply_truncated = truncated;
}

fn fail(result: &mut ProbeResult, category: &str, error: &str) {
    result.ok = false;
    result.category = category.to_string();
    result.error = truncate(error.trim(), ERROR_LIMIT).0;
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn truncate(text: &str, limit: usize) -> (String, bool) {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(limit).collect();
    let truncated = chars.next().is_some();
    (head, truncated)
}

fn protocol_label(relay: &RelayProfile) -> &'static str {
    if crate::official_login::member_uses_official_login_transport(relay) {
        return "responses";
    }
    match relay.protocol {
        RelayProtocol::Responses => "responses",
        RelayProtocol::ChatCompletions => "chat",
        RelayProtocol::AnthropicMessages => "anthropic",
    }
}

fn wire_label(wire: UpstreamWireApi) -> &'static str {
    match wire {
        UpstreamWireApi::ChatCompletions => "chat",
        UpstreamWireApi::AnthropicMessages => "anthropic",
        _ => "responses",
    }
}

/// 上游错误里提到模型不存在 / 不支持的说法（各家措辞不一）。
fn mentions_missing_model(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "model_not_found",
        "model not found",
        "no such model",
        "unknown model",
        "invalid model",
        "unsupported model",
        "model is not supported",
        "model not supported",
        "not a valid model",
        "does not exist",
        "模型不存在",
        "不支持的模型",
        "无可用渠道",
    ]
    .iter()
    .any(|hint| text.contains(hint))
}

/// 上游报错（HTTP 非 2xx）怎么归类：按网关的失败分类，再细分出「模型不对」。
fn failure_category(status: u16, class: Option<&FailureClass>, body: &[u8]) -> &'static str {
    let text = String::from_utf8_lossy(&body[..body.len().min(16 * 1024)]);
    let class = class
        .cloned()
        .unwrap_or_else(|| key_pool::classify_failure(status, &Default::default(), body));
    match class {
        FailureClass::AuthInvalid => "auth",
        FailureClass::QuotaExhausted { .. } => "quota",
        FailureClass::RateLimited { .. } => "rate_limited",
        _ if matches!(status, 401 | 403) => "auth",
        _ if status == 404 || mentions_missing_model(&text) => "not_found",
        FailureClass::ServerError => "server",
        _ => "client",
    }
}

/// HTTP 200 但流里报错时的归类。
fn stream_error_category(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if mentions_missing_model(text) {
        "not_found"
    } else if [
        "insufficient_quota",
        "usage_limit_reached",
        "quota",
        "insufficient balance",
        "insufficient_balance",
    ]
    .iter()
    .any(|hint| lower.contains(hint))
    {
        "quota"
    } else if lower.contains("rate limit") || lower.contains("rate_limit") {
        "rate_limited"
    } else {
        "stream"
    }
}

/// 没收到上游响应的错误：超时、连不上（含代理），其余归「其它」。
fn transport_category(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<reqwest::Error>() {
            if error.is_timeout() {
                return "timeout";
            }
            return "connect";
        }
    }
    let text = format!("{error:#}").to_ascii_lowercase();
    if text.contains("超时") || text.contains("timed out") || text.contains("timeout") {
        "timeout"
    } else {
        "other"
    }
}

/// 从流式回复里拼出文字。三种协议（Responses / Chat / Anthropic）的事件都认：
/// 网关把 Chat、Anthropic 上游的流原样交回，转换在转发给 Agent 时才做。
#[derive(Default)]
struct StreamText {
    raw: Vec<u8>,
    pending: Vec<u8>,
    reply: String,
    /// 收到过文字或思考的增量（算首字）。
    saw_output: bool,
    /// 认出过 SSE 的 `data:` 事件；一个都没有时按整段 JSON 读。
    saw_event: bool,
    error: Option<String>,
}

impl StreamText {
    fn feed(&mut self, bytes: &[u8]) {
        if self.raw.len() < BODY_LIMIT {
            self.raw.extend_from_slice(bytes);
        }
        self.pending.extend_from_slice(bytes);
        while let Some(position) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=position).collect();
            self.line(&String::from_utf8_lossy(&line));
        }
    }

    fn finish(&mut self) {
        let rest = std::mem::take(&mut self.pending);
        if !rest.is_empty() {
            self.line(&String::from_utf8_lossy(&rest));
        }
    }

    fn line(&mut self, line: &str) {
        let Some(data) = line.trim_end().strip_prefix("data:") else {
            return;
        };
        let data = data.trim();
        if data.is_empty() {
            return;
        }
        self.saw_event = true;
        if data == "[DONE]" {
            return;
        }
        if let Ok(value) = serde_json::from_str::<Value>(data) {
            self.event(&value);
        }
    }

    fn push(&mut self, text: &str) {
        if !text.is_empty() {
            self.saw_output = true;
            self.reply.push_str(text);
        }
    }

    fn event(&mut self, value: &Value) {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            // Responses
            "response.output_text.delta" => {
                self.push(
                    value
                        .get("delta")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            }
            "response.output_text.done" if self.reply.is_empty() => {
                self.push(
                    value
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
            }
            "response.completed" | "response.incomplete" | "response.done" => {
                if self.reply.is_empty()
                    && let Some(response) = value.get("response")
                {
                    let text = output_text(response);
                    self.push(&text);
                }
            }
            "response.failed" => {
                let message = value
                    .get("response")
                    .and_then(error_message)
                    .unwrap_or_else(|| value.to_string());
                self.error.get_or_insert(message);
            }
            "error" => {
                let message = error_message(value).unwrap_or_else(|| value.to_string());
                self.error.get_or_insert(message);
            }
            // Anthropic
            "content_block_delta" => {
                let delta = value.get("delta");
                if let Some(text) = delta
                    .and_then(|delta| delta.get("text"))
                    .and_then(Value::as_str)
                {
                    self.push(text);
                } else if delta
                    .and_then(|delta| delta.get("thinking"))
                    .and_then(Value::as_str)
                    .is_some_and(|text| !text.is_empty())
                {
                    self.saw_output = true;
                }
            }
            _ if kind.starts_with("response.reasoning") && kind.ends_with(".delta") => {
                self.saw_output = true;
            }
            _ => {}
        }
        // Chat Completions
        if let Some(choices) = value.get("choices").and_then(Value::as_array) {
            for choice in choices {
                let delta = choice.get("delta").or_else(|| choice.get("message"));
                let Some(delta) = delta else { continue };
                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    self.push(text);
                }
                if ["reasoning_content", "reasoning"].iter().any(|field| {
                    delta
                        .get(*field)
                        .and_then(Value::as_str)
                        .is_some_and(|text| !text.is_empty())
                }) {
                    self.saw_output = true;
                }
            }
        }
        if kind.is_empty()
            && value.get("error").is_some_and(|error| !error.is_null())
            && let Some(message) = error_message(value)
        {
            self.error.get_or_insert(message);
        }
    }
}

/// 取出错误说明：`error.message`、`error`（字符串）或 `message`。
fn error_message(value: &Value) -> Option<String> {
    match value.get("error").filter(|error| !error.is_null()) {
        Some(Value::String(message)) => Some(message.clone()),
        Some(error) => Some(
            error
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| error.to_string(), str::to_string),
        ),
        None => value
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Responses 结果里的文字（`output[].content[].text`）。
fn output_text(response: &Value) -> String {
    response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> StreamText {
        let mut text = StreamText::default();
        // 故意从中间切开，验证跨块的行能拼回来。
        let (left, right) = body.as_bytes().split_at(body.len() / 2);
        text.feed(left);
        text.feed(right);
        text.finish();
        text
    }

    #[test]
    fn reads_text_from_all_three_stream_formats() {
        let responses = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"O\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"K\"}\n\n";
        assert_eq!(parse(responses).reply, "OK");
        let chat = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\ndata: [DONE]\n\n";
        let chat = parse(chat);
        assert_eq!(chat.reply, "好");
        assert!(chat.saw_output && chat.saw_event);
        let anthropic = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"OK\"}}\n\n";
        assert_eq!(parse(anthropic).reply, "OK");
    }

    #[test]
    fn stream_errors_and_reasoning_only_streams() {
        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"message\":\"The model `x` does not exist\"}}}\n\n";
        let failed = parse(failed);
        assert_eq!(
            failed.error.as_deref(),
            Some("The model `x` does not exist")
        );
        assert_eq!(
            stream_error_category(failed.error.as_deref().unwrap()),
            "not_found"
        );
        let thinking =
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"…\"}\n\n";
        let thinking = parse(thinking);
        assert!(thinking.saw_output, "只有思考也算收到首字");
        assert!(thinking.reply.is_empty());
    }

    #[test]
    fn failure_categories_follow_status_and_body() {
        assert_eq!(
            failure_category(401, Some(&FailureClass::AuthInvalid), b"{}"),
            "auth"
        );
        assert_eq!(
            failure_category(403, Some(&FailureClass::Failover), b"{}"),
            "auth"
        );
        assert_eq!(
            failure_category(404, Some(&FailureClass::Failover), b"{}"),
            "not_found"
        );
        assert_eq!(
            failure_category(
                400,
                Some(&FailureClass::ClientError),
                b"{\"error\":{\"code\":\"model_not_found\"}}"
            ),
            "not_found"
        );
        assert_eq!(
            failure_category(
                429,
                Some(&FailureClass::RateLimited { retry_after: None }),
                b"{}"
            ),
            "rate_limited"
        );
        assert_eq!(
            failure_category(429, None, b"{\"error\":{\"code\":\"insufficient_quota\"}}"),
            "quota"
        );
        assert_eq!(
            failure_category(502, Some(&FailureClass::ServerError), b""),
            "server"
        );
        assert_eq!(
            failure_category(422, Some(&FailureClass::ClientError), b"bad"),
            "client"
        );
    }

    #[test]
    fn truncates_by_characters() {
        assert_eq!(truncate("你好世界", 2), ("你好".to_string(), true));
        assert_eq!(truncate("ok", 2), ("ok".to_string(), false));
    }
}
