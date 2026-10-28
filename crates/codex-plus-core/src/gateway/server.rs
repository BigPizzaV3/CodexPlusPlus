//! 网关的 HTTP 入口：`/gw/<agent>/...`。
//!
//! | 路径 | Agent | 说明 |
//! |---|---|---|
//! | `POST /gw/claude/v1/messages` | Claude Code | Anthropic Messages |
//! | `POST /gw/claude/v1/messages/count_tokens` | Claude Code | token 计数 |
//! | `GET /gw/claude/v1/models` | Claude Code | 模型列表（Anthropic 格式） |
//! | `POST /gw/grok/v1/responses` | Grok | Responses，复用 Codex 代理的输出链路 |
//! | `GET /gw/grok/v1/models` | Grok | 模型列表（OpenAI 格式） |
//! | `POST /gw/codex/v1/responses` | Codex | Responses，同上（保留 ChatGPT 订阅后端的私有事件） |
//! | `POST /gw/<agent>/v1/images/generations`、`/images/edits` | 任意 | 生图，发给「生图模型」（见 [`crate::gateway::images`]） |
//!
//! 身份靠路径，鉴权靠网关签发给该 Agent 的本地 token（`x-api-key` 或 `Authorization: Bearer`）。

use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::gateway::anthropic_inbound;
use crate::gateway::anthropic_outbound::{
    ResponsesSseToAnthropicSse, anthropic_error_body, anthropic_message_to_sse,
    responses_json_to_anthropic,
};
use crate::gateway::pipeline::{
    self, AnthropicPassthrough, GatewayRequest, GatewayUpstream, NativeAnthropicResponse,
};
use crate::gateway::route;
use crate::gateway::store::{GatewaySettings, GatewayStore, token_matches};
use crate::launcher::{
    ProxyOpener, ProxyRequestLog, SseSink, current_time_ms, decode_protocol_proxy_request_body,
    forward_protocol_proxy_stream, handle_protocol_proxy_connection, header_value_from_headers,
    log_helper_response, non_stream_body_to_responses, write_http_response,
};
use crate::protocol_proxy::{ProxySessionHeaders, UpstreamProxyResponse, UpstreamWireApi};
use crate::settings::SettingsStore;
use crate::tools::ToolId;

const JSON: &str = "application/json; charset=utf-8";

/// 把 `/gw/claude/v1/messages` 拆成 (Agent, `/messages`)：去掉 `/v1` 前缀，兼容不带版本的写法。
pub fn split_gateway_path(path: &str) -> Option<(ToolId, String)> {
    let rest = path.strip_prefix("/gw/")?;
    let (agent, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let agent = ToolId::parse(agent);
    if matches!(agent, ToolId::Unknown(_)) {
        return None;
    }
    let tail = format!("/{tail}");
    let tail = tail.strip_prefix("/v1").unwrap_or(&tail);
    let tail = if tail.is_empty() { "/" } else { tail };
    Some((agent, tail.trim_end_matches('/').to_string()))
}

fn client_token(headers: &str) -> Option<String> {
    if let Some(key) = header_value_from_headers(headers, "x-api-key")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Some(key);
    }
    header_value_from_headers(headers, "authorization").and_then(|value| {
        let value = value.trim();
        value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("bearer "))
            .map(|token| token.trim().to_string())
    })
}

async fn write_error(
    stream: &mut tokio::net::TcpStream,
    agent: &ToolId,
    status: u16,
    message: &str,
) -> anyhow::Result<()> {
    let body = match agent {
        ToolId::Claude => anthropic_error_body(status, message),
        _ => json!({ "error": { "message": message, "type": "gateway_error", "code": status } }),
    };
    write_http_response(
        stream,
        &crate::protocol_proxy::http_status_line(status),
        JSON,
        &serde_json::to_vec(&body)?,
    )
    .await?;
    stream.shutdown().await?;
    Ok(())
}

/// `/gw/*` 的总入口，由 helper 连接处理函数调用。
pub(crate) async fn handle(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let Some((agent, tail)) = split_gateway_path(path) else {
        return write_error(stream, &ToolId::Claude, 404, "未知的网关路径").await;
    };
    if method == "OPTIONS" {
        write_http_response(stream, "204 No Content", JSON, &[]).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let gateway = GatewayStore::default().load().unwrap_or_default();
    let Some(binding) = gateway.active_binding(&agent) else {
        let message = format!(
            "Codex++ 网关模式没有为 {} 开启，请在 Codex++ 里打开网关模式或切回直连供应商",
            crate::tools::tool_display_name(&agent)
        );
        return write_error(stream, &agent, 503, &message).await;
    };
    let provided = client_token(headers).unwrap_or_default();
    if !token_matches(&binding.token, &provided) {
        log_helper_response(
            "gateway.unauthorized",
            method,
            path,
            "401 Unauthorized",
            remote_addr_text,
        );
        return write_error(
            stream,
            &agent,
            401,
            "网关 token 无效，请在 Codex++ 里重新应用网关模式",
        )
        .await;
    }
    match (&agent, method, tail.as_str()) {
        (ToolId::Claude, "POST", "/messages") => {
            handle_claude_messages(
                stream,
                &gateway,
                method,
                path,
                headers,
                body,
                remote_addr_text,
            )
            .await
        }
        (ToolId::Claude, "POST", "/messages/count_tokens") => {
            handle_claude_count_tokens(stream, &gateway, headers, body).await
        }
        (_, "GET", "/models") => handle_models(stream, &gateway, &agent).await,
        (ToolId::Grok | ToolId::Codex, "POST", "/responses") => {
            let request_body = match decode_protocol_proxy_request_body(
                body,
                header_value_from_headers(headers, "content-encoding").as_deref(),
            ) {
                Ok(body) => body,
                Err(error) => return write_error(stream, &agent, 400, &error.to_string()).await,
            };
            let user_agent = header_value_from_headers(headers, "user-agent");
            let session_id = header_value_from_headers(headers, "session-id")
                .or_else(|| header_value_from_headers(headers, "x-grok-conv-id"));
            // Codex 带自己的会话头和实验特性头，照旧传给上游。
            let thread_id = header_value_from_headers(headers, "thread-id");
            let beta_features = header_value_from_headers(headers, "x-codex-beta-features");
            handle_protocol_proxy_connection(
                stream,
                &request_body,
                user_agent.as_deref(),
                beta_features.as_deref(),
                ProxySessionHeaders {
                    session_id: session_id.as_deref(),
                    thread_id: thread_id.as_deref(),
                    opencode_session: None,
                },
                method,
                path,
                remote_addr_text,
                ProxyOpener::Gateway(agent),
            )
            .await
        }
        (_, "POST", "/images/generations" | "/images/edits") => {
            let edits = tail == "/images/edits";
            handle_images(
                stream,
                &gateway,
                &agent,
                edits,
                headers,
                body,
                method,
                path,
                remote_addr_text,
            )
            .await
        }
        _ => write_error(stream, &agent, 404, &format!("网关不支持 {method} {tail}")).await,
    }
}

/// 生图：按「生图模型」排出上游依次试，鉴权失败、限流、上游出错时换下一家；最后一家的答复原样返回。
#[allow(clippy::too_many_arguments)]
async fn handle_images(
    stream: &mut tokio::net::TcpStream,
    gateway: &GatewaySettings,
    agent: &ToolId,
    edits: bool,
    headers: &str,
    body: &[u8],
    method: &str,
    path: &str,
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let targets = match crate::gateway::images::targets(gateway) {
        Ok(targets) => targets,
        Err(message) => return write_error(stream, agent, 400, &message).await,
    };
    let content_type = header_value_from_headers(headers, "content-type").unwrap_or_default();
    let user_agent = header_value_from_headers(headers, "user-agent");
    let count = targets.len();
    let mut last_error = String::new();
    for (index, target) in targets.into_iter().enumerate() {
        let has_more = index + 1 < count;
        let (keys, _) = crate::gateway::key_pool::order_keys_with(
            &target.relay,
            gateway.key_policy.mode,
            None,
            std::time::Instant::now(),
            true,
        );
        let Some(key) = keys.into_iter().next() else {
            last_error = format!("{} 的 Key 都在冷却或熔断中", target.relay.name);
            continue;
        };
        let relay = match crate::gateway::accounts::prepare_relay(&key.relay).await {
            Ok(relay) => relay,
            Err(error) => {
                last_error = format!("{}：{error:#}", target.relay.name);
                continue;
            }
        };
        let request = match &target.model {
            Some(model) => crate::gateway::images::rewrite_model(body, &content_type, model),
            None => body.to_vec(),
        };
        let mut upstream = match crate::protocol_proxy::open_gateway_image_request(
            &relay,
            edits,
            &request,
            &content_type,
            user_agent.as_deref(),
        )
        .await
        {
            Ok(upstream) => upstream,
            Err(error) => {
                last_error = format!("{}：{error:#}", target.relay.name);
                continue;
            }
        };
        if has_more && crate::gateway::images::should_try_next(upstream.status_code) {
            last_error = format!("{} 返回 {}", target.relay.name, upstream.status_code);
            continue;
        }
        let status = upstream.status();
        let content_type = if upstream.content_type.is_empty() {
            JSON.to_string()
        } else {
            upstream.content_type.clone()
        };
        let answer = upstream.read_body().await?;
        write_http_response(stream, &status, &content_type, &answer).await?;
        log_helper_response(
            if upstream.is_success() {
                "gateway.image_ok"
            } else {
                "gateway.image_upstream_error"
            },
            method,
            path,
            &status,
            remote_addr_text,
        );
        stream.shutdown().await?;
        return Ok(());
    }
    write_error(stream, agent, 502, &format!("生图失败：{last_error}")).await
}

/// Grok 等 Responses 客户端：按 Agent 绑定选上游，返回值交给 Codex 代理的输出链路。
pub(crate) async fn open_responses_for_agent(
    agent: &ToolId,
    request_body: &str,
    user_agent: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    live_request_id: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let gateway = GatewayStore::default().load().unwrap_or_default();
    let request: Value = serde_json::from_str(request_body)?;
    let model = request.get("model").and_then(Value::as_str).unwrap_or("");
    let conversation = session_headers.session_id.map(str::to_string).or_else(|| {
        request
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let plan = route::resolve_plan(&gateway, agent, model, conversation.as_deref())?;
    let outcome = pipeline::open(
        &settings,
        &plan,
        GatewayRequest {
            responses_body: request,
            anthropic: None,
            user_agent,
            session_headers,
            live_request_id,
            auto_retry: true,
            is_stream: false,
        },
    )
    .await?;
    let trace = outcome.trace;
    match outcome.upstream {
        GatewayUpstream::Pipeline(mut response) => {
            let mut merged = response.request_trace.take().unwrap_or_default();
            merged.model = trace.model;
            merged.attempts = trace.attempts.max(merged.attempts);
            if merged.relay_id.is_empty() {
                merged.relay_id = trace.relay_id;
                merged.relay_name = trace.relay_name;
                merged.upstream_model = trace.upstream_model;
            }
            response.request_trace = Some(merged);
            Ok(response)
        }
        GatewayUpstream::Cooling(wait) => Ok(cooling_response(wait)),
        GatewayUpstream::Native(_) => anyhow::bail!("Responses 客户端不会走 Anthropic 原样转发"),
    }
}

fn cooling_message(wait: std::time::Duration) -> String {
    format!(
        "网关的所有上游 Key 都在冷却或熔断中，约 {} 秒后恢复",
        wait.as_secs().max(1)
    )
}

fn cooling_response(wait: std::time::Duration) -> UpstreamProxyResponse {
    UpstreamProxyResponse {
        status_code: 429,
        content_type: JSON.to_string(),
        is_stream: false,
        wire_api: UpstreamWireApi::Responses,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: Some(
            serde_json::to_vec(&json!({
                "error": { "type": "rate_limit_error", "code": "gateway_keys_cooling", "message": cooling_message(wait) }
            }))
            .unwrap_or_default(),
        ),
        stream_prefix: Vec::new(),
        request_trace: None,
        exec_adapted: false,
        exec_outcome: None,
        response: None,
    }
}

/// 写出 Anthropic SSE：Responses SSE 先过转换器，再落到 TCP 连接。
struct AnthropicSink<'a> {
    inner: &'a mut tokio::net::TcpStream,
    converter: ResponsesSseToAnthropicSse,
}

impl SseSink for AnthropicSink<'_> {
    async fn send(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let converted = self.converter.push_bytes(bytes);
        if converted.is_empty() {
            return Ok(());
        }
        self.inner.write_all(&converted).await
    }
}

async fn write_stream_headers(stream: &mut tokio::net::TcpStream) -> anyhow::Result<()> {
    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

async fn write_with_retry_after(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    content_type: &str,
    retry_after: Option<&str>,
    body: &[u8],
) -> anyhow::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
        crate::protocol_proxy::http_status_line(status),
        body.len()
    );
    if let Some(retry_after) = retry_after.map(str::trim).filter(|value| !value.is_empty()) {
        head.push_str(&format!("Retry-After: {retry_after}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(())
}

async fn handle_claude_messages(
    stream: &mut tokio::net::TcpStream,
    gateway: &GatewaySettings,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
    remote_addr_text: Option<String>,
) -> anyhow::Result<()> {
    let agent = ToolId::Claude;
    let request_text = match decode_protocol_proxy_request_body(
        body,
        header_value_from_headers(headers, "content-encoding").as_deref(),
    ) {
        Ok(text) => text,
        Err(error) => return write_error(stream, &agent, 400, &error.to_string()).await,
    };
    let request: Value = match serde_json::from_str(&request_text) {
        Ok(value) => value,
        Err(error) => {
            return write_error(
                stream,
                &agent,
                400,
                &format!("请求体不是有效的 JSON：{error}"),
            )
            .await;
        }
    };
    let client_model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let wants_stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let session = header_value_from_headers(headers, "x-claude-code-session-id");
    let user_agent = header_value_from_headers(headers, "user-agent");
    let beta = header_value_from_headers(headers, "anthropic-beta");
    let settings = SettingsStore::default().load().unwrap_or_default();

    let mut request_log = ProxyRequestLog::begin(
        method,
        path,
        Some(&json!({ "model": client_model })),
        session.clone(),
    );
    request_log.set_agent(&agent);
    let live_request_id = request_log.entry.id.clone();

    let plan = match route::resolve_plan(gateway, &agent, &client_model, session.as_deref()) {
        Ok(plan) => plan,
        Err(error) => {
            let message = crate::protocol_proxy::error_chain(&error);
            request_log.record_error("503 Service Unavailable", message.clone());
            return write_error(stream, &agent, 503, &message).await;
        }
    };
    let responses_body = match anthropic_inbound::anthropic_to_responses(&request) {
        Ok(body) => body,
        Err(error) => {
            let message = error.to_string();
            request_log.record_error("400 Bad Request", message.clone());
            return write_error(stream, &agent, 400, &message).await;
        }
    };
    let input_estimate = anthropic_inbound::estimate_input_tokens(&request);
    let outcome = pipeline::open(
        &settings,
        &plan,
        GatewayRequest {
            responses_body: responses_body.clone(),
            anthropic: Some(AnthropicPassthrough {
                body: &request,
                beta: beta.as_deref(),
            }),
            user_agent: user_agent.as_deref(),
            session_headers: ProxySessionHeaders {
                session_id: session.as_deref(),
                thread_id: None,
                opencode_session: None,
            },
            live_request_id: Some(live_request_id.as_str()),
            // 「请求避震」对新旧两套同时生效（只在用户打开时起作用）。
            auto_retry: true,
            is_stream: wants_stream,
        },
    )
    .await;
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let message = crate::protocol_proxy::error_chain(&error);
            let status = if is_client_rejection(&error) {
                400
            } else {
                502
            };
            request_log.record_error(
                &crate::protocol_proxy::http_status_line(status),
                message.clone(),
            );
            log_helper_response(
                "gateway.claude_failed",
                method,
                path,
                &status.to_string(),
                remote_addr_text,
            );
            return write_error(stream, &agent, status, &message).await;
        }
    };
    let trace = outcome.trace;
    // 持有到响应转发完：路由页据此显示「正在回答」。
    let _busy = outcome.busy;
    match outcome.upstream {
        GatewayUpstream::Cooling(wait) => {
            request_log.record_trace(&trace, 429, false);
            request_log.record_error("429 Too Many Requests", cooling_message(wait));
            let body = serde_json::to_vec(&anthropic_error_body(429, &cooling_message(wait)))?;
            write_with_retry_after(
                stream,
                429,
                JSON,
                Some(&wait.as_secs().max(1).to_string()),
                &body,
            )
            .await?;
            stream.shutdown().await?;
            Ok(())
        }
        GatewayUpstream::Native(response) => {
            write_native(stream, &mut request_log, &trace, response).await
        }
        GatewayUpstream::Pipeline(mut response) => {
            request_log.record_trace(&trace, response.status_code, wants_stream);
            if !response.is_success() {
                let content_type = response.content_type.clone();
                let body = response.read_body().await.unwrap_or_default();
                let (message, _, _, _) = crate::protocol_proxy::upstream_error_parts(
                    response.status_code,
                    &content_type,
                    &body,
                );
                request_log.record_error(&response.status(), message.clone());
                let body =
                    serde_json::to_vec(&anthropic_error_body(response.status_code, &message))?;
                write_http_response(stream, &response.status(), JSON, &body).await?;
                stream.shutdown().await?;
                return Ok(());
            }
            let upstream_json = response.content_type.to_ascii_lowercase().contains("json");
            if wants_stream && upstream_json {
                // 上游没理会 stream:true、回了整段 JSON：转成完整消息后再拆成事件流。
                let body = response.read_body().await?;
                let responses_json =
                    non_stream_body_to_responses(response.wire_api, &body, Some(&responses_body))?;
                request_log.record_usage_json(&serde_json::to_vec(&responses_json)?);
                request_log.record_stream(None, None, None);
                let message = responses_json_to_anthropic(&responses_json, &client_model);
                write_stream_headers(stream).await?;
                stream
                    .write_all(&anthropic_message_to_sse(&message))
                    .await?;
                stream.shutdown().await?;
                return Ok(());
            }
            if wants_stream {
                write_stream_headers(stream).await?;
                let mut sink = AnthropicSink {
                    inner: stream,
                    converter: ResponsesSseToAnthropicSse::new(&client_model, input_estimate),
                };
                if let Some(body) = response.buffered_body.take() {
                    sink.send(&body).await?;
                } else {
                    let prefix = std::mem::take(&mut response.stream_prefix);
                    let wire_api = response.wire_api;
                    let (_failure, _usage, _detail, first_output_at_ms) =
                        forward_protocol_proxy_stream(
                            &mut sink,
                            &prefix,
                            response.take_response()?,
                            wire_api,
                            Some(&responses_body),
                            session.as_deref(),
                        )
                        .await?;
                    if let Some(first_output_at_ms) = first_output_at_ms {
                        request_log.mark_first_token_at(first_output_at_ms);
                    }
                }
                let tail = sink.converter.finish();
                if !tail.is_empty() {
                    sink.inner.write_all(&tail).await?;
                }
                let failure = sink.converter.failure().map(str::to_string);
                request_log.record_stream(
                    failure.as_deref(),
                    sink.converter.usage(),
                    failure.clone(),
                );
                stream.shutdown().await?;
                return Ok(());
            }
            let body = response.read_body().await?;
            let responses_json =
                non_stream_body_to_responses(response.wire_api, &body, Some(&responses_body))?;
            request_log.record_usage_json(&serde_json::to_vec(&responses_json)?);
            request_log.record_stream(None, None, None);
            let message = responses_json_to_anthropic(&responses_json, &client_model);
            write_http_response(stream, "200 OK", JSON, &serde_json::to_vec(&message)?).await?;
            stream.shutdown().await?;
            Ok(())
        }
    }
}

fn is_client_rejection(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::protocol_proxy::UnsupportedInputFile>()
        .is_some()
        || error
            .downcast_ref::<crate::protocol_proxy::UnsupportedNativeWebSearch>()
            .is_some()
        || error
            .downcast_ref::<crate::protocol_proxy::UnsupportedEncryptedAgentContent>()
            .is_some()
}

/// Anthropic 上游原样转发：失败响应体本来就是 Anthropic 格式，原样回给客户端。
async fn write_native(
    stream: &mut tokio::net::TcpStream,
    request_log: &mut ProxyRequestLog,
    trace: &crate::request_log::RequestTrace,
    mut response: NativeAnthropicResponse,
) -> anyhow::Result<()> {
    request_log.record_trace(trace, response.status_code, response.is_stream);
    if let Some(body) = response.buffered_body.take() {
        if !(200..300).contains(&response.status_code) {
            let (message, _, _, _) = crate::protocol_proxy::upstream_error_parts(
                response.status_code,
                &response.content_type,
                &body,
            );
            request_log.record_error(
                &crate::protocol_proxy::http_status_line(response.status_code),
                message,
            );
        }
        write_with_retry_after(
            stream,
            response.status_code,
            &response.content_type,
            response.retry_after.as_deref(),
            &body,
        )
        .await?;
        stream.shutdown().await?;
        return Ok(());
    }
    let Some(upstream) = response.response.take() else {
        anyhow::bail!("缺少上游响应");
    };
    if !response.is_stream {
        let body = upstream.bytes().await?;
        request_log.record_usage_json(&body);
        request_log.record_stream(None, anthropic_usage(&body), None);
        write_http_response(stream, "200 OK", &response.content_type, &body).await?;
        stream.shutdown().await?;
        return Ok(());
    }
    write_stream_headers(stream).await?;
    let mut observer = AnthropicSseObserver::default();
    let mut chunks = upstream.bytes_stream();
    let mut failure = None;
    use futures_util::StreamExt;
    while let Some(chunk) = chunks.next().await {
        match chunk {
            Ok(bytes) => {
                if !bytes.is_empty() {
                    if !observer.started {
                        observer.started = true;
                        request_log.mark_first_token_at(current_time_ms());
                    }
                    observer.push(&bytes);
                    stream.write_all(&bytes).await?;
                }
            }
            Err(error) => {
                failure = Some(error.without_url().to_string());
                break;
            }
        }
    }
    if failure.is_none() && !observer.stopped {
        failure = Some("上游在 message_stop 之前结束了响应流".to_string());
    }
    request_log.record_stream(failure.as_deref(), observer.usage(), failure.clone());
    stream.shutdown().await?;
    Ok(())
}

/// 非流式 Anthropic 响应里的 usage。
fn anthropic_usage(body: &[u8]) -> Option<crate::request_log::RequestUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let usage = value.get("usage")?;
    let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cached = number("cache_read_input_tokens");
    Some(crate::request_log::RequestUsage {
        input_tokens: number("input_tokens") + cached + number("cache_creation_input_tokens"),
        output_tokens: number("output_tokens"),
        cached_tokens: cached,
        reasoning_tokens: 0,
    })
}

/// 旁路观察原样转发的 Anthropic 事件流：只取 usage 和是否收到 message_stop。
#[derive(Default)]
struct AnthropicSseObserver {
    buffer: String,
    started: bool,
    stopped: bool,
    input_tokens: u64,
    cached_tokens: u64,
    output_tokens: u64,
}

impl AnthropicSseObserver {
    fn push(&mut self, bytes: &[u8]) {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        while let Some(index) = self.buffer.find("\n\n") {
            let block = self.buffer[..index].to_string();
            self.buffer.drain(..index + 2);
            let Some(data) = block.lines().find_map(|line| line.strip_prefix("data:")) else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
                continue;
            };
            let usage = match event.get("type").and_then(Value::as_str) {
                Some("message_start") => event.pointer("/message/usage"),
                Some("message_delta") => event.get("usage"),
                Some("message_stop") => {
                    self.stopped = true;
                    None
                }
                _ => None,
            };
            if let Some(usage) = usage {
                let number = |key: &str| usage.get(key).and_then(Value::as_u64);
                if let Some(input) = number("input_tokens") {
                    self.input_tokens = input
                        + number("cache_read_input_tokens").unwrap_or(0)
                        + number("cache_creation_input_tokens").unwrap_or(0);
                }
                if let Some(cached) = number("cache_read_input_tokens") {
                    self.cached_tokens = cached;
                }
                if let Some(output) = number("output_tokens") {
                    self.output_tokens = output;
                }
            }
        }
    }

    fn usage(&self) -> Option<crate::request_log::RequestUsage> {
        (self.input_tokens > 0 || self.output_tokens > 0).then(|| {
            crate::request_log::RequestUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                cached_tokens: self.cached_tokens,
                reasoning_tokens: 0,
            }
        })
    }
}

/// count_tokens：第一个候选是 Anthropic 上游时原样问它，否则本地估算。
async fn handle_claude_count_tokens(
    stream: &mut tokio::net::TcpStream,
    gateway: &GatewaySettings,
    headers: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let agent = ToolId::Claude;
    let mut request: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(error) => {
            return write_error(
                stream,
                &agent,
                400,
                &format!("请求体不是有效的 JSON：{error}"),
            )
            .await;
        }
    };
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // 只是数 token，不真正发请求：用预览，别推进模型组轮换和 Key 轮询的进度。
    if let Ok(plan) = route::preview_plan(gateway, &agent, &model, None)
        && let Some(candidate) = plan.candidates.first()
        && candidate.relay.protocol == crate::settings::RelayProtocol::AnthropicMessages
    {
        request["model"] = Value::String(candidate.upstream_model.clone());
        anthropic_inbound::sanitize_for_native_anthropic(&mut request);
        let relay = &candidate.relay;
        let endpoint = format!(
            "{}/count_tokens",
            crate::anthropic_bridge::anthropic_messages_url(relay.base_url.trim())
        );
        let user_agent = header_value_from_headers(headers, "user-agent").unwrap_or_default();
        if let Ok(client) = crate::http_client::proxied_client_for_relay(&user_agent, &relay) {
            let mut builder = client.post(&endpoint).json(&request);
            builder = crate::relay_headers::apply_for_protocol(builder, relay);
            if let Some(beta) = header_value_from_headers(headers, "anthropic-beta") {
                builder = builder.header("anthropic-beta", beta);
            }
            if let Ok(response) = crate::protocol_proxy::send_upstream_request(builder).await
                && response.status().is_success()
                && let Ok(bytes) = response.bytes().await
            {
                write_http_response(stream, "200 OK", JSON, &bytes).await?;
                stream.shutdown().await?;
                return Ok(());
            }
        }
    }
    let estimate = anthropic_inbound::estimate_input_tokens(&request);
    let body = serde_json::to_vec(&json!({ "input_tokens": estimate }))?;
    write_http_response(stream, "200 OK", JSON, &body).await?;
    stream.shutdown().await?;
    Ok(())
}

async fn handle_models(
    stream: &mut tokio::net::TcpStream,
    gateway: &GatewaySettings,
    agent: &ToolId,
) -> anyhow::Result<()> {
    let models = exposed_models(gateway, agent);
    let body = match agent {
        ToolId::Claude => json!({
            "data": models.iter().map(|model| json!({
                "type": "model",
                "id": model,
                "display_name": model,
                "created_at": "2026-01-01T00:00:00Z"
            })).collect::<Vec<_>>(),
            "has_more": false,
            "first_id": models.first(),
            "last_id": models.last()
        }),
        _ => json!({
            "object": "list",
            "data": models.iter().map(|model| json!({
                "id": model,
                "object": "model",
                "created": 0,
                "owned_by": "codex-plus"
            })).collect::<Vec<_>>()
        }),
    };
    write_http_response(stream, "200 OK", JSON, &serde_json::to_vec(&body)?).await?;
    stream.shutdown().await?;
    Ok(())
}

/// 暴露给 Agent 的模型：默认模型在前，然后是池里所有能用的模型和路由组，以及绑定里声明的模型。
pub fn exposed_models(gateway: &GatewaySettings, agent: &ToolId) -> Vec<String> {
    let Some(binding) = gateway.binding(agent) else {
        return Vec::new();
    };
    let mut models = Vec::new();
    let default = route::binding_default_model(gateway, binding);
    if !default.is_empty() {
        models.push(default);
    }
    let pool_models = route::pool_models(gateway, binding);
    for model in pool_models.into_iter().chain(binding.declared_models()) {
        if !models.contains(&model) {
            models.push(model);
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_paths_split_into_agent_and_tail() {
        assert_eq!(
            split_gateway_path("/gw/claude/v1/messages"),
            Some((ToolId::Claude, "/messages".to_string()))
        );
        assert_eq!(
            split_gateway_path("/gw/claude/messages/count_tokens"),
            Some((ToolId::Claude, "/messages/count_tokens".to_string()))
        );
        assert_eq!(
            split_gateway_path("/gw/grok/v1/responses"),
            Some((ToolId::Grok, "/responses".to_string()))
        );
        assert_eq!(
            split_gateway_path("/gw/grok/v1/models/"),
            Some((ToolId::Grok, "/models".to_string()))
        );
        assert_eq!(
            split_gateway_path("/gw/codex/v1/responses"),
            Some((ToolId::Codex, "/responses".to_string()))
        );
        assert_eq!(split_gateway_path("/gw/other/x"), None);
        assert_eq!(split_gateway_path("/v1/messages"), None);
    }

    #[test]
    fn client_token_reads_api_key_or_bearer() {
        let headers = "POST /gw/claude/v1/messages HTTP/1.1\r\nx-api-key: cpx-gw-claude-1\r\n";
        assert_eq!(client_token(headers).as_deref(), Some("cpx-gw-claude-1"));
        let headers = "POST / HTTP/1.1\r\nAuthorization: Bearer cpx-gw-grok-2\r\n";
        assert_eq!(client_token(headers).as_deref(), Some("cpx-gw-grok-2"));
        assert_eq!(client_token("POST / HTTP/1.1\r\n"), None);
    }

    #[test]
    fn native_observer_collects_usage_and_stop() {
        let mut observer = AnthropicSseObserver::default();
        observer.push(b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10,\"cache_read_input_tokens\":90,\"output_tokens\":1}}}\n\n");
        observer.push(b"event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":7}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
        let usage = observer.usage().unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.cached_tokens, 90);
        assert_eq!(usage.output_tokens, 7);
        assert!(observer.stopped);
    }
}
