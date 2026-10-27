//! 网关的候选循环：按路由计划逐个尝试上游，失败按分类换候选、冷却 Key。
//!
//! 每个候选的单次请求交给现有代理（`open_responses_proxy_request_for_relay`），
//! 协议转换、官方登录、exec 兼容都复用那边；Claude Code 遇到 Anthropic 协议的上游时
//! 直接原样转发 Anthropic 请求，不做任何转换。

use std::time::{Duration, Instant};

use anyhow::Context;
use serde_json::Value;

use crate::gateway::activity::{InFlight, RouteRecorder};
use crate::gateway::anthropic_inbound;
use crate::gateway::key_pool::{self, FailureClass};
use crate::gateway::route::{GatewayCandidate, RoutePlan};
use crate::protocol_proxy::{ForcedRelay, ProxySessionHeaders, UpstreamProxyResponse};
use crate::request_log::RequestTrace;
use crate::settings::{BackendSettings, RelayProtocol};

/// 读取失败响应体做分类时的上限。
const FAILURE_BODY_LIMIT: usize = 256 * 1024;

/// Anthropic 上游原样转发的结果。
pub struct NativeAnthropicResponse {
    pub status_code: u16,
    pub content_type: String,
    pub is_stream: bool,
    pub retry_after: Option<String>,
    /// 为 Some 时响应体已读完（失败响应）；否则从 `response` 读。
    pub buffered_body: Option<Vec<u8>>,
    pub response: Option<reqwest::Response>,
}

pub enum GatewayUpstream {
    /// 经现有代理链路（任意上游 → 可归一化成 Responses）。
    Pipeline(UpstreamProxyResponse),
    /// Anthropic 上游原样转发。
    Native(NativeAnthropicResponse),
    /// 全部候选都在冷却或熔断，最近一个多久后恢复。
    Cooling(Duration),
}

pub struct GatewayOutcome {
    pub upstream: GatewayUpstream,
    pub trace: RequestTrace,
    /// 成功时标记「正在由该上游回答」，持有到响应转发完为止。
    pub busy: Option<InFlight>,
}

impl GatewayUpstream {
    fn status_code(&self) -> Option<u16> {
        match self {
            GatewayUpstream::Pipeline(response) => Some(response.status_code),
            GatewayUpstream::Native(response) => Some(response.status_code),
            GatewayUpstream::Cooling(_) => None,
        }
    }

    fn is_success(&self) -> bool {
        self.status_code()
            .is_some_and(|status| (200..300).contains(&status))
    }
}

/// 客户端带来的 Anthropic 原始请求，只在 Claude Code 入口存在。
pub struct AnthropicPassthrough<'a> {
    pub body: &'a Value,
    pub beta: Option<&'a str>,
}

pub struct GatewayRequest<'a> {
    /// 中间格式（Responses）请求体。
    pub responses_body: Value,
    pub anthropic: Option<AnthropicPassthrough<'a>>,
    pub user_agent: Option<&'a str>,
    pub session_headers: ProxySessionHeaders<'a>,
    pub live_request_id: Option<&'a str>,
    pub auto_retry: bool,
    pub is_stream: bool,
}

/// 依次尝试路由计划里的候选，并把路由过程记到路由动态里。
pub async fn open(
    settings: &BackendSettings,
    plan: &RoutePlan,
    request: GatewayRequest<'_>,
) -> anyhow::Result<GatewayOutcome> {
    let mut recorder = RouteRecorder::new(plan);
    let result = open_candidates(settings, plan, request, &mut recorder).await;
    let answered = result
        .as_ref()
        .is_ok_and(|outcome| outcome.upstream.is_success());
    recorder.finish(answered);
    result
}

async fn open_candidates(
    settings: &BackendSettings,
    plan: &RoutePlan,
    request: GatewayRequest<'_>,
    recorder: &mut RouteRecorder,
) -> anyhow::Result<GatewayOutcome> {
    let mut trace = RequestTrace {
        model: plan.source_model.clone(),
        ..RequestTrace::default()
    };
    let conversation_seed = conversation_seed(&request);
    if plan.candidates.is_empty() {
        if let Some(wait) = plan.cooling {
            recorder.cooling(wait.as_millis() as u64);
            return Ok(GatewayOutcome {
                upstream: GatewayUpstream::Cooling(wait),
                trace,
                busy: None,
            });
        }
        anyhow::bail!("网关没有可用的上游：请检查绑定的供应商或路由组是否配置了地址和 Key");
    }
    let count = plan.candidates.len();
    let mut last_error: Option<anyhow::Error> = None;
    for (index, candidate) in plan.candidates.iter().enumerate() {
        let has_more = index + 1 < count;
        trace.attempts += 1;
        trace.relay_id = candidate.relay.id.clone();
        trace.relay_name = candidate.relay.name.clone();
        trace.upstream_model = candidate.upstream_model.clone();
        if let Some(id) = request.live_request_id {
            crate::request_log::update_running_retry(id, trace.attempts, 0, Vec::new());
        }
        // 账号类供应商（如 Copilot）在请求前换好短期 token。
        let prepared = match crate::gateway::accounts::prepare_relay(&candidate.relay).await {
            Ok(relay) => GatewayCandidate {
                relay,
                ..candidate.clone()
            },
            Err(error) => {
                recorder.step(candidate, "transport", Some("auth_invalid"), None);
                log_attempt_failure(candidate, None, Some(&error), has_more);
                last_error = Some(error);
                continue;
            }
        };
        // OpenCode Go / Zen：按模型换成它所在的协议和地址，带上会话头。
        let mut prepared = prepared;
        if let Err(reason) = crate::gateway::opencode::prepare(
            &mut prepared.relay,
            &prepared.upstream_model,
            conversation_seed.as_deref(),
        ) {
            let error = anyhow::anyhow!(reason);
            recorder.step(candidate, "transport", Some("unsupported_model"), None);
            log_attempt_failure(candidate, None, Some(&error), has_more);
            last_error = Some(error);
            continue;
        }
        let candidate = &prepared;
        let busy = recorder.begin(candidate);
        let native = request
            .anthropic
            .as_ref()
            .filter(|_| candidate.relay.protocol == RelayProtocol::AnthropicMessages);
        let attempt = match native {
            Some(passthrough) => {
                trace.wire_api = "AnthropicMessages".to_string();
                attempt_native(candidate, passthrough, &request).await
            }
            None => attempt_pipeline(settings, candidate, &request).await,
        };
        let failure = match attempt {
            Ok(Attempt::Success(upstream)) => {
                recorder.step(candidate, "answered", None, upstream.status_code());
                key_pool::record_success(&candidate.key, plan.conversation.as_deref());
                if let GatewayUpstream::Pipeline(response) = &upstream
                    && let Some(inner) = &response.request_trace
                {
                    trace.wire_api = inner.wire_api.clone();
                    trace.first_output_at_ms = inner.first_output_at_ms;
                    trace.auto_retries = inner.auto_retries;
                    trace.retry_reasons = inner.retry_reasons.clone();
                    trace.attempt_details = inner.attempt_details.clone();
                }
                return Ok(GatewayOutcome {
                    upstream,
                    trace,
                    busy: Some(busy),
                });
            }
            Ok(Attempt::Failed { upstream, class }) => {
                recorder.step(
                    candidate,
                    "failed",
                    Some(class.label()),
                    upstream.status_code(),
                );
                let failover = key_pool::record_failure(&candidate.key, &class, Instant::now());
                log_attempt_failure(candidate, Some(&class), None, has_more && failover);
                if !failover || !has_more {
                    return Ok(GatewayOutcome {
                        upstream,
                        trace,
                        busy: None,
                    });
                }
                None
            }
            Err(AttemptError::Fatal(error)) => return Err(error),
            Err(AttemptError::Transport(error)) => {
                recorder.step(candidate, "transport", Some("server_error"), None);
                key_pool::record_failure(
                    &candidate.key,
                    &FailureClass::ServerError,
                    Instant::now(),
                );
                log_attempt_failure(candidate, None, Some(&error), has_more);
                Some(error)
            }
        };
        drop(busy);
        if let Some(error) = failure {
            last_error = Some(error);
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("网关的所有上游都失败了")))
}

enum Attempt {
    Success(GatewayUpstream),
    Failed {
        upstream: GatewayUpstream,
        class: FailureClass,
    },
}

enum AttemptError {
    /// 请求本身不被支持（不可切换候选，直接报给客户端）。
    Fatal(anyhow::Error),
    /// 连接、超时等：换下一个候选。
    Transport(anyhow::Error),
}

async fn attempt_pipeline(
    settings: &BackendSettings,
    candidate: &GatewayCandidate,
    request: &GatewayRequest<'_>,
) -> Result<Attempt, AttemptError> {
    let mut body = request.responses_body.clone();
    if candidate.relay.protocol == RelayProtocol::Responses {
        anthropic_inbound::strip_unreplayable_reasoning(&mut body);
    }
    // ChatGPT 订阅后端只服务 Codex 自己的客户端：请求体、请求头都整理成 Codex CLI 发的样子。
    let chatgpt = crate::official_login::member_uses_official_login_transport(&candidate.relay);
    let mut relay = candidate.relay.clone();
    let mut cache_key = String::new();
    if chatgpt {
        let home = crate::codex_home::default_codex_home_dir();
        let prompt = crate::gateway::chatgpt_backend::codex_prompt_from_cache(
            &home,
            &candidate.upstream_model,
        );
        body["model"] = Value::String(candidate.upstream_model.clone());
        crate::gateway::chatgpt_backend::adapt_request(&mut body, prompt.as_deref());
        let version = crate::gateway::chatgpt_backend::codex_version(&home);
        relay.user_agent = crate::gateway::chatgpt_backend::codex_user_agent(&version);
        for (key, value) in [
            ("version", version.as_str()),
            ("OpenAI-Beta", "responses=experimental"),
        ] {
            if !relay
                .custom_headers
                .iter()
                .any(|header| header.key.eq_ignore_ascii_case(key))
            {
                relay
                    .custom_headers
                    .push(crate::settings::RelayHeaderKeyValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
            }
        }
        cache_key = body
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    // ChatGPT API（用 ChatGPT 登录）：直接发 api.openai.com，保留 Agent 自己的指令。
    let chatgpt_api = !chatgpt
        && crate::gateway::accounts::relay_account_kind(&candidate.relay).as_deref()
            == Some("chatgpt-api");
    if chatgpt_api {
        body["model"] = Value::String(candidate.upstream_model.clone());
        crate::gateway::chatgpt_backend::adapt_api_request(&mut body);
        cache_key = body
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    }
    // 后端按 session-id / thread-id 找会话的缓存，键取自 prompt_cache_key。
    let session_headers = if (chatgpt || chatgpt_api) && !cache_key.is_empty() {
        ProxySessionHeaders {
            session_id: Some(cache_key.as_str()),
            thread_id: Some(cache_key.as_str()),
            opencode_session: None,
        }
    } else {
        request.session_headers
    };
    let body = serde_json::to_string(&body).map_err(|error| AttemptError::Fatal(error.into()))?;
    let forced = ForcedRelay {
        relay,
        upstream_model: Some(candidate.upstream_model.clone()),
        auto_retry: request.auto_retry,
    };
    let mut response = match crate::protocol_proxy::open_responses_proxy_request_for_relay(
        &body,
        settings.clone(),
        forced,
        request.user_agent,
        "/responses",
        session_headers,
        request.live_request_id,
    )
    .await
    {
        Ok(response) => response,
        Err(error) if is_request_rejection(&error) => return Err(AttemptError::Fatal(error)),
        Err(error) => return Err(AttemptError::Transport(error)),
    };
    if response.is_success() {
        return Ok(Attempt::Success(GatewayUpstream::Pipeline(response)));
    }
    let headers = response
        .response
        .as_ref()
        .map(|inner| inner.headers().clone())
        .unwrap_or_default();
    let body = match response.response.as_mut() {
        Some(inner) => crate::protocol_proxy::read_limited_upstream_body(inner, FAILURE_BODY_LIMIT)
            .await
            .unwrap_or_default(),
        None => response.read_body().await.unwrap_or_default(),
    };
    let class = key_pool::classify_failure(response.status_code, &headers, &body);
    response.response = None;
    response.stream_prefix.clear();
    response.buffered_body = Some(body);
    Ok(Attempt::Failed {
        upstream: GatewayUpstream::Pipeline(response),
        class,
    })
}

/// 标识同一个对话的种子：客户端带的会话头，或请求里的缓存键，或按对话开头算的键。
fn conversation_seed(request: &GatewayRequest<'_>) -> Option<String> {
    let headers = &request.session_headers;
    if let Some(seed) = [
        headers.opencode_session,
        headers.session_id,
        headers.thread_id,
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|seed| !seed.is_empty())
    {
        return Some(seed.to_string());
    }
    let body = &request.responses_body;
    if let Some(key) = body
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
    {
        return Some(key.to_string());
    }
    let input = body
        .get("input")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    crate::gateway::chatgpt_backend::conversation_key(
        body.get("instructions")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        input,
    )
}

/// 这些错误是请求内容本身不被支持（附件、加密内容、原生搜索、缺视觉配置），换上游也一样。
fn is_request_rejection(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::protocol_proxy::UnsupportedEncryptedAgentContent>()
        .is_some()
        || error
            .downcast_ref::<crate::protocol_proxy::UnsupportedNativeWebSearch>()
            .is_some()
        || error
            .downcast_ref::<crate::protocol_proxy::UnsupportedInputFile>()
            .is_some()
        || matches!(
            error.downcast_ref::<crate::vision::VlmPreprocessError>(),
            Some(crate::vision::VlmPreprocessError::MissingConfiguration)
        )
}

async fn attempt_native(
    candidate: &GatewayCandidate,
    passthrough: &AnthropicPassthrough<'_>,
    request: &GatewayRequest<'_>,
) -> Result<Attempt, AttemptError> {
    let mut body = passthrough.body.clone();
    body["model"] = Value::String(candidate.upstream_model.clone());
    anthropic_inbound::sanitize_for_native_anthropic(&mut body);
    let relay = &candidate.relay;
    let endpoint = crate::anthropic_bridge::anthropic_messages_url(relay.base_url.trim());
    let user_agent = if relay.user_agent.trim().is_empty() {
        request.user_agent.unwrap_or_default().to_string()
    } else {
        relay.user_agent.trim().to_string()
    };
    let client = crate::http_client::proxied_client_for_relay(&user_agent, &relay)
        .map_err(AttemptError::Fatal)?;
    let mut builder = client
        .post(&endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    builder = crate::relay_headers::apply_for_protocol(builder, relay);
    let custom_beta = relay
        .custom_headers
        .iter()
        .any(|header| header.key.trim().eq_ignore_ascii_case("anthropic-beta"));
    if !custom_beta
        && let Some(beta) = passthrough
            .beta
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        builder = builder.header("anthropic-beta", beta);
    }
    if request.is_stream {
        builder = builder
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
    }
    builder = builder.json(&body);
    let mut response = match crate::protocol_proxy::send_upstream_request_for_responses(
        builder,
        request.is_stream,
    )
    .await
    .with_context(|| format!("供应商「{}」请求上游失败，endpoint: {endpoint}", relay.name))
    {
        Ok(response) => response,
        Err(error) => return Err(AttemptError::Transport(error)),
    };
    let status_code = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    if (200..300).contains(&status_code) {
        return Ok(Attempt::Success(GatewayUpstream::Native(
            NativeAnthropicResponse {
                status_code,
                is_stream: content_type.contains("text/event-stream"),
                content_type,
                retry_after,
                buffered_body: None,
                response: Some(response),
            },
        )));
    }
    let headers = response.headers().clone();
    let body = crate::protocol_proxy::read_limited_upstream_body(&mut response, FAILURE_BODY_LIMIT)
        .await
        .unwrap_or_default();
    let class = key_pool::classify_failure(status_code, &headers, &body);
    Ok(Attempt::Failed {
        upstream: GatewayUpstream::Native(NativeAnthropicResponse {
            status_code,
            content_type,
            is_stream: false,
            retry_after,
            buffered_body: Some(body),
            response: None,
        }),
        class,
    })
}

fn log_attempt_failure(
    candidate: &GatewayCandidate,
    class: Option<&FailureClass>,
    error: Option<&anyhow::Error>,
    will_failover: bool,
) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "gateway.upstream_failed",
        serde_json::json!({
            "relayId": candidate.relay.id,
            "relayName": candidate.relay.name,
            "keyId": candidate.key.key_id,
            "upstreamModel": candidate.upstream_model,
            "class": class.map(FailureClass::label),
            "error": error.map(crate::protocol_proxy::error_chain),
            "willFailover": will_failover,
        }),
    );
}
