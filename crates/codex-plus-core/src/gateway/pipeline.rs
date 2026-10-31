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
use crate::gateway::key_pool::{self, FailureClass, KeyLimitState, KeySlot};
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
    /// 成功时占着的限速名额（没设上限时是空名额）：持有到响应转发完为止，drop 时释放并发名额。
    pub slot: Option<KeySlot>,
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
    /// 发给上游的 Responses 路径：`/responses`，或旧版 Codex 远程压缩的 `/responses/compact`。
    pub upstream_path: &'a str,
    /// 客户端连接：因本地限速排队时用来发现客户端已断开，不再干等。
    pub client: Option<&'a tokio::net::TcpStream>,
    /// 所有候选都因本地限速满了时最多排队多久，一般是 [`key_pool::LIMIT_QUEUE_TIMEOUT`]。
    pub queue_timeout: Duration,
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
    let mut pending = fit_first(&plan.candidates, estimate_tokens(&request));
    if plan.candidates.is_empty() {
        let Some(wait) = plan.cooling else {
            anyhow::bail!("网关没有可用的上游：请检查绑定的供应商或模型组是否配置了地址和 Key");
        };
        // 所有 Key 都在冷却（#2802）：最早的那个很快恢复时等它一等再试，不马上回本地 429——
        // Codex 自己的重试几秒内就用完了，冷却稍长一点就会整轮失败，而直连时这几秒后上游多半已恢复。
        match wait_for_cooling_key(plan, &request).await? {
            Some(ready) => pending = vec![ready],
            None => {
                recorder.cooling(wait.as_millis() as u64);
                return Ok(GatewayOutcome {
                    upstream: GatewayUpstream::Cooling(wait),
                    trace,
                    busy: None,
                    slot: None,
                });
            }
        }
    }
    let mut last_error: Option<anyhow::Error> = None;
    // 主动限速：第一次因为候选都满了而排队的时刻，和累计排了多久。
    let mut queue_started: Option<Instant> = None;
    let mut queued = Duration::ZERO;
    loop {
        // 先登记「有名额释放」的通知再检查名额，检查和等待之间的释放不会漏掉。
        let released = key_pool::limit_released();
        tokio::pin!(released);
        released.as_mut().enable();
        let total = pending.len();
        // 这一轮因限速满了跳过的候选（按原顺序）：不算失败、不冷却，都试完了再排队等它们。
        let mut deferred: Vec<&GatewayCandidate> = Vec::new();
        for (index, candidate) in pending.into_iter().enumerate() {
            let Some(slot) =
                key_pool::try_acquire(&candidate.key, candidate.limits, Instant::now())
            else {
                deferred.push(candidate);
                continue;
            };
            let has_more = index + 1 < total || !deferred.is_empty();
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
                    // 请求没发出去：名额（连同 RPM 的这一次）退回。
                    slot.cancel();
                    recorder.step(candidate, "transport", Some("auth_invalid"), None);
                    crate::gateway::speed::record_failure(
                        &candidate.relay.id,
                        &candidate.upstream_model,
                    );
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
                slot.cancel();
                let error = anyhow::anyhow!(reason);
                recorder.step(candidate, "transport", Some("unsupported_model"), None);
                log_attempt_failure(candidate, None, Some(&error), has_more);
                last_error = Some(error);
                continue;
            }
            let candidate = &prepared;
            let busy = recorder.begin(candidate);
            let started = Instant::now();
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
                Ok(Attempt::Success(mut upstream)) => {
                    recorder.step(candidate, "answered", None, upstream.status_code());
                    // 排在它前面的候选因为限速满了被跳过：这次只是临时顶替，不改会话固定
                    // （同一家的 Key 粘性、模型组记住的成员都保持原样）。
                    let same_relay_skipped = deferred
                        .iter()
                        .any(|skipped| skipped.relay.id == candidate.relay.id);
                    let conversation = if same_relay_skipped {
                        None
                    } else {
                        plan.conversation.as_deref()
                    };
                    key_pool::record_success(&candidate.key, conversation);
                    crate::gateway::speed::record(
                        &candidate.relay.id,
                        &candidate.upstream_model,
                        started.elapsed(),
                    );
                    if deferred.is_empty() {
                        crate::gateway::route::remember_answer(plan, candidate);
                    }
                    observe_quota(&candidate.relay.id, &upstream);
                    if let GatewayUpstream::Pipeline(response) = &upstream
                        && let Some(inner) = &response.request_trace
                    {
                        trace.wire_api = inner.wire_api.clone();
                        trace.first_output_at_ms = inner.first_output_at_ms;
                        trace.auto_retries = inner.auto_retries;
                        trace.retry_reasons = inner.retry_reasons.clone();
                        trace.attempt_details = inner.attempt_details.clone();
                    }
                    if queue_started.is_some() {
                        // 请求日志的重试 / 备注栏标出「限速排队 Xs」。
                        let reason = queue_reason(queued);
                        trace.retry_reasons.push(reason.clone());
                        if let GatewayUpstream::Pipeline(response) = &mut upstream
                            && let Some(inner) = response.request_trace.as_mut()
                        {
                            inner.retry_reasons.push(reason);
                        }
                    }
                    return Ok(GatewayOutcome {
                        upstream,
                        trace,
                        busy: Some(busy),
                        slot: Some(slot),
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
                    // 失败也算「试过了」：速度优先的组不再把它当没测过的排第一。
                    crate::gateway::speed::record_failure(
                        &candidate.relay.id,
                        &candidate.upstream_model,
                    );
                    log_attempt_failure(candidate, Some(&class), None, has_more && failover);
                    if !failover || !has_more {
                        return Ok(GatewayOutcome {
                            upstream,
                            trace,
                            busy: None,
                            slot: None,
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
                    crate::gateway::speed::record_failure(
                        &candidate.relay.id,
                        &candidate.upstream_model,
                    );
                    log_attempt_failure(candidate, None, Some(&error), has_more);
                    Some(error)
                }
            };
            // 失败的响应体已经读完：并发名额马上还回去。
            drop(slot);
            drop(busy);
            if let Some(error) = failure {
                last_error = Some(error);
            }
        }
        if deferred.is_empty() {
            break;
        }
        // 剩下的候选都因为限速满了：排队等最早空出来的名额，有上限。
        let now = Instant::now();
        let deadline = *queue_started.get_or_insert(now) + request.queue_timeout;
        if now >= deadline {
            return Err(LocalRateLimited::new(&deferred, request.queue_timeout).into());
        }
        let retry_in = deferred
            .iter()
            .filter_map(|candidate| {
                match key_pool::limit_state(&candidate.key, candidate.limits, now) {
                    KeyLimitState::Free => Some(Duration::ZERO),
                    KeyLimitState::Full { retry_in } => retry_in,
                }
            })
            .min();
        let remaining = deadline - now;
        let sleep = retry_in.map_or(remaining, |retry_in| retry_in.min(remaining));
        if let Some(id) = request.live_request_id {
            crate::request_log::update_running_phase(id, "waiting");
        }
        tokio::select! {
            _ = &mut released => {}
            _ = tokio::time::sleep(sleep) => {}
            _ = client_gone(request.client) => {
                anyhow::bail!("客户端已断开，放弃等待 Codex++ 本地限速的名额");
            }
        }
        queued += now.elapsed();
        pending = deferred;
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("网关的所有上游都失败了")))
}

/// 所有 Key 都在冷却时，最早恢复的那个最多等这么久（#2802）。
///
/// 覆盖无提示 429 的前三次退避（5 / 10 / 20 秒）和上游给的短 Retry-After；
/// 更久的冷却（额度耗尽、上游明确要求等一分钟以上）照旧马上回 429，不让请求一直挂着。
pub const COOLING_WAIT_LIMIT: Duration = Duration::from_secs(20);

/// 等冷却中最早恢复的 Key 恢复，交回它；等不到（超过 [`COOLING_WAIT_LIMIT`]）返回 None。
///
/// 等的过程中别的请求可能又把它打进冷却（再 429 一次），这时按新的剩余时间接着等，总时长不超过上限；
/// 客户端断开就不再等。
async fn wait_for_cooling_key<'a>(
    plan: &'a RoutePlan,
    request: &GatewayRequest<'_>,
) -> anyhow::Result<Option<&'a GatewayCandidate>> {
    let Some(first) = plan.waiting.first() else {
        return Ok(None);
    };
    let started = Instant::now();
    let deadline = started + COOLING_WAIT_LIMIT;
    if started + first.remaining > deadline {
        return Ok(None);
    }
    if let Some(id) = request.live_request_id {
        crate::request_log::update_running_phase(id, "waiting");
    }
    loop {
        let now = Instant::now();
        // 冷却中的 Key 里现在还要等最短的那个。
        let next = plan
            .waiting
            .iter()
            .map(|item| {
                let remaining =
                    key_pool::cooling_remaining(&item.candidate.key, now).unwrap_or_default();
                (&item.candidate, remaining)
            })
            .min_by_key(|(_, remaining)| *remaining);
        let Some((candidate, remaining)) = next else {
            return Ok(None);
        };
        if remaining.is_zero() {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.cooling_wait",
                serde_json::json!({
                    "relayId": candidate.relay.id,
                    "keyId": candidate.key.key_id,
                    "waitedMs": started.elapsed().as_millis() as u64,
                }),
            );
            return Ok(Some(candidate));
        }
        if now + remaining > deadline {
            return Ok(None);
        }
        tokio::select! {
            _ = tokio::time::sleep(remaining) => {}
            _ = client_gone(request.client) => {
                anyhow::bail!("客户端已断开，放弃等待冷却中的上游 Key");
            }
        }
    }
}

/// 请求日志里「限速排队」的记录：`local_rate_limit_wait_ms:<毫秒>`，界面显示成「限速排队 Xs」。
pub const QUEUE_REASON_PREFIX: &str = "local_rate_limit_wait_ms:";

fn queue_reason(queued: Duration) -> String {
    format!("{QUEUE_REASON_PREFIX}{}", queued.as_millis())
}

/// 等到客户端断开才返回；没给连接、或客户端还连着（读到了多余数据）时一直挂着。
async fn client_gone(client: Option<&tokio::net::TcpStream>) {
    let Some(stream) = client else {
        return std::future::pending().await;
    };
    let mut probe = [0_u8; 1];
    match stream.peek(&mut probe).await {
        Ok(0) | Err(_) => {}
        Ok(_) => std::future::pending().await,
    }
}

/// 所有候选都因 Codex++ 本地限速满了、排队超时：不是上游的错误，回给客户端时用 429。
#[derive(Debug)]
pub struct LocalRateLimited {
    pub message: String,
}

impl LocalRateLimited {
    fn new(candidates: &[&GatewayCandidate], waited: Duration) -> Self {
        let mut providers: Vec<String> = Vec::new();
        for candidate in candidates {
            let limits = candidate.limits;
            let mut parts = Vec::new();
            if let Some(rpm) = limits.rpm {
                parts.push(format!("每分钟 {rpm} 次"));
            }
            if let Some(concurrency) = limits.concurrency {
                parts.push(format!("同时 {concurrency} 个"));
            }
            let name = crate::gateway::accounts::mask_account_name(&candidate.relay.name);
            let entry = if parts.is_empty() {
                format!("「{name}」")
            } else {
                format!("「{name}」（每个 Key {}）", parts.join("、"))
            };
            if !providers.contains(&entry) {
                providers.push(entry);
            }
        }
        Self {
            message: format!(
                "Codex++ 本地限速：{} 的 Key 都已达到你设置的请求上限，排队等待 {} 秒仍没有空出名额。\
                 这是 Codex++ 网关自己的限速，不是上游返回的错误；可以在网关「供应商池」里调高这家的限速上限，或给它添加更多 Key。",
                providers.join("、"),
                waited.as_secs()
            ),
        }
    }
}

impl std::fmt::Display for LocalRateLimited {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for LocalRateLimited {}

/// 请求比窗口还长的候选排到最后（不去掉）：它们多半只会拒绝，别家都不行时再试一次。
/// 没写窗口的候选当作装得下。
fn fit_first(candidates: &[GatewayCandidate], tokens: u64) -> Vec<&GatewayCandidate> {
    let (fits, short): (Vec<_>, Vec<_>) = candidates
        .iter()
        .partition(|candidate| candidate.window.is_none_or(|window| window >= tokens));
    fits.into_iter().chain(short).collect()
}

/// 请求大概有多少 token：按文字的字节数粗估（约 4 字节一个）。图片这类 `data:` 内容、
/// 加密的推理内容和签名按字节算会大出好几倍，不计入。
fn estimate_tokens(request: &GatewayRequest<'_>) -> u64 {
    let body = request
        .anthropic
        .as_ref()
        .map_or(&request.responses_body, |passthrough| passthrough.body);
    text_bytes(body) / 4
}

fn text_bytes(value: &Value) -> u64 {
    match value {
        Value::String(text) if text.starts_with("data:") => 0,
        Value::String(text) => text.len() as u64,
        Value::Array(items) => items.iter().map(text_bytes).sum(),
        Value::Object(fields) => fields
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "data" | "encrypted_content" | "signature"))
            .map(|(_, value)| text_bytes(value))
            .sum(),
        _ => 0,
    }
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
            // ChatGPT 后端的身份头由官方登录通道自己定，不沿用客户端带来的。
            client_identity: &[],
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
        request.upstream_path,
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

/// 只发一次、只发给这个候选（供应商池的「测试」用，见 [`crate::gateway::probe`]）。
///
/// 和候选循环里的一次尝试走同一条路（ChatGPT 后端的请求整理、协议转换、代理、认证都一样），
/// 但调用方要自己先做 `prepare_relay` / OpenCode 的 prepare；这里不占限速名额、不记 Key 冷却 /
/// 熔断 / 测速、不改会话固定、不写路由动态，也不记额度。
///
/// 成功时原样交回上游响应；上游报错时响应体已读进 `buffered_body`，并给出网关会怎么分类。
/// `Err` 是请求没能发出或没收到响应（连接失败、超时、请求本身不被支持）。
pub(crate) async fn attempt_once(
    settings: &BackendSettings,
    candidate: &GatewayCandidate,
    request: &GatewayRequest<'_>,
) -> anyhow::Result<(UpstreamProxyResponse, Option<FailureClass>)> {
    let upstream = match attempt_pipeline(settings, candidate, request).await {
        Ok(Attempt::Success(upstream)) => (upstream, None),
        Ok(Attempt::Failed { upstream, class }) => (upstream, Some(class)),
        Err(AttemptError::Fatal(error) | AttemptError::Transport(error)) => return Err(error),
    };
    match upstream {
        (GatewayUpstream::Pipeline(response), class) => Ok((response, class)),
        _ => anyhow::bail!("测试请求只走协议代理链路"),
    }
}

/// ChatGPT 后端的响应头带着这个账号的额度，顺手记下（模型组「额度优先」、供应商池显示用）。
fn observe_quota(relay_id: &str, upstream: &GatewayUpstream) {
    let headers = match upstream {
        GatewayUpstream::Pipeline(response) => {
            response.response.as_ref().map(reqwest::Response::headers)
        }
        GatewayUpstream::Native(response) => {
            response.response.as_ref().map(reqwest::Response::headers)
        }
        GatewayUpstream::Cooling(_) => None,
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    if let Some(quota) =
        headers.and_then(|headers| crate::gateway::quota::from_codex_headers(headers, now))
    {
        crate::gateway::quota::record(relay_id, quota);
    }
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
            // 账号类供应商的名字里带邮箱，日志里只留打码后的。
            "relayName": crate::gateway::accounts::mask_account_name(&candidate.relay.name),
            "keyId": candidate.key.key_id,
            "upstreamModel": candidate.upstream_model,
            "class": class.map(FailureClass::label),
            "error": error.map(crate::protocol_proxy::error_chain),
            "willFailover": will_failover,
        }),
    );
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use crate::gateway::key_pool::KeyRef;
    use serde_json::json;

    fn candidate(id: &str, window: Option<u64>) -> GatewayCandidate {
        GatewayCandidate {
            relay: crate::settings::RelayProfile {
                id: id.to_string(),
                ..Default::default()
            },
            key: KeyRef {
                relay_id: id.to_string(),
                key_id: String::new(),
                fingerprint: 0,
            },
            upstream_model: "m".to_string(),
            window,
            limits: Default::default(),
        }
    }

    #[test]
    fn too_short_windows_go_last_but_stay() {
        let candidates = vec![
            candidate("small", Some(1_000)),
            candidate("unknown", None),
            candidate("big", Some(1_000_000)),
        ];
        let order: Vec<&str> = fit_first(&candidates, 5_000)
            .iter()
            .map(|candidate| candidate.relay.id.as_str())
            .collect();
        assert_eq!(order, ["unknown", "big", "small"]);
        let order: Vec<&str> = fit_first(&candidates, 10)
            .iter()
            .map(|candidate| candidate.relay.id.as_str())
            .collect();
        assert_eq!(order, ["small", "unknown", "big"], "装得下就不动顺序");
    }

    #[test]
    fn estimate_skips_images_and_encrypted_blobs() {
        let body = json!({
            "input": [
                { "type": "message", "content": [
                    { "type": "input_text", "text": "a".repeat(4_000) },
                    { "type": "input_image", "image_url": format!("data:image/png;base64,{}", "A".repeat(400_000)) }
                ]},
                { "type": "reasoning", "encrypted_content": "B".repeat(400_000) }
            ]
        });
        let tokens = text_bytes(&body) / 4;
        assert!((1_000..1_100).contains(&tokens), "{tokens}");
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    use crate::gateway::key_pool::{KeyLimits, KeyRef};

    /// 排队时客户端断开：等待马上结束，不再占着连接干等。
    #[tokio::test]
    async fn queue_notices_when_the_client_disconnects() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let client = tokio::net::TcpStream::connect(address).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let waiting = tokio::spawn(async move { client_gone(Some(&server)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "客户端还连着时一直等");
        drop(client);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("断开后马上发现")
            .unwrap();
    }

    #[test]
    fn timeout_message_names_the_provider_and_its_limits() {
        let candidate = GatewayCandidate {
            relay: crate::settings::RelayProfile {
                id: "slow".to_string(),
                name: "慢供应商".to_string(),
                ..Default::default()
            },
            key: KeyRef {
                relay_id: "slow".to_string(),
                key_id: "a".to_string(),
                fingerprint: 0,
            },
            upstream_model: "m".to_string(),
            window: None,
            limits: KeyLimits {
                rpm: Some(5),
                concurrency: Some(1),
            },
        };
        let second = GatewayCandidate {
            key: KeyRef {
                key_id: "b".to_string(),
                ..candidate.key.clone()
            },
            ..candidate.clone()
        };
        let error = LocalRateLimited::new(&[&candidate, &second], key_pool::LIMIT_QUEUE_TIMEOUT);
        assert_eq!(
            error.message.matches("「慢供应商」").count(),
            1,
            "同一家的多个 Key 只列一次：{}",
            error.message
        );
        assert!(error.message.contains("每个 Key 每分钟 5 次、同时 1 个"));
        assert!(error.message.contains("120 秒"));
        assert_eq!(
            queue_reason(Duration::from_millis(1_500)),
            "local_rate_limit_wait_ms:1500"
        );
    }
}
