//! 压缩模型（#3142）：Codex 的上下文压缩请求改发到绑定里单独指定的（通常更便宜、更快的）模型，
//! 普通对话、工具调用仍走主模型。绑定里没填时完全不介入。
//!
//! 第一版只在能安全换模型的形态生效，其余照原模型路由：
//! - Codex 的内联压缩（非 OpenAI 供应商时发的普通 `/responses` 摘要请求）：答复是普通文字摘要，
//!   和哪个模型、哪家无关，任意可路由的压缩模型都行。
//! - v2 远程压缩（input 里带 `compaction_trigger`）：Responses 原生上游会签发只有它自己读得懂的
//!   加密压缩项，所以压缩模型只能落到主模型同一家；Chat / Anthropic 上游由 Codex++ 合成本地摘要
//!   （明文、与模型无关），可以跨家。
//! - 旧版 `/responses/compact` 端点：答复整个是上游原生 checkpoint，不改。
//! - 待压缩的历史里已有上游签发的原生压缩项时，只能交给主模型同一家（别家读不了那段密文）。
//! - WS 续接（带 `previous_response_id`）的请求只有增量，必须回原连接，不改。
//!
//! 压缩模型的窗口装不下待压缩内容、路由不到、或上游拒绝 / 出错时，回退到原模型重发一次；
//! 用了哪个模型、为什么回退记在请求日志的「重试原因」里（[`MODEL_REASON_PREFIX`] / [`FALLBACK_REASON_PREFIX`]）。

use serde_json::Value;

use crate::gateway::pipeline::{self, GatewayOutcome, GatewayRequest, GatewayUpstream};
use crate::gateway::route::{self, GatewayCandidate, RoutePlan};
use crate::gateway::store::{AgentBinding, GatewaySettings};
use crate::settings::{BackendSettings, RelayProtocol};
use crate::tools::ToolId;

/// 请求日志「重试原因」：这次压缩改发到了压缩模型，`compaction_model:<模型>`。
pub const MODEL_REASON_PREFIX: &str = "compaction_model:";
/// 请求日志「重试原因」：配了压缩模型但回退到了原模型，`compaction_fallback:<原因>:<压缩模型>`。
pub const FALLBACK_REASON_PREFIX: &str = "compaction_fallback:";

/// Codex 的压缩请求形态（只列第一版支持改模型的两种）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionKind {
    /// 内联压缩：普通 `/responses` 摘要请求。
    Inline,
    /// v2 远程压缩：input 里带 `compaction_trigger`。
    RemoteV2,
}

/// 能改发到压缩模型的压缩请求是哪一种；不是压缩请求、或是第一版不支持的形态（旧版
/// `/responses/compact` 端点）时为 None。
pub fn compaction_kind(request: &Value, request_path: &str) -> Option<CompactionKind> {
    if crate::protocol_proxy::is_responses_compact_proxy_path(request_path) {
        return None;
    }
    if crate::protocol_proxy::request_has_compaction_trigger(request) {
        return Some(CompactionKind::RemoteV2);
    }
    crate::protocol_proxy::is_inline_compaction_request(request).then_some(CompactionKind::Inline)
}

/// 回退到原模型的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// 压缩模型在池里路由不到（供应商关了、模型没了）。
    Unavailable,
    /// 压缩形态和压缩模型所在的上游不兼容（见模块说明）。
    Incompatible,
    /// 压缩模型的上下文窗口装不下待压缩内容。
    Window,
    /// 压缩模型的上游拒绝或出错。
    Failed,
}

impl Fallback {
    pub fn label(self) -> &'static str {
        match self {
            Fallback::Unavailable => "unavailable",
            Fallback::Incompatible => "incompatible",
            Fallback::Window => "window",
            Fallback::Failed => "failed",
        }
    }
}

/// 改发到压缩模型时用的路由与请求体。
#[derive(Debug)]
pub struct CompactionRoute {
    pub model: String,
    pub plan: RoutePlan,
    pub body: Value,
}

#[derive(Debug)]
pub enum Decision {
    /// 没配压缩模型、不是（支持的）压缩请求，或压缩模型就是当前模型：照原样走。
    NotApplicable,
    /// 改发到压缩模型。
    Use(CompactionRoute),
    /// 配了压缩模型但这次不能用，直接走原模型。
    Fallback { model: String, reason: Fallback },
}

/// 绑定里设的压缩模型（去掉窗口后缀）；空串是跟随当前模型。
pub fn configured_model(binding: &AgentBinding) -> Option<String> {
    let (model, _) = crate::model_suffix::parse_model_suffix(binding.compaction_model.trim());
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

/// 这次请求要不要、能不能改发到压缩模型。`main_plan` 是按原模型解析出的路由。
pub fn decide(
    gateway: &GatewaySettings,
    agent: &ToolId,
    request: &Value,
    request_path: &str,
    main_plan: &RoutePlan,
    conversation: Option<&str>,
) -> Decision {
    let Some(binding) = gateway.active_binding(agent) else {
        return Decision::NotApplicable;
    };
    let Some(model) = configured_model(binding) else {
        return Decision::NotApplicable;
    };
    let Some(kind) = compaction_kind(request, request_path) else {
        return Decision::NotApplicable;
    };
    let requested = request.get("model").and_then(Value::as_str).unwrap_or("");
    let (requested, _) = crate::model_suffix::parse_model_suffix(requested.trim());
    if requested.eq_ignore_ascii_case(&model) || main_plan.model.eq_ignore_ascii_case(&model) {
        return Decision::NotApplicable;
    }
    let fallback = |reason| Decision::Fallback {
        model: model.clone(),
        reason,
    };
    // WS 续接只带增量，必须回原连接、原模型。
    let continuation = request
        .get("previous_response_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty())
        || crate::responses_websocket::pinned_relay().is_some();
    if continuation {
        return fallback(Fallback::Incompatible);
    }
    let declared = binding
        .declared_models()
        .iter()
        .any(|declared| declared.eq_ignore_ascii_case(&model));
    if !declared && !route::model_is_routable(gateway, binding, &model) {
        return fallback(Fallback::Unavailable);
    }
    let Ok(mut plan) = route::resolve_plan(gateway, agent, &model, conversation) else {
        return fallback(Fallback::Unavailable);
    };
    if plan.candidates.is_empty() && plan.waiting.is_empty() {
        return fallback(Fallback::Unavailable);
    }
    // 主模型路由里的供应商：同一家签发的原生压缩项它自己读得懂。
    let main_relays: Vec<&str> = main_plan
        .candidates
        .iter()
        .chain(main_plan.waiting.iter().map(|item| &item.candidate))
        .map(|candidate| candidate.relay.id.as_str())
        .collect();
    let upstream_state = crate::protocol_proxy::input_has_upstream_compaction(request);
    let compatible = |candidate: &GatewayCandidate| {
        if main_relays.contains(&candidate.relay.id.as_str()) {
            return true;
        }
        if upstream_state {
            return false;
        }
        match kind {
            CompactionKind::Inline => true,
            // Responses 原生上游会签发别家读不了的加密压缩项；Chat / Anthropic 上游合成明文摘要。
            CompactionKind::RemoteV2 => effective_protocol(candidate)
                .is_some_and(|protocol| protocol != RelayProtocol::Responses),
        }
    };
    plan.candidates.retain(|candidate| compatible(candidate));
    plan.waiting.retain(|item| compatible(&item.candidate));
    if plan.candidates.is_empty() && plan.waiting.is_empty() {
        return fallback(Fallback::Incompatible);
    }
    // 窗口装不下待压缩内容的候选去掉（没写窗口的当作装得下）。
    let tokens = pipeline::estimate_body_tokens(request);
    let fits =
        |candidate: &GatewayCandidate| candidate.window.is_none_or(|window| window >= tokens);
    plan.candidates.retain(|candidate| fits(candidate));
    plan.waiting.retain(|item| fits(&item.candidate));
    if plan.candidates.is_empty() && plan.waiting.is_empty() {
        return fallback(Fallback::Window);
    }
    let mut body = request.clone();
    body["model"] = Value::String(model.clone());
    let effort = binding.compaction_reasoning_effort.trim();
    if !effort.is_empty() {
        if !body.get("reasoning").is_some_and(Value::is_object) {
            body["reasoning"] = serde_json::json!({});
        }
        body["reasoning"]["effort"] = Value::String(effort.to_string());
    }
    Decision::Use(CompactionRoute { model, plan, body })
}

/// 候选实际用的协议：OpenCode Go / Zen 按模型换协议，其余就是供应商自己的。按模型找不到路由时为 None。
fn effective_protocol(candidate: &GatewayCandidate) -> Option<RelayProtocol> {
    let mut relay = candidate.relay.clone();
    crate::gateway::opencode::prepare(&mut relay, &candidate.upstream_model, None).ok()?;
    Some(relay.protocol)
}

/// 按原模型的路由发请求；是压缩请求且配了压缩模型时先改发压缩模型，失败回退原模型重发一次。
///
/// 第二项是要追加到请求日志「重试原因」里的说明（用了压缩模型 / 回退原因），没介入时为空。
pub async fn open(
    settings: &BackendSettings,
    gateway: &GatewaySettings,
    agent: &ToolId,
    main_plan: &RoutePlan,
    request: GatewayRequest<'_>,
    conversation: Option<&str>,
) -> anyhow::Result<(GatewayOutcome, Vec<String>)> {
    let decision = decide(
        gateway,
        agent,
        &request.responses_body,
        request.upstream_path,
        main_plan,
        conversation,
    );
    let (model, reason) = match decision {
        Decision::NotApplicable => {
            return Ok((
                pipeline::open(settings, main_plan, request).await?,
                Vec::new(),
            ));
        }
        Decision::Fallback { model, reason } => (model, reason),
        Decision::Use(route) => {
            let attempt = GatewayRequest {
                responses_body: route.body,
                anthropic: None,
                user_agent: request.user_agent,
                session_headers: request.session_headers,
                live_request_id: request.live_request_id,
                auto_retry: request.auto_retry,
                is_stream: request.is_stream,
                upstream_path: request.upstream_path,
                client: request.client,
                queue_timeout: request.queue_timeout,
            };
            let detail = match pipeline::open(settings, &route.plan, attempt).await {
                Ok(outcome) if outcome.upstream.is_success() => {
                    log(agent, &route.model, None, None);
                    return Ok((
                        outcome,
                        vec![format!("{MODEL_REASON_PREFIX}{}", route.model)],
                    ));
                }
                Ok(outcome) => match &outcome.upstream {
                    GatewayUpstream::Cooling(wait) => {
                        format!("上游都在冷却，约 {} 秒后恢复", wait.as_secs().max(1))
                    }
                    GatewayUpstream::Pipeline(response) => {
                        format!("上游返回 {}", response.status_code)
                    }
                    GatewayUpstream::Native(response) => {
                        format!("上游返回 {}", response.status_code)
                    }
                },
                Err(error) => crate::protocol_proxy::error_chain(&error),
            };
            log(agent, &route.model, Some(Fallback::Failed), Some(&detail));
            (route.model, Fallback::Failed)
        }
    };
    if reason != Fallback::Failed {
        log(agent, &model, Some(reason), None);
    }
    let outcome = pipeline::open(settings, main_plan, request).await?;
    Ok((outcome, vec![fallback_reason(reason, &model)]))
}

/// 回退说明：`compaction_fallback:<原因>:<压缩模型>`。
pub fn fallback_reason(reason: Fallback, model: &str) -> String {
    format!("{FALLBACK_REASON_PREFIX}{}:{model}", reason.label())
}

fn log(agent: &ToolId, model: &str, fallback: Option<Fallback>, detail: Option<&str>) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "gateway.compaction_model",
        serde_json::json!({
            "agent": agent.as_str(),
            "compactionModel": model,
            "fallback": fallback.map(Fallback::label),
            "detail": detail,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_supported_compaction_shapes_only() {
        let trigger = json!({"model":"m","input":[{"type":"compaction_trigger"}]});
        assert_eq!(
            compaction_kind(&trigger, "/responses"),
            Some(CompactionKind::RemoteV2)
        );
        assert_eq!(
            compaction_kind(&trigger, "/responses/compact"),
            None,
            "旧版端点不改"
        );
        let inline = json!({"model":"m","tools":[],"input":[{"type":"message","role":"user","content":[
            {"type":"input_text","text":crate::protocol_proxy::COMPACTION_SUMMARY_INSTRUCTION}
        ]}]});
        assert_eq!(
            compaction_kind(&inline, "/responses"),
            Some(CompactionKind::Inline)
        );
        let ordinary =
            json!({"model":"m","input":[{"type":"message","role":"user","content":"修一下登录"}]});
        assert_eq!(compaction_kind(&ordinary, "/responses"), None);
    }

    #[test]
    fn fallback_reason_carries_reason_and_model() {
        assert_eq!(
            fallback_reason(Fallback::Window, "cheap/flash"),
            "compaction_fallback:window:cheap/flash"
        );
    }
}
