//! Codex Responses ↔ Anthropic Messages 协议桥。
//!
//! 形态选择：以 Chat Completions 作中间态复用既有实现 ——
//! 请求方向 `Responses → Chat → Anthropic Messages`，
//! 响应方向 `Anthropic Messages → Chat → Responses`（非流式 JSON 与 SSE 同构）。
//!
//! 这样工具调用配对、命名空间工具、图片、思考内容、用量统计等映射全部沿用
//! `protocol_proxy` 里已有测试覆盖的那一套，本模块只负责 Chat ↔ Anthropic 这一段，
//! 出问题时可以单独对拍，不必重新验证整条链路。
//!
//! 已知边界：
//! - 不做「adaptive thinking / output_config.effort」这类新档位映射，只用经典
//!   `thinking: {type: enabled, budget_tokens}`；
//! - 思考块签名依赖 Responses 的 `reasoning.encrypted_content` 回放（见下），
//!   历史里没有签名时宁可不开启思考，也不发一个必然 400 的请求。

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};

use crate::protocol_proxy::ChatSseToResponsesConverter;

/// Anthropic Messages API 版本头，与官方 SDK 一致。
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Anthropic 的 `max_tokens` 是必填项；Codex 请求没带上限时用它兜底。
pub const ANTHROPIC_DEFAULT_MAX_TOKENS: u64 = 8192;
/// 认证字段：`x-api-key`（Anthropic 原生）或 `authorization`（多数中转网关）。
pub const ANTHROPIC_AUTH_HEADER_API_KEY: &str = "x-api-key";
pub const ANTHROPIC_AUTH_HEADER_BEARER: &str = "authorization";
/// 伪装 Claude Code 客户端时使用的 UA / 头 / 首行身份（取自官方 Claude CLI）。
pub const CLAUDE_CODE_USER_AGENT: &str = "claude-cli/1.0.119 (external, cli)";
pub const CLAUDE_CODE_BETA: &str = "claude-code-20250219";
pub const CLAUDE_CODE_SYSTEM_IDENTITY: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";
/// 1M 上下文 beta 头；模型声明窗口达到该阈值时自动附带。
pub const ANTHROPIC_BETA_CONTEXT_1M: &str = "context-1m-2025-08-07";
pub const CONTEXT_1M_THRESHOLD: u64 = 1_000_000;
/// 思考块签名的回放信封前缀：签名只能靠客户端回放，必须能与其他供应商的
/// 密文区分开（不同前缀的 encrypted_content 一律忽略）。
pub const ANTHROPIC_THINKING_PREFIX: &str = "codex-plus-anthropic-thinking-v1:";

/// 思考模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnthropicThinkingMode {
    /// 默认：新代模型把 Codex 的等级透传给 Anthropic，旧代回落经典预算。
    #[default]
    Auto,
    /// 强制等级透传（`thinking.type = adaptive` + `output_config.effort`）。
    Adaptive,
    /// 强制经典预算（`thinking.budget_tokens`）。
    Budget,
    /// 完全不发 `thinking`。
    Off,
}

impl AnthropicThinkingMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "adaptive" | "effort" | "level" => Self::Adaptive,
            "budget" | "tokens" => Self::Budget,
            "off" | "none" | "disabled" => Self::Off,
            _ => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Adaptive => "adaptive",
            Self::Budget => "budget",
            Self::Off => "off",
        }
    }
}

/// Anthropic 上游的请求选项，由 `RelayProfile` 推导。
#[derive(Debug, Clone)]
pub struct AnthropicRequestOptions {
    /// `max_tokens` 的兜底值。
    pub max_tokens: u64,
    /// 用户在供应商里显式配置了 `max_tokens`：此时它优先于请求自带值。
    pub force_max_tokens: bool,
    /// 注入提示缓存断点（`cache_control`）。
    pub prompt_cache: bool,
    /// 伪装 Claude Code 客户端（系统提示首行身份；请求头由协议层补）。
    pub impersonate_claude_code: bool,
    /// 思考模式。
    pub thinking_mode: AnthropicThinkingMode,
}

impl Default for AnthropicRequestOptions {
    fn default() -> Self {
        Self {
            max_tokens: ANTHROPIC_DEFAULT_MAX_TOKENS,
            force_max_tokens: false,
            prompt_cache: true,
            impersonate_claude_code: false,
            thinking_mode: AnthropicThinkingMode::Auto,
        }
    }
}

/// 从供应商配置推导请求选项。`anthropicMaxTokens` 非 0 视为用户显式指定，
/// 此时它优先于请求自带的上限。
pub fn request_options(profile: &crate::settings::RelayProfile) -> AnthropicRequestOptions {
    AnthropicRequestOptions {
        max_tokens: profile.anthropic_max_tokens(),
        force_max_tokens: profile.anthropic_max_tokens != 0,
        prompt_cache: profile.anthropic_prompt_cache,
        impersonate_claude_code: profile.anthropic_impersonate_claude_code,
        thinking_mode: profile.anthropic_thinking_mode(),
    }
}

/// 上游 Messages 端点。Base URL 允许写成服务根地址、`/v1`、或完整端点：
/// 末尾带 `#` 表示「原样使用，不要补版本前缀」（与 `chat_completions_url` 同一约定）。
pub fn anthropic_messages_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/messages") {
        return base.to_string();
    }
    if skip_version_prefix || base.to_ascii_lowercase().ends_with("/v1") {
        return format!("{base}/messages");
    }
    format!("{base}/v1/messages")
}

/// Responses 请求体 → Anthropic Messages 请求体。
///
/// 除结构转换外还负责三件协议专有的事：
/// 1. `reasoning.effort` → `thinking.budget_tokens`（并按 `max_tokens` 夹取）；
/// 2. 把历史里带签名的思考块（存在 `reasoning.encrypted_content`）原样放回助手消息；
/// 3. 提示缓存断点与 Claude Code 身份伪装（按选项）。
pub fn responses_to_anthropic_messages(
    body: Value,
    options: &AnthropicRequestOptions,
) -> anyhow::Result<Value> {
    let effort = body
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .map(str::to_string);
    let thinking_groups = signed_thinking_groups_from_responses_input(body.get("input"));
    let chat = crate::protocol_proxy::responses_to_chat_completions(body)?;
    let mut result = chat_completions_to_anthropic_messages(chat, options)?;

    // 1M 标记：模型名里的 `[1m]` 上游不认，剥掉交给协议层补 beta 头。
    if let Some(model) = result.get("model").and_then(Value::as_str) {
        let stripped = strip_context_marker(model);
        if stripped != model {
            result["model"] = json!(stripped);
        }
    }

    apply_thinking(&mut result, effort.as_deref(), &thinking_groups, options.thinking_mode);
    if options.prompt_cache {
        inject_prompt_cache(&mut result);
    }
    if options.impersonate_claude_code {
        prepend_claude_code_identity(&mut result);
    }
    Ok(result)
}

/// 剥掉模型名末尾的 `[1m]` 标记（大小写不敏感），返回新的模型名。
pub fn strip_context_marker(model: &str) -> String {
    let trimmed = model.trim();
    let lowered = trimmed.to_ascii_lowercase();
    if lowered.ends_with("[1m]") {
        return trimmed[..trimmed.len() - 4].trim().to_string();
    }
    trimmed.to_string()
}

/// Codex 的 `reasoning.effort` → Anthropic `thinking.budget_tokens`。
///
/// 档位表：`xhigh`/`max`/`ultra` 归到最深一档，未知档位返回 `None`
/// （宁可不发 thinking，也不猜一个会吞掉采样参数的配置）。
pub fn effort_to_thinking_budget(effort: &str) -> Option<u64> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => Some(2048),
        "medium" => Some(8192),
        "high" => Some(16384),
        "xhigh" | "max" | "ultra" => Some(24576),
        _ => None,
    }
}

/// Codex 的等级 → Anthropic adaptive thinking 的 `output_config.effort`。
///
/// Anthropic 只认 `low` / `medium` / `high` / `max` 四档，Codex 的扩展档位
/// （`xhigh` / `ultra`，以及网关常见的 `max`）一律归到最深一档——原样透传会被上游拒绝。
pub fn effort_to_anthropic_effort(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "max" | "ultra" => Some("max"),
        _ => None,
    }
}

/// 模型是否支持 adaptive thinking（等级透传）。
///
/// 只认新代及 4.6 以后的家族：旧代模型收到 `adaptive` 不会报错，而是**静默忽略**
/// （实测 haiku-4-5 / opus-4-5 就是这样），因此把旧代交给经典预算更可控。
pub fn model_supports_adaptive_thinking(model: &str) -> bool {
    let name = model.trim().to_ascii_lowercase();
    let name = name.rsplit('/').next().unwrap_or(name.as_str());
    let name = name.strip_prefix("claude-").unwrap_or(name);
    const FAMILIES: &[&str] = &[
        "opus-5", "sonnet-5", "haiku-5", "fable-5", "mythos-5", "opus-4-6", "sonnet-4-6",
        "haiku-4-6", "opus-4-7", "sonnet-4-7", "opus-4-8", "sonnet-4-8",
    ];
    FAMILIES.iter().any(|family| name.starts_with(family))
}

/// 是否显式关掉了思考。
fn effort_disables_thinking(effort: Option<&str>) -> bool {
    matches!(
        effort.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("none" | "off" | "disabled")
    )
}

/// 把签名思考块编码进 `reasoning.encrypted_content`。
///
/// 编码的是**整组**思考块（而不是单块）：Chat 中枢每一轮只会产出一个 reasoning
/// item，多块思考如果各自编码就会丢掉其余块，回放时缺块同样 400。
pub fn encode_thinking_blocks(blocks: &[Value]) -> Option<String> {
    let usable: Vec<&Value> = blocks.iter().filter(|block| is_signed_thinking(block)).collect();
    if usable.is_empty() {
        return None;
    }
    let bytes = serde_json::to_vec(&usable).ok()?;
    Some(format!(
        "{ANTHROPIC_THINKING_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// 解开 `encrypted_content` 里的思考块；不是本桥产生的密文一律返回空。
pub fn decode_thinking_blocks(encrypted_content: &str) -> Vec<Value> {
    let Some(encoded) = encrypted_content.strip_prefix(ANTHROPIC_THINKING_PREFIX) else {
        return Vec::new();
    };
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(encoded) else {
        return Vec::new();
    };
    let Ok(decoded) = serde_json::from_slice::<Value>(&bytes) else {
        return Vec::new();
    };
    // 兼容早期「单块对象」写法，并对每块重新校验签名，避免回放非法块。
    let blocks = match decoded {
        Value::Array(items) => items,
        value @ Value::Object(_) => vec![value],
        _ => Vec::new(),
    };
    blocks
        .into_iter()
        .filter(is_signed_thinking)
        .collect()
}

fn is_signed_thinking(block: &Value) -> bool {
    match block.get("type").and_then(Value::as_str) {
        Some("thinking") => block
            .get("signature")
            .and_then(Value::as_str)
            .is_some_and(|signature| !signature.trim().is_empty()),
        Some("redacted_thinking") => block
            .get("data")
            .and_then(Value::as_str)
            .is_some_and(|data| !data.trim().is_empty()),
        _ => false,
    }
}

/// 从 Responses 历史的 reasoning item 里按序取出签名思考块组。
fn signed_thinking_groups_from_responses_input(input: Option<&Value>) -> Vec<Vec<Value>> {
    let Some(items) = input.and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
        .filter_map(|item| item.get("encrypted_content").and_then(Value::as_str))
        .map(decode_thinking_blocks)
        .filter(|blocks| !blocks.is_empty())
        .collect()
}

/// 决定这一轮是否开启 extended thinking，并把签名块放回助手消息。
///
/// 安全约束：历史里已经有工具调用、却拿不到签名块时**不开启思考**——Anthropic 要求
/// 工具循环里最后一条助手消息带完整签名的思考块，缺签名整轮 400，宁可不思考。
fn apply_thinking(
    result: &mut Value,
    effort: Option<&str>,
    groups: &[Vec<Value>],
    mode: AnthropicThinkingMode,
) {
    if mode == AnthropicThinkingMode::Off || effort_disables_thinking(effort) {
        return;
    }
    let Some(effort) = effort else {
        return;
    };
    let model = result
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // auto：新代模型透传等级，其余回落经典预算；adaptive 强制透传。
    let use_adaptive = match mode {
        AnthropicThinkingMode::Adaptive => true,
        AnthropicThinkingMode::Budget => false,
        _ => model_supports_adaptive_thinking(&model),
    };
    let plan = if use_adaptive {
        effort_to_anthropic_effort(effort).map(ThinkingPlan::Adaptive)
    } else {
        let Some(budget) = effort_to_thinking_budget(effort) else {
            return;
        };
        let max_tokens = result
            .get("max_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS);
        // Anthropic 要求 max_tokens > budget_tokens，且给可见回答留一半余量。
        let clamped = budget.min(max_tokens / 2);
        (clamped >= 1024).then_some(ThinkingPlan::Budget(clamped))
    };
    let Some(plan) = plan else {
        return;
    };
    let attached = attach_thinking_groups(result, groups);
    if !attached && has_tool_use_history(result) {
        return;
    }
    match plan {
        // 等级透传：预算由模型自己决定，不再做 max_tokens/2 压制。
        ThinkingPlan::Adaptive(level) => {
            result["thinking"] = json!({ "type": "adaptive" });
            result["output_config"] = json!({ "effort": level });
        }
        ThinkingPlan::Budget(budget) => {
            result["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        }
    }
    // 开启思考时 Anthropic 不接受 temperature / top_p 改动。
    if let Some(object) = result.as_object_mut() {
        object.remove("temperature");
        object.remove("top_p");
    }
}

/// 这一轮准备怎么开思考。
enum ThinkingPlan {
    /// `thinking.type = adaptive` + `output_config.effort`。
    Adaptive(&'static str),
    /// `thinking.type = enabled` + `budget_tokens`。
    Budget(u64),
}

fn has_tool_use_history(result: &Value) -> bool {
    result
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|blocks| {
                        blocks
                            .iter()
                            .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
                    })
            })
        })
}

/// 按序把签名思考块插到「带 tool_use 的助手消息」最前面。返回是否放回了至少一组。
fn attach_thinking_groups(result: &mut Value, groups: &[Vec<Value>]) -> bool {
    if groups.is_empty() {
        return false;
    }
    let Some(messages) = result.get_mut("messages").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut next_group = 0usize;
    let mut attached = false;
    for message in messages.iter_mut() {
        if next_group >= groups.len() {
            break;
        }
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_tool_use = message
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))
            });
        if !has_tool_use {
            continue;
        }
        let group = &groups[next_group];
        if let Some(content) = message.get_mut("content").and_then(Value::as_array_mut) {
            // 思考块必须在助手消息最前面（Anthropic 的校验要求）。
            let mut merged = group.clone();
            merged.append(content);
            *content = merged;
            next_group += 1;
            attached = true;
        }
    }
    attached
}

/// Anthropic 响应里的签名思考块（供 Responses 侧回放）。
pub fn thinking_blocks_from_anthropic_message(message: &Value) -> Vec<Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter(|block| is_signed_thinking(block))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 把签名思考块挂到 Responses 输出的 reasoning item 上。
///
/// Chat 中枢每轮只产出一个 reasoning item，因此整组签名共用一个信封；没有
/// reasoning item（例如思考文本为空）时补一个，否则 Codex 下一轮回放不出签名。
pub fn attach_thinking_to_responses(response: &mut Value, blocks: &[Value]) {
    let Some(encrypted) = encode_thinking_blocks(blocks) else {
        return;
    };
    let summary_text: String = blocks
        .iter()
        .filter_map(|block| block.get("thinking").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    let response_id = response
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("resp")
        .to_string();
    let Some(output) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(item) = output
        .iter_mut()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("reasoning"))
    {
        item["encrypted_content"] = json!(encrypted);
        return;
    }
    output.insert(
        0,
        json!({
            "id": format!("rs_{response_id}"),
            "type": "reasoning",
            "summary": if summary_text.is_empty() {
                json!([])
            } else {
                json!([{ "type": "summary_text", "text": summary_text }])
            },
            "encrypted_content": encrypted
        }),
    );
}

/// 注入提示缓存断点（最多 4 个，已有标记一律不动）。
///
/// 注入顺序：工具定义末尾 → 系统提示末尾 → 最新可缓存消息末尾 →
/// 更早的一条 user 锚点，让长工具循环里的稳定前缀也能命中 5 分钟缓存。
pub fn inject_prompt_cache(result: &mut Value) {
    const MAX_BREAKPOINTS: usize = 4;
    let existing = count_cache_breakpoints(result);
    if existing >= MAX_BREAKPOINTS {
        return;
    }
    let mut budget = MAX_BREAKPOINTS - existing;

    if budget > 0 {
        if let Some(last) = result
            .get_mut("tools")
            .and_then(Value::as_array_mut)
            .and_then(|tools| tools.last_mut())
        {
            if last.get("cache_control").is_none() {
                if let Some(object) = last.as_object_mut() {
                    object.insert("cache_control".to_string(), cache_control());
                    budget -= 1;
                }
            }
        }
    }

    if budget > 0 {
        // 字符串 system 必须先转成块数组才能挂 cache_control。
        if let Some(text) = result.get("system").and_then(Value::as_str).map(str::to_string) {
            result["system"] = json!([{ "type": "text", "text": text }]);
        }
        if let Some(last) = result
            .get_mut("system")
            .and_then(Value::as_array_mut)
            .and_then(|blocks| blocks.last_mut())
        {
            if last.get("cache_control").is_none() {
                if let Some(object) = last.as_object_mut() {
                    object.insert("cache_control".to_string(), cache_control());
                    budget -= 1;
                }
            }
        }
    }

    if budget > 0 {
        let Some(messages) = result.get_mut("messages").and_then(Value::as_array_mut) else {
            return;
        };
        for message in messages.iter_mut().rev() {
            if inject_message_breakpoint(message) {
                budget -= 1;
                break;
            }
        }
        // 长工具循环里最新断点可能离稳定前缀超过 20 个块，再补一个更早的 user 锚点。
        if budget > 0 && messages.len() >= 4 {
            let mut user_count = 0;
            for message in messages.iter_mut().rev() {
                if message.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                user_count += 1;
                if user_count == 2 {
                    let _ = inject_message_breakpoint(message);
                    break;
                }
            }
        }
    }
}

fn cache_control() -> Value {
    json!({ "type": "ephemeral" })
}

fn count_cache_breakpoints(value: &Value) -> usize {
    fn count(node: &Value) -> usize {
        match node {
            Value::Array(items) => items.iter().map(count).sum(),
            Value::Object(object) => {
                let own = usize::from(object.contains_key("cache_control"));
                own + object.values().map(count).sum::<usize>()
            }
            _ => 0,
        }
    }
    count(value)
}

/// 给消息最后一个非 thinking 内容块挂断点（工具循环通常以 user/tool_result 结束）。
fn inject_message_breakpoint(message: &mut Value) -> bool {
    let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return false;
    };
    for block in blocks.iter_mut().rev() {
        let block_type = block.get("type").and_then(Value::as_str).unwrap_or("");
        if matches!(block_type, "thinking" | "redacted_thinking") {
            continue;
        }
        if block.get("cache_control").is_some() {
            return false;
        }
        let Some(object) = block.as_object_mut() else {
            return false;
        };
        object.insert("cache_control".to_string(), cache_control());
        return true;
    }
    false
}

/// 伪装 Claude Code：系统提示首行必须是官方身份串（幂等）。
fn prepend_claude_code_identity(result: &mut Value) {
    let identity = json!({ "type": "text", "text": CLAUDE_CODE_SYSTEM_IDENTITY });
    let mut blocks: Vec<Value> = vec![identity];
    match result.get("system") {
        Some(Value::String(existing)) if !existing.is_empty() => {
            blocks.push(json!({ "type": "text", "text": existing }));
        }
        Some(Value::Array(existing)) => {
            if existing
                .first()
                .and_then(|block| block.get("text"))
                .and_then(Value::as_str)
                == Some(CLAUDE_CODE_SYSTEM_IDENTITY)
            {
                return;
            }
            blocks.extend(existing.iter().cloned());
        }
        _ => {}
    }
    result["system"] = Value::Array(blocks);
}

/// 上游 `anthropic-beta` 头取值：伪装身份与 1M 上下文按需组合，空则不发。
pub fn anthropic_beta_header(impersonate_claude_code: bool, context_1m: bool) -> Option<String> {
    let mut betas: Vec<&str> = Vec::new();
    if impersonate_claude_code {
        betas.push(CLAUDE_CODE_BETA);
    }
    if context_1m {
        betas.push(ANTHROPIC_BETA_CONTEXT_1M);
    }
    (!betas.is_empty()).then(|| betas.join(","))
}

/// 该模型是否声明了 ≥1M 的上下文窗口（决定要不要发 1M beta 头）。
///
/// 只认本仓已有的窗口声明机制：`modelWindows` 里该模型的显式窗口，其次是
/// profile 级 `contextWindow`。
pub fn model_declares_context_1m(
    profile: &crate::settings::RelayProfile,
    model: &str,
) -> bool {
    let model = model.trim();
    if model.is_empty() {
        return false;
    }
    let declared = serde_json::from_str::<Value>(profile.model_windows.trim())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .and_then(|map| {
            map.get(model)
                .or_else(|| map.get(model.rsplit('/').next().unwrap_or(model)))
                .cloned()
        })
        .and_then(|value| parse_declared_window(&value))
        .or_else(|| parse_declared_window(&Value::String(profile.context_window.clone())));
    declared.is_some_and(|window| window >= CONTEXT_1M_THRESHOLD)
}

fn parse_declared_window(value: &Value) -> Option<u64> {
    let raw = match value {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => crate::model_suffix::parse_model_suffix(&format!("x[{text}]")).1?,
        _ => return None,
    };
    (raw > 0).then_some(raw)
}

/// Chat Completions 请求体 → Anthropic Messages 请求体。
pub fn chat_completions_to_anthropic_messages(
    chat: Value,
    options: &AnthropicRequestOptions,
) -> anyhow::Result<Value> {
    let mut out = json!({});

    if let Some(model) = chat.get("model") {
        out["model"] = model.clone();
    }

    // Anthropic 必填项。优先级：供应商里显式配置的值 > 请求自带值
    // > 内置兜底——用户在表单里填了值就说明他知道真实上限，不该被请求悄悄盖掉。
    let requested = chat
        .get("max_tokens")
        .or_else(|| chat.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0);
    let max_tokens = if options.force_max_tokens {
        options.max_tokens
    } else {
        requested.unwrap_or(options.max_tokens)
    }
    .max(1);
    out["max_tokens"] = json!(max_tokens);

    if chat.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        out["stream"] = json!(true);
    }
    if let Some(temperature) = chat.get("temperature").and_then(Value::as_f64) {
        // Anthropic 的 temperature 上限是 1.0（OpenAI 允许到 2.0）。
        out["temperature"] = json!(temperature.clamp(0.0, 1.0));
    }
    if let Some(top_p) = chat.get("top_p") {
        out["top_p"] = top_p.clone();
    }
    if let Some(stop) = chat.get("stop") {
        let sequences: Vec<Value> = match stop {
            Value::String(text) => vec![json!(text)],
            Value::Array(items) => items.clone(),
            _ => Vec::new(),
        };
        if !sequences.is_empty() {
            out["stop_sequences"] = json!(sequences);
        }
    }

    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    let chat_messages = chat
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for message in &chat_messages {
        match message.get("role").and_then(Value::as_str).unwrap_or("") {
            // chat 侧已把 system 收敛到队首，这里再合并一次以防手写请求分散在多处。
            "system" | "developer" => {
                let text = content_to_text(message.get("content"));
                if !text.is_empty() {
                    system_parts.push(text);
                }
            }
            "assistant" => {
                if let Some(converted) = chat_assistant_to_anthropic(message) {
                    push_anthropic_message(&mut messages, converted);
                }
            }
            "tool" => {
                if let Some(converted) = chat_tool_message_to_anthropic(message) {
                    push_anthropic_message(&mut messages, converted);
                }
            }
            _ => {
                let blocks = chat_content_to_anthropic_blocks(message.get("content"));
                if !blocks.is_empty() {
                    push_anthropic_message(
                        &mut messages,
                        json!({ "role": "user", "content": blocks }),
                    );
                }
            }
        }
    }

    if !system_parts.is_empty() {
        out["system"] = json!(system_parts.join("\n\n"));
    }

    // Anthropic 要求 messages 非空且首条为 user。
    if messages.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": [{ "type": "text", "text": "(continuing the conversation)" }]
        }));
    } else if messages[0].get("role").and_then(Value::as_str) == Some("assistant") {
        messages.insert(
            0,
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "(continuing the conversation)" }]
            }),
        );
    }
    out["messages"] = json!(messages);

    let mut has_tools = false;
    if let Some(tools) = chat.get("tools").and_then(Value::as_array) {
        let converted: Vec<Value> = tools.iter().filter_map(chat_tool_to_anthropic).collect();
        if !converted.is_empty() {
            has_tools = true;
            out["tools"] = json!(converted);
        }
    }
    // Anthropic 在 tools 为空时拒绝 tool_choice。
    if has_tools {
        if let Some(choice) = chat
            .get("tool_choice")
            .and_then(chat_tool_choice_to_anthropic)
        {
            out["tool_choice"] = choice;
        }
        if chat.get("parallel_tool_calls").and_then(Value::as_bool) == Some(false) {
            if let Some(choice) = out.get_mut("tool_choice").and_then(Value::as_object_mut) {
                choice.insert("disable_parallel_tool_use".to_string(), json!(true));
            }
        }
    }

    Ok(out)
}

/// Anthropic Messages 响应体 → Chat Completions 响应体。
///
/// 转换结果继续交给 `protocol_proxy::chat_completion_to_response_with_request`，
/// 工具调用 / 思考内容 / 用量的 Responses 形态映射全部复用那条链路。
pub fn anthropic_message_to_chat_completion(message: Value) -> anyhow::Result<Value> {
    let id = message
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("msg_anthropic")
        .to_string();
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let created = message
        .get("created")
        .and_then(Value::as_u64)
        .unwrap_or_else(now_unix_seconds);

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls: Vec<Value> = Vec::new();
    for block in message
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        match block.get("type").and_then(Value::as_str).unwrap_or("") {
            "text" => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push_str(part);
                }
            }
            "thinking" | "redacted_thinking" => {
                if let Some(part) = block
                    .get("thinking")
                    .or_else(|| block.get("data"))
                    .and_then(Value::as_str)
                {
                    reasoning.push_str(part);
                }
            }
            "tool_use" => {
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| format!("toolu_{}", tool_calls.len()));
                let arguments = block
                    .get("input")
                    .map(|input| serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()))
                    .unwrap_or_else(|| "{}".to_string());
                tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": { "name": name, "arguments": arguments }
                }));
            }
            _ => {}
        }
    }

    let mut message_object = json!({ "role": "assistant" });
    if text.is_empty() {
        message_object["content"] = Value::Null;
    } else {
        message_object["content"] = json!(text);
    }
    if !reasoning.is_empty() {
        message_object["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message_object["tool_calls"] = json!(tool_calls);
    }

    let stop_reason = message.get("stop_reason").and_then(Value::as_str);
    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message_object,
            "finish_reason": anthropic_stop_reason_to_finish_reason(stop_reason),
        }],
        "usage": anthropic_usage_to_chat_usage(message.get("usage")),
    }))
}

fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn anthropic_stop_reason_to_finish_reason(stop_reason: Option<&str>) -> &'static str {
    match stop_reason.unwrap_or("") {
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

/// Anthropic usage → chat usage。
///
/// `chat_usage_to_responses_usage` 本身就认识 Anthropic 的
/// `input_tokens` / `output_tokens` / `cache_read_input_tokens`，这里只把
/// `cache_creation` 明细摊平并补上 `total_tokens`，让用量看板能正确计价。
fn anthropic_usage_to_chat_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object()) else {
        return Value::Null;
    };
    let mut out = usage.clone();
    if let Some(creation) = usage.get("cache_creation") {
        if let Some(value) = creation.get("ephemeral_5m_input_tokens") {
            out["cache_creation_5m_input_tokens"] = value.clone();
        }
        if let Some(value) = creation.get("ephemeral_1h_input_tokens") {
            out["cache_creation_1h_input_tokens"] = value.clone();
        }
    }
    if out.get("total_tokens").is_none() {
        let number = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        // Anthropic 的 `cache_creation_input_tokens` 是缓存写入总量，
        // `cache_creation` 里的 5m/1h 只是它的拆分；缺总量时用拆分求和兜底，
        // 两者都存在时不能相加，否则缓存 token 会被算两遍。
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| {
                let ephemeral = |key: &str| {
                    usage
                        .pointer(&format!("/cache_creation/{key}"))
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                };
                ephemeral("ephemeral_5m_input_tokens") + ephemeral("ephemeral_1h_input_tokens")
            });
        out["total_tokens"] = json!(
            number("input_tokens")
                + number("output_tokens")
                + number("cache_read_input_tokens")
                + cache_creation
        );
    }
    out
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.clone()),
                Value::Object(_) => part.get("text").and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn chat_content_to_anthropic_blocks(content: Option<&Value>) -> Vec<Value> {
    match content {
        Some(Value::String(text)) => {
            if text.is_empty() {
                Vec::new()
            } else {
                vec![json!({ "type": "text", "text": text })]
            }
        }
        Some(Value::Array(parts)) => parts.iter().filter_map(chat_part_to_anthropic_block).collect(),
        _ => Vec::new(),
    }
}

fn chat_part_to_anthropic_block(part: &Value) -> Option<Value> {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
    match part_type {
        "text" | "input_text" | "output_text" => part
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| json!({ "type": "text", "text": text })),
        "image_url" | "input_image" => {
            let raw = part.get("image_url")?;
            let url = raw
                .as_str()
                .or_else(|| raw.get("url").and_then(Value::as_str))?;
            anthropic_image_block(url)
        }
        // 上游自定义部件（如 VLM 分析结果）统一降级成文本，不能整轮丢弃。
        _ => part
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(|text| json!({ "type": "text", "text": text })),
    }
}

/// data URL → base64 源；http(s) URL → url 源。其余形态无法表达，直接丢弃。
fn anthropic_image_block(url: &str) -> Option<Value> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, data) = rest.split_once(',')?;
        let lowered = meta.to_ascii_lowercase();
        if !lowered.contains("base64") {
            return None;
        }
        let media_type = meta.split(';').next().unwrap_or("").trim();
        let media_type = if media_type.is_empty() {
            "image/png"
        } else {
            media_type
        };
        return Some(json!({
            "type": "image",
            "source": { "type": "base64", "media_type": media_type, "data": data }
        }));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(json!({
            "type": "image",
            "source": { "type": "url", "url": url }
        }));
    }
    None
}

fn chat_assistant_to_anthropic(message: &Value) -> Option<Value> {
    let mut blocks = chat_content_to_anthropic_blocks(message.get("content"));
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            let function = tool_call.get("function").unwrap_or(tool_call);
            let name = function
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let call_id = tool_call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| format!("toolu_{index}"));
            let input = tool_call_arguments_to_value(function.get("arguments"));
            blocks.push(json!({
                "type": "tool_use",
                "id": call_id,
                "name": name,
                "input": input
            }));
        }
    }
    if blocks.is_empty() {
        return None;
    }
    Some(json!({ "role": "assistant", "content": blocks }))
}

fn tool_call_arguments_to_value(arguments: Option<&Value>) -> Value {
    match arguments {
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                json!({})
            } else {
                serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| json!({}))
            }
        }
        Some(value) if value.is_object() => value.clone(),
        _ => json!({}),
    }
}

fn chat_tool_message_to_anthropic(message: &Value) -> Option<Value> {
    let tool_use_id = message
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if tool_use_id.is_empty() {
        return None;
    }
    let content = match message.get("content") {
        Some(Value::String(text)) => Value::String(text.clone()),
        Some(Value::Array(parts)) => {
            let blocks: Vec<Value> = parts.iter().filter_map(chat_part_to_anthropic_block).collect();
            if blocks.is_empty() {
                Value::String(String::new())
            } else {
                Value::Array(blocks)
            }
        }
        _ => Value::String(String::new()),
    };
    Some(json!({
        "role": "user",
        "content": [{
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content
        }]
    }))
}

/// Anthropic 要求同角色消息连续出现时合并（tool_result 也必须和相邻 user 消息同组）。
fn push_anthropic_message(messages: &mut Vec<Value>, message: Value) {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_string();
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some(role.as_str()) {
            let incoming = message
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
                content.extend(incoming);
                return;
            }
        }
    }
    messages.push(message);
}

fn chat_tool_to_anthropic(tool: &Value) -> Option<Value> {
    let function = tool.get("function").unwrap_or(tool);
    let name = function.get("name").and_then(Value::as_str)?.trim();
    if name.is_empty() {
        return None;
    }
    let mut out = json!({ "name": name });
    if let Some(description) = function.get("description").and_then(Value::as_str) {
        if !description.trim().is_empty() {
            out["description"] = json!(description);
        }
    }
    let mut schema = function
        .get("parameters")
        .or_else(|| function.get("input_schema"))
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    if schema.get("type").is_none() {
        schema["type"] = json!("object");
    }
    out["input_schema"] = schema;
    Some(out)
}

fn chat_tool_choice_to_anthropic(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(value) => match value.trim() {
            "auto" => Some(json!({ "type": "auto" })),
            "none" => Some(json!({ "type": "none" })),
            "required" | "any" => Some(json!({ "type": "any" })),
            _ => None,
        },
        Value::Object(_) => {
            let name = choice
                .pointer("/function/name")
                .or_else(|| choice.get("name"))
                .and_then(Value::as_str)?
                .trim();
            (!name.is_empty()).then(|| json!({ "type": "tool", "name": name }))
        }
        _ => None,
    }
}

// ============================================================================
// SSE：Anthropic 事件流 → Chat Completions 事件流 → Responses 事件流
// ============================================================================

/// 有状态地把 Anthropic SSE 事件流翻译成 Chat Completions SSE 分片。
///
/// 只做「事件 → chat chunk」的机械映射，终止语义（finish_reason / [DONE]）保持
/// 与 chat 上游一致，后续交给 `ChatSseToResponsesConverter` 产出 Responses 事件。
pub struct AnthropicSseToChatSse {
    buffer: String,
    utf8_remainder: Vec<u8>,
    message_id: String,
    model: String,
    created: u64,
    /// Anthropic 的 content block 下标 → chat 侧的 tool_calls 下标。
    tool_index_by_block: BTreeMap<u64, u64>,
    next_tool_index: u64,
    stopped: bool,
    failed: bool,
    compaction_stop_reason: bool,
    message_stopped: bool,
    compaction_stream_valid: bool,
    /// 正在累积的思考块（text + signature），content_block_stop 时收口。
    pending_thinking: Option<PendingThinking>,
    /// 已收口、带签名的思考块，供 Responses 侧回放。
    thinking_blocks: Vec<Value>,
}

#[derive(Default)]
struct PendingThinking {
    text: String,
    signature: Option<String>,
    redacted: Option<String>,
}

impl PendingThinking {
    fn into_block(self) -> Option<Value> {
        if let Some(data) = self.redacted.filter(|data| !data.trim().is_empty()) {
            return Some(json!({ "type": "redacted_thinking", "data": data }));
        }
        let signature = self.signature.filter(|value| !value.trim().is_empty())?;
        Some(json!({
            "type": "thinking",
            "thinking": self.text,
            "signature": signature
        }))
    }
}

impl Default for AnthropicSseToChatSse {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            message_id: "msg_anthropic".to_string(),
            model: String::new(),
            created: now_unix_seconds(),
            tool_index_by_block: BTreeMap::new(),
            next_tool_index: 0,
            stopped: false,
            failed: false,
            compaction_stop_reason: false,
            message_stopped: false,
            compaction_stream_valid: true,
            pending_thinking: None,
            thinking_blocks: Vec::new(),
        }
    }
}

impl AnthropicSseToChatSse {
    /// 摘要必须满足原始 Anthropic 完成契约，不能只信转换后的 finish_reason。
    pub(crate) fn has_successful_compaction_completion(&self) -> bool {
        self.message_stopped && self.compaction_stop_reason && !self.failed
            && self.compaction_stream_valid && self.utf8_remainder.is_empty()
            && self.buffer.lines().all(|line| line.trim().is_empty() || line.starts_with(':'))
    }

    pub fn has_failed(&self) -> bool {
        self.failed
    }

    /// 取出已收口的签名思考块（供 Responses 侧写进 encrypted_content）。
    pub fn take_thinking_blocks(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.thinking_blocks)
    }

    /// 只有收到 `message_stop`（或错误事件）才算正常终止，用于识别被掐断的流。
    pub fn has_terminal_event(&self) -> bool {
        self.stopped || self.failed
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut utf8 = self.utf8_remainder.clone();
        utf8.extend_from_slice(bytes);
        if std::str::from_utf8(&utf8).is_err_and(|error| error.error_len().is_some()) {
            self.compaction_stream_valid = false;
        }
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_block(&block, &mut output);
            if self.failed {
                break;
            }
        }
        output.into_bytes()
    }

    /// 上游流结束时调用。未见到 `message_stop` 时不补 `[DONE]`，
    /// 让上层按「流被掐断」处理，避免把不完整回答伪装成成功。
    pub fn finish(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        Vec::new()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        let mut event_name: Option<String> = None;
        let mut data_parts: Vec<String> = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = Some(event.trim().to_string());
            }
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data.to_string());
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data_text = data_parts.join("\n");
        if data_text.trim() == "[DONE]" {
            self.emit_done(output);
            return;
        }
        let Ok(data) = serde_json::from_str::<Value>(&data_text) else {
            self.compaction_stream_valid = false;
            return;
        };
        // 部分网关不写 `event:` 行，只把类型放在 JSON 的 `type` 字段里。
        let event = event_name
            .or_else(|| data.get("type").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default();
        self.handle_event(event.trim(), &data, output);
    }

    fn handle_event(&mut self, event: &str, data: &Value, output: &mut String) {
        match event {
            "message_start" => {
                if let Some(message) = data.get("message") {
                    if let Some(id) = message.get("id").and_then(Value::as_str) {
                        if !id.trim().is_empty() {
                            self.message_id = id.to_string();
                        }
                    }
                    if let Some(model) = message.get("model").and_then(Value::as_str) {
                        if !model.trim().is_empty() {
                            self.model = model.to_string();
                        }
                    }
                    let usage = message.get("usage").map(|usage| anthropic_usage_to_chat_usage(Some(usage)));
                    let chunk = self.base_chunk(json!({ "role": "assistant" }), None, usage);
                    emit_chat_chunk(output, &chunk);
                }
            }
            "content_block_start" => {
                let block = data.get("content_block").cloned().unwrap_or(Value::Null);
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "thinking" => {
                        self.finalize_pending_thinking();
                        self.pending_thinking = Some(PendingThinking::default());
                        return;
                    }
                    "redacted_thinking" => {
                        self.finalize_pending_thinking();
                        self.pending_thinking = Some(PendingThinking {
                            redacted: block
                                .get("data")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            ..PendingThinking::default()
                        });
                        return;
                    }
                    "tool_use" => {}
                    _ => {
                        self.finalize_pending_thinking();
                        return;
                    }
                }
                let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let tool_index = self.next_tool_index;
                self.next_tool_index += 1;
                self.tool_index_by_block.insert(block_index, tool_index);
                let call_id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let chunk = self.base_chunk(
                    json!({ "tool_calls": [{
                        "index": tool_index,
                        "id": call_id,
                        "type": "function",
                        "function": { "name": name, "arguments": "" }
                    }] }),
                    None,
                    None,
                );
                emit_chat_chunk(output, &chunk);
            }
            "content_block_delta" => {
                let block_index = data.get("index").and_then(Value::as_u64).unwrap_or(0);
                let delta = data.get("delta").cloned().unwrap_or(Value::Null);
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str) {
                            if !text.is_empty() {
                                let chunk = self.base_chunk(json!({ "content": text }), None, None);
                                emit_chat_chunk(output, &chunk);
                            }
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str) {
                            if let Some(pending) = self.pending_thinking.as_mut() {
                                pending.text.push_str(text);
                            }
                            if !text.is_empty() {
                                let chunk = self
                                    .base_chunk(json!({ "reasoning_content": text }), None, None);
                                emit_chat_chunk(output, &chunk);
                            }
                        }
                    }
                    "signature_delta" => {
                        if let Some(signature) = delta.get("signature").and_then(Value::as_str) {
                            if let Some(pending) = self.pending_thinking.as_mut() {
                                pending.signature = Some(signature.to_string());
                            }
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if partial.is_empty() {
                            return;
                        }
                        let tool_index = self
                            .tool_index_by_block
                            .get(&block_index)
                            .copied()
                            .unwrap_or(block_index);
                        let chunk = self.base_chunk(
                            json!({ "tool_calls": [{
                                "index": tool_index,
                                "function": { "arguments": partial }
                            }] }),
                            None,
                            None,
                        );
                        emit_chat_chunk(output, &chunk);
                    }
                    _ => {}
                }
            }
            "content_block_stop" => self.finalize_pending_thinking(),
            "message_delta" => {
                let stop_reason = data.pointer("/delta/stop_reason").and_then(Value::as_str);
                self.compaction_stop_reason = matches!(stop_reason, Some("end_turn" | "stop_sequence"));
                let finish_reason = anthropic_stop_reason_to_finish_reason(stop_reason);
                let usage = data
                    .get("usage")
                    .map(|usage| anthropic_usage_to_chat_usage(Some(usage)));
                let chunk = self.base_chunk(json!({}), Some(finish_reason), usage);
                emit_chat_chunk(output, &chunk);
            }
            "message_stop" => {
                self.message_stopped = true;
                self.emit_done(output);
            }
            "error" => {
                let error = data
                    .get("error")
                    .cloned()
                    .unwrap_or_else(|| json!({ "message": "上游返回错误事件" }));
                emit_chat_chunk(output, &json!({ "error": error }));
                self.failed = true;
            }
            // ping / content_block_stop / signature_delta 等无对应 chat 语义。
            _ => {}
        }
    }

    fn emit_done(&mut self, output: &mut String) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        output.push_str("data: [DONE]\n\n");
    }

    /// 收口当前思考块：带签名的进回放列表，没签名的直接丢（回放非法块会 400）。
    fn finalize_pending_thinking(&mut self) {
        if let Some(pending) = self.pending_thinking.take() {
            if let Some(block) = pending.into_block() {
                self.thinking_blocks.push(block);
            }
        }
    }

    fn base_chunk(
        &self,
        delta: Value,
        finish_reason: Option<&str>,
        usage: Option<Value>,
    ) -> Value {
        let mut chunk = json!({
            "id": self.message_id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{
                "index": 0,
                "delta": delta,
                "finish_reason": finish_reason,
            }],
        });
        if let Some(usage) = usage.filter(|value| !value.is_null()) {
            chunk["usage"] = usage;
        }
        chunk
    }
}

/// Anthropic SSE → Responses SSE 的有状态转换器。
pub struct AnthropicSseToResponsesConverter {
    bridge: AnthropicSseToChatSse,
    chat: ChatSseToResponsesConverter,
    patcher: ReasoningItemPatcher,
}

impl AnthropicSseToResponsesConverter {
    pub fn with_request(request: &Value) -> Self {
        Self {
            bridge: AnthropicSseToChatSse::default(),
            chat: ChatSseToResponsesConverter::with_request(request),
            patcher: ReasoningItemPatcher::default(),
        }
    }

    pub fn has_failed(&self) -> bool {
        self.chat.has_failed()
    }

    pub fn has_terminal_event(&self) -> bool {
        self.chat.has_terminal_event()
    }

    /// 本次请求的用量（请求日志 / 网关用量统计用）。
    pub(crate) fn request_usage(&self) -> Option<crate::request_log::RequestUsage> {
        self.chat.request_usage()
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        let chat_bytes = self.bridge.push_bytes(bytes);
        if chat_bytes.is_empty() {
            return Vec::new();
        }
        let responses_bytes = self.chat.push_bytes(&chat_bytes);
        self.patch(&responses_bytes)
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let tail = self.bridge.finish();
        let mut output = if tail.is_empty() {
            Vec::new()
        } else {
            self.chat.push_bytes(&tail)
        };
        output.extend(self.chat.finish());
        let mut patched = self.patch(&output);
        patched.extend(self.patcher.flush());
        patched
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        self.chat.fail(message, error_type)
    }

    /// 把已收口的签名思考块补进 Responses 事件里的 reasoning item。
    fn patch(&mut self, responses_bytes: &[u8]) -> Vec<u8> {
        let blocks = self.bridge.take_thinking_blocks();
        if !blocks.is_empty() {
            self.patcher.add_blocks(blocks);
        }
        if self.patcher.is_idle() {
            return responses_bytes.to_vec();
        }
        self.patcher.push_bytes(responses_bytes)
    }
}

impl Default for AnthropicSseToResponsesConverter {
    fn default() -> Self {
        Self {
            bridge: AnthropicSseToChatSse::default(),
            chat: ChatSseToResponsesConverter::default(),
            patcher: ReasoningItemPatcher::default(),
        }
    }
}

/// 给 Responses SSE 里还没带 `encrypted_content` 的 reasoning item 补签名信封。
///
/// 只在 `response.output_item.done` 与 `response.completed` 上打补丁：
/// `output_item.added` 发得比 signature_delta 早，那时签名还没到。
#[derive(Default)]
struct ReasoningItemPatcher {
    buffer: String,
    utf8_remainder: Vec<u8>,
    groups: Vec<Vec<Value>>,
    used: usize,
}

impl ReasoningItemPatcher {
    fn is_idle(&self) -> bool {
        self.groups.is_empty() && self.buffer.is_empty()
    }

    fn add_blocks(&mut self, blocks: Vec<Value>) {
        if !blocks.is_empty() {
            self.groups.push(blocks);
        }
    }

    fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
        append_utf8_safe(&mut self.buffer, &mut self.utf8_remainder, bytes);
        let mut output = String::new();
        while let Some(block) = take_sse_block(&mut self.buffer) {
            output.push_str(&self.patch_block(&block));
        }
        output.into_bytes()
    }

    fn flush(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return Vec::new();
        }
        let block = std::mem::take(&mut self.buffer);
        self.patch_block(&block).into_bytes()
    }

    fn patch_block(&mut self, block: &str) -> String {
        let mut data_lines: Vec<String> = Vec::new();
        for line in block.lines() {
            if let Some(data) = strip_sse_field(line, "data") {
                data_lines.push(data.to_string());
            }
        }
        if data_lines.is_empty() {
            return format!("{block}\n\n");
        }
        let data = data_lines.join("\n");
        if data.trim() == "[DONE]" {
            return format!("{block}\n\n");
        }
        let Ok(mut value) = serde_json::from_str::<Value>(&data) else {
            return format!("{block}\n\n");
        };
        if !self.patch_event(&mut value) {
            return format!("{block}\n\n");
        }
        // 只替换 data 行，保留 event 行等其余内容。
        let mut replaced = false;
        let mut lines: Vec<String> = Vec::new();
        for line in block.lines() {
            if !replaced && strip_sse_field(line, "data").is_some() {
                lines.push(format!("data: {value}"));
                replaced = true;
            } else {
                lines.push(line.to_string());
            }
        }
        format!("{}\n\n", lines.join("\n"))
    }

    fn patch_event(&mut self, value: &mut Value) -> bool {
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_item.done" => match value.get_mut("item") {
                Some(item) => self.patch_item(item),
                None => false,
            },
            "response.completed" => {
                let mut patched = false;
                if let Some(output) = value
                    .pointer_mut("/response/output")
                    .and_then(Value::as_array_mut)
                {
                    for item in output.iter_mut() {
                        patched |= self.patch_item(item);
                    }
                }
                patched
            }
            _ => false,
        }
    }

    fn patch_item(&mut self, item: &mut Value) -> bool {
        if item.get("type").and_then(Value::as_str) != Some("reasoning")
            || item.get("encrypted_content").is_some()
        {
            return false;
        }
        let Some(group) = self.groups.get(self.used) else {
            return false;
        };
        let Some(encrypted) = encode_thinking_blocks(group) else {
            return false;
        };
        item["encrypted_content"] = json!(encrypted);
        self.used += 1;
        true
    }
}

/// 非流式响应体统一成 chat 形态，供压缩聚合器解析（它只认 chat / Responses 结构）。
pub fn normalize_non_stream_body_for_compaction(
    wire_api: crate::protocol_proxy::UpstreamWireApi,
    body: &[u8],
) -> anyhow::Result<Vec<u8>> {
    if wire_api != crate::protocol_proxy::UpstreamWireApi::AnthropicMessages {
        return Ok(body.to_vec());
    }
    let message: Value = serde_json::from_slice(body)?;
    let chat = anthropic_message_to_chat_completion(message)?;
    Ok(serde_json::to_vec(&chat)?)
}

/// 一次性把整段 Anthropic SSE 转成 Responses SSE（非 launcher 路径使用）。
pub fn anthropic_sse_to_responses_sse(input: &str, request: &Value) -> Vec<u8> {
    let mut converter = AnthropicSseToResponsesConverter::with_request(request);
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    output
}

/// 测试 / 诊断用：一次性转成 chat SSE。
pub fn anthropic_sse_to_chat_sse(input: &str) -> String {
    let mut converter = AnthropicSseToChatSse::default();
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8_lossy(&output).to_string()
}

fn emit_chat_chunk(output: &mut String, chunk: &Value) {
    output.push_str("data: ");
    output.push_str(&chunk.to_string());
    output.push_str("\n\n");
}

fn take_sse_block(buffer: &mut String) -> Option<String> {
    let lf = buffer.find("\n\n").map(|index| (index, 2));
    let crlf = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) => {
            if left.0 <= right.0 {
                left
            } else {
                right
            }
        }
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => return None,
    };
    let block = buffer[..index].to_string();
    buffer.drain(..index + delimiter_len);
    Some(block)
}

fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut combined = Vec::new();
    if !remainder.is_empty() {
        combined.extend_from_slice(remainder);
        remainder.clear();
    }
    combined.extend_from_slice(bytes);
    match std::str::from_utf8(&combined) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid = error.valid_up_to();
            if valid > 0 {
                buffer.push_str(std::str::from_utf8(&combined[..valid]).unwrap_or_default());
            }
            if error.error_len().is_none() {
                remainder.extend_from_slice(&combined[valid..]);
            } else {
                buffer.push_str(&String::from_utf8_lossy(&combined[valid..]));
            }
        }
    }
}

fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options_with_max_tokens(max_tokens: u64) -> AnthropicRequestOptions {
        AnthropicRequestOptions {
            max_tokens,
            // 这些用例只关心结构转换，缓存断点由专门用例覆盖。
            prompt_cache: false,
            ..AnthropicRequestOptions::default()
        }
    }

    fn chat_request() -> Value {
        json!({
            "model": "claude-sonnet-5",
            "max_tokens": 512,
            "temperature": 1.7,
            "stream": true,
            "messages": [
                { "role": "system", "content": "你是助手" },
                { "role": "user", "content": [
                    { "type": "text", "text": "看图" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
                ]},
                { "role": "assistant", "content": null, "tool_calls": [
                    { "id": "call_1", "type": "function",
                      "function": { "name": "lookup", "arguments": "{\"q\":\"x\"}" } }
                ]},
                { "role": "tool", "tool_call_id": "call_1", "content": "结果" }
            ],
            "tools": [
                { "type": "function", "function": {
                    "name": "lookup", "description": "查一下",
                    "parameters": { "type": "object", "properties": { "q": { "type": "string" } } }
                }}
            ],
            "tool_choice": "auto"
        })
    }

    #[test]
    fn messages_url_normalizes_common_shapes() {
        assert_eq!(
            anthropic_messages_url("https://api.example.com"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/v1"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://api.example.com/v1/messages"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            anthropic_messages_url("https://gw.example.com/openai#"),
            "https://gw.example.com/openai/messages"
        );
    }

    #[test]
    fn chat_request_maps_to_anthropic_shape() {
        let converted = chat_completions_to_anthropic_messages(chat_request(), &options_with_max_tokens(8192)).unwrap();
        assert_eq!(converted["model"], "claude-sonnet-5");
        assert_eq!(converted["max_tokens"], 512);
        assert_eq!(converted["stream"], true);
        // OpenAI 的 temperature 上限是 2.0，Anthropic 只到 1.0。
        assert_eq!(converted["temperature"], 1.0);
        assert_eq!(converted["system"], "你是助手");
        assert_eq!(converted["messages"][0]["role"], "user");
        let blocks = converted["messages"][0]["content"].as_array().unwrap();
        assert_eq!(blocks[0], json!({ "type": "text", "text": "看图" }));
        assert_eq!(
            blocks[1],
            json!({ "type": "image", "source": {
                "type": "base64", "media_type": "image/png", "data": "AAAA" } })
        );
        // 助手工具调用 → tool_use
        assert_eq!(converted["messages"][1]["role"], "assistant");
        assert_eq!(
            converted["messages"][1]["content"][0],
            json!({ "type": "tool_use", "id": "call_1", "name": "lookup", "input": { "q": "x" } })
        );
        // 工具结果 → 紧随其后的 user + tool_result
        assert_eq!(converted["messages"][2]["role"], "user");
        assert_eq!(
            converted["messages"][2]["content"][0],
            json!({ "type": "tool_result", "tool_use_id": "call_1", "content": "结果" })
        );
        assert_eq!(converted["tools"][0]["name"], "lookup");
        assert_eq!(converted["tools"][0]["description"], "查一下");
        assert_eq!(converted["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(converted["tool_choice"], json!({ "type": "auto" }));
    }

    #[test]
    fn missing_max_tokens_falls_back_and_first_message_is_user() {
        let request = json!({
            "model": "claude-sonnet-5",
            "messages": [
                { "role": "assistant", "content": "上一条回答" },
                { "role": "user", "content": "继续" }
            ]
        });
        let converted = chat_completions_to_anthropic_messages(request, &options_with_max_tokens(4096)).unwrap();
        assert_eq!(converted["max_tokens"], 4096);
        let messages = converted["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["text"], "(continuing the conversation)");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn empty_tools_drop_tool_choice() {
        let request = json!({
            "model": "m",
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [],
            "tool_choice": "auto"
        });
        let converted = chat_completions_to_anthropic_messages(request, &options_with_max_tokens(8192)).unwrap();
        assert!(converted.get("tools").is_none());
        assert!(converted.get("tool_choice").is_none());
    }

    #[test]
    fn anthropic_message_maps_back_to_chat_completion() {
        let message = json!({
            "id": "msg_1",
            "model": "claude-sonnet-5",
            "stop_reason": "tool_use",
            "content": [
                { "type": "thinking", "thinking": "先想一下" },
                { "type": "text", "text": "我来查" },
                { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "q": "x" } }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 4,
                "cache_read_input_tokens": 2,
                "cache_creation": { "ephemeral_5m_input_tokens": 3 }
            }
        });
        let converted = anthropic_message_to_chat_completion(message).unwrap();
        assert_eq!(converted["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(converted["choices"][0]["message"]["content"], "我来查");
        assert_eq!(
            converted["choices"][0]["message"]["reasoning_content"],
            "先想一下"
        );
        assert_eq!(
            converted["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{\"q\":\"x\"}"
        );
        assert_eq!(converted["usage"]["cache_creation_5m_input_tokens"], 3);
        assert_eq!(converted["usage"]["total_tokens"], 19);
    }

    #[test]
    fn anthropic_sse_maps_text_thinking_and_tool_calls() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_9\",\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":7}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"想\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":2,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_2\",\"name\":\"lookup\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"q\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":2,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"x\\\"}\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let chat = anthropic_sse_to_chat_sse(stream);
        assert!(chat.contains("\"reasoning_content\":\"想\""), "{chat}");
        assert!(chat.contains("\"content\":\"你好\""), "{chat}");
        assert!(chat.contains("\"name\":\"lookup\""), "{chat}");
        assert!(chat.contains("\"arguments\":\"{\\\"q\\\":\""), "{chat}");
        assert!(chat.contains("\"finish_reason\":\"tool_calls\""), "{chat}");
        assert!(chat.trim_end().ends_with("data: [DONE]"), "{chat}");
    }

    #[test]
    fn anthropic_sse_converts_to_responses_events() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_9\",\"model\":\"claude-sonnet-5\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"你好\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let request = json!({ "model": "claude-sonnet-5" });
        let responses = String::from_utf8(anthropic_sse_to_responses_sse(stream, &request)).unwrap();
        assert!(responses.contains("response.created"), "{responses}");
        assert!(responses.contains("response.output_text.delta"), "{responses}");
        assert!(responses.contains("你好"), "{responses}");
        assert!(responses.contains("response.completed"), "{responses}");
    }

    #[test]
    fn truncated_anthropic_stream_has_no_terminal_event() {
        let mut converter = AnthropicSseToChatSse::default();
        let _ = converter.push_bytes(
            b"event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\"}}\n\n",
        );
        assert!(!converter.has_terminal_event());
        let _ = converter.finish();
        assert!(
            !converter.has_terminal_event(),
            "缺少 message_stop 不能被当成正常结束"
        );
    }

    #[test]
    fn effort_level_and_model_capability_mapping() {
        // Codex 的扩展档位归到 Anthropic 的最深一档；未知档位不透传。
        assert_eq!(effort_to_anthropic_effort("minimal"), Some("low"));
        assert_eq!(effort_to_anthropic_effort("low"), Some("low"));
        assert_eq!(effort_to_anthropic_effort("medium"), Some("medium"));
        assert_eq!(effort_to_anthropic_effort("high"), Some("high"));
        assert_eq!(effort_to_anthropic_effort("xhigh"), Some("max"));
        assert_eq!(effort_to_anthropic_effort("ultra"), Some("max"));
        assert_eq!(effort_to_anthropic_effort("max"), Some("max"));
        assert_eq!(effort_to_anthropic_effort("none"), None);
        assert_eq!(effort_to_anthropic_effort("bogus"), None);

        for capable in [
            "claude-sonnet-5",
            "claude-opus-5-5",
            "claude-haiku-5-5",
            "claude-fable-5-1",
            "claude-sonnet-4-6",
            "openrouter/claude-opus-5",
        ] {
            assert!(model_supports_adaptive_thinking(capable), "{capable} 应支持 adaptive");
        }
        for legacy in [
            "claude-opus-4-5",
            "claude-haiku-4-5",
            "claude-sonnet-4-5-20250929",
            "claude-3-7-sonnet",
        ] {
            assert!(!model_supports_adaptive_thinking(legacy), "{legacy} 不该走 adaptive");
        }
    }

    #[test]
    fn auto_mode_passes_level_for_new_models_and_budget_for_old_ones() {
        let request = |model: &str| {
            json!({
                "model": model,
                "max_output_tokens": 8192,
                "reasoning": { "effort": "high" },
                "input": "hi"
            })
        };

        // 新代：等级原样透传，不再受 max_tokens/2 压制。
        let new_gen =
            responses_to_anthropic_messages(request("claude-sonnet-5"), &options_with_max_tokens(8192))
                .unwrap();
        assert_eq!(new_gen["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(new_gen["output_config"], json!({ "effort": "high" }));
        assert!(new_gen["thinking"].get("budget_tokens").is_none());

        // 旧代：回落经典预算（high → 16384，被 max_tokens 夹到 4096）。
        let legacy =
            responses_to_anthropic_messages(request("claude-opus-4-5"), &options_with_max_tokens(8192))
                .unwrap();
        assert_eq!(legacy["thinking"]["type"], "enabled");
        assert_eq!(legacy["thinking"]["budget_tokens"], 4096);
        assert!(legacy.get("output_config").is_none());

        // Codex 扩展档位透传时会收敛成 max。
        let mut ultra = request("claude-opus-5");
        ultra["reasoning"]["effort"] = json!("ultra");
        let converted =
            responses_to_anthropic_messages(ultra, &options_with_max_tokens(8192)).unwrap();
        assert_eq!(converted["output_config"], json!({ "effort": "max" }));
    }

    #[test]
    fn forced_thinking_modes_override_model_capability() {
        let request = |model: &str| {
            json!({
                "model": model,
                "max_output_tokens": 8192,
                "reasoning": { "effort": "medium" },
                "input": "hi"
            })
        };
        let with_mode = |mode: AnthropicThinkingMode, legacy: bool| {
            let options = AnthropicRequestOptions {
                thinking_mode: mode,
                prompt_cache: false,
                ..AnthropicRequestOptions::default()
            };
            responses_to_anthropic_messages(
                request(if legacy { "claude-opus-4-5" } else { "claude-sonnet-5" }),
                &options,
            )
            .unwrap()
        };

        // 强制 adaptive：旧代也发等级（上游不支持时只会静默忽略，不报错）。
        let forced = with_mode(AnthropicThinkingMode::Adaptive, true);
        assert_eq!(forced["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(forced["output_config"], json!({ "effort": "medium" }));

        // 强制 budget：新代也走预算。
        let forced = with_mode(AnthropicThinkingMode::Budget, false);
        assert_eq!(forced["thinking"]["type"], "enabled");
        assert!(forced.get("output_config").is_none());

        // 关闭：两种模型都不发 thinking。
        for legacy in [true, false] {
            let off = with_mode(AnthropicThinkingMode::Off, legacy);
            assert!(off.get("thinking").is_none(), "legacy={legacy}");
            assert!(off.get("output_config").is_none(), "legacy={legacy}");
        }

        // 模式字符串解析（设置里是自由文本）。
        assert_eq!(AnthropicThinkingMode::parse(" adaptive "), AnthropicThinkingMode::Adaptive);
        assert_eq!(AnthropicThinkingMode::parse("BUDGET"), AnthropicThinkingMode::Budget);
        assert_eq!(AnthropicThinkingMode::parse("off"), AnthropicThinkingMode::Off);
        assert_eq!(AnthropicThinkingMode::parse(""), AnthropicThinkingMode::Auto);
        assert_eq!(AnthropicThinkingMode::parse("看不懂"), AnthropicThinkingMode::Auto);
    }

    #[test]
    fn explicit_max_tokens_wins_over_request_value() {
        let request = json!({
            "model": "m",
            "max_tokens": 512,
            "messages": [{ "role": "user", "content": "hi" }]
        });
        // 未显式配置：请求自带值优先。
        let converted =
            chat_completions_to_anthropic_messages(request.clone(), &options_with_max_tokens(8192))
                .unwrap();
        assert_eq!(converted["max_tokens"], 512);
        // 显式配置（用户在表单里填了）：覆盖请求值。
        let forced = AnthropicRequestOptions {
            max_tokens: 64000,
            force_max_tokens: true,
            prompt_cache: false,
            ..AnthropicRequestOptions::default()
        };
        let converted = chat_completions_to_anthropic_messages(request, &forced).unwrap();
        assert_eq!(converted["max_tokens"], 64000);
    }

    #[test]
    fn thinking_budget_follows_effort_and_clamps_to_half_of_max_tokens() {
        // 旧代模型（不支持 adaptive）在 auto 模式下走经典预算。
        // high → 16384，但 max_tokens 只有 3000，夹到一半（1500）给可见回答留余量。
        let body = json!({
            "model": "claude-opus-4-5",
            "max_output_tokens": 3000,
            "temperature": 0.3,
            "reasoning": { "effort": "high" },
            "input": "hi"
        });
        let converted =
            responses_to_anthropic_messages(body, &options_with_max_tokens(8192)).unwrap();
        assert_eq!(converted["thinking"]["type"], "enabled");
        assert_eq!(converted["thinking"]["budget_tokens"], 1500);
        // 开启思考时不能带 temperature（Anthropic 会拒绝）。
        assert!(converted.get("temperature").is_none());

        // 上限太小（1000 < 1024 下限）→ 干脆不开思考，保持普通采样。
        let small = json!({
            "model": "claude-opus-4-5",
            "max_output_tokens": 2000,
            "temperature": 0.3,
            "reasoning": { "effort": "medium" },
            "input": "hi"
        });
        let converted =
            responses_to_anthropic_messages(small, &options_with_max_tokens(8192)).unwrap();
        assert!(converted.get("thinking").is_none());

        // 未知档位 / 显式关闭都不发 thinking。
        for effort in ["none", "off", "disabled", "bogus"] {
            let body = json!({
                "model": "claude-opus-4-5",
                "reasoning": { "effort": effort },
                "input": "hi"
            });
            let converted =
                responses_to_anthropic_messages(body, &options_with_max_tokens(8192)).unwrap();
            assert!(converted.get("thinking").is_none(), "effort={effort}");
        }
    }

    #[test]
    fn thinking_is_skipped_when_tool_history_has_no_signature() {
        // 历史里有工具调用但拿不到签名块：开启思考必然被 Anthropic 拒绝，宁可不发。
        let body = json!({
            "model": "claude-sonnet-5",
            "reasoning": { "effort": "medium" },
            "input": [
                { "type": "function_call", "call_id": "c1", "name": "lookup", "arguments": "{}" },
                { "type": "function_call_output", "call_id": "c1", "output": "结果" }
            ]
        });
        let converted =
            responses_to_anthropic_messages(body, &options_with_max_tokens(8192)).unwrap();
        assert!(converted.get("thinking").is_none());
        let assistant = converted["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["content"][0]["type"], "tool_use");
    }

    /// 签名思考块的完整闭环：响应侧编码进 encrypted_content，下一轮原样放回。
    #[test]
    fn signed_thinking_survives_round_trip_through_responses() {
        let message = json!({
            "id": "msg_1",
            "model": "claude-sonnet-5",
            "stop_reason": "tool_use",
            "content": [
                { "type": "thinking", "thinking": "先想一下", "signature": "sig-abc" },
                { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "q": "x" } }
            ],
            "usage": { "input_tokens": 1, "output_tokens": 1 }
        });
        let blocks = thinking_blocks_from_anthropic_message(&message);
        assert_eq!(blocks.len(), 1);
        let chat = anthropic_message_to_chat_completion(message).unwrap();
        let mut response = crate::protocol_proxy::chat_completion_to_response_with_request(
            chat,
            &json!({ "model": "claude-sonnet-5" }),
        )
        .unwrap();
        attach_thinking_to_responses(&mut response, &blocks);
        let reasoning = response["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "reasoning")
            .cloned()
            .expect("思考内容应产出 reasoning item");
        let encrypted = reasoning["encrypted_content"].as_str().unwrap();
        assert!(encrypted.starts_with(ANTHROPIC_THINKING_PREFIX));
        assert_eq!(decode_thinking_blocks(encrypted)[0]["signature"], "sig-abc");

        // Codex 下一轮把该 reasoning item 原样放回 input。
        let next = json!({
            "model": "claude-sonnet-5",
            "reasoning": { "effort": "medium" },
            "input": [
                reasoning.clone(),
                { "type": "function_call", "call_id": "toolu_1", "name": "lookup",
                  "arguments": "{\"q\":\"x\"}" },
                { "type": "function_call_output", "call_id": "toolu_1", "output": "结果" }
            ]
        });
        let converted =
            responses_to_anthropic_messages(next, &options_with_max_tokens(16384)).unwrap();
        // 新代模型在 auto 模式下走等级透传，旧代走预算；两种都会回放签名。
        let thinking_type = converted["thinking"]["type"].as_str().unwrap_or("");
        assert!(
            matches!(thinking_type, "adaptive" | "enabled"),
            "思考未开启：{converted}"
        );
        let assistant = converted["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap();
        assert_eq!(assistant["content"][0]["type"], "thinking");
        assert_eq!(assistant["content"][0]["signature"], "sig-abc");
        assert_eq!(assistant["content"][1]["type"], "tool_use");
    }

    fn count_cache_breakpoints(value: &Value) -> usize {
        match value {
            Value::Array(items) => items.iter().map(count_cache_breakpoints).sum(),
            Value::Object(object) => {
                usize::from(object.contains_key("cache_control"))
                    + object.values().map(count_cache_breakpoints).sum::<usize>()
            }
            _ => 0,
        }
    }

    #[test]
    fn prompt_cache_injects_at_most_four_breakpoints() {
        let mut body = json!({
            "model": "claude-sonnet-5",
            "max_tokens": 1024,
            "system": "系统提示",
            "tools": [ { "name": "a", "input_schema": {} }, { "name": "b", "input_schema": {} } ],
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "1" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "2" }] },
                { "role": "user", "content": [{ "type": "text", "text": "3" }] },
                { "role": "assistant", "content": [{ "type": "text", "text": "4" }] },
                { "role": "user", "content": [{ "type": "text", "text": "5" }] }
            ]
        });
        inject_prompt_cache(&mut body);
        assert_eq!(count_cache_breakpoints(&body), 4);
        // 字符串 system 被展开成块数组，断点挂在最后一块上。
        assert_eq!(body["system"][0]["type"], "text");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");

        // 幂等：再注入一次不会超过上限，也不会重复标记。
        inject_prompt_cache(&mut body);
        assert_eq!(count_cache_breakpoints(&body), 4);

        // 调用方自己标满 4 个断点时一个都不动。
        let mut full = json!({
            "model": "m",
            "system": [ { "type": "text", "text": "s", "cache_control": { "type": "ephemeral" } } ],
            "tools": [ { "name": "a", "input_schema": {}, "cache_control": { "type": "ephemeral" } } ],
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "1",
                    "cache_control": { "type": "ephemeral" } }] },
                { "role": "user", "content": [{ "type": "text", "text": "2",
                    "cache_control": { "type": "ephemeral" } }] }
            ]
        });
        let before = full.clone();
        inject_prompt_cache(&mut full);
        assert_eq!(full, before);
    }

    #[test]
    fn claude_code_identity_is_prepended_once() {
        let mut body = json!({ "system": "你是助手" });
        prepend_claude_code_identity(&mut body);
        prepend_claude_code_identity(&mut body);
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), 2);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM_IDENTITY);
        assert_eq!(system[1]["text"], "你是助手");

        let mut empty = json!({});
        prepend_claude_code_identity(&mut empty);
        assert_eq!(empty["system"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn context_marker_and_beta_header() {
        assert_eq!(strip_context_marker("claude-sonnet-5[1m]"), "claude-sonnet-5");
        assert_eq!(strip_context_marker("claude-sonnet-5[1M]"), "claude-sonnet-5");
        assert_eq!(strip_context_marker("claude-sonnet-5"), "claude-sonnet-5");

        assert_eq!(
            anthropic_beta_header(true, true).unwrap(),
            "claude-code-20250219,context-1m-2025-08-07"
        );
        assert_eq!(
            anthropic_beta_header(false, true).unwrap(),
            "context-1m-2025-08-07"
        );
        assert!(anthropic_beta_header(false, false).is_none());

        let mut profile = crate::settings::RelayProfile {
            model_windows: r#"{"claude-sonnet-5":"1M"}"#.to_string(),
            ..crate::settings::RelayProfile::default()
        };
        assert!(model_declares_context_1m(&profile, "claude-sonnet-5"));
        profile.model_windows = r#"{"claude-sonnet-5":"200K"}"#.to_string();
        assert!(!model_declares_context_1m(&profile, "claude-sonnet-5"));
        // 每模型窗口写死不到 1M 时，不会被 profile 级 contextWindow 顶掉。
        profile.context_window = "1000000".to_string();
        assert!(!model_declares_context_1m(&profile, "claude-sonnet-5"));
        // 没有该模型的条目时才退回 profile 级 contextWindow。
        profile.model_windows = r#"{"other-model":"200K"}"#.to_string();
        assert!(model_declares_context_1m(&profile, "claude-sonnet-5"));
    }

    /// 流式：signature_delta 要落进 reasoning item 的 encrypted_content。
    #[test]
    fn stream_signature_is_attached_to_reasoning_item() {
        let stream = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_s\",\"model\":\"claude-sonnet-5\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"想\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"signature_delta\",\"signature\":\"sig-stream\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let output =
            String::from_utf8(anthropic_sse_to_responses_sse(stream, &json!({}))).unwrap();
        let encrypted = output
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find_map(|value| {
                value
                    .pointer("/item/encrypted_content")
                    .or_else(|| value.pointer("/response/output/0/encrypted_content"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .expect("reasoning item 必须带回放信封");
        let blocks = decode_thinking_blocks(&encrypted);
        assert_eq!(blocks[0]["signature"], "sig-stream");
        assert_eq!(blocks[0]["thinking"], "想");
    }

    #[test]
    fn error_event_surfaces_as_failure() {
        let mut converter = AnthropicSseToResponsesConverter::with_request(&json!({}));
        let _ = converter.push_bytes(
            "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"忙\"}}\n\n"
                .as_bytes(),
        );
        assert!(converter.has_failed());
        assert!(converter.has_terminal_event());
    }
}
