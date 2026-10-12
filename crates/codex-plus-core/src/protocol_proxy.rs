//! Codex Responses API 与 OpenAI Chat Completions 的本地协议转换。
//!
//! Codex Chat 与 Responses 协议之间的转换实现。

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use serde_json::{Map, Value, json};

use crate::relay_rotation::{RotationContext, RotationEvent};
use crate::settings::{RelayProtocol, SettingsStore};

pub const DEFAULT_PROTOCOL_PROXY_PORT: u16 = 57321;

/// 工具调用上需要跨 turn 原样带回的供应商附带数据，按 `call_id` 索引。
///
/// Gemini 3 系在 functionCall 上要求回传 `thought_signature`（OpenAI 兼容层放在
/// tool_call 的 `extra_content.google.thought_signature`），缺失时整轮 400
/// （issue #332 / #1012）。这类字段由上游产生、本仓不理解其语义，因此不硬编码
/// 任何字段名，而是把整个 `extra_content` 原样记住，下一轮构造请求时挂回对应
/// tool_call。只做透传，解析失败/无该字段时行为与改动前完全一致。
///
/// 键是 call_id：上游保证同一次调用内唯一，跨 turn 也由客户端原样回传。
fn tool_call_extra_content_cache() -> &'static Mutex<ExtraContentCache> {
    static CACHE: OnceLock<Mutex<ExtraContentCache>> = OnceLock::new();
    // 缓存上限：一次会话里同时活跃的工具调用远小于此数；超出后按插入序淘汰最旧的，
    // 避免长时间运行时无界增长。
    CACHE.get_or_init(|| Mutex::new(ExtraContentCache::new(256)))
}

#[derive(Debug)]
struct ExtraContentCache {
    entries: BTreeMap<String, Value>,
    order: VecDeque<String>,
    capacity: usize,
}

impl ExtraContentCache {
    fn new(capacity: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    fn remember(&mut self, call_id: &str, extra_content: Value) {
        if call_id.is_empty() {
            return;
        }
        if self
            .entries
            .insert(call_id.to_string(), extra_content)
            .is_none()
        {
            self.order.push_back(call_id.to_string());
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                // 只有仍是最旧那一条时才删，避免把刚更新过的键误删。
                if !self.order.contains(&oldest) {
                    self.entries.remove(&oldest);
                }
            }
        }
    }

    fn recall(&self, call_id: &str) -> Option<Value> {
        self.entries.get(call_id).cloned()
    }
}

/// 从 chat 侧的 tool_call 里取出需要透传的附带数据。
///
/// `extra_content` 是 OpenAI 兼容层的通用扩展位（Gemini 的 thought_signature 在此）；
/// 部分供应商也会用 `thought_signature` / `thoughtSignature` 直接挂在 tool_call 上，
/// 两种形态都收。
fn tool_call_extra_content(tool_call: &Value) -> Option<Value> {
    if let Some(value) = tool_call.get("extra_content") {
        if value.is_object() && !value.as_object().is_some_and(Map::is_empty) {
            return Some(value.clone());
        }
    }
    for key in ["thought_signature", "thoughtSignature"] {
        if let Some(value) = tool_call.get(key) {
            if !value.is_null() {
                return Some(json!({ "google": { "thought_signature": value } }));
            }
        }
    }
    None
}

/// 记住本次上游返回的工具调用附带数据（按 call_id）。
fn remember_tool_call_extra_content(call_id: &str, tool_call: &Value) {
    let Some(extra) = tool_call_extra_content(tool_call) else {
        return;
    };
    if let Ok(mut cache) = tool_call_extra_content_cache().lock() {
        cache.remember(call_id, extra);
    }
}

/// 取回该 call_id 之前记住的附带数据，用于下一轮请求回传。
fn recall_tool_call_extra_content(call_id: &str) -> Option<Value> {
    tool_call_extra_content_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.recall(call_id))
}

/// 协议代理的实际生效端口，默认 [`DEFAULT_PROTOCOL_PROXY_PORT`]，
/// 可用环境变量 `CODEX_PLUS_PROTOCOL_PROXY_PORT` 覆盖。
///
/// 端口要写进 `config.toml` 的 `base_url`，不能像普通 helper 端口那样自动换；
/// 但少数机器（issue #2189）上 57321 恰好被 Hyper-V/WSL 开机划进了 Windows
/// 动态端口排除区间，bind 报 os error 10013 永远起不来，只能整体挪一个端口。
/// 写入 base_url 与读取校验必须都走本函数，保证同一进程内一致。
pub fn protocol_proxy_port() -> u16 {
    std::env::var("CODEX_PLUS_PROTOCOL_PROXY_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_PROTOCOL_PROXY_PORT)
}
pub const NO_AUTH_PROXY_BEARER_TOKEN: &str = "codex-plus-no-auth";
const UPSTREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const UPSTREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const UPSTREAM_STREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(120);
const UPSTREAM_IMAGE_HEADER_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_NATIVE_COMPACTION_PROBE_BODY_BYTES: usize = 32 * 1024 * 1024;
const THINK_OPEN_TAG: &str = "<think>";
const THINK_CLOSE_TAG: &str = "</think>";
const EXTRA_CHAT_PASSTHROUGH_FIELDS: &[&str] = &[
    "frequency_penalty",
    "logit_bias",
    "logprobs",
    "metadata",
    "n",
    "presence_penalty",
    "response_format",
    "seed",
    "service_tier",
    "stop",
    "stream_options",
    "top_logprobs",
    "user",
];
const ERROR_BODY_PREVIEW_LIMIT: usize = 1024;

/// codex v2 远程压缩请求在 input 末尾携带的控制 item（openai/codex compact_remote_v2）。
const COMPACTION_TRIGGER_TYPE: &str = "compaction_trigger";
/// codex 期望响应里恰好包含一个的压缩结果 item，`encrypted_content` 只透传不校验。
const COMPACTION_OUTPUT_TYPE: &str = "compaction";
/// 本地摘要的持久化格式标识；原生 encrypted_content 不使用这个命名空间。
/// 后缀为 JSON string，不依赖客户端可能丢弃的 item id 或额外字段。
const LOCAL_COMPACTION_PREFIX: &str = "codexplusplus:summary:v1:";
/// 本地代理生成摘要时注入的 user 指令（对齐 openai/codex prompts/templates/compact/prompt.md）。
const COMPACTION_SUMMARY_INSTRUCTION: &str = "You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.\n\nInclude:\n- Current progress and key decisions made\n- Important context, constraints, or user preferences\n- What remains to be done (clear next steps)\n- Any critical data, examples, or references needed to continue\n\nBe concise, structured, and focused on helping the next LLM seamlessly continue the work.";
/// 历史回放时 `compaction` item 展开成的文本前缀（对齐 codex SUMMARY_PREFIX 语义）。
const COMPACTION_REPLAY_PREFIX: &str = "Another language model started to solve this problem and produced a summary of its thinking process. Here is the summary produced by the other language model:\n";

/// 判断 Responses 请求体是否为 codex v2 远程压缩请求。
/// 当前 Codex 把 trigger 追加在 input 末尾；这里仍接受任意位置，避免漏检。
pub fn request_has_compaction_trigger(body: &Value) -> bool {
    body.get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("type").and_then(Value::as_str) == Some(COMPACTION_TRIGGER_TYPE)
            })
        })
}

/// 从请求 input 中剥离全部 compaction_trigger 控制项。
/// 只在确认上游没有原生 compaction 项、需要改写成普通摘要时使用。
fn strip_compaction_trigger(mut body: Value) -> Value {
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.retain(|item| {
            item.get("type").and_then(Value::as_str) != Some(COMPACTION_TRIGGER_TYPE)
        });
    }
    body
}

/// 把压缩摘要请求改写成上游能理解的普通生成请求：
/// - input 末尾注入 user 摘要指令；
/// - tools/parallel_tool_calls 清空，避免摘要阶段触发工具调用。
fn rewrite_request_for_compaction(mut body: Value) -> Value {
    if let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) {
        items.push(json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": COMPACTION_SUMMARY_INSTRUCTION }]
        }));
    }
    body["tools"] = json!([]);
    body["tool_choice"] = json!("none");
    body["parallel_tool_calls"] = json!(false);
    body
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChatReasoningStyle {
    Default,
    DeepSeek,
    LowHigh,
    OpenRouter,
    Thinking,
    EnableThinking,
    ReasoningSplit,
}

#[derive(Debug, Clone, Default)]
struct CodexToolContext {
    custom_tools: BTreeMap<String, CodexCustomToolSpec>,
    function_tools: BTreeMap<String, CodexFunctionToolSpec>,
    has_custom_tools: bool,
    has_namespace_tools: bool,
}

#[derive(Debug, Clone)]
struct CodexCustomToolSpec {
    openai_name: String,
    kind: CodexCustomToolKind,
}

#[derive(Debug, Clone, Default)]
struct CodexFunctionToolSpec {
    namespace: String,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexCustomToolKind {
    Raw,
    ApplyPatch,
    BuiltIn,
    /// Codex 的 tool_search 工具（MCP 延迟加载检索，execution: client）。
    ToolSearch,
}

impl Default for CodexCustomToolKind {
    fn default() -> Self {
        Self::Raw
    }
}

impl CodexToolContext {
    fn is_custom_tool_proxy(&self, upstream_name: &str) -> bool {
        self.custom_tools.contains_key(upstream_name)
    }

    fn is_tool_search_proxy(&self, upstream_name: &str) -> bool {
        self.custom_tools.get(upstream_name).map(|spec| spec.kind)
            == Some(CodexCustomToolKind::ToolSearch)
    }

    /// 上游把 codex 的原生 `web_search` 工具当成普通自定义工具回传时，中转层
    /// 此前一律产出 `custom_tool_call`，而客户端只认 `web_search_call`
    /// （issue #1209 / #586：web_search 被错译成 custom_tool_call 后，客户端
    /// 拿不到搜索结果，表现为「工具调用后无反应」或上游报 tool 名不识别）。
    fn is_builtin_web_search_proxy(&self, upstream_name: &str) -> bool {
        self.custom_tools.get(upstream_name).is_some_and(|spec| {
            spec.kind == CodexCustomToolKind::BuiltIn && spec.openai_name == "web_search"
        })
    }

    fn original_custom_tool_name(&self, upstream_name: &str) -> String {
        self.custom_tools
            .get(upstream_name)
            .map(|spec| spec.openai_name.clone())
            .unwrap_or_else(|| upstream_name.to_string())
    }

    fn openai_name_for_function_tool(&self, upstream_name: &str) -> (String, String) {
        let Some(spec) = self.function_tools.get(upstream_name) else {
            return (upstream_name.to_string(), String::new());
        };
        let name = if spec.name.is_empty() {
            upstream_name.to_string()
        } else {
            spec.name.clone()
        };
        (name, spec.namespace.clone())
    }
}

pub fn local_responses_proxy_base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

#[derive(Debug)]
pub(crate) struct UnsupportedEncryptedAgentContent;

impl std::fmt::Display for UnsupportedEncryptedAgentContent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unsupported_encrypted_agent_content: Chat Completions 上游无法处理加密的 agent 消息内容，请使用支持该协议的 Responses 上游"
        )
    }
}

impl std::error::Error for UnsupportedEncryptedAgentContent {}

impl UnsupportedEncryptedAgentContent {
    pub(crate) fn response_body(&self) -> Value {
        json!({"error":{
            "type":"invalid_request_error",
            "code":"unsupported_encrypted_agent_content",
            "param":"input",
            "message":self.to_string()
        }})
    }
}

#[derive(Debug)]
pub(crate) struct UnsupportedNativeWebSearch;

impl std::fmt::Display for UnsupportedNativeWebSearch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "unsupported_native_web_search: Chat 路由不执行服务端原生搜索，请使用客户端搜索工具或支持原生搜索的 Responses 上游"
        )
    }
}

impl std::error::Error for UnsupportedNativeWebSearch {}

impl UnsupportedNativeWebSearch {
    pub(crate) fn response_body(&self) -> Value {
        json!({"error": {
            "type": "invalid_request_error", "code": "unsupported_native_web_search",
            "param": "tool_choice", "message": self.to_string()
        }})
    }
}

fn remove_unsupported_native_search(body: &mut Value) -> anyhow::Result<()> {
    let choice = body.get("tool_choice");
    if choice
        .and_then(|choice| choice.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "web_search" | "web_search_preview"))
    {
        return Err(UnsupportedNativeWebSearch.into());
    }
    let required = choice.and_then(Value::as_str) == Some("required");
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        let before = tools.len();
        // 同名 function/custom 和命名空间工具由客户端执行，必须保留。
        tools.retain(|tool| {
            !matches!(
                tool.get("type").and_then(Value::as_str),
                Some("web_search" | "web_search_preview")
            )
        });
        if before != tools.len() && required && tools.is_empty() {
            return Err(UnsupportedNativeWebSearch.into());
        }
    }
    Ok(())
}

fn input_has_encrypted_agent_content(input: &Value) -> bool {
    fn item_has_encrypted_content(item: &Value) -> bool {
        item.get("type").and_then(Value::as_str) == Some("encrypted_content")
            || item.get("content").is_some_and(|content| match content {
                Value::Array(parts) => parts.iter().any(|part| {
                    part.get("type").and_then(Value::as_str) == Some("encrypted_content")
                }),
                Value::Object(_) => {
                    content.get("type").and_then(Value::as_str) == Some("encrypted_content")
                }
                _ => false,
            })
    }
    match input {
        Value::Array(items) => items.iter().any(item_has_encrypted_content),
        Value::Object(_) => item_has_encrypted_content(input),
        _ => false,
    }
}

pub fn responses_to_chat_completions(body: Value) -> anyhow::Result<Value> {
    responses_to_chat_completions_with_options(body, false, false)
}

pub fn responses_to_chat_completions_with_options(
    mut body: Value,
    standard: bool,
    stabilize_prompt_prefix: bool,
) -> anyhow::Result<Value> {
    remove_unsupported_native_search(&mut body)?;
    expand_local_compaction_history(&mut body);
    if input_has_native_compaction(&body) {
        return Err(NativeCompactionUnsupported::history().into());
    }
    // agent 加密内容没有 Chat 文本映射。发送前明确拒绝；不按字段名猜测明文，
    // 也不遍历工具参数/输出里的用户 JSON。reasoning 和 compaction 保持原契约。
    if body
        .get("input")
        .is_some_and(input_has_encrypted_agent_content)
    {
        return Err(UnsupportedEncryptedAgentContent.into());
    }
    let mut result = json!({});

    if let Some(model) = body.get("model") {
        result["model"] = model.clone();
    }

    let mut messages = Vec::new();
    if let Some(instructions) = body.get("instructions") {
        let text = instruction_text(instructions);
        if !text.is_empty() {
            messages.push(json!({ "role": "system", "content": text }));
        }
    }

    if let Some(input) = body.get("input") {
        append_responses_input(input, &mut messages);
    }
    // 「prompt 前缀稳定层」（opt-in，默认关）：先把轮变的 resize notice 从所有角色
    // 的文本里剥离，避免它插进历史早段；后面的 collapse 改走规范序拼接。
    if stabilize_prompt_prefix {
        strip_image_resize_notice_blocks(&mut messages);
        // 历史轮附件样板归一（opt-in）：客户端附件轮重写历史样板被归一为常量。
        normalize_attachment_boilerplate(&mut messages);
    }
    // 必须在 enforce_tool_call_pairing 之前：配对一旦被错误摘除就无法恢复。
    relocate_interleaved_non_tool_messages(&mut messages);
    enforce_tool_call_pairing(&mut messages);
    // 配对判定之后仍有无主 tool 消息（上游会直接 400），降级成 user 保住内容。
    degrade_unpaired_tool_messages(&mut messages);
    // 必须在 enforce_tool_call_pairing 之后：它依赖 tool 消息的连续性，
    // 而这一步会往中间插入 user 消息。
    relocate_tool_output_images(&mut messages);
    ensure_tool_call_reasoning_content(&mut messages);
    normalize_chat_messages(&mut messages);
    let model = body.get("model").and_then(Value::as_str).unwrap_or("");
    normalize_image_data_urls_for_model(&mut messages, model);
    let mut messages = collapse_system_messages_to_head(messages, stabilize_prompt_prefix);
    if stabilize_prompt_prefix {
        relocate_turn_aborted_blocks(&mut messages);
    }
    if stabilize_prompt_prefix {
        // 轮次标记：把「本轮要回答的指令」显式钉在最后一条 user 消息上，
        // 根治上下文重建（切模型/压缩）时旧问题被回放成最新指令的串线问题。
        mark_current_user_turn(&mut messages);
    }
    result["messages"] = json!(messages);
    if let Some(value) = body.get("max_output_tokens") {
        if is_openai_o_series(model) {
            result["max_completion_tokens"] = value.clone();
        } else {
            result["max_tokens"] = value.clone();
        }
    }
    if let Some(value) = body.get("max_tokens") {
        result["max_tokens"] = value.clone();
    }
    if let Some(value) = body.get("max_completion_tokens") {
        result["max_completion_tokens"] = value.clone();
    }

    for key in ["temperature", "top_p", "stream"] {
        if let Some(value) = body.get(key) {
            result[key] = value.clone();
        }
    }
    if body.get("stream").and_then(Value::as_bool).unwrap_or(false) {
        let mut stream_options = body
            .get("stream_options")
            .cloned()
            .unwrap_or_else(|| json!({}));
        stream_options["include_usage"] = json!(true);
        result["stream_options"] = stream_options;
    }

    apply_chat_reasoning_options(&mut result, &body, model, standard);

    let mut tool_context = build_codex_tool_context(body.get("tools"));
    // Codex 客户端把 tool_search 命中的 MCP 命名空间挂在历史里的 tool_search_output
    // item 上，却不会把它们提升进下一轮请求的 tools 数组。这里补上这一跳：先登记进
    // tool context（模型调用回来时才能还原 namespace），再用既有 namespace 链路展开
    // 成 mcp__<server>__<tool>，否则上游只看到命名空间外壳，调不到内层工具。
    let harvested_namespaces = collect_tool_search_output_namespaces(&body);
    for namespace_tool in &harvested_namespaces {
        add_namespace_tools_to_context(&mut tool_context, namespace_tool);
    }
    let mut has_chat_tools = false;
    let mut converted = Vec::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        converted = responses_tools_to_chat_tools(tools, &tool_context);
    }
    for namespace_tool in &harvested_namespaces {
        converted.extend(namespace_tool_to_chat_tools(namespace_tool, &tool_context));
    }
    dedup_chat_tools_by_name(&mut converted);
    if !converted.is_empty() {
        has_chat_tools = true;
        result["tools"] = json!(converted);
    }

    if has_chat_tools {
        if let Some(tool_choice) = body
            .get("tool_choice")
            .and_then(|value| responses_tool_choice_to_chat(value, &tool_context))
        {
            result["tool_choice"] = tool_choice;
        }
        if let Some(value) = body.get("parallel_tool_calls") {
            result["parallel_tool_calls"] = value.clone();
        }
    }

    for key in EXTRA_CHAT_PASSTHROUGH_FIELDS {
        if *key == "stream_options" && result.get("stream_options").is_some() {
            continue;
        }
        if let Some(value) = body.get(*key) {
            result[*key] = value.clone();
        }
    }

    Ok(result)
}

pub fn chat_completion_to_response(body: Value) -> anyhow::Result<Value> {
    chat_completion_to_response_with_context(body, &CodexToolContext::default(), None)
}

pub fn chat_completion_to_response_with_request(
    body: Value,
    original_request: &Value,
) -> anyhow::Result<Value> {
    let mut context = build_codex_tool_context(original_request.get("tools"));
    // 反向同样要认领 tool_search_output 里的命名空间，否则模型调用
    // mcp__<server>__<tool> 回来时查不到 namespace，还原不成 Responses item。
    for namespace_tool in &collect_tool_search_output_namespaces(original_request) {
        add_namespace_tools_to_context(&mut context, namespace_tool);
    }
    chat_completion_to_response_with_context(body, &context, Some(original_request))
}

fn chat_completion_to_response_with_context(
    body: Value,
    tool_context: &CodexToolContext,
    original_request: Option<&Value>,
) -> anyhow::Result<Value> {
    let choices = body
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("chat response missing choices"))?;
    let choice = choices
        .first()
        .ok_or_else(|| anyhow::anyhow!("chat response choices is empty"))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow::anyhow!("chat response choice missing message"))?;

    let response_id = response_id_from_chat_id(body.get("id").and_then(Value::as_str));
    let mut output = Vec::new();
    if let Some(reasoning) = chat_reasoning_to_response_output_item(message, &response_id) {
        output.push(reasoning);
    }
    if let Some(message) = chat_message_to_response_output_item(message, &response_id) {
        output.push(message);
    }
    output.extend(chat_tool_calls_to_response_output_items(
        message,
        tool_context,
    ));

    let invalid_web_search = output.iter().any(invalid_web_search_output_item);
    let mut response = json!({
        "id": response_id,
        "object": "response",
        "created_at": body.get("created").and_then(Value::as_u64).unwrap_or(0),
        "status": if invalid_web_search {
            "failed"
        } else {
            response_status(choice.get("finish_reason").and_then(Value::as_str))
        },
        "model": body.get("model").and_then(Value::as_str).unwrap_or(""),
        "output": output,
        "usage": chat_usage_to_responses_usage(body.get("usage"))
    });

    if invalid_web_search {
        response["error"] = json!({
            "type": "invalid_upstream_tool_call",
            "message": "Chat 上游返回了未执行的原生 web_search 调用，请使用客户端搜索工具"
        });
    }

    if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
        response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
    }
    copy_response_request_fields(&mut response, original_request);

    Ok(response)
}

pub struct ProxyHttpResponse {
    pub status: String,
    pub content_type: String,
    pub body: Vec<u8>,
}

pub struct UpstreamProxyResponse {
    pub status_code: u16,
    pub content_type: String,
    pub is_stream: bool,
    pub wire_api: UpstreamWireApi,
    /// 仅标记需要代理重组的合成摘要兼容路径。
    pub compaction: bool,
    /// 原生上游已返回恰好一个 compaction 项或完整失败流时为真，
    /// 调用方必须原样透传。
    pub native_compaction_passthrough: bool,
    /// 响应体已经读完。为 Some 时不要再读 response。
    pub buffered_body: Option<Vec<u8>>,
    pub response: Option<reqwest::Response>,
}

/// 客户端会话头白名单；不接受鉴权或任意其他客户端请求头。
#[derive(Clone, Copy, Default)]
pub struct ProxySessionHeaders<'a> {
    pub session_id: Option<&'a str>,
    pub thread_id: Option<&'a str>,
    pub opencode_session: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum UpstreamWireApi {
    Responses,
    ChatCompletions,
    AudioTranscriptions,
    ImageGenerations,
    ImageEdits,
}

#[derive(Debug, Clone)]
struct ModelRouteSelection {
    relay: crate::settings::RelayProfile,
    source_relay_id: String,
    source_model: String,
    upstream_model: String,
}

impl UpstreamProxyResponse {
    pub fn status(&self) -> String {
        http_status_line(self.status_code)
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status_code)
    }

    pub async fn read_body(&mut self) -> anyhow::Result<Vec<u8>> {
        if let Some(body) = &self.buffered_body {
            return Ok(body.clone());
        }
        let response = self.response.take().context("缺少上游响应")?;
        Ok(response.bytes().await?.to_vec())
    }

    pub fn take_response(&mut self) -> anyhow::Result<reqwest::Response> {
        self.response.take().context("缺少上游响应")
    }
}

pub fn upstream_header_timeout() -> Duration {
    UPSTREAM_HEADER_TIMEOUT
}

pub fn upstream_stream_header_timeout() -> Duration {
    UPSTREAM_STREAM_HEADER_TIMEOUT
}

pub fn upstream_http_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(UPSTREAM_CONNECT_TIMEOUT)
        .user_agent("CodexPlusPlus/ProtocolProxy")
        .build()
        .context("failed to build upstream HTTP client")
}

pub async fn send_upstream_request(
    request: reqwest::RequestBuilder,
) -> anyhow::Result<reqwest::Response> {
    send_upstream_request_with_header_timeout(request, UPSTREAM_HEADER_TIMEOUT).await
}

pub async fn send_upstream_request_for_responses(
    request: reqwest::RequestBuilder,
    is_stream: bool,
) -> anyhow::Result<reqwest::Response> {
    let timeout = response_header_timeout(is_stream);
    send_upstream_request_with_header_timeout(request, timeout).await
}

pub async fn send_upstream_request_with_header_timeout(
    request: reqwest::RequestBuilder,
    timeout: Duration,
) -> anyhow::Result<reqwest::Response> {
    tokio::time::timeout(timeout, request.send())
        .await
        .with_context(|| format!("上游请求超过 {} 秒未返回响应头", timeout.as_secs()))?
        .context("上游请求失败")
}

pub struct ChatSseToResponsesConverter {
    buffer: String,
    utf8_remainder: Vec<u8>,
    state: ChatSseState,
    failed: bool,
}

/// 原生 Responses 只旁路观察终止事件，不改写任何响应字节，也不保存诊断正文。
/// 只保留每个事件的有限前缀，避免图片等大 payload 令诊断缓冲无界增长。
#[derive(Default)]
pub(crate) struct NativeResponsesSseObserver {
    event_prefix: Vec<u8>,
    event_tail: [u8; 4],
    event_too_large: bool,
    unclassified_large_event: bool,
    completed: bool,
    failure: Option<&'static str>,
}

const NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT: usize = 64 * 1024;

impl NativeResponsesSseObserver {
    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if self.event_prefix.len() < NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT {
                self.event_prefix.push(*byte);
            } else {
                self.event_too_large = true;
            }
            self.event_tail.rotate_left(1);
            self.event_tail[3] = *byte;
            if self.event_tail.ends_with(b"\n\n") || self.event_tail == *b"\r\n\r\n" {
                self.observe_event();
                self.event_prefix.clear();
                self.event_tail = [0; 4];
                self.event_too_large = false;
            }
        }
    }

    fn observe_event(&mut self) {
        let prefix = String::from_utf8_lossy(&self.event_prefix);
        let event_name = prefix
            .lines()
            .find_map(|line| strip_sse_field(line, "event"));
        let data = prefix
            .lines()
            .filter_map(|line| strip_sse_field(line, "data"))
            .collect::<Vec<_>>()
            .join("\n");
        // 只观察已完整解析的顶层 type；后续大 payload 超过前缀上限时，
        // 忽略其解析错误，保留此前读到的类型。它不是原生响应的有效性校验器。
        struct EventType<'a>(&'a mut Option<String>);
        impl<'de> serde::de::Visitor<'de> for EventType<'_> {
            type Value = ();
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an SSE event object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                while let Some(key) = map.next_key::<String>()? {
                    if key == "type" {
                        *self.0 = Some(map.next_value::<String>()?);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                Ok(())
            }
        }
        let mut deserializer = serde_json::Deserializer::from_str(&data);
        let mut event_type = None;
        let _ = serde::Deserializer::deserialize_map(&mut deserializer, EventType(&mut event_type));
        match event_type.as_deref().or(event_name.map(str::trim)) {
            Some("response.completed") => self.completed = true,
            Some("response.failed") => {
                self.failure = Some("原生 Responses 上游返回 response.failed")
            }
            Some("response.incomplete") => {
                self.failure = Some("原生 Responses 上游返回 response.incomplete")
            }
            Some("error") => self.failure = Some("原生 Responses 上游返回 error 事件"),
            None if self.event_too_large => self.unclassified_large_event = true,
            _ => {}
        }
    }

    pub(crate) fn finish(self) -> Option<String> {
        // failed/incomplete 优先于 completed；[DONE] 或 HTTP EOF 都不能证明成功。
        self.failure
            .or_else(|| {
                if self.completed {
                    None
                } else if self.unclassified_large_event {
                    Some("原生 Responses 流终止状态未知（诊断事件超限），未观察到完成事件")
                } else {
                    Some("原生 Responses 上游在完成事件前结束了响应流")
                }
            })
            .map(str::to_string)
    }
}

#[cfg(test)]
mod native_responses_observer_tests {
    use super::{NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT, NativeResponsesSseObserver};

    #[test]
    fn fragmented_crlf_and_data_only_terminal_events_are_observed() {
        for payload in [
            "event: response.completed\r\ndata: {\"type\":\"response.completed\",\"text\":\"摘要\"}\r\n\r\n",
            "data: {\"response\":{},\"type\":\"response.completed\"}\n\n",
        ] {
            let mut observer = NativeResponsesSseObserver::default();
            for byte in payload.as_bytes().chunks(1) {
                observer.push_bytes(byte);
            }
            assert!(observer.finish().is_none());
        }
    }

    #[test]
    fn large_payloads_keep_diagnostic_memory_bounded_and_allow_terminal_observation() {
        for prefix in [
            "event: response.completed\ndata: {\"output\":\"",
            "data: {\"type\":\"response.completed\",\"output\":\"",
        ] {
            let mut observer = NativeResponsesSseObserver::default();
            observer.push_bytes(prefix.as_bytes());
            observer.push_bytes(&vec![b'x'; NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT * 2]);
            assert!(observer.event_prefix.len() <= NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT);
            observer.push_bytes(b"\"}\n\n");
            assert!(observer.finish().is_none());
        }
    }

    #[test]
    fn failed_event_after_completed_still_reports_fixed_failure_without_payload() {
        let mut observer = NativeResponsesSseObserver::default();
        observer.push_bytes(b"data: {\"type\":\"response.completed\"}\n\ndata: {\"type\":\"response.failed\",\"error\":\"private-payload\"}\n\n");
        assert_eq!(
            observer.finish().as_deref(),
            Some("原生 Responses 上游返回 response.failed")
        );
    }

    #[test]
    fn unclassified_large_event_reports_unknown_state_without_payload() {
        let mut observer = NativeResponsesSseObserver::default();
        observer.push_bytes(b"data: {\"output\":\"");
        observer.push_bytes(&vec![b'x'; NATIVE_SSE_DIAGNOSTIC_PREFIX_LIMIT * 2]);
        observer.push_bytes(b"\",\"type\":\"response.completed\"}\n\n");
        assert!(observer.finish().unwrap().contains("状态未知"));
    }
}

/// codex v2 远程压缩的响应包装器：把上游摘要文本（无论 Responses 还是
/// Chat 上游、流式还是非流式）封装成「恰好一个 `compaction` 输出项」的
/// Responses SSE 流。输出项通过 output_item.done 显式交付给客户端，
/// 不能只放进 response.completed.response.output。
pub struct CompactionSseConverter {
    response_id: String,
    compaction_id: String,
    model: String,
    summary: String,
    final_summary: Option<String>,
    failed: Option<(String, Option<String>)>,
    /// 跨网络 chunk 攒 SSE 事件的缓冲（见 push_upstream_bytes）。
    sse_buffer: String,
    sse_utf8_remainder: Vec<u8>,
    responses_wire: bool,
}

impl CompactionSseConverter {
    pub fn new(model: &str) -> Self {
        Self {
            response_id: format!("resp_compact_{}", chrono_now_millis()),
            compaction_id: format!("cmp_{}", uuid::Uuid::new_v4().simple()),
            model: model.to_string(),
            summary: String::new(),
            final_summary: None,
            failed: None,
            sse_buffer: String::new(),
            sse_utf8_remainder: Vec::new(),
            responses_wire: true,
        }
    }

    /// 标记上游是 Chat Completions（SSE chunk 的增量在 `choices[].delta.content`）。
    /// Responses 上游默认，增量在 `response.output_text.delta` 事件里。
    pub fn with_chat_upstream(mut self) -> Self {
        self.responses_wire = false;
        self
    }

    /// 追加上游输出的一块文本内容（非流式路径直接喂完整摘要）。
    pub fn push_summary_text(&mut self, text: &str) {
        self.summary.push_str(text);
    }

    /// 当前已收集的原始摘要文本（剥 think 之前），供空摘要判定。
    pub fn summary_text(&self) -> &str {
        &self.summary
    }

    /// 喂入上游流式响应的一个网络 chunk。SSE 事件可能被 TCP 拆开，
    /// 内部按 `\n\n` 边界缓冲；残缺块留在缓冲区等下一个 chunk。
    pub fn push_upstream_bytes(&mut self, bytes: &[u8]) {
        append_utf8_safe(&mut self.sse_buffer, &mut self.sse_utf8_remainder, bytes);
        while let Some(block) = take_sse_block(&mut self.sse_buffer) {
            if block.trim().is_empty() {
                continue;
            }
            self.handle_upstream_sse_block(&block);
        }
    }

    fn handle_upstream_sse_block(&mut self, block: &str) {
        let mut event_name = "";
        let mut data_parts: Vec<&str> = Vec::new();
        for line in block.lines() {
            if let Some(event) = strip_sse_field(line, "event") {
                event_name = event.trim();
            }
            if let Some(data) = strip_sse_field(line, "data") {
                data_parts.push(data);
            }
        }
        if data_parts.is_empty() {
            return;
        }
        let data = data_parts.join("\n");
        if data.trim() == "[DONE]" {
            return;
        }
        let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        let event_type = chunk
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or(event_name);
        if matches!(
            event_type,
            "response.failed" | "response.incomplete" | "error"
        ) || chunk.get("error").is_some_and(|error| !error.is_null())
        {
            let error = chunk
                .pointer("/response/error")
                .or_else(|| chunk.get("error"));
            self.fail(
                error
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("Compaction upstream stream failed or was incomplete")
                    .to_string(),
                error
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            );
            return;
        }
        if self.responses_wire {
            match event_type {
                // Responses 流优先读取正文增量，排除携带同名 delta 字段的推理事件。
                "response.output_text.delta" => {
                    if let Some(delta) = chunk.get("delta").and_then(Value::as_str) {
                        self.summary.push_str(delta);
                    }
                }
                // 部分兼容实现不发送 delta，只在完成事件里携带完整正文。
                "response.output_text.done" => {
                    self.capture_final_summary(chunk.get("text"));
                }
                "response.content_part.done" => {
                    self.capture_final_summary(chunk.pointer("/part/text"));
                }
                "response.output_item.done" => {
                    let text = chunk
                        .get("item")
                        .map(extract_summary_text_from_response_item)
                        .unwrap_or_default();
                    self.capture_final_summary(Some(&json!(text)));
                }
                "response.completed" => {
                    let text = chunk
                        .get("response")
                        .map(extract_summary_text_from_responses)
                        .unwrap_or_default();
                    self.capture_final_summary(Some(&json!(text)));
                }
                _ => {}
            }
        } else if let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        {
            if let Some(content) = choice
                .get("delta")
                .and_then(|delta| delta.get("content"))
                .and_then(Value::as_str)
            {
                self.summary.push_str(content);
            }
            self.capture_final_summary(
                choice
                    .get("message")
                    .and_then(|message| message.get("content")),
            );
        }
    }

    fn capture_final_summary(&mut self, text: Option<&Value>) {
        let Some(text) = text
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        else {
            return;
        };
        self.final_summary = Some(text.to_string());
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        self.failed = Some((message, error_type));
        Vec::new()
    }

    /// 收尾：产出完整的 compaction 响应 SSE。
    /// 链式推理上游（DeepSeek thinking 等）会把推理过程以 `<think>` 块
    /// 混进正文，这里统一剥掉首部完整 think 块，只保留真正的摘要答案；
    /// think 块未闭合（上游截断）时整个丢弃——剩下的只有推理残片。
    pub fn finish(mut self) -> Vec<u8> {
        if let Some(final_summary) = self.final_summary.take() {
            self.summary = final_summary;
        }
        if let Some((_reasoning, answer)) = split_leading_think_block(&self.summary) {
            self.summary = answer;
        } else if self.summary.trim_start().starts_with(THINK_OPEN_TAG) {
            self.summary = String::new();
        }
        self.summary = self.summary.trim().to_string();
        // 剥掉 think 块后为空（上游空输出、纯推理文本、截断导致块未闭合），
        // 一律按失败返回，避免向 codex 交付 `completed + 空 encrypted_content`
        // 的空 checkpoint 静默清空会话。
        if self.failed.is_none() && self.summary.is_empty() {
            self.failed = Some((
                "上游返回了空摘要，无法完成压缩".to_string(),
                Some("compaction_empty_summary".to_string()),
            ));
        }
        let mut output = String::new();
        let mut response = json!({
            "id": self.response_id,
            "object": "response",
            "created_at": chrono_now_millis() / 1000,
            "status": "in_progress",
            "model": self.model,
            "output": [],
            "usage": default_responses_usage()
        });
        push_sse(
            &mut output,
            "response.created",
            json!({"type": "response.created", "sequence_number": 0, "response": response}),
        );
        if let Some((message, error_type)) = self.failed {
            response["status"] = json!("failed");
            response["error"] = json!({
                "code": error_type.unwrap_or_else(|| "compaction_failed".to_string()),
                "message": message
            });
            push_sse(
                &mut output,
                "response.failed",
                json!({"type": "response.failed", "sequence_number": 1, "response": response}),
            );
            output.push_str("data: [DONE]\n\n");
            return output.into_bytes();
        }
        let compaction_item = json!({
            "id": self.compaction_id,
            "type": COMPACTION_OUTPUT_TYPE,
            "encrypted_content": format!("{LOCAL_COMPACTION_PREFIX}{}", json!(self.summary))
        });
        for (sequence_number, event_type) in [
            (1, "response.output_item.added"),
            (2, "response.output_item.done"),
        ] {
            push_sse(
                &mut output,
                event_type,
                json!({
                    "type": event_type,
                    "sequence_number": sequence_number,
                    "output_index": 0,
                    "item": compaction_item
                }),
            );
        }
        response["status"] = json!("completed");
        response["output"] = json!([compaction_item]);
        push_sse(
            &mut output,
            "response.completed",
            json!({
                "type": "response.completed",
                "sequence_number": 3,
                "response": response
            }),
        );
        output.push_str("data: [DONE]\n\n");
        output.into_bytes()
    }
}

fn chrono_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 非流式压缩响应包装：从上游 JSON 响应里提取 assistant 文本并封装成
/// 恰好一个 `compaction` 输出项的 Responses 响应。
pub fn wrap_non_stream_response_as_compaction(
    upstream_body: &[u8],
    model: &str,
) -> anyhow::Result<Vec<u8>> {
    let upstream_json: Value = serde_json::from_slice(upstream_body)?;
    let mut converter = CompactionSseConverter::new(model);
    if let Some(error) = upstream_json.get("error").filter(|value| !value.is_null()) {
        converter.fail(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("compaction upstream error")
                .to_string(),
            None,
        );
        return Ok(converter.finish());
    }
    let responses_text = extract_summary_text_from_responses(&upstream_json);
    let text = if responses_text.is_empty() {
        extract_summary_text_from_chat(&upstream_json)
    } else {
        responses_text
    };
    if text.is_empty() {
        converter.fail(
            "上游返回了空摘要，无法完成压缩".to_string(),
            Some("compaction_empty_summary".to_string()),
        );
        return Ok(converter.finish());
    }
    converter.push_summary_text(&text);
    Ok(converter.finish())
}

/// 从 Responses JSON 响应（`output[].content[].text`）提取 assistant 文本。
fn extract_summary_text_from_responses(response: &Value) -> String {
    let Some(items) = response.get("output").and_then(Value::as_array) else {
        return String::new();
    };
    let mut texts = Vec::new();
    for item in items {
        let text = extract_summary_text_from_response_item(item);
        if !text.is_empty() {
            texts.push(text);
        }
    }
    texts.join("\n")
}

fn extract_summary_text_from_response_item(item: &Value) -> String {
    match item.get("type").and_then(Value::as_str) {
        Some("message") => item
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// 从 Chat Completions JSON 响应提取 assistant 文本。
fn extract_summary_text_from_chat(response: &Value) -> String {
    response
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn response_body_is_stream(content_type: &str, body: &[u8]) -> bool {
    if content_type.contains("text/event-stream") {
        return true;
    }
    let trimmed = body
        .iter()
        .copied()
        .skip_while(|byte| byte.is_ascii_whitespace())
        .take(6)
        .collect::<Vec<_>>();
    trimmed.starts_with(b"data:") || trimmed.starts_with(b"event:")
}

fn output_compaction_count(output: Option<&Value>) -> Option<usize> {
    let items = output?.as_array()?;
    let compactions = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE));
    if compactions
        .clone()
        .any(|item| usable_compaction_content(item).is_none())
    {
        return Some(0);
    }
    Some(compactions.count())
}

async fn read_limited_upstream_body(
    response: &mut reqwest::Response,
    limit: usize,
) -> anyhow::Result<Vec<u8>> {
    if let Some(content_length) = response.content_length()
        && content_length > limit as u64
    {
        anyhow::bail!("上游响应体超过 {limit} 字节限制");
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("上游响应体超过 {limit} 字节限制");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn usable_compaction_content(item: &Value) -> Option<&str> {
    item.get("encrypted_content")?
        .as_str()
        .filter(|content| !content.is_empty())
}

fn json_compaction_count(value: &Value) -> Option<usize> {
    if let Some(count) = output_compaction_count(value.get("output")) {
        return Some(count);
    }
    value
        .get("response")
        .and_then(|response| output_compaction_count(response.get("output")))
}

/// 按 Codex 实际消费的 SSE item-done 事件统计原生压缩项。
/// 非流式调用仍兼容 JSON output，但流式请求不能把 JSON 当作原生 v2 透传。
fn upstream_compaction_item_count(body: &[u8], content_type: &str) -> Option<usize> {
    if response_body_is_stream(content_type, body) {
        let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
        let mut count = 0;
        for block in text.split("\n\n") {
            let mut event_name = None;
            let mut data_parts = Vec::new();
            for line in block.lines() {
                if let Some(event) = strip_sse_field(line, "event") {
                    event_name = Some(event.trim());
                }
                if let Some(data) = strip_sse_field(line.trim_end_matches('\r'), "data") {
                    data_parts.push(data);
                }
            }
            if data_parts.is_empty() {
                continue;
            }
            let data = data_parts.join("\n");
            if data.trim() == "[DONE]" {
                continue;
            }
            let Ok(json) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let event = event_name.or_else(|| json.get("type").and_then(Value::as_str));
            match event {
                Some("response.output_item.done")
                    if json
                        .get("item")
                        .and_then(|item| item.get("type"))
                        .and_then(Value::as_str)
                        == Some(COMPACTION_OUTPUT_TYPE) =>
                {
                    if usable_compaction_content(json.get("item")?).is_none() {
                        return Some(0);
                    }
                    count += 1;
                }
                Some("response.completed") => {
                    json.get("response")?.get("id")?.as_str()?;
                    return Some(count);
                }
                Some("response.failed" | "response.incomplete" | "error") => return None,
                _ => {}
            }
        }
        return None;
    }
    let Ok(json) = serde_json::from_slice::<Value>(body) else {
        return None;
    };
    json_compaction_count(&json)
}

fn compaction_input_rejected(status_code: u16, body: &[u8]) -> bool {
    if status_code != 400 && status_code != 422 {
        return false;
    }
    // 只读取错误本身，避免回显请求中的 trigger 让无关参数错误触发降级。
    let text = match serde_json::from_slice::<Value>(body) {
        Ok(json) => {
            let error = json.get("error").unwrap_or(&json);
            [
                error.as_str(),
                error.get("message").and_then(Value::as_str),
                error.get("code").and_then(Value::as_str),
                error.get("param").and_then(Value::as_str),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
        }
        Err(_) => String::from_utf8_lossy(body).into_owned(),
    }
    .to_ascii_lowercase();
    let unrecognized = text.contains("unknown")
        || text.contains("unrecognized")
        || text.contains("unsupported")
        || text.contains("unexpected")
        || text.contains("not supported")
        || text.contains("invalid type")
        || text.contains("invalid_type")
        || text.contains("additional propert");
    text.contains(COMPACTION_TRIGGER_TYPE) && unrecognized
}

fn sse_block_type(block: &str) -> Option<String> {
    let mut event_name = None;
    let mut data_parts = Vec::new();
    for line in block.lines() {
        if let Some(event) = strip_sse_field(line, "event") {
            event_name = Some(event.trim().to_string());
        }
        if let Some(data) = strip_sse_field(line.trim_end_matches('\r'), "data") {
            data_parts.push(data);
        }
    }
    if let Some(name) = event_name.filter(|name| !name.is_empty()) {
        return Some(name);
    }
    let data = data_parts.join("\n");
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    serde_json::from_str::<Value>(data).ok().and_then(|json| {
        json.get("type")
            .and_then(Value::as_str)
            .map(ToString::to_string)
    })
}

fn upstream_has_terminal_failure(body: &[u8], content_type: &str) -> bool {
    if response_body_is_stream(content_type, body) {
        let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
        return text.split("\n\n").any(|block| {
            matches!(
                sse_block_type(block).as_deref(),
                Some("response.failed" | "response.incomplete" | "error")
            )
        });
    }
    serde_json::from_slice::<Value>(body).is_ok_and(|json| {
        json.get("error").is_some_and(|error| !error.is_null())
            || matches!(
                json.get("status").and_then(Value::as_str),
                Some("failed" | "incomplete")
            )
    })
}

fn render_native_compaction_as_sse(body: &[u8]) -> Option<Vec<u8>> {
    let json = serde_json::from_slice::<Value>(body).ok()?;
    let response = if json.get("output").is_some() {
        json
    } else {
        json.get("response")?.clone()
    };
    let output = response.get("output")?.as_array()?;
    let mut compaction_items = output.iter().filter(|item| {
        item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE)
            && usable_compaction_content(item).is_some()
    });
    let item = compaction_items.next()?.clone();
    if compaction_items.next().is_some() {
        return None;
    }
    let mut sse = String::new();
    let mut created_response = response.clone();
    created_response["status"] = json!("in_progress");
    created_response["output"] = json!([]);
    push_sse(
        &mut sse,
        "response.created",
        json!({
            "type": "response.created",
            "sequence_number": 0,
            "response": created_response
        }),
    );
    push_sse(
        &mut sse,
        "response.output_item.added",
        json!({
            "type": "response.output_item.added",
            "sequence_number": 1,
            "output_index": 0,
            "item": item
        }),
    );
    push_sse(
        &mut sse,
        "response.output_item.done",
        json!({
            "type": "response.output_item.done",
            "sequence_number": 2,
            "output_index": 0,
            "item": item
        }),
    );
    push_sse(
        &mut sse,
        "response.completed",
        json!({
            "type": "response.completed",
            "sequence_number": 3,
            "response": response
        }),
    );
    sse.push_str("data: [DONE]\n\n");
    Some(sse.into_bytes())
}

fn incomplete_compaction_stream(body: &[u8]) -> Vec<u8> {
    let mut output = String::from_utf8_lossy(body)
        .into_owned()
        .replace("\r\n", "\n");
    if !output.is_empty() && !output.ends_with("\n\n") {
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output.push('\n');
    }
    push_sse(
        &mut output,
        "response.failed",
        json!({
            "type": "response.failed",
            "response": {
                "id": "resp_compact_incomplete",
                "status": "failed",
                "error": {
                    "message": "remote compaction v2 stream closed before response.completed",
                    "type": "stream_error"
                }
            }
        }),
    );
    output.into_bytes()
}

#[derive(Debug)]
struct NativeCompactionUnsupported {
    message: &'static str,
}

impl NativeCompactionUnsupported {
    fn history() -> Self {
        Self {
            message: "This compaction checkpoint cannot be converted to a text summary. Use a compatible Responses upstream or start a new conversation.",
        }
    }
}

impl std::fmt::Display for NativeCompactionUnsupported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for NativeCompactionUnsupported {}

fn native_compaction_unsupported_result(
    error: anyhow::Error,
) -> anyhow::Result<UpstreamProxyResponse> {
    if error
        .downcast_ref::<NativeCompactionUnsupported>()
        .is_some()
    {
        let body = json!({
            "error": {
                "message": error.to_string(),
                "type": "compaction_incompatible",
                "code": "native_compaction_unsupported"
            }
        });
        Ok(UpstreamProxyResponse {
            status_code: 400,
            content_type: "application/json".to_string(),
            is_stream: false,
            wire_api: UpstreamWireApi::Responses,
            compaction: false,
            native_compaction_passthrough: false,
            buffered_body: Some(serde_json::to_vec(&body).unwrap_or_default()),
            response: None,
        })
    } else {
        Err(error)
    }
}

fn input_has_native_compaction(body: &Value) -> bool {
    let is_native =
        |item: &Value| item.get("type").and_then(Value::as_str) == Some(COMPACTION_OUTPUT_TYPE);
    match body.get("input") {
        Some(Value::Array(items)) => items.iter().any(is_native),
        Some(Value::Object(_)) => is_native(&body["input"]),
        _ => false,
    }
}

/// 只展开带版本标识、可解码的本地摘要；未知/旧格式仍视为原生状态。
fn expand_compaction_item(item: &Value) -> Option<Value> {
    if item.get("type").and_then(Value::as_str) != Some(COMPACTION_OUTPUT_TYPE) {
        return None;
    }
    let encoded = item
        .get("encrypted_content")?
        .as_str()?
        .strip_prefix(LOCAL_COMPACTION_PREFIX)?;
    let summary: String = serde_json::from_str(encoded).ok()?;
    if summary.trim().is_empty() {
        return None;
    }
    let mut text = COMPACTION_REPLAY_PREFIX.to_string();
    text.push_str(&summary);
    Some(json!({
        "type": "message",
        "role": "user",
        "content": [{ "type": "input_text", "text": text }]
    }))
}

fn expand_local_compaction_history(body: &mut Value) {
    match body.get_mut("input") {
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(message) = expand_compaction_item(item) {
                    *item = message;
                }
            }
        }
        Some(input @ Value::Object(_)) => {
            if let Some(message) = expand_compaction_item(input) {
                *input = json!([message]);
            }
        }
        _ => {}
    }
}

impl Default for ChatSseToResponsesConverter {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            utf8_remainder: Vec::new(),
            state: ChatSseState::default(),
            failed: false,
        }
    }
}

impl ChatSseToResponsesConverter {
    pub fn has_failed(&self) -> bool {
        self.failed
    }

    pub fn has_terminal_event(&self) -> bool {
        self.failed || self.state.completed || self.state.finish_reason.is_some()
    }

    pub fn with_request(original_request: &Value) -> Self {
        Self {
            state: ChatSseState::with_request(original_request),
            ..Self::default()
        }
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) -> Vec<u8> {
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

    pub fn finish(&mut self) -> Vec<u8> {
        if !self.utf8_remainder.is_empty() {
            self.buffer
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }

        let mut output = String::new();
        if !self.failed {
            self.state.finalize_into(&mut output);
        }
        output.into_bytes()
    }

    pub fn fail(&mut self, message: String, error_type: Option<String>) -> Vec<u8> {
        let mut output = String::new();
        self.state.failed_into(&mut output, message, error_type);
        self.failed = true;
        output.into_bytes()
    }

    fn handle_block(&mut self, block: &str, output: &mut String) {
        let mut event_name: Option<String> = None;
        let mut data_parts = Vec::new();
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
        let data = data_parts.join("\n");
        if data.trim() == "[DONE]" {
            self.state.finalize_into(output);
            return;
        }

        let Ok(chunk) = serde_json::from_str::<Value>(&data) else {
            return;
        };
        if event_name.as_deref() == Some("error") || chunk.get("error").is_some() {
            let (message, error_type) = extract_chat_sse_error(&chunk);
            self.state.failed_into(output, message, error_type);
            self.failed = true;
            return;
        }
        self.state.handle_chat_chunk_into(&chunk, output);
    }
}

pub fn is_responses_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/responses"
            | "/v1/responses"
            | "/v1/v1/responses"
            | "/codex/v1/responses"
            | "/responses/compact"
            | "/v1/responses/compact"
            | "/v1/v1/responses/compact"
            | "/codex/v1/responses/compact"
    )
}

pub fn is_responses_compact_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/responses/compact"
            | "/v1/responses/compact"
            | "/v1/v1/responses/compact"
            | "/codex/v1/responses/compact"
    )
}

pub fn is_chat_completions_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/chat/completions"
            | "/v1/chat/completions"
            | "/v1/v1/chat/completions"
            | "/codex/v1/chat/completions"
    )
}

pub fn is_models_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/models" | "/v1/models" | "/v1/v1/models" | "/codex/v1/models"
    )
}

pub fn is_audio_transcriptions_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/audio/transcriptions"
            | "/v1/audio/transcriptions"
            | "/v1/v1/audio/transcriptions"
            | "/codex/v1/audio/transcriptions"
    )
}

pub fn is_image_generations_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/images/generations"
            | "/v1/images/generations"
            | "/v1/v1/images/generations"
            | "/codex/v1/images/generations"
    )
}

pub fn is_image_edits_proxy_path(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    matches!(
        path,
        "/images/edits" | "/v1/images/edits" | "/v1/v1/images/edits" | "/codex/v1/images/edits"
    )
}

pub async fn open_responses_proxy_request(
    body: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path(body, original_user_agent, "/responses").await
}

pub async fn open_responses_proxy_request_for_path(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path_with_beta(body, original_user_agent, request_path, None)
        .await
}

/// 只转发客户端声明的 beta 能力，不转发客户端鉴权。
pub async fn open_responses_proxy_request_for_path_with_beta(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_for_path_with_session_headers(
        body,
        original_user_agent,
        request_path,
        beta_features,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_for_path_with_session_headers(
    body: &str,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        original_user_agent,
        request_path,
        beta_features,
        session_headers,
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings(
    body: &str,
    settings: crate::settings::BackendSettings,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        "/responses",
        None,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        request_path,
        None,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path_and_beta(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
    beta_features: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_for_path_and_session_headers(
        body,
        settings,
        request_path,
        beta_features,
        ProxySessionHeaders::default(),
    )
    .await
}

pub async fn open_responses_proxy_request_with_settings_for_path_and_session_headers(
    body: &str,
    settings: crate::settings::BackendSettings,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_responses_proxy_request_with_settings_and_user_agent(
        body,
        settings,
        None,
        request_path,
        beta_features,
        session_headers,
    )
    .await
}

async fn open_responses_proxy_request_with_settings_and_user_agent(
    body: &str,
    settings: crate::settings::BackendSettings,
    original_user_agent: Option<&str>,
    request_path: &str,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let mut request_json: Value = serde_json::from_str(body)?;
    expand_local_compaction_history(&mut request_json);
    let is_stream = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let source_model = request_json
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let aggregate_model = match crate::aggregate_catalog::resolve_active_model(&settings, &source_model) {
        Ok(binding) => binding,
        Err(error) => {
            return Ok(UpstreamProxyResponse {
                status_code: 400,
                content_type: "application/json".to_string(),
                is_stream: false,
                wire_api: UpstreamWireApi::Responses,
                compaction: false,
                native_compaction_passthrough: false,
                buffered_body: Some(serde_json::to_vec(&json!({
                    "error": {
                        "type": "invalid_request_error",
                        "code": "aggregate_model_unavailable",
                        "message": error.to_string()
                    }
                }))?),
                response: None,
            });
        }
    };
    if let Some(binding) = &aggregate_model {
        request_json["model"] = Value::String(binding.upstream_model.clone());
    }
    let model_route = if aggregate_model.is_some() {
        None
    } else {
        select_model_route(&settings, &source_model)?
    };
    if let Some(route) = &model_route
        && route.upstream_model != source_model
    {
        request_json["model"] = Value::String(route.upstream_model.clone());
    }
    let model = (!source_model.trim().is_empty()).then(|| source_model.clone());
    let context = RotationContext {
        conversation_id: conversation_id_from_responses_request(&request_json),
        model,
    };
    let (relay, relays) = if let Some(binding) = &aggregate_model {
        (binding.relay.clone(), vec![binding.relay.clone()])
    } else if let Some(route) = &model_route {
        (route.relay.clone(), vec![route.relay.clone()])
    } else {
        let relay = crate::relay_rotation::select_relay_for_request(&settings, context)?;
        let mut relays = vec![relay.clone()];
        relays.extend(crate::relay_rotation::fallback_relays_after(
            &settings, &relay.id,
        )?);
        (relay, relays)
    };
    debug_assert_eq!(
        relays.first().map(|item| item.id.as_str()),
        Some(relay.id.as_str())
    );
    let strict_aggregate_model = aggregate_model.is_some();
    let relay_count = relays.len();
    for (attempt, relay) in relays.iter().cloned().enumerate() {
        let has_more_candidates = attempt + 1 < relay_count;
        validate_upstream(&relay)?;
        // 官方登录成员：用该账号 OAuth 凭据发往 chatgpt.com，而不是 API Key。
        let official_member =
            crate::official_login::member_uses_official_login_transport(&relay);
        let mut official_credential = if official_member {
            match crate::official_login::resolve_official_credential(&relay, false).await {
                Ok(credential) => Some(credential),
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.official_login_unavailable",
                        json!({
                            "relayId": relay.id,
                            "relayName": relay.name,
                            "error": error.to_string()
                        }),
                    );
                    if !strict_aggregate_model {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                    }
                    if has_more_candidates {
                        continue;
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        let official_base = official_member.then(crate::official_login::official_base_url);
        let model_override = aggregate_model
            .as_ref()
            .map(|binding| binding.upstream_model.clone())
            .or_else(|| aggregate_upstream_model_override(&settings, &relay));
        let preserve_native_request = relay.protocol == RelayProtocol::Responses
            && (request_has_compaction_trigger(&request_json)
                || is_responses_compact_proxy_path(request_path));
        let probe_native_compaction = request_has_compaction_trigger(&request_json)
            && relay.protocol == RelayProtocol::Responses
            && !is_responses_compact_proxy_path(request_path);
        let (mut endpoint, mut upstream_body, mut wire_api, compaction) =
            match upstream_request_parts(
                &relay,
                request_json.clone(),
                request_path,
                model_override.as_deref(),
                preserve_native_request,
                official_base.as_deref(),
            )
            .await
            {
                Ok(parts) => parts,
                Err(error) => {
                    if has_more_candidates
                        && !strict_aggregate_model
                        && error.is::<NativeCompactionUnsupported>()
                    {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                        continue;
                    }
                    return native_compaction_unsupported_result(error);
                }
            };
        let is_compaction_request = compaction || probe_native_compaction;
        let header_timeout = response_header_timeout(is_stream);
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.upstream_request",
            json!({
                "relayId": relay.id,
                "relayName": relay.name,
                "endpoint": endpoint,
                "wireApi": wire_api,
                "stream": is_stream,
                "attempt": attempt + 1,
                "candidateCount": relay_count,
                "headerTimeoutSeconds": header_timeout.as_secs(),
                "modelRoute": model_route.as_ref().map(|route| json!({
                    "sourceRelayId": route.source_relay_id,
                    "sourceModel": route.source_model,
                    "targetRelayId": route.relay.id,
                    "upstreamModel": route.upstream_model
                }))
            }),
        );
        let mut upstream = match send_responses_upstream_request(
            &relay,
            &endpoint,
            &upstream_body,
            wire_api,
            is_stream,
            original_user_agent,
            beta_features,
            session_headers,
            official_credential.as_ref(),
        )
        .await
        {
            Ok(upstream) => upstream,
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "protocol_proxy.upstream_request_failed",
                    json!({
                        "relayId": relay.id,
                        "relayName": relay.name,
                        "endpoint": endpoint,
                        "wireApi": wire_api,
                        "stream": is_stream,
                        "attempt": attempt + 1,
                        "candidateCount": relay_count,
                        "headerTimeoutSeconds": header_timeout.as_secs(),
                        "willFailover": has_more_candidates,
                        "error": error.to_string()
                    }),
                );
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                if has_more_candidates {
                    continue;
                }
                return Err(error).with_context(|| {
                    format!(
                        "供应商「{}」请求上游失败，endpoint: {}",
                        relay.name, endpoint
                    )
                });
            }
        };
        let mut status_code = upstream.status().as_u16();
        let mut content_type = upstream
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();

        // 官方登录 token 可能在 exp 之前就被撤销：401 时强制刷新并原样重试一次，
        // 仍失败再走正常的失败切换/返回路径。
        if status_code == 401 && official_member {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.official_login_retry",
                json!({
                    "relayId": relay.id,
                    "relayName": relay.name,
                    "endpoint": endpoint
                }),
            );
            match crate::official_login::resolve_official_credential(&relay, true).await {
                Ok(credential) => {
                    official_credential = Some(credential);
                    match send_responses_upstream_request(
                        &relay,
                        &endpoint,
                        &upstream_body,
                        wire_api,
                        is_stream,
                        original_user_agent,
                        beta_features,
                        session_headers,
                        official_credential.as_ref(),
                    )
                    .await
                    {
                        Ok(retried) => {
                            upstream = retried;
                            status_code = upstream.status().as_u16();
                            content_type = upstream
                                .headers()
                                .get(reqwest::header::CONTENT_TYPE)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or("")
                                .to_string();
                        }
                        Err(error) => {
                            if !strict_aggregate_model {
                                crate::relay_rotation::record_relay_request_failure(&settings);
                            }
                            if has_more_candidates {
                                continue;
                            }
                            return Err(error).with_context(|| {
                                format!(
                                    "供应商「{}」请求上游失败，endpoint: {}",
                                    relay.name, endpoint
                                )
                            });
                        }
                    }
                }
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.official_login_refresh_failed",
                        json!({
                            "relayId": relay.id,
                            "relayName": relay.name,
                            "error": error.to_string()
                        }),
                    );
                }
            }
        }

        if probe_native_compaction {
            let success = (200..300).contains(&status_code);
            let probed = match read_limited_upstream_body(
                &mut upstream,
                MAX_NATIVE_COMPACTION_PROBE_BODY_BYTES,
            )
            .await
            {
                Ok(body) => body.to_vec(),
                Err(error) => {
                    return Err(error.into());
                }
            };
            let stream_response = response_body_is_stream(&content_type, &probed);

            if success && upstream_has_terminal_failure(&probed, &content_type) {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                return Ok(UpstreamProxyResponse {
                    status_code,
                    is_stream: stream_response,
                    content_type,
                    wire_api,
                    compaction: false,
                    native_compaction_passthrough: stream_response,
                    buffered_body: Some(probed),
                    response: None,
                });
            }
            if success && stream_response {
                match upstream_compaction_item_count(&probed, &content_type) {
                    Some(1) => {
                        if !strict_aggregate_model {
                            crate::relay_rotation::record_relay_request_event(
                                &settings,
                                RotationEvent::Success,
                            );
                        }
                        return Ok(UpstreamProxyResponse {
                            status_code,
                            is_stream: true,
                            content_type,
                            wire_api,
                            compaction: false,
                            native_compaction_passthrough: true,
                            buffered_body: Some(probed),
                            response: None,
                        });
                    }
                    None => {
                        if !strict_aggregate_model {
                            crate::relay_rotation::record_relay_request_failure(&settings);
                        }
                        return Ok(UpstreamProxyResponse {
                            status_code,
                            is_stream: true,
                            content_type: "text/event-stream".to_string(),
                            wire_api,
                            compaction: false,
                            native_compaction_passthrough: true,
                            buffered_body: Some(incomplete_compaction_stream(&probed)),
                            response: None,
                        });
                    }
                    Some(_) => {}
                }
            } else if success
                && upstream_compaction_item_count(&probed, &content_type) == Some(1)
            {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_event(
                        &settings,
                        RotationEvent::Success,
                    );
                }
                if !is_stream {
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: false,
                        content_type,
                        wire_api,
                        compaction: false,
                        native_compaction_passthrough: true,
                        buffered_body: Some(probed),
                        response: None,
                    });
                }
                if let Some(sse) = render_native_compaction_as_sse(&probed) {
                    return Ok(UpstreamProxyResponse {
                        status_code,
                        is_stream: true,
                        content_type: "text/event-stream".to_string(),
                        wire_api,
                        compaction: false,
                        native_compaction_passthrough: true,
                        buffered_body: Some(sse),
                        response: None,
                    });
                }
            }
            if !success && !compaction_input_rejected(status_code, &probed) {
                if !strict_aggregate_model {
                    crate::relay_rotation::record_relay_request_failure(&settings);
                }
                if has_more_candidates {
                    continue;
                }
                return Ok(UpstreamProxyResponse {
                    status_code,
                    is_stream: false,
                    content_type,
                    wire_api,
                    compaction: false,
                    native_compaction_passthrough: false,
                    buffered_body: Some(probed),
                    response: None,
                });
            }

            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.compaction_fallback",
                json!({
                    "relayId": relay.id,
                    "statusCode": status_code,
                }),
            );
            let fallback = upstream_request_parts(
                &relay,
                request_json.clone(),
                request_path,
                model_override.as_deref(),
                false,
                official_base.as_deref(),
            )
            .await;
            let (fallback_endpoint, fallback_body, fallback_wire, _) = match fallback {
                Ok(parts) => parts,
                Err(error) => {
                    if has_more_candidates
                        && !strict_aggregate_model
                        && error.is::<NativeCompactionUnsupported>()
                    {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                        continue;
                    }
                    return native_compaction_unsupported_result(error);
                }
            };
            endpoint = fallback_endpoint;
            upstream_body = fallback_body;
            wire_api = fallback_wire;
            upstream = match send_responses_upstream_request(
                &relay,
                &endpoint,
                &upstream_body,
                wire_api,
                is_stream,
                original_user_agent,
                beta_features,
                session_headers,
                official_credential.as_ref(),
            )
            .await
            {
                Ok(upstream) => upstream,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "protocol_proxy.upstream_request_failed",
                        json!({
                            "relayId": relay.id,
                            "endpoint": endpoint,
                            "compactionFallback": true,
                            "error": error.to_string()
                        }),
                    );
                    if !strict_aggregate_model {
                        crate::relay_rotation::record_relay_request_failure(&settings);
                    }
                    if has_more_candidates {
                        continue;
                    }
                    return Err(error).with_context(|| {
                        format!(
                            "供应商「{}」请求上游失败，endpoint: {}",
                            relay.name, endpoint
                        )
                    });
                }
            };
            status_code = upstream.status().as_u16();
            content_type = upstream
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("")
                .to_string();
        }

        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.upstream_response",
            json!({
                "relayId": relay.id,
                "relayName": relay.name,
                "endpoint": endpoint,
                "wireApi": wire_api,
                "stream": is_stream,
                "statusCode": status_code,
                "attempt": attempt + 1,
                "candidateCount": relay_count,
                "headerTimeoutSeconds": header_timeout.as_secs(),
                "willFailover": has_more_candidates && !(200..300).contains(&status_code)
            }),
        );
        if !strict_aggregate_model {
            crate::relay_rotation::record_relay_request_event(
                &settings,
                if (200..300).contains(&status_code) {
                    RotationEvent::Success
                } else {
                    RotationEvent::Failure
                },
            );
        }
        if (200..300).contains(&status_code) || !has_more_candidates {
            // 请求了 SSE 的兼容上游也可能返回 JSON 摘要；让调用方选择 JSON 包装器。
            let json_compaction = is_compaction_request
                && content_type
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"));
            return Ok(UpstreamProxyResponse {
                status_code,
                is_stream: (is_stream && !json_compaction)
                    || content_type.contains("text/event-stream"),
                content_type,
                wire_api,
                compaction: is_compaction_request,
                native_compaction_passthrough: false,
                buffered_body: None,
                response: Some(upstream),
            });
        }
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.upstream_failover",
            json!({
                "relayId": relay.id,
                "relayName": relay.name,
                "endpoint": endpoint,
                "wireApi": wire_api,
                "stream": is_stream,
                "statusCode": status_code,
                "attempt": attempt + 1,
                "candidateCount": relay_count,
                "headerTimeoutSeconds": header_timeout.as_secs()
            }),
        );
    }
    anyhow::bail!("未找到可用的聚合供应商成员")
}

fn select_model_route(
    settings: &crate::settings::BackendSettings,
    model: &str,
) -> anyhow::Result<Option<ModelRouteSelection>> {
    if model.is_empty() || settings.active_aggregate_relay_profile().is_some() {
        return Ok(None);
    }

    let source = settings.active_relay_profile();
    let Some(route) = source
        .model_routes
        .iter()
        .find(|route| route.model.trim() == model)
    else {
        return Ok(None);
    };
    let target_relay_id = route.target_relay_id.trim();
    if target_relay_id == source.id {
        anyhow::bail!("模型路由不能指向当前供应商自身：{model}");
    }
    let target = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == target_relay_id)
        .cloned()
        .with_context(|| format!("模型路由目标供应商不存在：{target_relay_id}"))?;
    if target.relay_mode == crate::settings::RelayMode::Aggregate {
        anyhow::bail!("模型路由目标不能是聚合供应商：{}", target.name);
    }
    if target.protocol != RelayProtocol::Responses {
        anyhow::bail!("模型路由目标必须使用 Responses API：{}", target.name);
    }

    let upstream_model = if route.target_model.trim().is_empty() {
        model.to_string()
    } else {
        route.target_model.trim().to_string()
    };
    Ok(Some(ModelRouteSelection {
        relay: target,
        source_relay_id: source.id,
        source_model: model.to_string(),
        upstream_model,
    }))
}

fn aggregate_upstream_model_override(
    settings: &crate::settings::BackendSettings,
    relay: &crate::settings::RelayProfile,
) -> Option<String> {
    settings.active_aggregate_relay_profile()?;
    let model = crate::relay_config::relay_profile_model(relay);
    let model = model.trim();
    (!model.is_empty()).then(|| model.to_string())
}

pub async fn open_models_proxy_request(
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    validate_upstream(&relay)?;

    // 官方登录成员没有 base_url：模型列表同样发往 chatgpt.com 后端并带账号身份。
    let official_member = crate::official_login::member_uses_official_login_transport(&relay);
    let official_credential = if official_member {
        Some(crate::official_login::resolve_official_credential(&relay, false).await?)
    } else {
        None
    };
    let base_url = if official_member {
        crate::official_login::official_base_url()
    } else {
        relay.base_url.clone()
    };
    let endpoint = models_url(&base_url);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.models_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": UpstreamWireApi::Responses
        }),
    );
    let request = crate::http_client::proxied_client(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ))?
    .get(endpoint);
    let request = match &official_credential {
        Some(credential) => with_official_login_auth(request, &relay, credential),
        None => with_relay_auth(request, &relay),
    };
    let upstream = send_upstream_request(request).await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api: UpstreamWireApi::Responses,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        response: Some(upstream),
    })
}

pub async fn open_audio_transcriptions_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    validate_upstream(&relay)?;
    if crate::official_login::member_uses_official_login_transport(&relay) {
        anyhow::bail!("音频转写暂不支持官方登录成员「{}」", relay.name);
    }
    let content_type = content_type.trim();
    if content_type.is_empty() {
        anyhow::bail!("Audio transcriptions 请求缺少 Content-Type");
    }

    let endpoint = audio_transcriptions_url(&relay.base_url);
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.audio_transcriptions_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": UpstreamWireApi::AudioTranscriptions,
            "bodyBytes": body.len()
        }),
    );
    let request = crate::http_client::proxied_client(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ))?
    .post(endpoint)
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(body.to_vec());
    let upstream = send_upstream_request(with_relay_auth(request, &relay)).await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api: UpstreamWireApi::AudioTranscriptions,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        response: Some(upstream),
    })
}

pub async fn open_image_generations_proxy_request(
    body: &[u8],
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_image_proxy_request(
        body,
        "application/json",
        original_user_agent,
        ImageProxyEndpoint::Generations,
    )
    .await
}

pub async fn open_image_edits_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    open_image_proxy_request(
        body,
        content_type,
        original_user_agent,
        ImageProxyEndpoint::Edits,
    )
    .await
}

#[derive(Debug, Clone, Copy)]
enum ImageProxyEndpoint {
    Generations,
    Edits,
}

impl ImageProxyEndpoint {
    fn url(self, base_url: &str) -> String {
        match self {
            Self::Generations => image_generations_url(base_url),
            Self::Edits => image_edits_url(base_url),
        }
    }

    fn wire_api(self) -> UpstreamWireApi {
        match self {
            Self::Generations => UpstreamWireApi::ImageGenerations,
            Self::Edits => UpstreamWireApi::ImageEdits,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Generations => "image_generations",
            Self::Edits => "image_edits",
        }
    }
}

async fn open_image_proxy_request(
    body: &[u8],
    content_type: &str,
    original_user_agent: Option<&str>,
    endpoint_kind: ImageProxyEndpoint,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = crate::relay_rotation::select_relay_for_probe(&settings)?;
    if crate::official_login::member_uses_official_login_transport(&relay) {
        anyhow::bail!("图片请求暂不支持官方登录成员「{}」", relay.name);
    }
    let base_url = if relay.upstream_base_url.trim().is_empty() {
        crate::relay_config::relay_profile_base_url(&relay)
    } else {
        relay.upstream_base_url.trim().to_string()
    };
    if is_local_protocol_proxy_base_url(&base_url) {
        anyhow::bail!("图片上游 Base URL 不能指向本地协议代理");
    }
    if base_url.trim().is_empty() {
        anyhow::bail!("图片上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("图片上游 Key 不能为空");
    }
    let content_type = content_type.trim();
    let content_type = if content_type.is_empty() {
        match endpoint_kind {
            ImageProxyEndpoint::Generations => "application/json",
            ImageProxyEndpoint::Edits => {
                anyhow::bail!("图片 edits 请求缺少 Content-Type");
            }
        }
    } else {
        content_type
    };
    let endpoint = endpoint_kind.url(&base_url);
    let wire_api = endpoint_kind.wire_api();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.image_request",
        json!({
            "relayId": relay.id,
            "relayName": relay.name,
            "endpoint": endpoint,
            "wireApi": wire_api,
            "bodyBytes": body.len(),
            "endpointKind": endpoint_kind.name()
        }),
    );
    let request = crate::http_client::proxied_client(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ))?
    .post(endpoint)
    .header(reqwest::header::CONTENT_TYPE, content_type)
    .body(body.to_vec());
    let upstream = send_upstream_request_with_header_timeout(
        with_relay_auth(request, &relay),
        UPSTREAM_IMAGE_HEADER_TIMEOUT,
    )
    .await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: false,
        content_type,
        wire_api,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        response: Some(upstream),
    })
}

fn response_header_timeout(is_stream: bool) -> Duration {
    if is_stream {
        UPSTREAM_STREAM_HEADER_TIMEOUT
    } else {
        UPSTREAM_HEADER_TIMEOUT
    }
}

pub async fn open_chat_completions_proxy_request(
    body: &str,
    original_user_agent: Option<&str>,
) -> anyhow::Result<UpstreamProxyResponse> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let relay = settings.active_relay_profile();
    if relay.protocol != RelayProtocol::ChatCompletions {
        anyhow::bail!("当前中转未启用 Chat Completions 协议代理");
    }
    if relay.base_url.trim().is_empty() {
        anyhow::bail!("Chat Completions 上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("Chat Completions 上游 Key 不能为空");
    }

    let request_json: Value = serde_json::from_str(body)?;
    let is_stream = request_json
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let request = crate::http_client::proxied_client(&effective_user_agent(
        &relay.user_agent,
        original_user_agent,
    ))?
    .post(chat_completions_url(&relay.base_url))
    .header(reqwest::header::CONTENT_TYPE, "application/json")
    .json(&request_json);
    let upstream = with_relay_auth(request, &relay).send().await?;
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();

    Ok(UpstreamProxyResponse {
        status_code,
        is_stream: is_stream || content_type.contains("text/event-stream"),
        content_type,
        wire_api: UpstreamWireApi::ChatCompletions,
        compaction: false,
        native_compaction_passthrough: false,
        buffered_body: None,
        response: Some(upstream),
    })
}

/// 补齐搜索历史的工具声明，不删历史，也不为无工具的压缩请求启用搜索。
pub fn ensure_web_search_tool_for_history(request_json: &mut Value) -> bool {
    let Some(input) = request_json.get("input").and_then(Value::as_array) else {
        return false;
    };
    if !input.iter().any(|item| item["type"] == "web_search_call") {
        return false;
    }
    let is_web_search = |tool: &Value| {
        matches!(
            tool.get("type").and_then(Value::as_str),
            Some("web_search" | "web_search_preview" | "web_search_preview_2025_03_11")
        )
    };
    let mut has_additional_tools = false;
    for tool in input
        .iter()
        .filter(|item| item["type"] == "additional_tools")
        .filter_map(|item| item.get("tools").and_then(Value::as_array))
        .flatten()
    {
        has_additional_tools = true;
        if is_web_search(tool) {
            return false;
        }
    }
    let has_tools = match request_json.get("tools") {
        None | Some(Value::Null) => false,
        Some(Value::Array(tools)) => {
            if tools.iter().any(is_web_search) {
                return false;
            }
            !tools.is_empty()
        }
        _ => return false,
    } || has_additional_tools;
    let tool = json!({"type": "web_search", "external_web_access": false});
    if let Some(tools) = request_json.get_mut("tools").and_then(Value::as_array_mut) {
        tools.push(tool);
    } else {
        request_json["tools"] = json!([tool]);
    }
    if !has_tools
        && (matches!(request_json.get("tool_choice"), None | Some(Value::Null))
            || matches!(
                request_json.get("tool_choice").and_then(Value::as_str),
                Some("auto" | "none")
            ))
    {
        request_json["tool_choice"] = json!("none");
    }
    true
}

/// 前缀对齐诊断事件（命名沿用 prefix_stabilizer_* 风格）。
fn log_compaction_align(report: &crate::compaction_align::CompactionAlignReport) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "protocol_proxy.prefix_stabilizer_compaction_appended",
        json!({
            "instances": report.instances,
            "moved": report.moved,
            "appended": report.appended,
            "toolsBackfilled": report.tools_backfilled,
            "skippedMixed": report.skipped_mixed,
        }),
    );
}

async fn upstream_request_parts(
    relay: &crate::settings::RelayProfile,
    mut request_json: Value,
    request_path: &str,
    model_override: Option<&str>,
    preserve_compaction_trigger: bool,
    official_base: Option<&str>,
) -> anyhow::Result<(String, Value, UpstreamWireApi, bool)> {
    let compact = is_responses_compact_proxy_path(request_path)
        || request_has_compaction_trigger(&request_json);
    let is_v2_compaction = compact && request_has_compaction_trigger(&request_json);
    if relay.protocol == RelayProtocol::ChatCompletions
        && input_has_native_compaction(&request_json)
    {
        return Err(NativeCompactionUnsupported::history().into());
    }
    // 原生 v2 上游要先看到 compaction_trigger。只有确认它没有返回 compaction 项后才剥掉重试。
    if compact && !preserve_compaction_trigger {
        request_json = rewrite_request_for_compaction(strip_compaction_trigger(request_json));
        if input_has_native_compaction(&request_json) {
            return Err(NativeCompactionUnsupported::history().into());
        }
    }
    let synthetic_compaction = compact && relay.protocol == RelayProtocol::ChatCompletions;
    if synthetic_compaction && relay.stabilize_prompt_prefix {
        // 稳定层：压缩轮自带 tools:[]，转换层会丢掉折叠历史 tool 项的 schema，
        // 全历史重构、前缀全弃。rewrite 已在上方公共块完成且对 ChatCompletions 恒执行
        // （preserve_native_request 要求 Responses 协议），这里只做 tools 字节回填
        //（缓存按 prompt_cache_key 取回，与 rewrite 顺序无关）+ 前缀对齐：
        // 把客户端塞进 system 区的指令复制件搬走。
        if let Some(tools) = crate::compaction_tools::preserved_tools(&request_json) {
            request_json["tools"] = tools;
        }
        if let Some((aligned, report)) =
            crate::compaction_align::align_compaction_request(&request_json)
        {
            log_compaction_align(&report);
            request_json = aligned;
        }
    } else if relay.stabilize_prompt_prefix {
        // 514-msg 流式压缩路径（codex++ 自动压缩）：既不走 /responses/compact
        // 也不带 compaction_trigger，compact 判定落空；由前缀对齐按指令签名检出，
        // 做「末尾唯一 user 指令 + tools 字节回填」。检出即跳过 remember_tools，
        // 杜绝把压缩轮的 tools=[] 毒化进回填缓存；未检出才是普通轮，照常记录。
        let aligned = if relay.protocol == RelayProtocol::ChatCompletions {
            crate::compaction_align::align_compaction_request(&request_json)
        } else {
            None
        };
        match aligned {
            Some((aligned, report)) => {
                log_compaction_align(&report);
                request_json = aligned;
            }
            None => crate::compaction_tools::remember_tools(&request_json),
        }
    }
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        request_json["model"] = json!(model);
    }
    // 探针用 key 需在 request_json 被转换层 move 之前取出。
    let probe_key = if relay.protocol == RelayProtocol::ChatCompletions
        && relay.stabilize_prompt_prefix
        && !compact
    {
        request_json
            .get("prompt_cache_key")
            .and_then(Value::as_str)
            .map(str::to_string)
    } else {
        None
    };
    let mut body = match relay.protocol {
        RelayProtocol::Responses => request_json,
        RelayProtocol::ChatCompletions => responses_to_chat_completions_with_options(
            request_json,
            relay.standard_openai_protocol,
            relay.stabilize_prompt_prefix,
        )?,
    };
    if relay.protocol == RelayProtocol::Responses {
        if relay.web_search_history_compat_enabled()
            && !is_responses_compact_proxy_path(request_path)
            && ensure_web_search_tool_for_history(&mut body)
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.web_search_history_compat",
                json!({"action": "declare_web_search", "inputContainsWebSearchCall": true}),
            );
        }
        normalize_responses_item_ids(&mut body);
    }

    // 前缀分叉探针（opt-in）：普通轮观察「system 块序列 + tools」前缀，
    // 字节突变即 dump 分叉点（重启首轮重放取证用）；压缩轮前缀本就有意
    // 重写，跳过观察以免噪声。
    if relay.protocol == RelayProtocol::ChatCompletions && relay.stabilize_prompt_prefix && !compact {
        if let Some(report) = crate::prefix_divergence::observe(probe_key.as_deref(), &body) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                if matches!(report.kind, crate::prefix_divergence::DivergenceKind::Baseline) {
                    "protocol_proxy.prefix_stabilizer_prefix_baseline"
                } else {
                    "protocol_proxy.prefix_stabilizer_prefix_divergence"
                },
                json!({
                    "oldLen": report.old_len,
                    "newLen": report.new_len,
                    "firstDiff": report.first_diff,
                    "oldSnip": report.old_snip,
                    "newSnip": report.new_snip,
                }),
            );
        }
    }

    // Image handling (per-model): send-as-is / strip / VLM analysis
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if !model.is_empty() {
        use crate::vision::ImageHandling;
        match crate::vision::image_handling_mode(&model, &relay.model_vlm) {
            ImageHandling::SendAsIs => { /* 不做任何处理 */ }
            ImageHandling::Strip => {
                for key in &["messages", "input"] {
                    if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                        crate::vision::strip_images_only(arr);
                    }
                }
            }
            ImageHandling::Vlm => {
                if !relay.vlm_api_key.is_empty()
                    && !relay.vlm_model.is_empty()
                    && !relay.vlm_base_url.is_empty()
                {
                    let vlm_config = crate::vision::VlmConfig {
                        api_key: relay.vlm_api_key.clone(),
                        model: relay.vlm_model.clone(),
                        base_url: relay.vlm_base_url.clone(),
                    };

                    for key in &["messages", "input"] {
                        if let Some(arr) = body.get_mut(key).and_then(Value::as_array_mut) {
                            crate::vision::strip_image_blocks(
                                arr,
                                &vlm_config,
                                &relay.model_windows,
                                &relay.context_window,
                                &model,
                                relay.protocol == crate::settings::RelayProtocol::Responses,
                            )
                            .await;
                        }
                    }
                }
            }
        }
    }

    if guard_inline_image_data_urls(&mut body) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "inline_image_data_url_leak",
            json!({ "model": model, "protocol": format!("{:?}", relay.protocol) }),
        );
        debug_assert!(false, "base64 图片泄漏进文本字段，检查协议转换路径");
    }

    let wire_api = match relay.protocol {
        RelayProtocol::Responses => UpstreamWireApi::Responses,
        RelayProtocol::ChatCompletions => UpstreamWireApi::ChatCompletions,
    };
    // 官方登录成员没有用户填写的 base_url，固定发往 chatgpt.com 后端。
    let endpoint_base = official_base
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(relay.base_url.trim());
    Ok((
        match relay.protocol {
            // v2 压缩请求走普通 /responses 端点（compaction_trigger 在 input 里）；
            // 显式 legacy compact 保留原端点。
            RelayProtocol::Responses if compact && !is_v2_compaction => {
                responses_compact_url(endpoint_base)
            }
            RelayProtocol::Responses => responses_url(endpoint_base),
            RelayProtocol::ChatCompletions => chat_completions_url(endpoint_base),
        },
        body,
        wire_api,
        synthetic_compaction,
    ))
}

async fn send_responses_upstream_request(
    relay: &crate::settings::RelayProfile,
    endpoint: &str,
    upstream_body: &Value,
    wire_api: UpstreamWireApi,
    is_stream: bool,
    original_user_agent: Option<&str>,
    beta_features: Option<&str>,
    session_headers: ProxySessionHeaders<'_>,
    official_credential: Option<&crate::official_login::OfficialCredential>,
) -> anyhow::Result<reqwest::Response> {
    let mut builder = upstream_request_builder(
        crate::http_client::proxied_client(&effective_user_agent(
            &relay.user_agent,
            original_user_agent,
        ))?,
        endpoint,
        relay,
        is_stream,
        upstream_body,
        official_credential,
    );
    builder = with_client_session_headers(builder, relay, session_headers);
    if wire_api == UpstreamWireApi::Responses {
        if let Some(value) = beta_features.filter(|value| !value.is_empty()) {
            builder = builder.header("x-codex-beta-features", value);
        }
    }
    send_upstream_request_for_responses(builder, is_stream).await
}

fn upstream_request_builder(
    client: reqwest::Client,
    endpoint: &str,
    relay: &crate::settings::RelayProfile,
    is_stream: bool,
    upstream_body: &Value,
    official_credential: Option<&crate::official_login::OfficialCredential>,
) -> reqwest::RequestBuilder {
    let mut builder = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json");
    builder = match official_credential {
        Some(credential) => with_official_login_auth(builder, relay, credential),
        None => with_relay_auth(builder, relay),
    };
    if is_stream {
        builder = builder
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
    }
    builder.json(upstream_body)
}

fn validate_upstream(relay: &crate::settings::RelayProfile) -> anyhow::Result<()> {
    // 纯官方 profile 没有 URL/Key：凭据齐全时走官方登录通道，否则给出可行动的错误。
    if crate::official_login::is_official_login_profile(relay) && relay.base_url.trim().is_empty()
    {
        if crate::official_login::official_login_available(relay) {
            return Ok(());
        }
        anyhow::bail!(
            "官方登录成员「{}」缺少 ChatGPT 登录凭据，请先在管理器中完成或导入官方登录",
            relay.name
        );
    }
    if relay.base_url.trim().is_empty() {
        anyhow::bail!("上游 Base URL 不能为空");
    }
    if relay.api_key.trim().is_empty() && !relay.uses_no_auth() {
        anyhow::bail!("上游 Key 不能为空");
    }
    Ok(())
}

fn with_relay_auth(
    request: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
) -> reqwest::RequestBuilder {
    // 认证（API Key / 无认证）+ 供应商自定义请求头，统一在 relay_headers 里决定优先级。
    crate::relay_headers::apply(request, relay)
}

/// 官方登录成员的上游认证：OAuth Bearer + `chatgpt-account-id`，
/// 供应商自定义头照常保留（显式 `Authorization` 除外，官方身份不能被用户头覆盖）。
fn with_official_login_auth(
    mut builder: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
    credential: &crate::official_login::OfficialCredential,
) -> reqwest::RequestBuilder {
    // originator 是官方后端识别客户端来源的固定头（Codex CLI 同值）。
    builder = builder
        .bearer_auth(credential.access_token.trim())
        .header("originator", "codex_cli_rs");
    if !credential.account_id.trim().is_empty() {
        builder = builder.header("chatgpt-account-id", credential.account_id.trim());
    }
    let custom_headers: Vec<crate::settings::RelayHeaderKeyValue> = relay
        .custom_headers
        .iter()
        .filter(|header| !header.key.trim().eq_ignore_ascii_case("authorization"))
        .cloned()
        .collect();
    crate::relay_headers::apply_headers(builder, &custom_headers)
}

fn with_client_session_headers(
    mut builder: reqwest::RequestBuilder,
    relay: &crate::settings::RelayProfile,
    headers: ProxySessionHeaders<'_>,
) -> reqwest::RequestBuilder {
    for (name, value) in [
        ("session-id", headers.session_id),
        ("thread-id", headers.thread_id),
        ("x-opencode-session", headers.opencode_session),
    ] {
        // 显式供应商配置优先，避免 reqwest 追加出两个同名头。
        if relay
            .custom_headers
            .iter()
            .any(|header| header.key.trim().eq_ignore_ascii_case(name))
        {
            continue;
        }
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            continue;
        };
        if let Ok(value) = reqwest::header::HeaderValue::from_str(value) {
            builder = builder.header(name, value);
        }
    }
    builder
}

fn conversation_id_from_responses_request(body: &Value) -> Option<String> {
    for key in ["conversation", "conversation_id", "previous_response_id"] {
        if let Some(value) = body.get(key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn effective_user_agent(configured_user_agent: &str, original_user_agent: Option<&str>) -> String {
    let configured_user_agent = configured_user_agent.trim();
    if !configured_user_agent.is_empty() {
        return configured_user_agent.to_string();
    }
    original_user_agent
        .map(str::trim)
        .filter(|user_agent| !user_agent.is_empty())
        .unwrap_or("")
        .to_string()
}

pub async fn handle_responses_proxy_request(body: &str) -> anyhow::Result<ProxyHttpResponse> {
    let request_json: Value = serde_json::from_str(body)?;
    let mut upstream = open_responses_proxy_request(body, None).await?;
    let is_compaction = upstream.compaction;
    let status_code = upstream.status_code;
    let upstream_content_type = upstream.content_type.clone();
    let is_stream = upstream.is_stream;
    let wire_api = upstream.wire_api;
    let upstream_body = upstream.read_body().await?;

    if !(200..300).contains(&status_code) {
        let error =
            responses_error_from_upstream(status_code, &upstream_content_type, &upstream_body);
        return Ok(ProxyHttpResponse {
            status: http_status_line(status_code),
            content_type: "application/json; charset=utf-8".to_string(),
            body: serde_json::to_vec(&error)?,
        });
    }

    if upstream.native_compaction_passthrough {
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: if upstream_content_type.is_empty() {
                if is_stream {
                    "text/event-stream; charset=utf-8".to_string()
                } else {
                    "application/json; charset=utf-8".to_string()
                }
            } else {
                upstream_content_type
            },
            body: upstream_body,
        });
    }

    if is_compaction {
        // v2 压缩：无论上游协议/是否流式，都重组为单个 compaction 输出项。
        let model = request_json
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !is_stream {
            return Ok(ProxyHttpResponse {
                status: "200 OK".to_string(),
                content_type: "text/event-stream; charset=utf-8".to_string(),
                body: wrap_non_stream_response_as_compaction(&upstream_body, model)?,
            });
        }
        let mut converter = CompactionSseConverter::new(model);
        if wire_api != UpstreamWireApi::Responses {
            converter = converter.with_chat_upstream();
        }
        // 整包已收齐，直接喂给有状态 SSE 解析器（与 launcher 逐 chunk 路径同逻辑）。
        converter.push_upstream_bytes(&upstream_body);
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: "text/event-stream; charset=utf-8".to_string(),
            body: converter.finish(),
        });
    }

    if wire_api == UpstreamWireApi::Responses {
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: if upstream_content_type.is_empty() {
                "application/json; charset=utf-8".to_string()
            } else {
                upstream_content_type
            },
            body: upstream_body.to_vec(),
        });
    }

    if is_stream {
        let text = String::from_utf8_lossy(&upstream_body);
        return Ok(ProxyHttpResponse {
            status: "200 OK".to_string(),
            content_type: "text/event-stream; charset=utf-8".to_string(),
            body: chat_sse_to_responses_sse_with_request(&text, &request_json).into_bytes(),
        });
    }

    let chat_json: Value = serde_json::from_slice(&upstream_body)?;
    let response_json = chat_completion_to_response_with_request(chat_json, &request_json)?;
    Ok(ProxyHttpResponse {
        status: "200 OK".to_string(),
        content_type: "application/json; charset=utf-8".to_string(),
        body: serde_json::to_vec(&response_json)?,
    })
}

pub fn chat_completions_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/chat/completions") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/chat/completions")
    } else {
        format!("{base}/v1/chat/completions")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn responses_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/responses") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/responses")
    } else {
        format!("{base}/v1/responses")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn responses_compact_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/responses/compact") {
        return base.to_string();
    }
    format!("{}/compact", responses_url(base_url).trim_end_matches('/'))
}

pub fn audio_transcriptions_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base.to_ascii_lowercase().ends_with("/audio/transcriptions") {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/audio/transcriptions")
    } else {
        format!("{base}/v1/audio/transcriptions")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub fn image_generations_url(base_url: &str) -> String {
    image_endpoint_url(base_url, "generations")
}

pub fn image_edits_url(base_url: &str) -> String {
    image_endpoint_url(base_url, "edits")
}

fn image_endpoint_url(base_url: &str, endpoint: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let base = base_url.trim().trim_end_matches('#').trim_end_matches('/');
    if base
        .to_ascii_lowercase()
        .ends_with(&format!("/images/{endpoint}"))
    {
        return base.to_string();
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(base) || !origin_only {
        format!("{base}/images/{endpoint}")
    } else {
        format!("{base}/v1/images/{endpoint}")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

fn is_local_protocol_proxy_base_url(base_url: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url.trim()) else {
        return false;
    };
    if !url.scheme().eq_ignore_ascii_case("http") || url.port() != Some(protocol_proxy_port()) {
        return false;
    }
    matches!(
        url.host_str(),
        Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
    )
}

#[cfg(test)]
mod image_proxy_tests {
    use super::UPSTREAM_IMAGE_HEADER_TIMEOUT;
    use super::is_local_protocol_proxy_base_url;
    use std::time::Duration;

    #[test]
    fn image_requests_allow_ten_minutes_for_response_headers() {
        assert_eq!(UPSTREAM_IMAGE_HEADER_TIMEOUT, Duration::from_secs(600));
    }

    #[test]
    fn local_protocol_proxy_detection_covers_common_loopback_forms() {
        for base_url in [
            "http://127.0.0.1:57321",
            "http://127.0.0.1:57321/",
            "http://127.0.0.1:57321/v1",
            "http://localhost:57321/v1/",
            "http://[::1]:57321/v1",
        ] {
            assert!(is_local_protocol_proxy_base_url(base_url), "{base_url}");
        }

        for base_url in [
            "https://127.0.0.1:57321/v1",
            "http://127.0.0.1:57322/v1",
            "http://api.example.test:57321/v1",
            "not-a-url",
        ] {
            assert!(!is_local_protocol_proxy_base_url(base_url), "{base_url}");
        }
    }
}

pub fn models_url(base_url: &str) -> String {
    let skip_version_prefix = base_url.trim().ends_with('#');
    let mut base = base_url
        .trim()
        .trim_end_matches('#')
        .trim_end_matches('/')
        .to_string();
    if base.to_ascii_lowercase().ends_with("/chat/completions") {
        base.truncate(base.len() - "/chat/completions".len());
    }
    if base.to_ascii_lowercase().ends_with("/models") {
        return base;
    }
    let origin_only = base
        .split_once("://")
        .map_or(!base.contains('/'), |(_, rest)| !rest.contains('/'));
    let mut url = if skip_version_prefix || has_version_suffix(&base) || !origin_only {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    };
    while url.contains("/v1/v1") {
        url = url.replace("/v1/v1", "/v1");
    }
    url
}

pub(crate) fn has_version_suffix(base_url: &str) -> bool {
    let segment = base_url.rsplit('/').next().unwrap_or(base_url);
    let Some(rest) = segment.strip_prefix('v') else {
        return false;
    };
    rest.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

pub fn chat_sse_to_responses_sse(input: &str) -> String {
    let mut converter = ChatSseToResponsesConverter::default();
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8(output).unwrap_or_default()
}

pub fn chat_sse_to_responses_sse_with_request(input: &str, original_request: &Value) -> String {
    let mut converter = ChatSseToResponsesConverter::with_request(original_request);
    let mut output = converter.push_bytes(input.as_bytes());
    output.extend(converter.finish());
    String::from_utf8(output).unwrap_or_default()
}

pub fn response_id_from_chat_id(id: Option<&str>) -> String {
    let id = id.unwrap_or("compat");
    if id.starts_with("resp_") {
        id.to_string()
    } else {
        format!("resp_{id}")
    }
}

/// `resp_xxx` → `xxx`。message item 的 id 必须以 `msg_` 开头，
/// 直接拼在 `response_id` 后面会得到上游拒收的 `resp_xxx_msg`（#1431）。
fn response_id_body(response_id: &str) -> &str {
    response_id
        .strip_prefix("resp_")
        .filter(|value| !value.is_empty())
        .unwrap_or(response_id)
}

fn push_sse(output: &mut String, event: &str, data: Value) {
    output.push_str("event: ");
    output.push_str(event);
    output.push_str("\ndata: ");
    output.push_str(&serde_json::to_string(&data).unwrap_or_default());
    output.push_str("\n\n");
}

#[derive(Debug, Default)]
struct TextItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Default)]
struct ReasoningItemState {
    output_index: Option<u32>,
    item_id: String,
    text: String,
    added: bool,
    done: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum InlineThinkMode {
    #[default]
    Detecting,
    Reasoning,
    Text,
}

#[derive(Debug, Default)]
struct InlineThinkState {
    mode: InlineThinkMode,
    buffer: String,
}

#[derive(Debug, Default)]
struct ToolCallState {
    output_index: Option<u32>,
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    added: bool,
    done: bool,
}

#[derive(Debug)]
struct ChatSseState {
    response_started: bool,
    completed: bool,
    response_id: String,
    model: String,
    created_at: u64,
    next_output_index: u32,
    text: TextItemState,
    reasoning: ReasoningItemState,
    inline_think: InlineThinkState,
    tools: BTreeMap<usize, ToolCallState>,
    output_items: Vec<(u32, Value)>,
    latest_usage: Option<Value>,
    finish_reason: Option<String>,
    tool_context: CodexToolContext,
    original_request: Option<Value>,
}

impl Default for ChatSseState {
    fn default() -> Self {
        Self {
            response_started: false,
            completed: false,
            response_id: "resp_compat".to_string(),
            model: String::new(),
            created_at: 0,
            next_output_index: 0,
            text: TextItemState::default(),
            reasoning: ReasoningItemState::default(),
            inline_think: InlineThinkState::default(),
            tools: BTreeMap::new(),
            output_items: Vec::new(),
            latest_usage: None,
            finish_reason: None,
            tool_context: CodexToolContext::default(),
            original_request: None,
        }
    }
}

impl ChatSseState {
    fn with_request(original_request: &Value) -> Self {
        let mut tool_context = build_codex_tool_context(original_request.get("tools"));
        for namespace_tool in &collect_tool_search_output_namespaces(original_request) {
            add_namespace_tools_to_context(&mut tool_context, namespace_tool);
        }
        Self {
            tool_context,
            original_request: Some(original_request.clone()),
            ..Self::default()
        }
    }

    fn handle_chat_chunk_into(&mut self, chunk: &Value, output: &mut String) {
        if let Some(id) = chunk.get("id").and_then(Value::as_str) {
            self.response_id = response_id_from_chat_id(Some(id));
        }
        if let Some(model) = chunk.get("model").and_then(Value::as_str) {
            if !model.is_empty() {
                self.model = model.to_string();
            }
        }
        if let Some(created) = chunk.get("created").and_then(Value::as_u64) {
            self.created_at = created;
        }
        self.ensure_response_started_into(output);

        if let Some(usage) = chunk.get("usage").filter(|value| !value.is_null()) {
            self.latest_usage = Some(chat_usage_to_responses_usage(Some(usage)));
        }

        let Some(choice) = chunk
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return;
        };

        if let Some(delta) = choice.get("delta") {
            if let Some(reasoning) = chat_delta_reasoning_text(delta) {
                self.push_reasoning_delta_into(&reasoning, output);
            }

            if let Some(content) = delta.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    self.push_content_delta_into(content, output);
                }
            }

            if let Some(tool_calls) = delta.get("tool_calls").and_then(Value::as_array) {
                self.flush_inline_think_at_boundary_into(output);
                self.finalize_reasoning_into(output);
                for tool_call in tool_calls {
                    self.push_tool_call_delta_into(tool_call, output);
                }
            }
        }

        if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(finish_reason.to_string());
        }
    }

    fn push_content_delta_into(&mut self, delta: &str, output: &mut String) {
        match self.inline_think.mode {
            InlineThinkMode::Text => {
                self.finalize_reasoning_into(output);
                self.push_text_delta_into(delta, output);
            }
            InlineThinkMode::Detecting => {
                self.inline_think.buffer.push_str(delta);
                match leading_think_prefix_decision(&self.inline_think.buffer) {
                    ThinkPrefixDecision::NeedMore => {}
                    ThinkPrefixDecision::Reasoning => {
                        self.inline_think.mode = InlineThinkMode::Reasoning;
                        self.drain_complete_inline_think_into(output);
                    }
                    ThinkPrefixDecision::Text => {
                        self.inline_think.mode = InlineThinkMode::Text;
                        let text = std::mem::take(&mut self.inline_think.buffer);
                        self.finalize_reasoning_into(output);
                        self.push_text_delta_into(&text, output);
                    }
                }
            }
            InlineThinkMode::Reasoning => {
                self.inline_think.buffer.push_str(delta);
                self.drain_complete_inline_think_into(output);
            }
        }
    }

    fn drain_complete_inline_think_into(&mut self, output: &mut String) {
        let Some((reasoning, answer)) = split_leading_think_block(&self.inline_think.buffer) else {
            return;
        };
        self.inline_think.mode = InlineThinkMode::Text;
        self.inline_think.buffer.clear();
        if !reasoning.is_empty() {
            self.push_reasoning_delta_into(&reasoning, output);
            self.finalize_reasoning_into(output);
        }
        if !answer.is_empty() {
            self.push_text_delta_into(&answer, output);
        }
    }

    fn flush_inline_think_at_boundary_into(&mut self, output: &mut String) {
        match self.inline_think.mode {
            InlineThinkMode::Text => {}
            InlineThinkMode::Detecting => {
                self.inline_think.mode = InlineThinkMode::Text;
                let text = std::mem::take(&mut self.inline_think.buffer);
                if !text.is_empty() {
                    self.finalize_reasoning_into(output);
                    self.push_text_delta_into(&text, output);
                }
            }
            InlineThinkMode::Reasoning => {
                let buffered = std::mem::take(&mut self.inline_think.buffer);
                self.inline_think.mode = InlineThinkMode::Text;
                if let Some((reasoning, answer)) = split_leading_think_block(&buffered) {
                    if !reasoning.is_empty() {
                        self.push_reasoning_delta_into(&reasoning, output);
                        self.finalize_reasoning_into(output);
                    }
                    if !answer.is_empty() {
                        self.push_text_delta_into(&answer, output);
                    }
                    return;
                }
                let reasoning = strip_leading_think_open_tag(&buffered).unwrap_or(buffered);
                if !reasoning.is_empty() {
                    self.push_reasoning_delta_into(&reasoning, output);
                    self.finalize_reasoning_into(output);
                }
            }
        }
    }

    fn ensure_response_started_into(&mut self, output: &mut String) {
        if self.response_started {
            return;
        }
        self.response_started = true;
        push_sse(
            output,
            "response.created",
            json!({
                "type": "response.created",
                "response": self.base_response("in_progress", Vec::new())
            }),
        );
        push_sse(
            output,
            "response.in_progress",
            json!({
                "type": "response.in_progress",
                "response": self.base_response("in_progress", Vec::new())
            }),
        );
    }

    fn push_reasoning_delta_into(&mut self, delta: &str, output: &mut String) {
        if !self.reasoning.added {
            let output_index = self.next_output_index();
            let item_id = format!("rs_{}", self.response_id);
            self.reasoning.output_index = Some(output_index);
            self.reasoning.item_id = item_id.clone();
            self.reasoning.added = true;

            push_sse(
                output,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "reasoning",
                        "status": "in_progress",
                        "reasoning_content": "",
                        "summary": []
                    }
                }),
            );
            push_sse(
                output,
                "response.reasoning_summary_part.added",
                json!({
                    "type": "response.reasoning_summary_part.added",
                    "item_id": self.reasoning.item_id,
                    "output_index": output_index,
                    "summary_index": 0,
                    "part": { "type": "summary_text", "text": "" }
                }),
            );
        }

        self.reasoning.text.push_str(delta);
        let output_index = self.reasoning.output_index.unwrap_or(0);
        push_sse(
            output,
            "response.reasoning_summary_text.delta",
            json!({
                "type": "response.reasoning_summary_text.delta",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "delta": delta
            }),
        );
    }

    fn push_text_delta_into(&mut self, delta: &str, output: &mut String) {
        if !self.text.added {
            let output_index = self.next_output_index();
            let item_id = format!("msg_{}", response_id_body(&self.response_id));
            self.text.output_index = Some(output_index);
            self.text.item_id = item_id.clone();
            self.text.added = true;
            push_sse(
                output,
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": item_id,
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": []
                    }
                }),
            );
            push_sse(
                output,
                "response.content_part.added",
                json!({
                    "type": "response.content_part.added",
                    "item_id": self.text.item_id,
                    "output_index": output_index,
                    "content_index": 0,
                    "part": { "type": "output_text", "text": "", "annotations": [] }
                }),
            );
        }

        self.text.text.push_str(delta);
        let output_index = self.text.output_index.unwrap_or(0);
        push_sse(
            output,
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "delta": delta
            }),
        );
    }

    fn push_tool_call_delta_into(&mut self, tool_call: &Value, output: &mut String) {
        let chat_index = tool_call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
        let id_delta = tool_call
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let function = tool_call.get("function").unwrap_or(&Value::Null);
        let name_delta = function
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string);
        let args_delta = function
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let mut should_add = false;
        let mut output_index = None;
        let mut item_id = String::new();
        let mut pending_arguments = String::new();

        {
            let state = self.tools.entry(chat_index).or_default();
            if let Some(id) = id_delta {
                state.call_id = id;
            }
            // 流式场景下 thought_signature/extra_content 通常随首个分片到达，
            // 此时 call_id 已就位；按 call_id 记住，供下一轮回传（issue #332/#1012）。
            if !state.call_id.is_empty() {
                remember_tool_call_extra_content(&state.call_id, tool_call);
            }
            if let Some(name) = name_delta {
                if !name.is_empty() {
                    state.name = name;
                }
            }
            if !args_delta.is_empty() {
                state.arguments.push_str(&args_delta);
            }

            // Custom tool output items must use the `ctc_` ID namespace. Some
            // Chat Completions providers send the call ID before the function
            // name, so wait for the name when the request includes custom tools
            // instead of emitting a provisional `function_call` with an `fc_`
            // ID that cannot later be replayed as a `custom_tool_call`.
            let waiting_for_custom_tool_name =
                self.tool_context.has_custom_tools && state.name.is_empty();
            if !state.added
                && (!state.call_id.is_empty() || !state.name.is_empty())
                && !waiting_for_custom_tool_name
            {
                should_add = true;
                pending_arguments = state.arguments.clone();
            } else if state.added {
                output_index = state.output_index;
                item_id = state.item_id.clone();
            }
        }

        if should_add {
            let assigned = self.next_output_index();
            let state = self.tools.get_mut(&chat_index).expect("tool state exists");
            state.added = true;
            if state.call_id.is_empty() {
                state.call_id = format!("call_{chat_index}");
            }
            if state.name.is_empty() {
                state.name = "unknown_tool".to_string();
            }
            state.output_index = Some(assigned);
            state.item_id = tool_call_item_id(&state.call_id, &state.name, &self.tool_context);
            let added_item = tool_call_added_item(state, assigned, &self.tool_context);
            push_sse(output, "response.output_item.added", added_item);
            if !pending_arguments.is_empty() {
                push_tool_call_delta_sse(
                    output,
                    state,
                    assigned,
                    &pending_arguments,
                    &self.tool_context,
                );
            }
        } else if !args_delta.is_empty() {
            if let Some(output_index) = output_index {
                let state = ToolCallState {
                    output_index: Some(output_index),
                    item_id,
                    name: self
                        .tools
                        .get(&chat_index)
                        .map(|state| state.name.clone())
                        .unwrap_or_default(),
                    call_id: self
                        .tools
                        .get(&chat_index)
                        .map(|state| state.call_id.clone())
                        .unwrap_or_default(),
                    ..ToolCallState::default()
                };
                push_tool_call_delta_sse(
                    output,
                    &state,
                    output_index,
                    &args_delta,
                    &self.tool_context,
                );
            }
        }
    }

    fn finalize_into(&mut self, output: &mut String) {
        if self.completed {
            return;
        }
        self.ensure_response_started_into(output);
        self.flush_inline_think_at_boundary_into(output);
        self.finalize_reasoning_into(output);
        self.finalize_text_into(output);
        self.finalize_tools_into(output);

        if self
            .output_items
            .iter()
            .any(|(_, item)| invalid_web_search_output_item(item))
        {
            let mut response = self.base_response("failed", self.completed_output_items());
            response["error"] = json!({
                "type": "invalid_upstream_tool_call",
                "message": "Chat 上游返回了未执行的原生 web_search 调用，请使用客户端搜索工具"
            });
            copy_response_request_fields(&mut response, self.original_request.as_ref());
            push_sse(
                output,
                "response.failed",
                json!({
                    "type": "response.failed",
                    "response": response
                }),
            );
            self.completed = true;
            return;
        }

        let status = response_status(self.finish_reason.as_deref());
        let mut response = self.base_response(status, self.completed_output_items());
        if status == "incomplete" {
            response["incomplete_details"] = json!({ "reason": "max_output_tokens" });
        }
        copy_response_request_fields(&mut response, self.original_request.as_ref());
        push_sse(
            output,
            "response.completed",
            json!({
                "type": "response.completed",
                "response": response
            }),
        );
        output.push_str("data: [DONE]\n\n");
        self.completed = true;
    }

    fn finalize_reasoning_into(&mut self, output: &mut String) {
        if !self.reasoning.added || self.reasoning.done {
            return;
        }
        let output_index = self.reasoning.output_index.unwrap_or(0);
        let item = json!({
            "id": self.reasoning.item_id,
            "type": "reasoning",
            "reasoning_content": self.reasoning.text,
            "summary": [{ "type": "summary_text", "text": self.reasoning.text }]
        });
        self.output_items.push((output_index, item.clone()));
        self.reasoning.done = true;
        push_sse(
            output,
            "response.reasoning_summary_text.done",
            json!({
                "type": "response.reasoning_summary_text.done",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "text": self.reasoning.text
            }),
        );
        push_sse(
            output,
            "response.reasoning_summary_part.done",
            json!({
                "type": "response.reasoning_summary_part.done",
                "item_id": self.reasoning.item_id,
                "output_index": output_index,
                "summary_index": 0,
                "part": { "type": "summary_text", "text": self.reasoning.text }
            }),
        );
        push_sse(
            output,
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        );
    }

    fn finalize_text_into(&mut self, output: &mut String) {
        if !self.text.added || self.text.done {
            return;
        }
        let output_index = self.text.output_index.unwrap_or(0);
        let item = json!({
            "id": self.text.item_id,
            "type": "message",
            "status": "completed",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": self.text.text, "annotations": [] }]
        });
        self.output_items.push((output_index, item.clone()));
        self.text.done = true;
        push_sse(
            output,
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "text": self.text.text
            }),
        );
        push_sse(
            output,
            "response.content_part.done",
            json!({
                "type": "response.content_part.done",
                "item_id": self.text.item_id,
                "output_index": output_index,
                "content_index": 0,
                "part": { "type": "output_text", "text": self.text.text, "annotations": [] }
            }),
        );
        push_sse(
            output,
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": item
            }),
        );
    }

    fn finalize_tools_into(&mut self, output: &mut String) {
        let keys: Vec<usize> = self.tools.keys().copied().collect();
        for key in keys {
            if self.tools.get(&key).map(|state| state.done).unwrap_or(true) {
                continue;
            }
            if self
                .tools
                .get(&key)
                .map(|state| !state.added && !state.done)
                .unwrap_or(false)
            {
                let assigned = self.next_output_index();
                let state = self.tools.get_mut(&key).expect("tool state exists");
                state.added = true;
                if state.call_id.is_empty() {
                    state.call_id = format!("call_{key}");
                }
                if state.name.is_empty() {
                    state.name = "unknown_tool".to_string();
                }
                state.output_index = Some(assigned);
                state.item_id = tool_call_item_id(&state.call_id, &state.name, &self.tool_context);
                let added_item = tool_call_added_item(state, assigned, &self.tool_context);
                push_sse(output, "response.output_item.added", added_item);
            }

            let state = self.tools.get_mut(&key).expect("tool state exists");
            let output_index = state.output_index.unwrap_or(0);
            let item = tool_call_done_item(state, &self.tool_context);
            state.done = true;
            self.output_items.push((output_index, item.clone()));
            push_tool_call_done_sse(output, state, output_index, &self.tool_context);
            push_sse(
                output,
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": item
                }),
            );
        }
    }

    fn failed_into(&mut self, output: &mut String, message: String, error_type: Option<String>) {
        self.completed = true;
        let mut error = json!({ "message": message });
        if let Some(error_type) = error_type.filter(|value| !value.is_empty()) {
            error["type"] = json!(error_type);
        }
        let mut response = self.base_response("failed", self.completed_output_items());
        response["error"] = error;
        push_sse(
            output,
            "response.failed",
            json!({
                "type": "response.failed",
                "response": response
            }),
        );
    }

    fn completed_output_items(&self) -> Vec<Value> {
        let mut output_items = self.output_items.clone();
        output_items.sort_by_key(|(output_index, _)| *output_index);
        output_items.into_iter().map(|(_, item)| item).collect()
    }

    fn base_response(&self, status: &str, output: Vec<Value>) -> Value {
        json!({
            "id": self.response_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": output,
            "usage": self.latest_usage.clone().unwrap_or_else(default_responses_usage)
        })
    }

    fn next_output_index(&mut self) -> u32 {
        let index = self.next_output_index;
        self.next_output_index += 1;
        index
    }
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

fn chat_delta_reasoning_text(delta: &Value) -> Option<String> {
    extract_reasoning_field_text(delta)
}

enum ThinkPrefixDecision {
    NeedMore,
    Reasoning,
    Text,
}

fn leading_think_prefix_decision(buffer: &str) -> ThinkPrefixDecision {
    let trimmed = buffer.trim_start();
    if trimmed.is_empty() {
        return ThinkPrefixDecision::NeedMore;
    }
    if trimmed.starts_with(THINK_OPEN_TAG) {
        return ThinkPrefixDecision::Reasoning;
    }
    if THINK_OPEN_TAG.starts_with(trimmed) {
        return ThinkPrefixDecision::NeedMore;
    }
    ThinkPrefixDecision::Text
}

fn extract_chat_sse_error(value: &Value) -> (String, Option<String>) {
    let error = value.get("error").unwrap_or(value);
    let message = error
        .as_str()
        .map(ToString::to_string)
        .or_else(|| {
            error
                .get("message")
                .or_else(|| error.get("detail"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| error.to_string());
    let error_type = error
        .get("type")
        .or_else(|| error.get("code"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    (message, error_type)
}

fn http_status_line(status: u16) -> String {
    match status {
        200 => "200 OK".to_string(),
        400 => "400 Bad Request".to_string(),
        401 => "401 Unauthorized".to_string(),
        403 => "403 Forbidden".to_string(),
        404 => "404 Not Found".to_string(),
        429 => "429 Too Many Requests".to_string(),
        500 => "500 Internal Server Error".to_string(),
        502 => "502 Bad Gateway".to_string(),
        503 => "503 Service Unavailable".to_string(),
        _ => format!("{status} Upstream"),
    }
}

pub fn responses_error_from_upstream(status_code: u16, content_type: &str, body: &[u8]) -> Value {
    let (message, error_type, code, param) = upstream_error_parts(status_code, content_type, body);
    let mut error = json!({
        "message": message,
        "type": error_type.unwrap_or_else(|| "upstream_error".to_string()),
    });
    if let Some(code) = code {
        error["code"] = json!(code);
    }
    if let Some(param) = param {
        error["param"] = json!(param);
    }
    json!({ "error": error })
}

fn upstream_error_parts(
    status_code: u16,
    content_type: &str,
    body: &[u8],
) -> (String, Option<String>, Option<String>, Option<String>) {
    if content_type.to_ascii_lowercase().contains("json") {
        if let Ok(value) = serde_json::from_slice::<Value>(body) {
            let error = value.get("error").unwrap_or(&value);
            let message = error
                .get("message")
                .or_else(|| error.get("detail"))
                .or_else(|| error.get("error"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| truncate_error_preview(&value.to_string()));
            let error_type = error
                .get("type")
                .or_else(|| error.get("error_type"))
                .and_then(Value::as_str)
                .map(ToString::to_string);
            let code = error.get("code").and_then(|value| {
                value
                    .as_str()
                    .map(ToString::to_string)
                    .or_else(|| value.as_i64().map(|number| number.to_string()))
            });
            let param = error
                .get("param")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            return (message, error_type, code, param);
        }
    }

    let preview = truncate_error_preview(&String::from_utf8_lossy(body));
    let message = if preview.trim().is_empty() {
        format!("Upstream returned HTTP {status_code}")
    } else {
        preview
    };
    (message, None, Some(status_code.to_string()), None)
}

fn truncate_error_preview(input: &str) -> String {
    input.chars().take(ERROR_BODY_PREVIEW_LIMIT).collect()
}

/// Responses 协议要求每个 input item 的 `id` 前缀与它的 `type` 对应，
/// 例如 `message` 必须是 `msg_`、`function_call` 必须是 `fc_`。
/// 前缀不匹配时上游直接以 `[ApiIdParam] [input[N].id] [invalid_id_prefix]` 拒绝**整份**请求，
/// 于是这段历史被反复重放、会话永久不可用（#1431 / #1781 / #1796）。
///
/// 前缀表取自 Codex 自己的 rollout 记录（`~/.codex/sessions/**/rollout-*.jsonl`），
/// 不是猜测：`message`/`reasoning`/`custom_tool_call`/`custom_tool_call_output`/
/// `function_call`/`function_call_output` 分别对应 msg_/rs_/ctc_/ctco_/fc_/fco_。
const RESPONSES_ITEM_ID_PREFIXES: &[(&str, &str)] = &[
    ("message", "msg_"),
    ("reasoning", "rs_"),
    ("compaction", "cmp_"),
    ("function_call", "fc_"),
    ("function_call_output", "fco_"),
    ("custom_tool_call", "ctc_"),
    ("custom_tool_call_output", "ctco_"),
];

/// 供集成测试直接验证前缀归一结果：`upstream_request_parts` 需要真实网络，
/// 用它测不方便。
#[doc(hidden)]
pub fn normalize_responses_item_ids_for_test(body: &mut Value) {
    normalize_responses_item_ids(body);
}

/// 出站前把所有已知 item 的 id 前缀修正到与 `type` 一致。
///
/// 只认前缀表里的类型，未知类型原样通过——宁可放过，也不要把看不出类型语义的 id 改坏。
fn normalize_responses_item_ids(body: &mut Value) {
    let Some(input) = body.get_mut("input") else {
        return;
    };
    match input {
        Value::Array(items) => {
            for item in items {
                normalize_responses_item_id(item);
            }
        }
        Value::Object(_) => normalize_responses_item_id(input),
        _ => {}
    }
}

fn normalize_responses_item_id(item: &mut Value) {
    let Some(item_type) = item.get("type").and_then(Value::as_str) else {
        return;
    };
    let Some((_, prefix)) = RESPONSES_ITEM_ID_PREFIXES
        .iter()
        .find(|(kind, _)| *kind == item_type)
    else {
        return;
    };
    let Some(id) = item.get("id").and_then(Value::as_str) else {
        return;
    };
    // 只有「前缀 + 非空后缀」才算已经合规。id 恰好等于前缀本身（`fc_`）是空壳，
    // 放它过去会直接触发上游的 invalid_id_prefix，所以落到下面按 call_id 重建。
    if id.len() > prefix.len() && id.starts_with(prefix) {
        return;
    }
    // 剥掉 id 上现有的前缀再换新的。取**最长**匹配：`fc_` 是 `fco_` 的前缀，
    // 先撞上短的会把 `fco_call_a` 剥成 `call_a` 再换成 `fc_call_a`，
    // 把 function_call_output 错改成 function_call。
    // `item_` 是历史版本 Codex++ 自己造的前缀；`cp_` 来自压缩项错误使用
    // `cp_{response_id}` 的版本；`resp_` 来自历史版本把 message item 命名成
    // `{response_id}_msg`（#1431 / #1781），都要一并剥掉。
    let suffix = ["item_", "cp_", "resp_"]
        .into_iter()
        .chain(RESPONSES_ITEM_ID_PREFIXES.iter().map(|(_, known)| *known))
        .filter_map(|known| id.strip_prefix(known).map(|rest| (known.len(), rest)))
        .max_by_key(|(len, _)| *len)
        .map(|(_, rest)| rest)
        .unwrap_or(id);
    // 剥完是空串说明 id 恰好只由某个前缀组成，退回 call_id，再退回原 id。
    let suffix = if suffix.is_empty() {
        item.get("call_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or(id)
    } else {
        suffix
    };
    item["id"] = json!(format!("{prefix}{suffix}"));
}

fn append_responses_input(input: &Value, messages: &mut Vec<Value>) {
    match input {
        Value::String(text) => messages.push(json!({ "role": "user", "content": text })),
        Value::Array(items) => {
            let mut pending_tool_calls = Vec::new();
            let mut pending_reasoning = Vec::new();
            let mut seen_tool_call_ids = BTreeSet::new();
            for item in items {
                append_responses_item(
                    item,
                    messages,
                    &mut pending_tool_calls,
                    &mut pending_reasoning,
                    &mut seen_tool_call_ids,
                );
            }
            flush_tool_calls(messages, &mut pending_tool_calls, &mut pending_reasoning);
            flush_reasoning(messages, &mut pending_reasoning);
        }
        Value::Object(_) => {
            let mut pending_tool_calls = Vec::new();
            let mut pending_reasoning = Vec::new();
            let mut seen_tool_call_ids = BTreeSet::new();
            append_responses_item(
                input,
                messages,
                &mut pending_tool_calls,
                &mut pending_reasoning,
                &mut seen_tool_call_ids,
            );
            flush_tool_calls(messages, &mut pending_tool_calls, &mut pending_reasoning);
            flush_reasoning(messages, &mut pending_reasoning);
        }
        _ => {}
    }
}

fn append_responses_item(
    item: &Value,
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Vec<String>,
    seen_tool_call_ids: &mut BTreeSet<String>,
) {
    match item.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            let name = responses_history_function_name(item);
            if name.is_empty() {
                return;
            }
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            seen_tool_call_ids.insert(call_id.to_string());
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": responses_arguments_to_chat(item.get("arguments").unwrap_or(&json!({})))
                }
            }));
            // 把上一轮上游附带的透传数据挂回（Gemini thought_signature 等）。
            // 缺失时保持原样，不影响其它供应商。
            if let Some(extra) = recall_tool_call_extra_content(call_id) {
                if let Some(last) = pending_tool_calls.last_mut() {
                    last["extra_content"] = extra;
                }
            }
        }
        Some("function_call_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(
                    call_id,
                    item.get("output").unwrap_or(&Value::Null),
                ));
                return;
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(item.get("output").unwrap_or(&Value::Null))
            }));
        }
        Some("custom_tool_call") => {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("");
            let input = item
                .get("input")
                .or_else(|| item.get("arguments"))
                .unwrap_or(&Value::Null);
            let (name, arguments) = build_custom_tool_call_history(name, input);
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            seen_tool_call_ids.insert(call_id.to_string());
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": arguments
                }
            }));
        }
        Some("web_search_call") => {
            // codex 的 web_search 由客户端执行，item 是「调用+结果」合体形态。
            // 此前该 item 没有 content 字段，回放时整项被 `_` 兜底静默丢弃：
            // 模型既看不到自己搜过、也看不到结果，导致重复搜索与轮次失联
            // （2026-10-11 多 Agent 会话实锤：3 轮 8 次重复搜索、两轮空 FINAL）。
            // 回放为 assistant 调用 + tool 结果消息：query/sources 存在则透传，
            // 被客户端剥离时如实告知状态，杜绝模型失忆重搜。
            let call_id = item.get("id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            let action = item.get("action").cloned().unwrap_or_else(|| json!({}));
            let query = action
                .get("query")
                .and_then(Value::as_str)
                .or_else(|| action.get("input").and_then(Value::as_str))
                .unwrap_or("");
            let arguments = serde_json::to_string(&json!({ "input": query }))
                .unwrap_or_else(|_| "{\"input\":\"\"}".to_string());
            seen_tool_call_ids.insert(call_id.to_string());
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": { "name": "web_search", "arguments": arguments }
            }));
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            let status = item.get("status").and_then(Value::as_str).unwrap_or("completed");
            let content = match action.get("sources").or_else(|| action.get("results")) {
                Some(sources) if !sources.is_null() => {
                    serde_json::to_string(sources).unwrap_or_else(|_| format!("Web search ({status}) results: {sources}"))
                }
                _ => format!(
                    "Web search ({status}) was performed; its query and results were not preserved in this history. Do not repeat it; continue with the available context."
                ),
            };
            messages.push(json!({ "role": "tool", "tool_call_id": call_id, "content": content }));
        }
        Some("tool_search_call") => {
            // Codex 的 tool_search 是客户端执行的代理工具，历史回放时按
            // function tool_call 形态映射进 chat 消息流。
            let call_id = item
                .get("call_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            seen_tool_call_ids.insert(call_id.to_string());
            pending_tool_calls.push(json!({
                "id": call_id,
                "type": "function",
                "function": {
                    "name": "tool_search",
                    "arguments": responses_arguments_to_chat(
                        item.get("arguments").unwrap_or(&json!({}))
                    )
                }
            }));
        }
        Some("tool_search_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            let output = item.get("tools").unwrap_or(&Value::Null);
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(call_id, output));
                return;
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(output)
            }));
        }
        Some("custom_tool_call_output") => {
            let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            if !seen_tool_call_ids.contains(call_id) {
                flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(
                    call_id,
                    item.get("output").unwrap_or(&Value::Null),
                ));
                return;
            }
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(item.get("output").unwrap_or(&Value::Null))
            }));
        }
        Some("tool_call") => {
            if let Some(tool_use) = item.get("tool_use") {
                let call_id = tool_use
                    .get("id")
                    .or_else(|| item.get("call_id"))
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if call_id.is_empty() {
                    return;
                }
                seen_tool_call_ids.insert(call_id.to_string());
                pending_tool_calls.push(json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": tool_use.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": responses_arguments_to_chat(tool_use.get("input").unwrap_or(&json!({})))
                    }
                }));
            }
        }
        Some("tool_result") => {
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            let content = item.get("content").unwrap_or(&Value::Null);
            let call_id = content
                .get("tool_use_id")
                .or_else(|| item.get("tool_call_id"))
                .or_else(|| item.get("call_id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return;
            }
            let output = content.get("content").unwrap_or(content);
            if !seen_tool_call_ids.contains(call_id) {
                flush_reasoning(messages, pending_reasoning);
                messages.push(orphan_tool_output_message(call_id, output));
                return;
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": tool_output_content(output)
            }));
        }
        Some("reasoning") => {
            if let Some(text) = responses_reasoning_text(item) {
                if !text.is_empty() {
                    pending_reasoning.push(text);
                }
            }
        }
        Some(COMPACTION_TRIGGER_TYPE) => {
            // 控制项不进上游历史；正常请求不该出现，出现即忽略。
        }
        _ => {
            flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
            if let Some(content) = item.get("content") {
                let role = responses_role_to_chat_role(item.get("role").and_then(Value::as_str));
                if content.is_null() && role != "assistant" {
                    return;
                }
                let mut message = json!({
                    "role": role,
                    "content": responses_content_to_chat_content(role, content)
                });
                if role == "assistant" {
                    if !pending_reasoning.is_empty() && pending_tool_calls.is_empty() {
                        message["reasoning_content"] =
                            json!(std::mem::take(pending_reasoning).join("\n"));
                    }
                } else if !pending_reasoning.is_empty() {
                    flush_tool_calls(messages, pending_tool_calls, pending_reasoning);
                    flush_reasoning(messages, pending_reasoning);
                }
                messages.push(message);
            }
        }
    }
}

fn orphan_tool_output_message(call_id: &str, output: &Value) -> Value {
    // 这条已经是 user 消息，multi-part 图片可以直接内联，不必走 relocate。
    if let Value::Array(parts) = tool_output_content(output) {
        let mut content = vec![json!({
            "type": "text",
            "text": format!("Function call output ({call_id}):")
        })];
        content.extend(parts);
        return json!({ "role": "user", "content": content });
    }
    json!({
        "role": "user",
        "content": format!(
            "Function call output ({call_id}): {}",
            response_output_text(output)
        )
    })
}

/// Chat Completions 上游（DeepSeek thinking 模式尤其严格）要求带 `tool_calls` 的
/// assistant 消息后面必须紧跟每个 `tool_call_id` 对应的 `tool` 消息。中断/回滚过的
/// 一轮会话可能留下没有 output 的 `function_call`，直接转发会被上游 400
/// （insufficient tool messages following tool_calls message）。
///
/// 这里把没有配对 output 的 tool_call 从消息里摘掉，降级成文本保留在历史中，
/// 避免丢失「模型曾试图调用某工具」这一信息。
/// 把插在「连续 tool 结果」之间的非 tool 消息整体搬到该 tool 区之后
/// （issue #2275 / #2257）。
///
/// 上游 codex 会把 `<image_resize_notice>` 这类提示以 developer（映射成 system）
/// 或 user 消息的形式插在两条 tool 结果之间，形成夹心结构：
/// `assistant(tool_calls=[a,b]) → tool(a) → developer → tool(b)`。
/// `enforce_tool_call_pairing` 用 `take_while` 只收集「role 连续为 tool」的后续消息，
/// 数到夹心就停，于是 followers 只有 1 条、`b` 被误判 orphaned 并从 `tool_calls`
/// 摘掉，但 `b` 的 tool 消息还留在原地 —— 这正是「role 'tool' 无前置 tool_calls」
/// 与「No tool output found for tool call」的来源。
///
/// 这里在配对判定**之前**把夹心消息移到 tool 区之后，使 tool 结果重新连续。
/// system 形态的夹心会被后续的 `collapse_system_messages_to_head` 带到头部（合法），
/// user 形态的则留在 tool 区之后（同样合法）。
fn relocate_interleaved_non_tool_messages(messages: &mut Vec<Value>) {
    let mut index = 0;
    while index < messages.len() {
        let Some(tool_calls) = messages[index].get("tool_calls").and_then(Value::as_array) else {
            index += 1;
            continue;
        };
        let mut unanswered: BTreeSet<String> = tool_calls
            .iter()
            .filter_map(|tool_call| tool_call.get("id").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        if unanswered.is_empty() {
            index += 1;
            continue;
        }

        // 从本条 assistant 往后扫，收集本轮的所有 tool 结果与夹在其中的非 tool 消息，
        // 直到 tool_call 集合配齐、撞上下一条 assistant（新轮次，不能越界）、或到底。
        let mut tool_messages: Vec<Value> = Vec::new();
        let mut interleaved: Vec<Value> = Vec::new();
        let mut scan = index + 1;
        while scan < messages.len() {
            let role = messages[scan].get("role").and_then(Value::as_str);
            if role == Some("assistant") {
                break;
            }
            if role == Some("tool") {
                if let Some(id) = messages[scan].get("tool_call_id").and_then(Value::as_str) {
                    unanswered.remove(id);
                }
                tool_messages.push(messages[scan].clone());
            } else if !tool_messages.is_empty() {
                // 只有已经收到过 tool 结果之后的夹心才值得搬：
                // 本条 assistant 尚未收到任何结果时，中间的消息是正常历史，不是夹心。
                interleaved.push(messages[scan].clone());
            }
            scan += 1;
            if unanswered.is_empty() {
                break;
            }
        }

        if interleaved.is_empty() {
            index += 1;
            continue;
        }

        // 重建这段区间：tool 结果。配齐时）在前、夹心消息在后。
        let rebuilt: Vec<Value> = tool_messages.into_iter().chain(interleaved).collect();
        let rebuilt_len = rebuilt.len();
        messages.splice(index + 1..scan, rebuilt);
        // 跳过刚重建的区间，避免在搬动过的消息上重复扫描导致死循环。
        index += 1 + rebuilt_len;
    }
}

/// 把没有前置 `tool_calls` 的 tool 消息降级成 user（issue #2275 / #2257 的防线）。
///
/// `enforce_tool_call_pairing` 目前只清理 assistant 侧（把 orphaned 的 tool_call
/// 从 `tool_calls` 摘掉），被摘掉的那些 tool 消息本身仍留在原位，上游会直接
/// 400「No tool output found for tool call」。这里把它们降级成 user，保住内容
/// 且不再触发协议错误。
fn degrade_unpaired_tool_messages(messages: &mut [Value]) {
    let mut known: BTreeSet<String> = BTreeSet::new();
    for message in messages.iter() {
        if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
            for tool_call in tool_calls {
                if let Some(id) = tool_call.get("id").and_then(Value::as_str) {
                    known.insert(id.to_string());
                }
            }
        }
    }

    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let paired = message
            .get("tool_call_id")
            .and_then(Value::as_str)
            .is_some_and(|id| known.contains(id));
        if paired {
            continue;
        }
        let content = message.get("content").cloned().unwrap_or(Value::Null);
        *message = json!({
            "role": "user",
            "content": content
        });
    }
}

fn enforce_tool_call_pairing(messages: &mut [Value]) {
    let mut index = 0;
    while index < messages.len() {
        if messages[index].get("role").and_then(Value::as_str) != Some("assistant") {
            index += 1;
            continue;
        }
        let Some(tool_calls) = messages[index].get("tool_calls").and_then(Value::as_array) else {
            index += 1;
            continue;
        };
        if tool_calls.is_empty() {
            index += 1;
            continue;
        }

        // 收集紧跟其后的 tool 消息所应答的 id
        let mut answered = BTreeSet::new();
        let mut followers = 0;
        for message in messages[index + 1..]
            .iter()
            .take_while(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        {
            followers += 1;
            if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                answered.insert(id.to_string());
            }
        }

        // 位于历史尾部的 tool_call 是「刚发起、output 还没回来」的正常形态，
        // 上游本就期待它；只有序列越过了它却没应答才是非法的。
        if index + 1 + followers >= messages.len() {
            index += 1;
            continue;
        }

        let (kept, orphaned): (Vec<Value>, Vec<Value>) =
            tool_calls.iter().cloned().partition(|tool_call| {
                tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| answered.contains(id))
            });
        if orphaned.is_empty() {
            index += 1;
            continue;
        }

        let notes = orphaned
            .iter()
            .map(|tool_call| {
                let name = tool_call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");
                format!("Abandoned function call ({id}): {name}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        if kept.is_empty() {
            if let Some(message) = messages[index].as_object_mut() {
                message.remove("tool_calls");
            }
        } else {
            messages[index]["tool_calls"] = json!(kept);
        }
        append_text_to_assistant_message(&mut messages[index], &notes);
        index += 1;
    }
}

/// `role:"tool"` 消息在多数 Chat Completions 上游（DeepSeek 在内）只接受字符串 content，
/// 塞 multi-part `image_url` 会被 400；而 `enforce_tool_call_pairing` 又要求 tool 消息
/// 紧跟在 assistant(tool_calls) 之后**连续**排列 —— 把图片消息插在两条 tool 消息中间，
/// 会让后面那条不再被计入 followers，对应的 tool_call 被误判成 orphaned 而摘掉。
///
/// 两个约束都要满足，所以这里的做法是：tool 消息本身降级成文本占位，图片一路收集到
/// 连续 tool 区结束，再作为一条 user 消息整体插在其后。
fn relocate_tool_output_images(messages: &mut Vec<Value>) {
    let mut index = 0;
    while index < messages.len() {
        if messages[index].get("role").and_then(Value::as_str) != Some("tool") {
            index += 1;
            continue;
        }
        let mut images = Vec::new();
        let mut end = index;
        while end < messages.len()
            && messages[end].get("role").and_then(Value::as_str) == Some("tool")
        {
            take_images_from_tool_message(&mut messages[end], &mut images);
            end += 1;
        }
        if !images.is_empty() {
            let mut content = vec![json!({
                "type": "text",
                "text": "Images returned by the tool call(s) above:"
            })];
            content.append(&mut images);
            messages.insert(end, json!({ "role": "user", "content": content }));
            end += 1;
        }
        index = end;
    }
}

/// 摘掉单条 tool 消息里的图片块，content 降级为纯文本。
/// 没有文本时补占位符 —— 空字符串 content 会被部分上游拒绝。
fn take_images_from_tool_message(message: &mut Value, images: &mut Vec<Value>) {
    let Some(parts) = message.get("content").and_then(Value::as_array) else {
        return;
    };

    let mut texts = Vec::new();
    let mut found = Vec::new();
    for part in parts {
        if is_image_part(part) {
            if let Some(image) = image_part_to_chat(part) {
                found.push(image);
            }
            continue;
        }
        if let Some(text) = part.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            texts.push(text.to_string());
        }
    }
    if found.is_empty() {
        return;
    }

    let placeholder = if found.len() == 1 {
        "[image]".to_string()
    } else {
        format!("[{} images]", found.len())
    };
    message["content"] = json!(if texts.is_empty() {
        placeholder
    } else {
        format!("{}\n{placeholder}", texts.join("\n"))
    });
    images.append(&mut found);
}

fn append_text_to_assistant_message(message: &mut Value, text: &str) {
    if text.is_empty() {
        return;
    }
    let existing = match message.get("content") {
        Some(Value::String(content)) => content.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    message["content"] = if existing.trim().is_empty() {
        json!(text)
    } else {
        json!(format!("{existing}\n{text}"))
    };
}

/// DeepSeek thinking 模式要求带 `tool_calls` 的 assistant 消息回传 `reasoning_content`，
/// 否则报 400（The `reasoning_content` in the thinking mode must be passed back to the API）。
/// 历史里没有 reasoning 项时（例如上游没回传 summary，或被裁剪掉了）补一个占位说明，
/// 只补 content 和 reasoning_content 同时为空的情况，不覆盖真实 reasoning。
fn ensure_tool_call_reasoning_content(messages: &mut [Value]) {
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        if !has_tool_calls {
            continue;
        }
        let has_content = message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.trim().is_empty());
        let has_reasoning = message
            .get("reasoning_content")
            .and_then(Value::as_str)
            .is_some_and(|reasoning| !reasoning.trim().is_empty());
        if has_content || has_reasoning {
            continue;
        }
        message["reasoning_content"] = json!("Calling the requested tool.");
    }
}

fn normalize_chat_messages(messages: &mut [Value]) {
    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let has_content = match message.get("content") {
            Some(Value::Null) | None => false,
            Some(Value::String(_)) => true,
            Some(Value::Array(parts)) => !parts.is_empty(),
            Some(_) => true,
        };
        let has_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .is_some_and(|tool_calls| !tool_calls.is_empty());
        if !has_content && !has_tool_calls {
            message["content"] = json!("");
        }
    }
}

/// 把所有 system 消息折叠成一条、前置到数组头部。
///
/// 之前只处理 content 是字符串的 system 消息：数组形态（content parts）与 null
/// 的会原样留在**原位**，于是出现「system 不在最前」——MiniMax 这类上游会直接
/// 拒绝（issue #1394 / #410）；空内容的 system 也照样发出去。
/// 现在无论 content 是什么形态都统一收拢，空的直接丢弃。
/// 取出 system 消息的纯文本：字符串直接返回，content parts 数组把 text 段拼起来，
/// 其余形态（null / 对象）按空处理。
fn system_message_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

/// 「prompt 前缀稳定层」（mediafix4）易变注入块名单。
///
/// 取证（MEDIAFIX3_TURN_BOUNDARY.md 第五节）确认：下列块随会话状态整段出现/消失，
/// 到达顺序轮轮漂移——混进折叠后的 system 头部块后，上游 token 前缀会在块区分叉，
/// 导致每轮全量重算。排序时把它们钉在块尾（按名字典序），保证新出现的注入块
/// 永远追加在 system 区最尾，历史前缀不被平移。
/// 名单保守：只收取证确认轮变的块；其余具名块（如 AGENTS、skills_instructions）
/// 不参与重排，一旦出现即保持到达位置。
/// `image_resize_notice` 已被剥离步骤删尽，保留在名单中仅作为非成对残留路径的兜底。
const VOLATILE_INJECTION_BLOCK_NAMES: &[&str] = &[
    "apps_instructions",
    "collaboration_mode",
    "image_resize_notice",
    "multi_agent_mode",
    "recommended_plugins",
];

const IMAGE_RESIZE_NOTICE_OPEN: &str = "<image_resize_notice>";
const IMAGE_RESIZE_NOTICE_CLOSE: &str = "</image_resize_notice>";

/// 从所有角色（system/user/tool，developer 此前已映射为 system）的消息文本中
/// 剥离 `<image_resize_notice>…</image_resize_notice>` 块及其残片。
///
/// 消息被块整体消耗时整条删除（不留空消息）；非成对残留（孤儿标签连同其
/// 残片）一并移除并记 debug 日志——残片属易变内容，按普通文本留在前缀里
/// 会让后续轮次的前缀字节级分叉。
/// 变换幂等：第二次运行零命中，不会改写已清洁的消息。
fn strip_image_resize_notice_blocks(messages: &mut Vec<Value>) {
    let mut unpaired = false;
    messages.retain_mut(|message| {
        // 带 tool_calls 的结构消息不允许因剥离被删（会破坏配对协议）。
        let has_tool_calls = message.get("tool_calls").is_some();
        let Some(content) = message.get_mut("content") else {
            return true;
        };
        match content {
            Value::String(text) => {
                let (cleaned, removed, has_unpaired) = strip_notice_blocks_in_text(text);
                unpaired = unpaired || has_unpaired;
                if !removed {
                    return true;
                }
                *text = cleaned;
                // retain 返回 false 才整条删除：无结构负载的纯 notice 消息删除；
                // 带 tool_calls 的骨架消息保留，内容已清空。
                !text.is_empty() || has_tool_calls
            }
            Value::Array(parts) => {
                let mut removed_any = false;
                for part in parts.iter_mut() {
                    // 只处理带 text 字段的 part；image_url 等形态不受影响。
                    let Some(Value::String(text)) = part.get_mut("text") else {
                        continue;
                    };
                    let (cleaned, removed, has_unpaired) = strip_notice_blocks_in_text(text);
                    unpaired = unpaired || has_unpaired;
                    if !removed {
                        continue;
                    }
                    removed_any = true;
                    *text = cleaned;
                }
                if !removed_any {
                    return true;
                }
                parts.retain(|part| match part.get("text") {
                    Some(Value::String(text)) => !text.is_empty(),
                    _ => true,
                });
                if parts.is_empty() && !has_tool_calls {
                    return false;
                }
                true
            }
            _ => true,
        }
    });
    if unpaired {
        // 非成对残片已随本步骤归零；只记 debug 日志，不影响转发。
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "protocol_proxy.prefix_stabilizer_unpaired_notice_stripped",
            json!({
                "note": "存在非成对的 image_resize_notice 标记，孤儿标签连同残片已按契约移除",
                "volatileBlocks": VOLATILE_INJECTION_BLOCK_NAMES,
            }),
        );
    }
}

/// 从单段文本剥离 notice 相关内容，返回（清洁后文本, 是否删除过内容, 是否存在非成对标记）。
///
/// 三态契约（前缀字节级稳定优先于残片原文保全，仅在 opt-in 开关开启时生效）：
/// - 完整块（开始标签后随结束标签）：整块删除；
/// - 孤儿结束标签（先于任何开始标签出现）：文本段开头到该标签（含）视为残片，整体删除；
/// - 孤儿开始标签（其后无配对结束标签）：该标签（含）到文本段结尾视为残片，整体删除。
/// 上游客户端把 notice 以整块或「开始标签+上半残体」「下半残体+结束标签」的跨字段
/// 拆分形态注入历史早段，且轮轮重写；残片只要进入前缀就必然漂移。删除范围只由
/// 标签位置决定、与内容无关，因此变换确定且幂等。前提：轮变内容只出现在 notice
/// 标签内部；若客户端日后把用户原文并入残片，删除会一并吞掉该部分，属契约内代价。
/// 未删除任何内容时返回的清洁文本无意义，调用方凭 removed=false 保持原文逐字节不变。
fn strip_notice_blocks_in_text(text: &str) -> (String, bool, bool) {
    let mut segments: Vec<&str> = Vec::new();
    let mut rest = text;
    let mut removed = false;
    let mut unpaired = false;
    loop {
        let open_at = rest.find(IMAGE_RESIZE_NOTICE_OPEN);
        let close_at = rest.find(IMAGE_RESIZE_NOTICE_CLOSE);
        let first_is_open = match (open_at, close_at) {
            (None, None) => {
                segments.push(rest);
                break;
            }
            (Some(open), Some(close)) if close < open => false,
            (Some(_), Some(_)) => true,
            (Some(_), None) => true,
            (None, Some(_)) => false,
        };
        if first_is_open {
            let open = open_at.unwrap();
            match close_at {
                Some(close) if close > open => {
                    // 完整块：保留块前文本，跳过块体
                    removed = true;
                    segments.push(&rest[..open]);
                    rest = &rest[close + IMAGE_RESIZE_NOTICE_CLOSE.len()..];
                }
                _ => {
                    // 孤儿开始标签：该标签（含）到文本段结尾整体视为残片删除
                    unpaired = true;
                    removed = true;
                    segments.push(&rest[..open]);
                    break;
                }
            }
        } else {
            // 孤儿结束标签（含「结束先于开始」形态）：文本段开头到该标签（含）删除
            let close = close_at.unwrap();
            unpaired = true;
            removed = true;
            rest = &rest[close + IMAGE_RESIZE_NOTICE_CLOSE.len()..];
        }
    }
    if !removed {
        return (String::new(), false, unpaired);
    }
    // 删块后的空白残迹整理：非空段之间用空行拼接，不留空段。
    let cleaned = segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (cleaned, true, unpaired)
}

/// 把 system 文本切成顶层块序列：（块名, 块体）；块名为 None 表示普通文本段。
// ———— 图片附件轮「前缀塌陷」根治层（opt-in，随 stabilize_prompt_prefix 生效）————
//
// 取证（PREFIX_DRIFT_FORENSICS.md）定案的两个漂移源与对应变换：
// 1. `normalize_attachment_boilerplate`：客户端在用户图片附件轮**重渲染历史带图
//    user 消息**，附件样板一次性增长（新增 `Image attachment: true`、
//    `Distinguish instructions...` 等行）并固化。新旧两种形态都归一为同一常量
//    占位前缀，客户端的重写被归一层消化，token 前缀不再分叉。
// 2. open_page 系块内容归一：`codex_apps_open_page_instructions` /
//    `external_codex_apps_open_page` 块随开页状态重写或重复注入（+273B 形态），
//    在块区统一替换为常量空体块并去重；裸文本形态先按签名包成具名块。
// 3. 易变重注入块同名去重：客户端累积重注入 `codex_apps_client_time_context`
//    等具名锚块（时区块单实例 271B，每实例步进 +273B），同名只保留到达序
//    最后一个实例（dedup-last），内容零改写，保留块区字典序的自然位置，
//    不进「内容钉死为常量」的钉尾区。
// 以上变换都确定、幂等；不含签名/名单块的消息与文本逐字节不变。

/// 内容轮变的具名块名单：块体随会话状态（打开的 Page 等）改写或重复注入，
/// 归一为常量空体块并同名去重。块名保留在块区（字典序位置不变），只有体被钉死。
const VOLATILE_CONTENT_BLOCK_NAMES: &[&str] =
    &["codex_apps_open_page_instructions", "external_codex_apps_open_page"];

/// 易变同名重注入块名单：客户端累积式重注入（时区锚块约每 2 小时一次），
/// 块体语义有效、零改写；同名只保留到达序**最后一个实例**（时间语义取最新），
/// 块位置仍由块名字典序决定。实例数 >1 时经诊断日志记一条，供线上核查注入频率。
const DEDUP_LAST_BLOCK_NAMES: &[&str] = &["codex_apps_client_time_context"];

/// 已知可能以「裸文本段」形态出现的易变注入签名：（行首签名, 包裹用的块名）。
/// 包裹后进入块区，再被名单归一；只收取证确认过的开头句，保守防误吞。
const BARE_VOLATILE_INJECTION_SIGNATURES: &[(&str, &str)] = &[
    (
        "The codex_apps_open_page_instructions context records",
        "codex_apps_open_page_instructions",
    ),
    (
        "The codex_apps_open_page context records",
        "codex_apps_open_page_instructions",
    ),
];

/// 把裸文本形态的易变注入段（签名行 + 随后连续非空行 = 一段）包成具名块。
/// 幂等：包裹后的块体在后续归一中被替换为空体，不再含签名；带标签的原文行
/// 以 `<` 开头，不会命中行首签名，不会被二次包裹。无签名时借用原文零拷贝。
fn wrap_bare_volatile_injections(text: &str) -> std::borrow::Cow<'_, str> {
    if !BARE_VOLATILE_INJECTION_SIGNATURES
        .iter()
        .any(|(signature, _)| text.contains(signature))
    {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out_lines: Vec<String> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let Some((_, block_name)) = BARE_VOLATILE_INJECTION_SIGNATURES
            .iter()
            .find(|(signature, _)| line.trim_start().starts_with(signature))
        else {
            out_lines.push(line.to_string());
            continue;
        };
        let mut paragraph = vec![line.to_string()];
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            paragraph.push(lines.next().unwrap_or_default().to_string());
        }
        out_lines.push(format!("<{block_name}>\n{}\n</{block_name}>", paragraph.join("\n")));
    }
    std::borrow::Cow::Owned(out_lines.join("\n"))
}

/// 名单块的归一形态：同名块恒为该常量（split 的块体带标签，这里连标签一起重建）。
fn normalize_volatile_block(name: Option<&str>, block: &str) -> String {
    match name {
        // 名单块：块体（含标签）整体替换为常量；同名块排序后内容全等，便于去重。
        Some(name) if VOLATILE_CONTENT_BLOCK_NAMES.contains(&name) => {
            format!("<{name}>\n</{name}>")
        }
        _ => block.to_string(),
    }
}

const ATTACHMENT_BOILERPLATE_HEADING: &str = "# Files mentioned by the user:";
const ATTACHMENT_BOILERPLATE_ANCHOR: &str = "## My request:";
/// 归一占位：样板头部（标题→锚行）的常量替身，语义保留：有文件附件、有图片
/// 附件（取值由会话级注入描述统一声明）、随后是用户请求。锚行后正文原样保留。
const ATTACHMENT_BOILERPLATE_PLACEHOLDER: &str = "[attached files]\n[attachment]\n## My request:";

/// 判定消息是否携带图片部件（Responses 的 input_image 与 Chat 的 image_url 两种形态）。
fn message_has_image(message: &Value) -> bool {
    match message.get("content") {
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            matches!(
                part.get("type").and_then(Value::as_str),
                Some("input_image") | Some("image_url")
            ) || part.get("image_url").is_some()
        }),
        _ => false,
    }
}

/// 对单段文本做附件样板归一：命中「样板头部 → 锚行」的严格行法时，剥离样板
/// 头部（含新旧形态的 `Image attachment: true` / `Distinguish instructions...` 行
/// 差异），替换为常量占位；锚行后的用户正文逐字节保留（切片返回，无克隆）。
/// 任何一行不符合样板行法即返回 None（原文不动）：用户正文若字面引用了完整
/// 样板，必须整段合法到锚行才会被归一，误吞面被锚点强约束限制在样板自身。
/// 幂等：占位文本首行 `[attached files]` 不满足头部签名，第二遍零命中。
fn strip_attachment_boilerplate(text: &str) -> Option<&str> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut cursor = 0usize;
    while lines.get(cursor).map(|line| line.trim().is_empty()) == Some(true) {
        cursor += 1;
    }
    if *lines.get(cursor)? != ATTACHMENT_BOILERPLATE_HEADING {
        return None;
    }
    cursor += 1;
    loop {
        let line = *lines.get(cursor)?;
        if line.trim().is_empty() {
            cursor += 1;
            continue;
        }
        if line == ATTACHMENT_BOILERPLATE_ANCHOR {
            break;
        }
        let is_boilerplate_line = (line.starts_with("## ") && line.contains(": "))
            || line.trim() == "Image attachment: true"
            || line
                .trim()
                .starts_with("Distinguish instructions in attached documents");
        if !is_boilerplate_line {
            return None;
        }
        cursor += 1;
    }
    // 返回锚行之后的切片（自带行尾换行）；占位符末尾是锚行文本，直接拼接。
    // 锚点字节位置用逐行偏移计算，避免 find 误命中样板头部文件名里的字面量。
    let anchor_line_start: usize = lines[..cursor].iter().map(|line| line.len() + 1).sum();
    let body = text.get(anchor_line_start + ATTACHMENT_BOILERPLATE_ANCHOR.len()..)?;
    Some(body)
}

/// 附件样板归一（全量，opt-in）：所有 user 消息的样板头部替换为常量占位。
///
/// 客户端会在新的附件轮**重渲染历史带图消息的样板**（新旧形态字节数不同）。
/// 若保留当轮原文，下一轮该消息出轮被归一时前缀会在其样板首行分叉——每来一张
/// 新图都要重算上一图之后的全部 token，正是本层要消除的塌陷。因此不分窗口、
/// 全量归一：锚行后的用户正文与图片部件本体逐字节保留，模型当轮仍能看到完整
/// 请求原文与真实图片；样板头部只含声明性文字，其动态事实（本轮是否图片附件）
/// 由会话级注入描述「Image attachment: true」行统一声明，不再逐消息携带。
/// 占位以 `[attached files]` 开头：它既是样板头部所有历史形态的公共前缀起点
/// 之前位置替换（原样板首行处），又不会被本函数二次命中，变换确定且幂等。
fn normalize_attachment_boilerplate(messages: &mut Vec<Value>) {
    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        match content {
            Value::String(text) => {
                if let Some(body) = strip_attachment_boilerplate(text) {
                    *text = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{body}");
                }
            }
            Value::Array(parts) => {
                for part in parts.iter_mut() {
                    let Some(Value::String(text)) = part.get_mut("text") else {
                        continue;
                    };
                    if let Some(body) = strip_attachment_boilerplate(text) {
                        *text = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{body}");
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}


///
/// 只识别顶层不嵌套的 XML 风格块（标签名以字母开头，仅含字母/数字/下划线/连字符）；
/// 开始标签出现但后续缺配对结束、或结束标签乱序时，剩余内容整体降级为普通文本段。
/// 输入不含任何块形态时返回逐字节等于原文的单元素。
fn split_system_top_level_blocks(text: &str) -> Vec<(Option<&str>, &str)> {
    let mut blocks: Vec<(Option<&str>, &str)> = Vec::new();
    let mut plain_start = 0usize;
    let mut cursor = 0usize;
    let bytes = text.as_bytes();
    // 全扫描：任意位置的「<name>…完整闭合…</name>」都视为顶层块（只认首个
    // 闭合、不处理嵌套）；其余字节一律留在普通文本段，逐字节不变。
    while cursor < bytes.len() {
        if bytes[cursor] != b'<' {
            cursor += 1;
            continue;
        }
        let Some((name, body_start)) = open_tag_at(text, cursor) else {
            cursor += 1;
            continue;
        };
        // find_close_tag 返回结束标签 `>` 之后的偏移；找不到配对该处按普通文本继续扫
        let Some(block_end) = find_close_tag(text, body_start, name) else {
            cursor += 1;
            continue;
        };
        if plain_start < cursor {
            blocks.push((None, &text[plain_start..cursor]));
        }
        blocks.push((Some(name), &text[cursor..block_end]));
        cursor = block_end;
        plain_start = block_end;
    }
    if blocks.is_empty() {
        return vec![(None, text)];
    }
    if plain_start < text.len() {
        blocks.push((None, &text[plain_start..]));
    }
    blocks
}

fn is_block_name_char(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_' || value == b'-'
}

/// 识别 at 处的 `<name>`（或 `<name >`）开始标签，返回（块名, 块体起始偏移）。
/// 块名须以字母开头、仅含字母/数字/下划线/连字符；带属性、自闭合形态一律视为非块，
/// 保持原文逐字节不变。
fn open_tag_at(text: &str, at: usize) -> Option<(&str, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'<') {
        return None;
    }
    let start = at + 1;
    if !bytes.get(start)?.is_ascii_alphabetic() {
        return None;
    }
    let mut index = start;
    while index < bytes.len() && is_block_name_char(bytes[index]) {
        index += 1;
    }
    let name_end = index;
    while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
        index += 1;
    }
    if bytes.get(index) != Some(&b'>') {
        return None;
    }
    Some((&text[start..name_end], index + 1))
}

/// 从 from 起搜索 `</name>`（名前名后允许空白）。
/// 返回结束标签 `>` 之后的字节偏移；扫到结尾仍无配对则返回 None，调用方据此降级。
fn find_close_tag(text: &str, from: usize, name: &str) -> Option<usize> {
    let mut search = from;
    loop {
        let found = text.get(search..)?.find("</")? + search;
        let bytes = text.as_bytes();
        let mut index = found + 2;
        while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len() && is_block_name_char(bytes[index]) {
            index += 1;
        }
        if &text[name_start..index] != name {
            // 名字不匹配的结束标签或非标签文本：跳过已扫部分继续找
            search = index.max(found + 2);
            continue;
        }
        while matches!(bytes.get(index), Some(b' ') | Some(b'\t')) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'>') {
            return Some(index + 1);
        }
        search = index.max(found + 2);
    }
}

/// 块的排序权重：普通文本段取 (0, 空名) 保持到达相对序在前；具名块取 (1, 块名)，
/// 易变名单与未知名同档，统一按块名字典序钉在块尾。
/// 名单的语义是「取证确认轮变、靠本层规范序兜底的块」：具名 XML 块无论是否在
/// 名单内都会被规范重排；名单是取证文档与后续扩展点（新增轮变块只需加名单）。
fn block_sort_rank(name: Option<&str>) -> (u8, &str) {
    match name {
        None => (0, ""),
        Some(name) => (1, name),
    }
}

/// 稳定重排 system 头部块（「prompt 前缀稳定层」的规范序部分）。
///
/// 把每个到达 chunk 切成顶层块后按全局规则重排：普通文本段保持到达相对序在前；
/// 具名块进块区按名字典序（名单内与未知名同档）。排序只依赖块名与到达相对序，
/// 不依赖到达位置：同一块集合任意排列输入，输出逐字节一致（确定性）；
/// 对输出再切块再排序，块序列不变（幂等）。
///
/// 易变重注入同名块（`DEDUP_LAST_BLOCK_NAMES`）在规范序后做「同名去重、保留
/// 最后一个实例」：内容全等时仍满足置换不变；内容不同（日期翻转）按到达序
/// 取最新，属每日至多一次的可控变化。块体逐字节保留客户端原文。
///
/// 已知限制（写入产物报告）：文本段按全局到达序在前，若已出现的历史文本段在
/// 后续轮次整体缺失，其后的文本段会前移导致局部分叉；已定案的轮变源中 notice
/// 走剥离、其余均为具名块，不受此影响。
fn order_system_chunks_stably(chunks: &[String]) -> String {
    // rank 里的块名拷进拥有类型，避免借用生命周期复杂化；排序成本与正确性无关。
    let mut parts: Vec<((u8, String), String)> = Vec::new();
    for chunk in chunks {
        // 裸文本形态的易变注入先包成具名块，再进统一切块与归一。
        let wrapped = wrap_bare_volatile_injections(chunk);
        for (name, block) in split_system_top_level_blocks(&wrapped) {
            // 名单块体归一为常量：内容轮变/重复注入都不再进入 token 前缀。
            let block = normalize_volatile_block(name, block);
            let (kind, name_part) = block_sort_rank(name);
            parts.push(((kind, name_part.to_string()), block));
        }
    }
    // sort_by_key 是稳定排序：rank 相同的块（文本段/重复同名块）保持到达相对序
    parts.sort_by_key(|part| part.0.clone());
    // dedup-last 预扫描：统计易变重注入块的实例数；同名块规范序下必然相邻，
    // 只放行最后一个实例；实例数 >1 记一条诊断事件供线上核查注入频率。
    let mut dedup_last: Vec<(&str, usize, usize)> = Vec::new();
    for ((kind, name_part), _) in parts.iter() {
        if *kind == 1 && DEDUP_LAST_BLOCK_NAMES.contains(&name_part.as_str()) {
            match dedup_last
                .iter_mut()
                .find(|(name, ..)| *name == name_part.as_str())
            {
                Some(entry) => entry.1 += 1,
                None => dedup_last.push((name_part.as_str(), 1, 0)),
            }
        }
    }
    for (name, instances, _) in dedup_last.iter() {
        if *instances > 1 {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "protocol_proxy.prefix_stabilizer_dedup_last_folded",
                json!({ "blockName": name, "instances": instances }),
            );
        }
    }
    // 常量块同名去重：只保留第一个，消除重复注入造成的块区平移。
    let mut constant_seen: Vec<&str> = Vec::new();
    // 切分结果是否含具名块：决定文本段空白是「块间分隔」（可归一）还是
    // 「块体本身」（透传），见下方 push 循语义。
    let has_named_blocks = parts.iter().any(|((kind, _), _)| *kind == 1);
    let mut out: Vec<&str> = Vec::with_capacity(parts.len());
    for ((kind, name_part), text) in parts.iter() {
        if *kind == 1
            && VOLATILE_CONTENT_BLOCK_NAMES.contains(&name_part.as_str())
        {
            if constant_seen.contains(&name_part.as_str()) {
                continue;
            }
            constant_seen.push(name_part.as_str());
        }
        if *kind == 1 && DEDUP_LAST_BLOCK_NAMES.contains(&name_part.as_str()) {
            // 只放行到达序最后一个实例，之前的同名实例折叠；块体本身零改写。
            if let Some(entry) = dedup_last
                .iter_mut()
                .find(|(name, ..)| *name == name_part.as_str())
            {
                entry.2 += 1;
                if entry.2 < entry.1 {
                    continue;
                }
            }
        }
        // 块体原文透传（维护者 P2：裁剪块体首尾空白违背「非易变内容逐字节
        // 不变」的 opt-in 契约）；纯空白块仍跳过，本块只消分隔噪声。
        // 语义分桶（维护者 P2）：切分结果含具名块时，文本段首尾空白属
        // 「块间分隔」，归一不伤内容且保幂等；整段只有纯文本时，空白就是
        // 块体本身，逐字节透传。具名块的尖括号标签同理属结构，两端参与
        // 分隔归一、标签内内容零改写。
        let text = if has_named_blocks { text.trim() } else { text };
        if text.trim().is_empty() {
            continue;
        }
        out.push(text);
    }
    out.join("\n\n")
}


/// 「prompt 前缀稳定层」（opt-in）：客户端把 `<turn_aborted>` 状态块注入
/// system 区尾部（2026-10-11 探针基线实锤：sys 尾部 @50259，重启重放时
/// 块的出现的/消失让 sys 字节 +223~347B，前缀在历史最前端分叉，重启首轮
/// 全历史冷算）。块的语义是「紧邻下一轮的一次性提示」，正确归宿是瞬态区：
/// 这里把它从头部 system 合并文本中摘除，改挂到最后一条 user 消息之前。
/// 转换产物不进客户端历史，下一轮客户端再注入时再摘，sys 头部字节恒稳，
/// 前缀恢复纯追加。无块时零改写，与旧行为逐字节一致。
fn relocate_turn_aborted_blocks(messages: &mut Vec<Value>) {
    const OPEN: &str = "<turn_aborted>";
    const CLOSE: &str = "</turn_aborted>";
    let Some(head) = messages.first_mut() else { return };
    if head.get("role").and_then(Value::as_str) != Some("system") {
        return;
    }
    let Some(Value::String(content)) = head.get("content") else { return };
    if !content.contains(OPEN) {
        return;
    }
    let content = content.clone();
    if !content.contains(OPEN) {
        return;
    }
    let mut stripped = String::with_capacity(content.len());
    let mut blocks: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while let Some(open_rel) = content[cursor..].find(OPEN) {
        let open = cursor + open_rel;
        stripped.push_str(&content[cursor..open]);
        match content[open..].find(CLOSE) {
            Some(close_rel) => {
                let end = open + close_rel + CLOSE.len();
                blocks.push(content[open..end].to_string());
                cursor = end;
            }
            None => {
                // 残缺块（只见开标签）：整段保守放弃，不改 system。
                stripped.push_str(&content[open..]);
                cursor = content.len();
            }
        }
    }
    if cursor < content.len() {
        stripped.push_str(&content[cursor..]);
    }
    if blocks.is_empty() {
        return;
    }
    // 残缺回退判定：stripped 只剩空白说明块外正文为空，宁留原状。
    if stripped.trim().is_empty() {
        return;
    }
    if let Some(Value::String(target)) = head.get_mut("content") {
        *target = stripped.trim_end().to_string();
    } else {
        return;
    }
    let block_messages: Vec<Value> = blocks
        .into_iter()
        .map(|block| json!({ "role": "system", "content": block }))
        .collect();
    // 插到最后一条 user 消息之前；没有 user 消息则挂在 system 之后。
    let insert_at = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .unwrap_or(1)
        .max(1);
    messages.splice(insert_at..insert_at, block_messages);
}

const CURRENT_TURN_OPEN: &str = "<current_user_turn>";
const CURRENT_TURN_CLOSE: &str = "</current_user_turn>";

/// 轮次标记（opt-in，随 stabilize_prompt_prefix 生效）：上下文重建（切模型/
/// 压缩/换手）时，已闭环的旧问题可能被回放成「历史里最后一条指令」，模型无法
/// 从文本层分辨新旧，于是出现回答几天前旧问题的串线。这里把「本轮要回答的
/// 指令」显式钉在最后一条 user 消息上：固定文本、无计数，历史里早已解包的
/// 消息逐字节不变，新增 token 只落在本来就要重算的消息流末尾，不碰前缀缓存。
fn mark_current_user_turn(messages: &mut Vec<Value>) {
    // 幂等：历史回放时上一轮的标记会随原文回到请求里，先全量解包再重新钉，
    // 保证同一历史消息在任意请求里的字节一致（前缀缓存依赖这一点）。
    for message in messages.iter_mut() {
        if message.get("role").and_then(Value::as_str) == Some("user") {
            unwrap_current_turn_mark(message);
        }
    }
    // 只在请求确实以 user 输入收尾时才钉标记（维护者 P1）：等待工具结果后的
    // 继续请求末尾是 tool/assistant 角色，此时最后一条 user 是历史旧问题，
    // 钉上会把历史问题误导成「当前问题」。非 user 收尾：解包已完成，直接返回。
    match messages
        .last()
        .and_then(|message| message.get("role").and_then(Value::as_str))
    {
        Some("user") => wrap_current_turn_mark(messages.last_mut().unwrap()),
        _ => return,
    }
}

/// 从 user 消息文本里剥离轮次标记。只作用于首轮文本段：包装只会碰首轮文本段，
/// 解包同样只碰它；中段出现同款文本属构造输入，保守不动。
fn unwrap_current_turn_mark(message: &mut Value) {
    match message.get_mut("content") {
        Some(Value::String(text)) => *text = strip_current_turn_pair(text),
        Some(Value::Array(parts)) => {
            let first_index = parts
                .iter()
                .position(|part| part.get("type").and_then(Value::as_str) == Some("text"));
            let Some(first_index) = first_index else { return };
            let text_count = parts
                .iter()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                .count();
            let texts: Vec<String> = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str).map(str::to_string))
                .collect();
            // 多 text 段是历史拼接形态（旧轮原文+空行+末轮原文）：先合并再解包，
            // 保持与单段形态一致的字节语义。
            let merged = if text_count > 1 {
                Some(texts.join("\n\n"))
            } else {
                texts.first().cloned()
            };
            let Some(merged) = merged else { return };
            let stripped = strip_current_turn_pair(&merged);
            if let Some(Value::String(text)) = parts[first_index].get_mut("text") {
                *text = stripped;
            }
            if text_count > 1 {
                let mut seen = 0usize;
                for part in parts
                    .iter_mut()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
                {
                    seen += 1;
                    if seen > 1 {
                        if let Some(Value::String(text)) = part.get_mut("text") {
                            *text = String::new();
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn strip_current_turn_pair(text: &str) -> String {
    let Some(start) = text.find(CURRENT_TURN_OPEN) else {
        return text.to_string();
    };
    let Some(end_rel) = text[start..].find(CURRENT_TURN_CLOSE) else {
        return text.to_string();
    };
    let inner = &text[start + CURRENT_TURN_OPEN.len()..start + end_rel];
    let head = text[..start].trim_end_matches('\n');
    let tail = text[start + end_rel + CURRENT_TURN_CLOSE.len()..].trim_start_matches('\n');
    format!("{}{}{}", head, inner.trim_start_matches('\n'), tail)
}

/// 把轮次标记钉进 user 消息的首个文本段；没有文本段（纯图片轮）则在头部新建一段。
fn wrap_current_turn_mark(message: &mut Value) {
    let marked = |text: &str| {
        if text.trim().is_empty() {
            text.to_string()
        } else {
            format!(
                "{}\n{}\n{}\n",
                CURRENT_TURN_OPEN,
                text.trim_end(),
                CURRENT_TURN_CLOSE
            )
        }
    };
    match message.get_mut("content") {
        Some(Value::String(text)) => {
            let next = marked(text);
            *text = next;
        }
        Some(Value::Array(parts)) => {
            if let Some(first) = parts
                .iter_mut()
                .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))
            {
                if let Some(Value::String(text)) = first.get_mut("text") {
                    let next = marked(text);
                    *text = next;
                }
            } else {
                parts.insert(0, json!({ "type": "text", "text": marked("") }));
                // marked("") 为空时（无文本段的空轮）不新建，下面回收。
                if parts
                    .first()
                    .and_then(|part| part.get("text").and_then(Value::as_str))
                    .map(str::is_empty)
                    .unwrap_or(false)
                {
                    parts.remove(0);
                }
            }
        }
        _ => {}
    }
}

fn collapse_system_messages_to_head(messages: Vec<Value>, stabilize: bool) -> Vec<Value> {
    let mut system_chunks = Vec::new();
    let mut rest = Vec::with_capacity(messages.len());

    for message in messages {
        if message.get("role").and_then(Value::as_str) == Some("system") {
            let text = system_message_text(message.get("content"));
            if !text.trim().is_empty() {
                system_chunks.push(text);
            }
            continue;
        }
        rest.push(message);
    }

    let mut output = Vec::with_capacity(rest.len() + usize::from(!system_chunks.is_empty()));
    if !system_chunks.is_empty() {
        // 前缀稳定层开启时改走规范序拼接；关闭时与历史行为逐字节一致。
        let content = if stabilize {
            order_system_chunks_stably(&system_chunks)
        } else {
            system_chunks.join("\n\n")
        };
        output.push(json!({ "role": "system", "content": content }));
    }
    output.extend(rest);
    output
}

fn responses_role_to_chat_role(role: Option<&str>) -> &'static str {
    match role {
        Some("developer") | Some("system") => "system",
        Some("assistant") => "assistant",
        Some("tool") => "tool",
        Some("latest_reminder") => "user",
        Some("user") | None => "user",
        Some(_) => "user",
    }
}

fn flush_tool_calls(
    messages: &mut Vec<Value>,
    pending_tool_calls: &mut Vec<Value>,
    pending_reasoning: &mut Vec<String>,
) {
    if pending_tool_calls.is_empty() {
        return;
    }

    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("assistant") {
            merge_tool_calls_into_message(last, std::mem::take(pending_tool_calls));
            // 合并路径同样要消费 pending_reasoning（issue #2210）。
            // 触发时序：reasoning item → 不带 tool_calls 的 assistant 文本消息
            // （pending_tool_calls 为空，reasoning 被附加到该文本消息并 take）→
            // function_call。此时最后一条已是 assistant，走本分支提前 return，
            // 若这里不追加，随后的 reasoning 就随函数返回被静默丢弃；
            // 而 ensure_tool_call_reasoning_content 只补 content 与
            // reasoning_content 同时为空的占位，content 非空时补不上。
            if !pending_reasoning.is_empty() {
                append_reasoning_to_assistant_message(
                    last,
                    &std::mem::take(pending_reasoning).join("\n"),
                );
            }
            return;
        }
    }

    let mut message = json!({
        "role": "assistant",
        "content": "",
        "tool_calls": std::mem::take(pending_tool_calls)
    });
    if !pending_reasoning.is_empty() {
        message["reasoning_content"] = json!(std::mem::take(pending_reasoning).join("\n"));
    }
    messages.push(message);
}

fn flush_reasoning(messages: &mut Vec<Value>, pending_reasoning: &mut Vec<String>) {
    if pending_reasoning.is_empty() {
        return;
    }
    let reasoning = std::mem::take(pending_reasoning).join("\n");
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("assistant") {
            append_reasoning_to_assistant_message(last, &reasoning);
            return;
        }
    }
    messages.push(json!({
        "role": "assistant",
        "content": "",
        "reasoning_content": reasoning
    }));
}

fn append_reasoning_to_assistant_message(message: &mut Value, reasoning: &str) {
    if reasoning.is_empty() {
        return;
    }
    let existing = message
        .get("reasoning_content")
        .and_then(Value::as_str)
        .unwrap_or("");
    message["reasoning_content"] = if existing.is_empty() {
        json!(reasoning)
    } else {
        json!(format!("{existing}\n{reasoning}"))
    };
    if message.get("content").is_none() || message.get("content") == Some(&Value::Null) {
        message["content"] = json!("");
    }
}

fn merge_tool_calls_into_message(message: &mut Value, incoming: Vec<Value>) {
    let Some(object) = message.as_object_mut() else {
        return;
    };
    let existing = object
        .entry("tool_calls".to_string())
        .or_insert_with(|| json!([]));
    let Some(existing_array) = existing.as_array_mut() else {
        *existing = json!(incoming);
        return;
    };
    for tool_call in incoming {
        let id = tool_call.get("id").and_then(Value::as_str).unwrap_or("");
        if !id.is_empty()
            && existing_array
                .iter()
                .any(|item| item.get("id").and_then(Value::as_str) == Some(id))
        {
            continue;
        }
        existing_array.push(tool_call);
    }
    if message.get("content").is_none() || message.get("content") == Some(&Value::Null) {
        message["content"] = json!("");
    }
}

fn responses_reasoning_text(item: &Value) -> Option<String> {
    extract_reasoning_summary_text(item).or_else(|| extract_reasoning_field_text(item))
}

fn is_image_part(part: &Value) -> bool {
    matches!(
        part.get("type").and_then(Value::as_str),
        Some("input_image") | Some("image_url")
    )
}

/// 把 Responses 的 `input_image`（`image_url` 可能是裸字符串）与已经是 Chat 形态的
/// `image_url` 统一成 Chat Completions 的 `{"type":"image_url","image_url":{"url":…}}`。
///
/// 返回 `None` 表示这不是图片块、或 url 为空不值得转发。
fn image_part_to_chat(part: &Value) -> Option<Value> {
    if !is_image_part(part) {
        return None;
    }
    let raw = part.get("image_url")?;
    let image_url = if raw.is_object() {
        raw.clone()
    } else {
        json!({ "url": raw.as_str().unwrap_or_default() })
    };
    if image_url
        .get("url")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return None;
    }
    Some(json!({ "type": "image_url", "image_url": image_url }))
}

fn responses_content_to_chat_content(_role: &str, content: &Value) -> Value {
    if content.is_null() || content.is_string() {
        return content.clone();
    }

    let Some(parts) = content.as_array() else {
        return content.clone();
    };
    let mut chat_parts = Vec::new();
    let mut has_non_text_part = false;

    for part in parts {
        match part.get("type").and_then(Value::as_str).unwrap_or("") {
            "input_text" | "output_text" | "text" => {
                if let Some(value) = part.get("text").and_then(Value::as_str) {
                    if !value.is_empty() {
                        chat_parts.push(json!({ "type": "text", "text": value }));
                    }
                }
            }
            "refusal" => {
                if let Some(value) = part.get("refusal").and_then(Value::as_str) {
                    if !value.is_empty() {
                        chat_parts.push(json!({ "type": "text", "text": value }));
                    }
                }
            }
            "input_image" | "image_url" => {
                if let Some(image) = image_part_to_chat(part) {
                    chat_parts.push(image);
                    has_non_text_part = true;
                }
            }
            _ => {}
        }
    }

    if !has_non_text_part {
        return Value::String(
            chat_parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }

    Value::Array(chat_parts)
}

fn responses_history_function_name(item: &Value) -> String {
    let name = item.get("name").and_then(Value::as_str).unwrap_or("");
    let namespace = item.get("namespace").and_then(Value::as_str).unwrap_or("");
    if name.is_empty() {
        String::new()
    } else if namespace.is_empty() {
        name.to_string()
    } else {
        flatten_namespace_tool_name(namespace, name)
    }
}

fn build_codex_tool_context(tools: Option<&Value>) -> CodexToolContext {
    let mut context = CodexToolContext::default();
    let Some(tools) = tools.and_then(Value::as_array) else {
        return context;
    };

    for tool in tools {
        if let Some(name) = tool.as_str().filter(|name| !name.is_empty()) {
            context.custom_tools.insert(
                name.to_string(),
                CodexCustomToolSpec {
                    openai_name: name.to_string(),
                    kind: CodexCustomToolKind::Raw,
                },
            );
            context.has_custom_tools = true;
            continue;
        }
        let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
        match tool_type {
            "custom" => {
                let Some(name) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                else {
                    continue;
                };
                let kind = detect_codex_custom_tool_kind(tool, name);
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind,
                    },
                );
                context.has_custom_tools = true;
            }
            "function" => {
                if let Some(name) = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                {
                    context.function_tools.insert(
                        name.to_string(),
                        CodexFunctionToolSpec {
                            name: name.to_string(),
                            namespace: String::new(),
                        },
                    );
                }
            }
            "namespace" => add_namespace_tools_to_context(&mut context, tool),
            // Codex 的 tool_search（MCP 延迟加载检索）是客户端执行的代理工具：
            // 转发层把它当作 custom 代理工具登记，模型调用回来时还原成
            // tool_search_call item，检索本身仍由 Codex 客户端执行。
            "tool_search" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("tool_search");
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind: CodexCustomToolKind::ToolSearch,
                    },
                );
                context.has_custom_tools = true;
            }
            "web_search" | "local_shell" | "computer_use" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(tool_type);
                context.custom_tools.insert(
                    name.to_string(),
                    CodexCustomToolSpec {
                        openai_name: name.to_string(),
                        kind: CodexCustomToolKind::BuiltIn,
                    },
                );
                context.has_custom_tools = true;
            }
            _ => {}
        }
    }

    context
}

fn add_namespace_tools_to_context(context: &mut CodexToolContext, namespace_tool: &Value) {
    let namespace = namespace_tool
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(children) = namespace_tool.get("tools").and_then(Value::as_array) else {
        return;
    };
    for child in children {
        if child.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(name) = child
            .get("name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let flat = flatten_namespace_tool_name(namespace, name);
        if namespace.is_empty() {
            context.function_tools.insert(
                flat,
                CodexFunctionToolSpec {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                },
            );
        } else if context
            .function_tools
            .get(&flat)
            .is_none_or(|spec| !spec.namespace.is_empty())
        {
            context.function_tools.insert(
                flat,
                CodexFunctionToolSpec {
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                },
            );
            context.has_namespace_tools = true;
        }
    }
}

fn responses_tools_to_chat_tools(tools: &[Value], context: &CodexToolContext) -> Vec<Value> {
    let mut converted = Vec::new();
    for tool in tools {
        if let Some(name) = tool.as_str().filter(|name| !name.is_empty()) {
            converted.push(generic_custom_proxy_tool(name, ""));
            continue;
        }
        match tool.get("type").and_then(Value::as_str).unwrap_or("") {
            "function" => {
                if let Some(tool) = responses_function_tool_to_chat_tool(tool) {
                    converted.push(tool);
                }
            }
            "custom" | "web_search" | "local_shell" | "computer_use" => {
                let tool_type = tool.get("type").and_then(Value::as_str).unwrap_or("");
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or(tool_type);
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if detect_codex_custom_tool_kind(tool, name) == CodexCustomToolKind::ApplyPatch {
                    converted.push(custom_proxy_tool_with_original_definition(
                        tool,
                        name,
                        description,
                    ));
                } else {
                    converted.push(generic_custom_proxy_tool(name, description));
                }
            }
            "namespace" => converted.extend(namespace_tool_to_chat_tools(tool, context)),
            // tool_search 透传为 chat 的 function 工具，名字保持 tool_search，
            // 检索由 Codex 客户端执行（execution: client），转发层只做搬运。
            "tool_search" => {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .unwrap_or("tool_search");
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let parameters = tool.get("parameters").cloned().unwrap_or_else(|| json!({}));
                converted.push(json!({
                    "type": "function",
                    "function": {
                        "name": name,
                        "description": description,
                        "parameters": parameters
                    }
                }));
            }
            _ => {}
        }
    }
    converted
}

fn detect_codex_custom_tool_kind(tool: &Value, name: &str) -> CodexCustomToolKind {
    if name == "apply_patch" {
        return CodexCustomToolKind::ApplyPatch;
    }
    if let Some(definition) = tool.pointer("/format/definition").and_then(Value::as_str) {
        if definition.contains("begin_patch")
            && definition.contains("end_patch")
            && definition.contains("add_hunk")
        {
            return CodexCustomToolKind::ApplyPatch;
        }
    }
    if matches!(
        tool.get("type").and_then(Value::as_str),
        Some("web_search" | "local_shell" | "computer_use")
    ) {
        CodexCustomToolKind::BuiltIn
    } else {
        CodexCustomToolKind::Raw
    }
}

fn responses_function_tool_to_chat_tool(tool: &Value) -> Option<Value> {
    if tool.get("type").and_then(Value::as_str) != Some("function") {
        return None;
    }
    if tool.get("function").is_some() {
        let mut chat_tool = tool.clone();
        if let Some(strict) = tool.get("strict").cloned() {
            if let Some(function) = chat_tool.get_mut("function").and_then(Value::as_object_mut) {
                function.entry("strict".to_string()).or_insert(strict);
            }
            if let Some(object) = chat_tool.as_object_mut() {
                object.remove("strict");
            }
        }
        if let Some(function) = chat_tool.get_mut("function").and_then(Value::as_object_mut) {
            let normalized =
                normalize_chat_tool_parameters(function.get("parameters").unwrap_or(&json!({})));
            function.insert("parameters".to_string(), normalized);
        }
        return Some(chat_tool);
    }
    let mut function = json!({
        "name": tool.get("name").and_then(Value::as_str).unwrap_or(""),
        "description": tool.get("description").cloned().unwrap_or(Value::Null),
        "parameters": normalize_chat_tool_parameters(tool.get("parameters").unwrap_or(&json!({})))
    });
    if let Some(strict) = tool.get("strict") {
        function["strict"] = strict.clone();
    }
    Some(json!({
        "type": "function",
        "function": function
    }))
}

fn namespace_tool_to_chat_tools(namespace_tool: &Value, context: &CodexToolContext) -> Vec<Value> {
    let namespace = namespace_tool
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let namespace_description = namespace_tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let Some(children) = namespace_tool.get("tools").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut converted = Vec::new();
    for child in children {
        if child.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        let Some(name) = child
            .get("name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        let flat = flatten_namespace_tool_name(namespace, name);
        if namespace != ""
            && context
                .function_tools
                .get(&flat)
                .is_some_and(|spec| spec.namespace.is_empty())
        {
            continue;
        }
        let description = combine_namespace_description(
            namespace_description,
            child
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(""),
        );
        let mut function = json!({
            "name": flat,
            "parameters": normalize_chat_tool_parameters(child.get("parameters").unwrap_or(&json!({})))
        });
        if !description.is_empty() {
            function["description"] = json!(description);
        }
        converted.push(json!({
            "type": "function",
            "function": function
        }));
    }
    converted
}

fn normalize_chat_tool_parameters(parameters: &Value) -> Value {
    let mut normalized = if parameters.is_object() {
        parameters.clone()
    } else {
        json!({})
    };
    // 裸 `$ref` 已经是完整 schema，补默认字段会人为制造 sibling。
    let is_bare_ref = normalized
        .as_object()
        .is_some_and(|object| object.len() == 1 && object.contains_key("$ref"));
    if !is_bare_ref {
        // `type: null` 与缺失等价：严格供应商（如 deepseek）会以
        // `got 'type: null'` 拒绝整个请求（issue #2247）。
        if normalized.get("type").is_none_or(Value::is_null) {
            normalized["type"] = json!("object");
        }
        if normalized.get("properties").is_none() {
            normalized["properties"] = json!({});
        }
        if normalized.get("required").is_none() {
            normalized["required"] = json!([]);
        }
    }
    let normalized = inline_ref_siblings(&normalized);
    // 必须先内联再摊平：合并分支时 $defs 已摊开才拿得到真实属性（issue #2367）。
    flatten_top_level_combinators(normalized)
}

/// JSON Schema 的顶层组合器（`oneOf` / `anyOf` / `allOf`）。
const SCHEMA_COMBINATOR_KEYS: [&str; 3] = ["oneOf", "anyOf", "allOf"];

/// 摊平工具 schema **顶层**的组合器（issue #2367）。
///
/// zod-to-json-schema 会生成形如
/// `{type:"object", properties:{}, oneOf:[{$ref:"#/$defs/__schema0"},…], $defs:{…}}`
/// 的 schema。部分上游（Anthropic 系）拒绝顶层 `oneOf`，直接整轮 400、模型完全不可用。
/// 全仓原本对 `oneOf` 零处理。
///
/// 策略（保守优先，绝不让整轮失败）：
/// 1. 顶层有组合器时，逐分支归一化后再摊平；
/// 2. `properties` 取各分支并集，同名属性都是 object 时递归合并其 properties；
///    `required` 取交集（只有所有分支都要求才算必须）；
/// 3. 剥掉组合器键，补回 `type:"object"`；
/// 4. 无法摊平（分支不是对象、合并后 properties 为空）时把各分支塞进一个带
///    description 的私有字段，保住信息且 schema 仍合法；
/// 5. 结果仍不是合法对象 schema 时原样返回——宁可交给上游判断，也不静默丢掉工具。
fn flatten_top_level_combinators(schema: Value) -> Value {
    let Some(object) = schema.as_object() else {
        return schema;
    };
    let Some((key, branches)) = SCHEMA_COMBINATOR_KEYS.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_array)
            .map(|a| (*key, a))
    }) else {
        return schema;
    };
    if branches.is_empty() {
        return schema;
    }

    // 分支常是裸 `$ref`（`{ "$ref": "#/$defs/__schema0" }`），它自己不带 `$defs`，
    // 所以必须拿**父级**的 $defs 来解析——只对分支单独调 inline_ref_siblings
    // 永远解析不出来（实测：分支落进降级路径，properties 为空）。
    let defs = object.get("$defs").and_then(Value::as_object);

    let mut properties = Map::new();
    let mut required: Option<BTreeSet<String>> = None;
    let mut flattenable = true;

    for branch in branches {
        let normalized = match resolve_local_definition(
            branch
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(local_definition_name)
                .unwrap_or(""),
            defs,
            &mut Vec::new(),
        ) {
            // 分支是裸 $ref：用父级 $defs 解析出真实 schema。
            Ok(Some(resolved)) if branch.as_object().is_some_and(|o| o.len() == 1) => {
                normalize_schema_value(&resolved, defs, &mut Vec::new()).unwrap_or(resolved)
            }
            _ => inline_ref_siblings(branch),
        };
        let Some(branch_object) = normalized.as_object() else {
            flattenable = false;
            break;
        };
        let branch_properties = branch_object.get("properties").and_then(Value::as_object);
        let branch_required: BTreeSet<String> = branch_object
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // 分支没有 properties 时（例如 type:"string"）无法并入对象 schema。
        if branch_properties.is_none() && !branch_required.is_empty() {
            flattenable = false;
            break;
        }
        for (name, value) in branch_properties.into_iter().flatten() {
            merge_schema_property(&mut properties, name, value);
        }
        required = Some(match required {
            None => branch_required,
            Some(current) => current.intersection(&branch_required).cloned().collect(),
        });
    }

    if !flattenable || properties.is_empty() {
        // 降级：把分支原样塞进一个带说明的字段，schema 依然合法。
        let mut fallback = object.clone();
        fallback.insert("type".to_string(), json!("object"));
        fallback.insert("properties".to_string(), json!({}));
        let mut description: Vec<String> = Vec::new();
        for branch in branches {
            if let Some(text) = branch.get("description").and_then(Value::as_str) {
                if !text.trim().is_empty() {
                    description.push(text.trim().to_string());
                }
            }
        }
        fallback.insert(
            "x-merged-combinator".to_string(),
            json!({
                "kind": key,
                "branches": branches,
                "description": description.join("\n")
            }),
        );
        for combinator in SCHEMA_COMBINATOR_KEYS {
            fallback.remove(combinator);
        }
        return Value::Object(fallback);
    }

    let mut flattened = object.clone();
    for combinator in SCHEMA_COMBINATOR_KEYS {
        flattened.remove(combinator);
    }
    flattened.insert("type".to_string(), json!("object"));
    flattened.insert("properties".to_string(), Value::Object(properties));
    flattened.insert(
        "required".to_string(),
        json!(required.unwrap_or_default().into_iter().collect::<Vec<_>>()),
    );
    Value::Object(flattened)
}

/// 把分支属性并入目标 properties；同名且两侧都是 object schema 时递归合并
/// （properties 并集、required 取交集），否则保留先到的一方（非破坏性）。
fn merge_schema_property(properties: &mut Map<String, Value>, name: &str, value: &Value) {
    let Some(existing) = properties.get_mut(name) else {
        properties.insert(name.to_string(), value.clone());
        return;
    };
    let (Some(left), Some(right)) = (existing.as_object(), value.as_object()) else {
        return;
    };
    if left.get("properties").is_none() || right.get("properties").is_none() {
        return;
    }
    let mut merged = left.clone();
    let mut merged_properties = left
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (child_name, child_value) in right
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        merge_schema_property(&mut merged_properties, child_name, child_value);
    }
    merged.insert("properties".to_string(), Value::Object(merged_properties));

    let left_required: BTreeSet<String> = left
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let right_required: BTreeSet<String> = right
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    merged.insert(
        "required".to_string(),
        json!(
            left_required
                .intersection(&right_required)
                .cloned()
                .collect::<Vec<_>>()
        ),
    );
    *existing = Value::Object(merged);
}

fn inline_ref_siblings(root: &Value) -> Value {
    let defs = root.get("$defs").and_then(Value::as_object);
    let mut resolving = Vec::new();
    normalize_schema_value(root, defs, &mut resolving).unwrap_or_else(|_| root.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalRefNormalizationError {
    Cycle,
}

fn normalize_schema_value(
    node: &Value,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Value, LocalRefNormalizationError> {
    match node {
        Value::Array(items) => Ok(Value::Array(
            items
                .iter()
                .map(|item| normalize_schema_value(item, defs, resolving))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Value::Object(object) => normalize_schema_object(object, defs, resolving),
        _ => Ok(node.clone()),
    }
}

fn normalize_schema_object(
    object: &Map<String, Value>,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Value, LocalRefNormalizationError> {
    if object.len() > 1
        && let Some(reference) = object.get("$ref").and_then(Value::as_str)
        && let Some(name) = local_definition_name(reference)
    {
        match resolve_local_definition(name, defs, resolving)? {
            Some(Value::Object(mut merged)) if merged.get("$ref").is_none() => {
                for (key, value) in object {
                    if key != "$ref" {
                        merged.insert(key.clone(), normalize_schema_value(value, defs, resolving)?);
                    }
                }
                return Ok(Value::Object(merged));
            }
            Some(_) | None => {}
        }
    }

    let mut normalized = Map::new();
    for (key, value) in object {
        // JSON Schema 的 type 必须是字符串，null 恒非法，剥掉等价于未声明。
        if key == "type" && value.is_null() {
            continue;
        }
        normalized.insert(key.clone(), normalize_schema_value(value, defs, resolving)?);
    }
    Ok(Value::Object(normalized))
}

fn resolve_local_definition(
    name: &str,
    defs: Option<&Map<String, Value>>,
    resolving: &mut Vec<String>,
) -> Result<Option<Value>, LocalRefNormalizationError> {
    let Some(defs) = defs else {
        return Ok(None);
    };
    let Some(target) = defs.get(name) else {
        return Ok(None);
    };
    if resolving.iter().any(|current| current == name) {
        return Err(LocalRefNormalizationError::Cycle);
    }

    resolving.push(name.to_string());
    let resolved = if let Some(alias) = bare_local_ref_name(target) {
        resolve_local_definition(alias, Some(defs), resolving)
    } else {
        normalize_schema_value(target, Some(defs), resolving).map(Some)
    };
    resolving.pop();
    resolved
}

fn bare_local_ref_name(node: &Value) -> Option<&str> {
    let object = node.as_object()?;
    if object.len() != 1 {
        return None;
    }
    local_definition_name(object.get("$ref")?.as_str()?)
}

fn local_definition_name(reference: &str) -> Option<&str> {
    let name = reference.strip_prefix("#/$defs/")?;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    Some(name)
}

fn generic_custom_proxy_tool(name: &str, description: &str) -> Value {
    let description = if description.trim().is_empty() {
        format!("FREEFORM custom tool: {name}. Put only the tool input text here.")
    } else {
        format!(
            "{}\n\nThis is a FREEFORM tool. Do not wrap the input in JSON or markdown.",
            description.trim()
        )
    };
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Raw freeform input for this custom tool."
                    }
                },
                "required": ["input"]
            }
        }
    })
}

fn custom_proxy_tool_with_original_definition(
    tool: &Value,
    name: &str,
    description: &str,
) -> Value {
    let mut description = if description.trim().is_empty() {
        format!("FREEFORM custom tool: {name}. Put only the tool input text here.")
    } else {
        format!(
            "{}\n\nThis is a FREEFORM tool. Do not wrap the input in JSON or markdown.",
            description.trim()
        )
    };
    description.push_str("\n\nOriginal tool definition:\n```json\n");
    description.push_str(&canonical_json_string(tool));
    description.push_str("\n```");

    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "input": {
                        "type": "string",
                        "description": "Raw string input for the original custom tool. Preserve formatting exactly and follow the original tool definition embedded in the description."
                    }
                },
                "required": ["input"]
            }
        }
    })
}

/// 从历史里的 tool_search_output item 收集 Codex 客户端检索命中的命名空间工具。
///
/// 条目形如 `{ type: "namespace", name, description, tools: [...] }`，必须走
/// `add_namespace_tools_to_context` + `namespace_tool_to_chat_tools` 展开成
/// `mcp__<server>__<tool>`。只取 namespace 名字的话上游只能看到外壳，
/// 调不到内层工具，反向转换也还原不出 namespace 字段。
fn collect_tool_search_output_namespaces(body: &Value) -> Vec<Value> {
    let mut collected: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let Some(items) = body.get("input").and_then(Value::as_array) else {
        return collected;
    };

    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("tool_search_output") {
            continue;
        }
        let Some(tools) = item.get("tools").and_then(Value::as_array) else {
            continue;
        };
        for tool in tools {
            let Some(name) = tool
                .get("name")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if seen.insert(name.to_string()) {
                collected.push(tool.clone());
            }
        }
    }

    collected
}

/// chat tools 按函数名去重，保留首次出现。
fn dedup_chat_tools_by_name(tools: &mut Vec<Value>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    tools.retain(|tool| {
        match tool
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            Some(name) => seen.insert(name.to_string()),
            None => true,
        }
    });
}

fn combine_namespace_description(namespace_description: &str, child_description: &str) -> String {
    let namespace_description = namespace_description.trim();
    let child_description = child_description.trim();
    match (
        namespace_description.is_empty(),
        child_description.is_empty(),
    ) {
        (true, true) => String::new(),
        (true, false) => child_description.to_string(),
        (false, true) => namespace_description.to_string(),
        (false, false) => format!("{namespace_description}\n\n{child_description}"),
    }
}

fn flatten_namespace_tool_name(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        return name.to_string();
    }
    if name.is_empty() {
        return namespace.to_string();
    }
    if namespace.ends_with("__") || name.starts_with("__") {
        format!("{namespace}{name}")
    } else {
        format!("{namespace}__{name}")
    }
}

fn responses_tool_choice_to_chat(tool_choice: &Value, context: &CodexToolContext) -> Option<Value> {
    match tool_choice {
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("function") => {
            if let Some(namespace) = object.get("namespace").and_then(Value::as_str) {
                let name = object.get("name").and_then(Value::as_str).unwrap_or("");
                return Some(json!({
                    "type": "function",
                    "function": {
                        "name": flatten_namespace_tool_name(namespace, name)
                    }
                }));
            }
            if let Some(function) = object.get("function").and_then(Value::as_object) {
                if let Some(namespace) = function.get("namespace").and_then(Value::as_str) {
                    let name = function.get("name").and_then(Value::as_str).unwrap_or("");
                    return Some(json!({
                        "type": "function",
                        "function": {
                            "name": flatten_namespace_tool_name(namespace, name)
                        }
                    }));
                }
            }
            Some(json!({
                "type": "function",
                "function": {
                    "name": object.get("name").and_then(Value::as_str).unwrap_or("")
                }
            }))
        }
        Value::Object(object) if object.get("type").and_then(Value::as_str) == Some("custom") => {
            let name = object.get("name").and_then(Value::as_str)?;
            let spec = context.custom_tools.get(name)?;
            let upstream_name = spec.openai_name.clone();
            Some(json!({
                "type": "function",
                "function": { "name": upstream_name }
            }))
        }
        other => Some(other.clone()),
    }
}

fn chat_reasoning_to_response_output_item(message: &Value, response_id: &str) -> Option<Value> {
    let reasoning = chat_reasoning_text(message)?;
    if reasoning.is_empty() {
        return None;
    }
    Some(json!({
        "id": format!("rs_{response_id}"),
        "type": "reasoning",
        "reasoning_content": reasoning,
        "summary": [{ "type": "summary_text", "text": reasoning }]
    }))
}

fn chat_reasoning_text(message: &Value) -> Option<String> {
    if let Some(reasoning) = extract_reasoning_field_text(message) {
        return Some(reasoning);
    }

    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if let Some((reasoning, _answer)) = split_leading_think_block(content) {
            if !reasoning.is_empty() {
                return Some(reasoning);
            }
        }
    }

    None
}

fn chat_message_to_response_output_item(message: &Value, response_id: &str) -> Option<Value> {
    let mut content = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        let text = split_leading_think_block(text)
            .map(|(_reasoning, answer)| answer)
            .unwrap_or_else(|| text.to_string());
        if !text.is_empty() {
            content.push(json!({ "type": "output_text", "text": text, "annotations": [] }));
        }
    } else if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for part in parts {
            match part.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" | "output_text" => {
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            content.push(
                                json!({ "type": "output_text", "text": text, "annotations": [] }),
                            );
                        }
                    }
                }
                "refusal" => {
                    if let Some(refusal) = part.get("refusal").and_then(Value::as_str) {
                        if !refusal.is_empty() {
                            content.push(json!({ "type": "refusal", "refusal": refusal }));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(refusal) = message.get("refusal").and_then(Value::as_str) {
        if !refusal.is_empty() {
            content.push(json!({ "type": "refusal", "refusal": refusal }));
        }
    }

    if content.is_empty() {
        return None;
    }

    Some(json!({
        "id": format!("msg_{}", response_id_body(response_id)),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": content
    }))
}

fn chat_tool_calls_to_response_output_items(
    message: &Value,
    tool_context: &CodexToolContext,
) -> Vec<Value> {
    let mut output = Vec::new();
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (index, tool_call) in tool_calls.iter().enumerate() {
            output.push(chat_tool_call_to_response_item(
                tool_call,
                index,
                tool_context,
            ));
        }
    } else if let Some(function_call) = message.get("function_call") {
        output.push(chat_legacy_function_call_to_response_item(
            function_call,
            tool_context,
        ));
    }
    output
}

fn chat_tool_call_to_response_item(
    tool_call: &Value,
    index: usize,
    tool_context: &CodexToolContext,
) -> Value {
    let call_id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("call_{index}"));
    let function = tool_call.get("function").unwrap_or(&Value::Null);
    let name = function.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = responses_arguments_to_chat(function.get("arguments").unwrap_or(&json!({})));
    // 记住供应商在本轮工具调用上附带的透传数据（Gemini thought_signature 等），
    // 下一轮构造请求时挂回，否则整轮 400（issue #332 / #1012）。
    remember_tool_call_extra_content(&call_id, tool_call);
    response_tool_call_item(&call_id, name, &arguments, tool_context)
}

fn chat_legacy_function_call_to_response_item(
    function_call: &Value,
    tool_context: &CodexToolContext,
) -> Value {
    let call_id = function_call
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("call_0");
    let name = function_call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let arguments =
        responses_arguments_to_chat(function_call.get("arguments").unwrap_or(&json!({})));
    response_tool_call_item(call_id, name, &arguments, tool_context)
}

fn tool_call_added_item(
    state: &ToolCallState,
    output_index: u32,
    tool_context: &CodexToolContext,
) -> Value {
    if tool_context.is_builtin_web_search_proxy(&state.name) {
        // 客户端只认 web_search_call；用 custom_tool_call 会被当作未知工具丢弃，
        // 上游那轮的搜索结果就再也回不到会话里（issue #1209 / #586）。
        return json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": {
                "id": web_search_item_id(&state.call_id),
                "type": "web_search_call",
                "status": "in_progress",
                "call_id": state.call_id,
                "action": web_search_action_from_arguments(&state.arguments)
            }
        });
    }
    if tool_context.is_custom_tool_proxy(&state.name) {
        if tool_context.is_tool_search_proxy(&state.name) {
            return json!({
                "type": "response.output_item.added",
                "output_index": output_index,
                "item": {
                    "id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                    "type": "tool_search_call",
                    "status": "in_progress",
                    "call_id": state.call_id,
                    "execution": "client",
                    "arguments": {}
                }
            });
        }
        return json!({
            "type": "response.output_item.added",
            "output_index": output_index,
            "item": {
                "id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                "type": "custom_tool_call",
                "status": "in_progress",
                "call_id": state.call_id,
                "name": tool_context.original_custom_tool_name(&state.name),
                "input": ""
            }
        });
    }
    let (display_name, namespace) = tool_context.openai_name_for_function_tool(&state.name);
    let mut item = json!({
        "type": "response.output_item.added",
        "output_index": output_index,
        "item": {
            "id": state.item_id,
            "type": "function_call",
            "status": "in_progress",
            "call_id": state.call_id,
            "name": display_name,
            "arguments": ""
        }
    });
    if !namespace.is_empty() {
        item["item"]["namespace"] = json!(namespace);
    }
    item
}

fn push_tool_call_delta_sse(
    output: &mut String,
    state: &ToolCallState,
    output_index: u32,
    delta: &str,
    tool_context: &CodexToolContext,
) {
    if tool_context.is_tool_search_proxy(&state.name)
        || tool_context.is_builtin_web_search_proxy(&state.name)
    {
        // tool_search 与原生 web_search 都走 function 参数流，客户端按
        // tool_search_call.arguments / web_search_call 聚合。
        push_sse(
            output,
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": state.item_id,
                "output_index": output_index,
                "delta": delta
            }),
        );
    } else if tool_context.is_custom_tool_proxy(&state.name) {
        let _ = delta;
    } else {
        push_sse(
            output,
            "response.function_call_arguments.delta",
            json!({
                "type": "response.function_call_arguments.delta",
                "item_id": state.item_id,
                "output_index": output_index,
                "delta": delta
            }),
        );
    }
}

fn push_tool_call_done_sse(
    output: &mut String,
    state: &ToolCallState,
    output_index: u32,
    tool_context: &CodexToolContext,
) {
    if tool_context.is_tool_search_proxy(&state.name) {
        push_sse(
            output,
            "response.function_call_arguments.done",
            json!({
                "type": "response.function_call_arguments.done",
                "item_id": state.item_id,
                "output_index": output_index,
                "arguments": state.arguments
            }),
        );
        return;
    }
    if tool_context.is_custom_tool_proxy(&state.name) {
        push_sse(
            output,
            "response.custom_tool_call_input.delta",
            json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": tool_call_item_id(&state.call_id, &state.name, tool_context),
                "call_id": state.call_id,
                "output_index": output_index,
                "delta": reconstruct_custom_tool_call_input_with_context(
                    tool_context,
                    &state.name,
                    &state.arguments
                )
            }),
        );
        return;
    }
    push_sse(
        output,
        "response.function_call_arguments.done",
        json!({
            "type": "response.function_call_arguments.done",
            "item_id": state.item_id,
            "output_index": output_index,
            "arguments": state.arguments
        }),
    );
}

fn tool_call_done_item(state: &ToolCallState, tool_context: &CodexToolContext) -> Value {
    response_tool_call_item(&state.call_id, &state.name, &state.arguments, tool_context)
}

fn response_tool_call_item(
    call_id: &str,
    name: &str,
    arguments: &str,
    tool_context: &CodexToolContext,
) -> Value {
    if tool_context.is_builtin_web_search_proxy(name) {
        // 客户端只认 web_search_call，custom_tool_call 会被当作未知工具丢弃。
        return json!({
            "id": web_search_item_id(call_id),
            "type": "web_search_call",
            "status": "failed",
            "call_id": call_id,
            "action": web_search_action_from_arguments(arguments)
        });
    }
    if tool_context.is_custom_tool_proxy(name) {
        if tool_context.is_tool_search_proxy(name) {
            // 官方客户端的 tool_search handler 只接受 tool_search_call item，
            // function_call 形态会被拒绝，因此必须还原成专属 item 类型；
            // arguments 转发层原样搬运（对象字符串双向保真）。
            return json!({
                "id": format!("tsc_{call_id}"),
                "type": "tool_search_call",
                "status": "completed",
                "call_id": call_id,
                "execution": "client",
                "arguments": responses_arguments_to_chat_parse(arguments)
            });
        }
        return json!({
            "id": tool_call_item_id(call_id, name, tool_context),
            "type": "custom_tool_call",
            "status": "completed",
            "call_id": call_id,
            "name": tool_context.original_custom_tool_name(name),
            "input": reconstruct_custom_tool_call_input_with_context(tool_context, name, arguments)
        });
    }
    let (display_name, namespace) = tool_context.openai_name_for_function_tool(name);
    let mut item = json!({
        "id": format!("fc_{call_id}"),
        "type": "function_call",
        "status": "completed",
        "call_id": call_id,
        "name": display_name,
        "arguments": arguments
    });
    if !namespace.is_empty() {
        item["namespace"] = json!(namespace);
    }
    item
}

/// 官方客户端给 `web_search_call` 分配的 item id 前缀（见 codex id_prefix）。
fn web_search_item_id(call_id: &str) -> String {
    format!("ws_{call_id}")
}

/// 把上游 web_search 工具调用的 arguments 映射成客户端 action 结构。
///
/// 客户端 rollout 里的形态是 `{"type":"search","query":…,"queries":[…]}` 或
/// `{"type":"open_page","url":…}`。上游各家字段名不一（input / query /
/// queries / url），
/// 这里按存在性择一，都取不到就退化成 `{"type":"search"}`，让客户端自己判空。
fn web_search_action_from_arguments(arguments: &str) -> Value {
    let parsed = responses_arguments_to_chat_parse(arguments);
    let query = parsed
        .get("query")
        .and_then(Value::as_str)
        .or_else(|| parsed.get("input").and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty());
    let url = parsed
        .get("url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    if let Some(url) = url {
        return json!({ "type": "open_page", "url": url });
    }
    let queries = parsed
        .get("queries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|value| json!(value))
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty());
    match (query, queries) {
        (Some(query), Some(queries)) => {
            json!({ "type": "search", "query": query, "queries": queries })
        }
        (Some(query), None) => json!({ "type": "search", "query": query }),
        (None, Some(queries)) => {
            let first = queries
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            json!({ "type": "search", "query": first, "queries": queries })
        }
        (None, None) => json!({ "type": "search" }),
    }
}

fn web_search_action_has_target(action: &Value) -> bool {
    action
        .get("query")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || action
            .get("queries")
            .and_then(Value::as_array)
            .is_some_and(|queries| {
                queries
                    .iter()
                    .any(|query| query.as_str().is_some_and(|value| !value.trim().is_empty()))
            })
        || action
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
}

fn invalid_web_search_output_item(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("web_search_call")
}

fn tool_call_item_id(call_id: &str, name: &str, tool_context: &CodexToolContext) -> String {
    if tool_context.is_builtin_web_search_proxy(name) {
        return web_search_item_id(call_id);
    }
    let prefix = if tool_context.is_custom_tool_proxy(name) {
        if tool_context.is_tool_search_proxy(name) {
            // 官方客户端给 tool_search_call 分配的 item id 前缀（见 codex id_prefix）。
            return format!("tsc_{call_id}");
        }
        "ctc_"
    } else {
        "fc_"
    };
    format!("{prefix}{call_id}")
}

fn split_leading_think_block(text: &str) -> Option<(String, String)> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    if !after_ws.starts_with(THINK_OPEN_TAG) {
        return None;
    }
    let body_start = leading_ws_len + THINK_OPEN_TAG.len();
    let close_relative = text[body_start..].find(THINK_CLOSE_TAG)?;
    let close_start = body_start + close_relative;
    let answer_start = close_start + THINK_CLOSE_TAG.len();
    Some((
        text[body_start..close_start].trim().to_string(),
        strip_think_answer_separator(&text[answer_start..]).to_string(),
    ))
}

fn strip_leading_think_open_tag(text: &str) -> Option<String> {
    let leading_ws_len = text.len() - text.trim_start().len();
    let after_ws = &text[leading_ws_len..];
    after_ws
        .strip_prefix(THINK_OPEN_TAG)
        .map(|value| value.trim().to_string())
}

fn strip_think_answer_separator(text: &str) -> &str {
    text.trim_start_matches(['\r', '\n', '\t', ' '])
}

fn extract_reasoning_field_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "reasoning"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(reasoning) = value.get("reasoning") {
        for key in ["content", "text", "summary"] {
            if let Some(text) = reasoning.get(key).and_then(Value::as_str) {
                if !text.is_empty() {
                    return Some(text.to_string());
                }
            }
        }
    }

    value
        .get("reasoning_details")
        .and_then(extract_reasoning_details_text)
}

fn extract_reasoning_details_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.to_string()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(extract_reasoning_detail_part_text)
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!text.is_empty()).then_some(text)
        }
        Value::Object(_) => extract_reasoning_detail_part_text(value),
        _ => None,
    }
}

fn extract_reasoning_detail_part_text(value: &Value) -> Option<String> {
    for key in ["text", "content", "summary"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    if let Some(parts) = value.get("parts").and_then(Value::as_array) {
        let text = parts
            .iter()
            .filter_map(extract_reasoning_detail_part_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        return (!text.is_empty()).then_some(text);
    }

    None
}

fn extract_reasoning_summary_text(value: &Value) -> Option<String> {
    for key in ["reasoning_content", "content", "text"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }

    let summary = value.get("summary")?;
    if let Some(text) = summary.as_str() {
        return (!text.is_empty()).then(|| text.to_string());
    }

    let parts = summary.as_array()?;
    let text = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
                .or_else(|| part.as_str())
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    (!text.is_empty()).then_some(text)
}

fn default_responses_usage() -> Value {
    // Codex 把 output_tokens_details.reasoning_tokens 当必填解析,
    // 兜底 usage 也必须带齐该结构。
    json!({
        "input_tokens": 0,
        "output_tokens": 0,
        "total_tokens": 0,
        "output_tokens_details": { "reasoning_tokens": 0 }
    })
}

fn chat_usage_to_responses_usage(usage: Option<&Value>) -> Value {
    let Some(usage) = usage.filter(|value| value.is_object() && !value.is_null()) else {
        return default_responses_usage();
    };
    let mut input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .or_else(|| usage.get("promptTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut input_tokens_include_cache = usage.get("prompt_tokens").is_some();
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .or_else(|| usage.get("candidatesTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let mut cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
        .or_else(|| usage.get("cachedContentTokenCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_5m = usage
        .get("cache_creation_5m_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_1h = usage
        .get("cache_creation_1h_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let has_claude_cache_fields = usage.get("cache_read_input_tokens").is_some()
        || usage.get("cache_creation_input_tokens").is_some()
        || usage.get("cache_creation_5m_input_tokens").is_some()
        || usage.get("cache_creation_1h_input_tokens").is_some();
    let has_cache_details = cached_tokens > 0
        || usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .is_some()
        || usage
            .pointer("/input_tokens_details/cached_tokens")
            .is_some();

    if let Some(value) = usage.get("input_tokens").and_then(Value::as_u64) {
        input_tokens = value;
        input_tokens_include_cache = false;
    }
    if let Some(cache_read) = usage.get("cache_read_input_tokens").and_then(Value::as_u64) {
        cached_tokens = cache_read;
    }
    if let Some(prompt_tokens) = usage.get("promptTokenCount").and_then(Value::as_u64) {
        cached_tokens = usage
            .get("cachedContentTokenCount")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        input_tokens = prompt_tokens.saturating_sub(cached_tokens);
        input_tokens_include_cache = false;
    }

    let usage_input_tokens = if input_tokens_include_cache {
        input_tokens.saturating_sub(
            cached_tokens
                + effective_cache_creation_tokens(
                    cache_creation,
                    cache_creation_5m,
                    cache_creation_1h,
                ),
        )
    } else {
        input_tokens
    };
    let should_recalculate_total = usage.get("total_tokens").is_none()
        || cached_tokens > 0
        || effective_cache_creation_tokens(cache_creation, cache_creation_5m, cache_creation_1h)
            > 0
        || usage.get("promptTokenCount").is_some();
    let total_tokens = if should_recalculate_total {
        usage_input_tokens
            + output_tokens
            + cached_tokens
            + effective_cache_creation_tokens(cache_creation, cache_creation_5m, cache_creation_1h)
    } else {
        usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(usage_input_tokens + output_tokens)
    };
    let mut result = json!({
        "input_tokens": usage_input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens
    });

    if !has_claude_cache_fields && has_cache_details && cached_tokens > 0 {
        result["input_tokens_details"] = json!({ "cached_tokens": cached_tokens });
    }
    if let Some(details) = usage.get("completion_tokens_details") {
        // Codex parses output_tokens_details.reasoning_tokens as a required field;
        // upstreams (e.g. Kimi) omit the key when a response had no reasoning,
        // which makes the Responses client fail with "missing field
        // `reasoning_tokens`" and abort the whole turn. Default it to 0.
        let mut details = details.clone();
        if details.is_object() && details.get("reasoning_tokens").is_none() {
            details["reasoning_tokens"] = json!(0);
        }
        result["output_tokens_details"] = details;
    } else {
        // 上游连 completion_tokens_details 都没给时同样补全, 避免
        // Codex 解析 response.completed 时缺字段断流。
        result["output_tokens_details"] = json!({ "reasoning_tokens": 0 });
    }
    if let Some(cache_read) = usage.get("cache_read_input_tokens") {
        result["cache_read_input_tokens"] = cache_read.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_input_tokens") {
        result["cache_creation_input_tokens"] = cache_creation.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_5m_input_tokens") {
        result["cache_creation_5m_input_tokens"] = cache_creation.clone();
    }
    if let Some(cache_creation) = usage.get("cache_creation_1h_input_tokens") {
        result["cache_creation_1h_input_tokens"] = cache_creation.clone();
    }
    let cache_ttl = match (cache_creation_5m > 0, cache_creation_1h > 0) {
        (true, true) => Some("mixed"),
        (true, false) => Some("5m"),
        (false, true) => Some("1h"),
        (false, false) => None,
    };
    if let Some(cache_ttl) = cache_ttl {
        result["cache_ttl"] = json!(cache_ttl);
    }
    result
}

fn effective_cache_creation_tokens(
    cache_creation: u64,
    cache_creation_5m: u64,
    cache_creation_1h: u64,
) -> u64 {
    if cache_creation > 0 {
        cache_creation
    } else {
        cache_creation_5m + cache_creation_1h
    }
}

fn response_status(finish_reason: Option<&str>) -> &'static str {
    match finish_reason {
        Some("length") => "incomplete",
        _ => "completed",
    }
}

fn response_output_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => canonical_json_string(other),
    }
}

const IMAGE_DATA_URL_PREFIX: &str = "data:image/";

/// 这些上游只接受**纯 base64**，不认 `data:image/png;base64,` 前缀，收到就 400
/// （issue #2031：GLM-5.3-Flash 的本地图片被拒）。只对这类供应商剥离前缀，
/// 标准 data URL 上游（OpenAI 等）保持原样，免得把能用的改坏。
fn upstream_needs_bare_base64(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("glm-") || model.starts_with("zhipu") || model.contains("/glm-")
}

/// 就地改写 messages 里 `image_url` 的 url：对需要纯 base64 的模型剥掉 data URL 前缀。
fn normalize_image_data_urls_for_model(messages: &mut [Value], model: &str) {
    if !upstream_needs_bare_base64(model) {
        return;
    }
    for message in messages.iter_mut() {
        let Some(parts) = message.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for part in parts.iter_mut() {
            if !is_image_part(part) {
                continue;
            }
            let Some(url) = part
                .get_mut("image_url")
                .and_then(|value| {
                    if value.is_object() {
                        value.get_mut("url")
                    } else {
                        Some(value)
                    }
                })
                .and_then(|value| value.as_str().map(str::to_string))
            else {
                continue;
            };
            let Some(bare) = strip_data_url_prefix(&url) else {
                continue;
            };
            let target = part.get_mut("image_url").expect("checked above");
            if target.is_object() {
                target["url"] = json!(bare);
            } else {
                *target = json!(bare);
            }
        }
    }
}

/// `data:image/png;base64,AAAA` → `AAAA`；不是 data URL 则返回 `None`。
fn strip_data_url_prefix(url: &str) -> Option<String> {
    let rest = url.strip_prefix(IMAGE_DATA_URL_PREFIX)?;
    let (_, payload) = rest.split_once(";base64,")?;
    Some(payload.to_string())
}

/// 若 `text` 含 base64 图片 data URL，返回替换成占位符后的文本；否则 `None`。
fn redact_image_data_urls(text: &str) -> Option<String> {
    if !text.contains(IMAGE_DATA_URL_PREFIX) {
        return None;
    }
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(IMAGE_DATA_URL_PREFIX) {
        out.push_str(&rest[..start]);
        out.push_str("[image omitted]");
        let tail = &rest[start..];
        // data URL 由 base64 字母表加少量分隔符组成，遇到其它字符即结束。
        let end = tail
            .find(|c: char| {
                !(c.is_ascii_alphanumeric()
                    || matches!(c, '+' | '/' | '=' | ':' | ';' | ',' | '.' | '-' | '_'))
            })
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// base64 图片一旦落进文本字段就是灾难：上游会把它当普通文本 tokenize，而 base64
/// 高熵、几乎没有可复用的 BPE merge（约 1.36 字符/token），一张 2MB 的图能膨胀到
/// 约 200 万 token 并直接撑爆上下文窗口。图片的唯一合法归宿是 `image_url` 子树。
///
/// 这是最后一道兜底：出站前扫一遍 body，把漏进文本字段的 data URL 换成占位符。
/// 命中即说明某条协议转换路径有 bug，记诊断日志便于定位。
fn guard_inline_image_data_urls(value: &mut Value) -> bool {
    match value {
        Value::Object(map) => {
            let mut hit = false;
            for (key, child) in map.iter_mut() {
                // image_url 子树是图片的合法归宿，跳过。
                if key == "image_url" {
                    continue;
                }
                hit |= guard_inline_image_data_urls(child);
            }
            hit
        }
        Value::Array(items) => {
            let mut hit = false;
            for item in items {
                hit |= guard_inline_image_data_urls(item);
            }
            hit
        }
        Value::String(text) => match redact_image_data_urls(text) {
            Some(cleaned) => {
                *text = cleaned;
                true
            }
            None => false,
        },
        _ => false,
    }
}

/// tool 输出可能带图 —— `view_image` 的结果就是
/// `function_call_output.output[] = [{"type":"input_image","image_url":"data:image/png;base64,…"}]`。
///
/// 直接走 `response_output_text` 会把整个数组 JSON 序列化成字符串，于是 base64 被当作
/// 普通文本送进上游 tokenizer。base64 是 BPE 最不擅长的输入（高熵、无可复用 merge，
/// 约 1.36 字符/token），一张 2MB 的 PNG 因此膨胀到约 200 万 token 并撑爆上下文窗口；
/// 同一张图走 `image_url` 只需几百 token，因为供应商在 tokenize 之前就把 base64 解码回
/// 像素、按尺寸切 patch 计数。
///
/// 所以这里在**有图时**保留结构化的 `image_url` part，交给
/// `relocate_tool_output_images` 在满足 tool 配对约束的前提下搬到后续 user 消息。
/// 无图时原样返回 `response_output_text` 的结果，保持既有行为不变。
fn tool_output_content(output: &Value) -> Value {
    let Some(parts) = output.as_array() else {
        return json!(response_output_text(output));
    };
    if !parts.iter().any(is_image_part) {
        return json!(response_output_text(output));
    }

    let mut chat_parts = Vec::new();
    for part in parts {
        if is_image_part(part) {
            if let Some(image) = image_part_to_chat(part) {
                chat_parts.push(image);
            }
            continue;
        }
        let text = match part.get("type").and_then(Value::as_str) {
            Some("input_text") | Some("output_text") | Some("text") => part
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // 非文本非图片的块（结构化工具结果等）保留原有的 JSON 表示，
            // 免得静默丢信息。
            _ => canonical_json_string(part),
        };
        if !text.is_empty() {
            chat_parts.push(json!({ "type": "text", "text": text }));
        }
    }
    Value::Array(chat_parts)
}

fn build_custom_tool_call_history(name: &str, input: &Value) -> (String, String) {
    let input = response_output_text(input);
    let name = legacy_apply_patch_proxy_name(name).unwrap_or(name);
    (name.to_string(), json!({ "input": input }).to_string())
}

/// 旧版本曾把 apply_patch 拆成多个结构化代理动作。新请求只声明
/// `apply_patch`，历史里的旧动作名必须在回放前迁移，避免上游收到
/// 未声明工具名。
fn legacy_apply_patch_proxy_name(name: &str) -> Option<&'static str> {
    match name {
        "apply_patch_add_file"
        | "apply_patch_delete_file"
        | "apply_patch_update_file"
        | "apply_patch_replace_file"
        | "apply_patch_batch" => Some("apply_patch"),
        _ => None,
    }
}

fn reconstruct_custom_tool_call_input_with_context(
    _tool_context: &CodexToolContext,
    _upstream_name: &str,
    arguments: &str,
) -> String {
    reconstruct_custom_tool_call_input(arguments)
}

fn reconstruct_custom_tool_call_input(arguments: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(arguments) else {
        return arguments.to_string();
    };
    value
        .get("input")
        .map(response_output_text)
        .unwrap_or_else(|| arguments.to_string())
}

fn copy_response_request_fields(response: &mut Value, original_request: Option<&Value>) {
    let Some(original_request) = original_request else {
        return;
    };
    for key in [
        "instructions",
        "max_output_tokens",
        "parallel_tool_calls",
        "previous_response_id",
        "reasoning",
        "temperature",
        "tool_choice",
        "tools",
        "top_p",
        "metadata",
    ] {
        if let Some(value) = original_request.get(key) {
            response[key] = value.clone();
        }
    }
}

fn responses_arguments_to_chat(value: &Value) -> String {
    match value {
        Value::String(text) => normalize_chat_tool_arguments_string(text),
        Value::Object(_) => canonical_json_string(value),
        Value::Null => "{}".to_string(),
        other => canonical_json_string(&json!({ "input": other })),
    }
}

/// 把 chat 侧的 arguments 字符串解析回 JSON Value，供需要对象形态参数的
/// item（如 tool_search_call）使用；解析失败时退回空对象，避免构造非法 item。
fn responses_arguments_to_chat_parse(arguments: &str) -> Value {
    let trimmed = arguments.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| json!({}))
}

fn normalize_chat_tool_arguments_string(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(_)) => trimmed.to_string(),
        Ok(value) => canonical_json_string(&json!({ "input": value })),
        Err(_) => canonical_json_string(&json!({ "input": text })),
    }
}

fn instruction_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .or_else(|| part.as_str())
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => other.as_str().unwrap_or_default().to_string(),
    }
}

fn canonical_json_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_default(),
        Value::Array(values) => {
            let parts = values.iter().map(canonical_json_string).collect::<Vec<_>>();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            let parts = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_default();
                    format!("{key}:{}", canonical_json_string(value))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", parts.join(","))
        }
    }
}

fn apply_chat_reasoning_options(result: &mut Value, body: &Value, model: &str, standard: bool) {
    let Some(reasoning_enabled) = reasoning_requested(body) else {
        return;
    };
    let style = if standard {
        ChatReasoningStyle::Default
    } else {
        infer_chat_reasoning_style(model)
    };

    match style {
        ChatReasoningStyle::Thinking => {
            result["thinking"] = json!({
                "type": if reasoning_enabled {
                    kimi_thinking_enabled_type(model)
                } else {
                    "disabled"
                }
            });
        }
        ChatReasoningStyle::EnableThinking => {
            result["enable_thinking"] = json!(reasoning_enabled);
        }
        ChatReasoningStyle::ReasoningSplit => {
            result["reasoning_split"] = json!(reasoning_enabled);
        }
        _ => {}
    }

    if !reasoning_enabled {
        if style == ChatReasoningStyle::OpenRouter {
            result["reasoning"] = json!({ "effort": "none" });
        }
        return;
    }

    let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) else {
        return;
    };
    let Some(mapped) = map_chat_reasoning_effort(effort, style) else {
        return;
    };

    match style {
        ChatReasoningStyle::OpenRouter => {
            result["reasoning"] = json!({ "effort": mapped });
        }
        ChatReasoningStyle::DeepSeek
        | ChatReasoningStyle::LowHigh
        | ChatReasoningStyle::Default
            if supports_reasoning_effort(model)
                || (standard && model.to_ascii_lowercase().contains("gemini")) =>
        {
            result["reasoning_effort"] = json!(mapped);
        }
        // Kimi For Coding (K3 / K2.7 Code): 官方接受 reasoning_effort 三档
        // low/high/max (默认 high), 且服务端会把 medium→high、xhigh→max。
        // 仅限 for-coding 模型 ID, 避免给 glm/mimo/kimi-k2 等其它
        // Thinking 方言上游误发该字段。
        ChatReasoningStyle::Thinking if is_kimi_coding_model(model) => {
            result["reasoning_effort"] = json!(mapped);
        }
        _ => {}
    }
}

fn reasoning_requested(body: &Value) -> Option<bool> {
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(Value::as_str) {
        return Some(!matches!(
            effort.trim().to_ascii_lowercase().as_str(),
            "none" | "off" | "disabled"
        ));
    }

    body.get("reasoning").map(|value| !value.is_null())
}

fn infer_chat_reasoning_style(model: &str) -> ChatReasoningStyle {
    let model = model.to_ascii_lowercase();
    if model.contains("openrouter") || model.starts_with("openrouter/") {
        return ChatReasoningStyle::OpenRouter;
    }
    if model.contains("deepseek") {
        return ChatReasoningStyle::DeepSeek;
    }
    if model.contains("qwen") || model.contains("dashscope") || model.contains("bailian") {
        return ChatReasoningStyle::EnableThinking;
    }
    if model.contains("kimi")
        || model.contains("moonshot")
        || model.starts_with("k3")
        || model.contains("glm")
        || model.contains("zhipu")
        || model.contains("z.ai")
        || model.contains("mimo")
    {
        return ChatReasoningStyle::Thinking;
    }
    if model.contains("minimax") {
        return ChatReasoningStyle::ReasoningSplit;
    }
    if model.contains("siliconflow") {
        return ChatReasoningStyle::EnableThinking;
    }
    if model.contains("stepfun") || model.contains("step-3.5-flash-2603") {
        return ChatReasoningStyle::LowHigh;
    }
    ChatReasoningStyle::Default
}

fn map_chat_reasoning_effort(effort: &str, style: ChatReasoningStyle) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    if matches!(effort.as_str(), "none" | "off" | "disabled") {
        return None;
    }

    match style {
        ChatReasoningStyle::DeepSeek => match effort.as_str() {
            "max" | "xhigh" => Some("max"),
            _ => Some("high"),
        },
        ChatReasoningStyle::LowHigh => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            _ => Some("high"),
        },
        ChatReasoningStyle::OpenRouter => match effort.as_str() {
            "max" | "xhigh" => Some("xhigh"),
            "high" => Some("high"),
            "medium" => Some("medium"),
            "low" => Some("low"),
            "minimal" => Some("minimal"),
            _ => None,
        },
        // Kimi For Coding 官方映射: minimal/low→low, medium/high→high,
        // xhigh/max→max。注意不能直接透传 "minimal", 服务端不认会 400。
        ChatReasoningStyle::Thinking => match effort.as_str() {
            "minimal" | "low" => Some("low"),
            "medium" | "high" => Some("high"),
            "xhigh" | "max" => Some("max"),
            _ => None,
        },
        _ => match effort.as_str() {
            "minimal" => Some("minimal"),
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "xhigh" => Some("xhigh"),
            "max" => Some("max"),
            _ => None,
        },
    }
}

/// Kimi For Coding 专属模型 ID(k3 / k3-256k / kimi-for-coding[-highspeed])。
/// 只有这些上游接受 `reasoning_effort` 三档; kimi-k2-thinking 等旧模型
/// 仍只发 thinking 开关。
fn is_kimi_coding_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.starts_with("k3") || model.contains("kimi-k3") || model.contains("for-coding")
}

fn kimi_thinking_enabled_type(model: &str) -> &'static str {
    if is_kimi_coding_model(model) {
        "adaptive"
    } else {
        "enabled"
    }
}

fn supports_reasoning_effort(model: &str) -> bool {
    is_openai_o_series(model)
        || model
            .to_lowercase()
            .strip_prefix("gpt-")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|ch| ch.is_ascii_digit() && ch >= '5')
        || infer_chat_reasoning_style(model) == ChatReasoningStyle::DeepSeek
        || infer_chat_reasoning_style(model) == ChatReasoningStyle::LowHigh
        // #2464 已验证的网关模型；其它 Gemini 由供应商的标准协议开关显式启用。
        || model.eq_ignore_ascii_case("gemini-3.8-flash")
}

fn is_openai_o_series(model: &str) -> bool {
    model.len() > 1
        && model.starts_with('o')
        && model
            .as_bytes()
            .get(1)
            .is_some_and(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod response_compatibility_tests {
    use super::*;

    #[test]
    fn chat_search_filter_preserves_client_tools_and_rejects_forced_native_search() {
        let native = json!({"type":"web_search"});
        for client in [
            json!({"type":"function", "name":"web_search", "parameters":{"type":"object"}}),
            json!({"type":"custom", "name":"web_search"}),
            json!({"type":"tool_search", "execution":"client"}),
            json!({"type":"namespace", "name":"search", "tools":[]}),
        ] {
            let mut body =
                json!({"tools":[native.clone(),client.clone()],"tool_choice":"required"});
            remove_unsupported_native_search(&mut body).unwrap();
            assert_eq!(body["tools"], json!([client]));
        }
        for choice in [json!("required"), json!({"type":"web_search"})] {
            let mut body = json!({"tools":[native.clone()],"tool_choice":choice});
            assert!(
                remove_unsupported_native_search(&mut body)
                    .unwrap_err()
                    .is::<UnsupportedNativeWebSearch>()
            );
        }
        let mut automatic = json!({"tools":[native],"tool_choice":"auto"});
        remove_unsupported_native_search(&mut automatic).unwrap();
        assert_eq!(automatic["tools"], json!([]));
    }

    #[test]
    fn native_search_with_query_is_still_not_executed_by_chat_converter() {
        let request = json!({"tools":[{"type":"web_search"}]});
        let response = chat_completion_to_response_with_request(
            json!({
                "choices":[{"message":{"tool_calls":[{"id":"call_search", "function":{
                    "name":"web_search", "arguments":"{\"query\":\"Codex\"}"
                }}]},"finish_reason":"tool_calls"}]
            }),
            &request,
        )
        .unwrap();
        assert_eq!(response["status"], "failed");
        assert_eq!(response["output"][0]["status"], "failed");
    }

    #[test]
    fn gemini_reasoning_conversion_respects_standard_protocol_opt_in() {
        for standard in [false, true] {
            let body = responses_to_chat_completions_with_options(
                json!({
                    "model":"gemini-3.8-flash", "input":"hi", "reasoning":{"effort":"high"}
                }),
                standard,
                false,
            )
            .unwrap();
            assert_eq!(body["reasoning_effort"], "high");
        }
        let request =
            json!({"model":"google/gemini-other", "input":"hi", "reasoning":{"effort":"high"}});
        assert!(
            responses_to_chat_completions(request.clone())
                .unwrap()
                .get("reasoning_effort")
                .is_none()
        );
        assert_eq!(
            responses_to_chat_completions_with_options(request, true, false).unwrap()["reasoning_effort"],
            "high"
        );
    }

    #[test]
    fn gemini_models_receive_reasoning_effort() {
        assert!(supports_reasoning_effort("gemini-3.8-flash"));
        assert!(!supports_reasoning_effort("google/gemini-2.5-pro"));
        assert!(!supports_reasoning_effort("glm-5.3"));
    }

    #[test]
    fn empty_web_search_call_is_failed_in_non_stream_response() {
        let request = json!({
            "model": "gemini-3.8-flash",
            "tools": [{ "type": "web_search" }]
        });
        let upstream = json!({
            "id": "chatcmpl-search",
            "model": "gemini-3.8-flash",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_search",
                        "type": "function",
                        "function": { "name": "web_search", "arguments": "{}" }
                    }]
                }
            }]
        });

        let response = chat_completion_to_response_with_request(upstream, &request).unwrap();
        assert_eq!(
            response.get("status").and_then(Value::as_str),
            Some("failed")
        );
        assert_eq!(
            response.pointer("/error/type").and_then(Value::as_str),
            Some("invalid_upstream_tool_call")
        );
    }

    #[test]
    fn empty_web_search_call_is_failed_in_stream_response() {
        let request = json!({
            "model": "gemini-3.8-flash",
            "tools": [{ "type": "web_search" }]
        });
        let chunk = json!({
            "id": "chatcmpl-search",
            "model": "gemini-3.8-flash",
            "choices": [{
                "finish_reason": "tool_calls",
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_search",
                        "function": { "name": "web_search", "arguments": "{}" }
                    }]
                }
            }]
        });
        let mut state = ChatSseState::with_request(&request);
        let mut stream = String::new();
        state.handle_chat_chunk_into(&chunk, &mut stream);
        state.finalize_into(&mut stream);
        assert!(stream.contains("response.failed"));
        assert!(!stream.contains("response.completed"));
    }

    #[test]
    fn web_search_action_requires_query_or_url() {
        assert!(!web_search_action_has_target(&json!({ "type": "search" })));
        assert!(web_search_action_has_target(
            &json!({ "type": "search", "query": "Codex" })
        ));
        assert!(web_search_action_has_target(
            &web_search_action_from_arguments(r#"{"input":"Codex"}"#)
        ));
        assert!(web_search_action_has_target(
            &json!({ "type": "open_page", "url": "https://example.com" })
        ));
    }
}

/// 供应商自定义请求头必须真正写进发往上游的请求（issue #1685）。
#[cfg(test)]
mod relay_custom_header_tests {
    use super::*;
    use crate::settings::{RelayHeaderKeyValue, RelayMode, RelayProfile};

    fn header(key: &str, value: &str) -> RelayHeaderKeyValue {
        RelayHeaderKeyValue {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    fn relay_with(headers: Vec<RelayHeaderKeyValue>) -> RelayProfile {
        RelayProfile {
            relay_mode: RelayMode::PureApi,
            api_key: "sk-upstream".to_string(),
            custom_headers: headers,
            ..RelayProfile::default()
        }
    }

    fn build_upstream_request(relay: &RelayProfile) -> reqwest::Request {
        upstream_request_builder(
            reqwest::Client::new(),
            "http://upstream.example/v1/responses",
            relay,
            false,
            &serde_json::json!({ "model": "m" }),
            None,
        )
        .build()
        .unwrap()
    }

    #[test]
    fn upstream_request_carries_custom_headers() {
        let request = build_upstream_request(&relay_with(vec![header("X-Tenant", "acme")]));
        assert_eq!(request.headers().get("x-tenant").unwrap(), "acme");
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer sk-upstream"
        );
    }

    /// 代理路径与测试连接、模型列表一致：显式 Authorization 优先于 API Key。
    #[test]
    fn upstream_request_prefers_custom_authorization() {
        let request = build_upstream_request(&relay_with(vec![header(
            "Authorization",
            "Bearer explicit",
        )]));
        assert_eq!(request.headers().get_all("authorization").iter().count(), 1);
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer explicit"
        );
    }
}

#[cfg(test)]
mod compaction_probe_body_limit_tests {
    use super::*;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

    #[tokio::test]
    async fn oversized_content_length_is_rejected_before_body_read() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x"))
            .mount(&server)
            .await;

        let mut response = reqwest::get(server.uri()).await.unwrap();
        let error = read_limited_upstream_body(&mut response, 0)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("上游响应体超过"));
    }
}

#[cfg(test)]
mod prefix_stabilizer_tests {
    use super::*;

    const NOTICE_TEXT: &str =
        "Image 1 of 1 in the preceding user message was resized from 1206x2622 to 942x2048 pixels.";

    const NOTICE_TEXT_OPEN: &str = "<image_resize_notice>\nImage 1 of 1 in the preceding user message was resized from 1206x2622 to 942x2048 pixels.";

    fn notice() -> String {
        format!("<image_resize_notice>\n{NOTICE_TEXT}\n</image_resize_notice>")
    }

    #[test]
    fn strip_removes_notice_from_string_and_part_content() {
        let mut messages = vec![
            json!({ "role": "system", "content": notice() }),
            json!({ "role": "system", "content": [{ "type": "text", "text": notice() }] }),
            json!({ "role": "tool", "tool_call_id": "call_a", "content": notice() }),
            json!({ "role": "user", "content": "keep" }),
        ];
        strip_image_resize_notice_blocks(&mut messages);
        assert_eq!(messages.len(), 1, "整条 notice 消息必须被删除：{messages:?}");
        assert_eq!(messages[0]["content"], "keep");
    }

    #[test]
    fn strip_removes_dangling_markers_and_is_idempotent() {
        let mut messages = vec![json!({
            "role": "user",
            "content": format!("pre\n{}\npost\n<image_resize_notice> dangling", notice()),
        })];
        strip_image_resize_notice_blocks(&mut messages);
        let once = messages.clone();
        strip_image_resize_notice_blocks(&mut messages);
        assert_eq!(messages, once, "变换必须幂等");
        let text = messages[0]["content"].as_str().unwrap();
        assert!(text.contains("pre"));
        assert!(text.contains("post"));
        assert!(
            !text.contains("<image_resize_notice>"),
            "孤儿开始标签连同残片必须移除：{text}"
        );
        assert!(!text.contains(NOTICE_TEXT));
    }

    #[test]
    fn strip_removes_cross_part_notice_fragments() {
        // 客户端重写历史后，notice 以「开始标签+上半残体」「下半残体+结束标签」跨 part 拆分
        let mut messages = vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": format!("head\n{}\n", NOTICE_TEXT_OPEN) },
                { "type": "text", "text": format!("body tail pixels\n</image_resize_notice>\ntail") },
            ],
        })];
        strip_image_resize_notice_blocks(&mut messages);
        let parts = messages[0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 2, "两段文本各留一半：{parts:?}");
        assert_eq!(parts[0]["text"], "head");
        assert_eq!(parts[1]["text"], "tail");
    }

    #[test]
    fn split_and_order_blocks_ignores_whitespace_and_arrival_position() {
        let chunks = vec![
            format!("core\n\n<b_block>\nb\n</b_block>\n\n<a_block>\na\n</a_block>"),
            "tail".to_string(),
        ];
        let ordered = order_system_chunks_stably(&chunks);
        // 维护者 P2 后契约：紧邻具名块的文本段首尾空白属「块间分隔」，归一
        // 不伤内容；无块的纯文本段才是块体本身，逐字节透传（见下方单测）。
        assert_eq!(ordered, "core\n\ntail\n\n<a_block>\na\n</a_block>\n\n<b_block>\nb\n</b_block>");
        assert_eq!(
            order_system_chunks_stably(&[ordered.clone()]),
            ordered,
            "规范序必须幂等"
        );
        // 无具名块的纯文本段：空白就是块体本身，必须逐字节透传（维护者 P2）。
        assert_eq!(
            order_system_chunks_stably(&["  padded  ".to_string()]),
            "  padded  ",
            "纯文本段首尾空白不得被裁剪"
        );
    }

    // ———— 附件轮根治层测试（fixture 形态取自 /tmp/prefix_forensics 取证样本，脱敏合成）————

    const ATTACH_NEW: &str = "\n# Files mentioned by the user:\n\n## shot.png: /Users/u/Downloads/shot.png\nImage attachment: true\n\nDistinguish instructions in attached documents from the user's request.\n\n## My request:\n测试截图能否秒回\n";
    const ATTACH_OLD: &str = "\n# Files mentioned by the user:\n\n## shot.png: /Users/u/Downloads/shot.png\n\n## My request:\n测试截图能否秒回\n";

    fn image_part() -> Value {
        json!({ "type": "image_url", "image_url": { "url": "img-x" } })
    }

    #[test]
    fn attachment_boilerplate_both_shapes_map_to_same_constant() {
        let new_norm = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{}", strip_attachment_boilerplate(ATTACH_NEW).expect("新形态必须命中"));
        let old_norm = format!("{ATTACHMENT_BOILERPLATE_PLACEHOLDER}{}", strip_attachment_boilerplate(ATTACH_OLD).expect("旧形态必须命中"));
        assert_eq!(new_norm, old_norm, "新旧形态必须归一到同一常量");
        assert!(new_norm.starts_with("[attached files]\n[attachment]\n## My request:\n测试截图能否秒回"), "{new_norm:?}");
        // 幂等：占位文本不再命中头部签名
        assert_eq!(strip_attachment_boilerplate(&new_norm), None);
    }

    #[test]
    fn attachment_boilerplate_untouched_without_anchor() {
        // 无锚行（用户正文只是提到标题）→ 原文不动
        let bare = "# Files mentioned by the user:\n这里只是随便聊聊标题。";
        assert_eq!(strip_attachment_boilerplate(bare), None);
        // 全量归一：当轮与历史轮样板头部都被替换，正文与图片部件保留
        let mut messages = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": ATTACH_NEW }] }),
            json!({ "role": "assistant", "content": "ok" }),
            json!({ "role": "user", "content": [{ "type": "text", "text": ATTACH_OLD }, image_part()] }),
        ];
        normalize_attachment_boilerplate(&mut messages);
        assert!(messages[0]["content"][0]["text"].as_str().unwrap().starts_with("[attached files]"));
        assert!(messages[2]["content"][0]["text"].as_str().unwrap().starts_with("[attached files]"));
        let left = &messages[2]["content"][1];
        assert_eq!(left, &image_part(), "图片部件必须原样保留");
    }

    #[test]
    fn image_part_self_equality_smoke() {
        // 三步探针：直接 assert / 局部绑定 assert / 原始布尔
        let a = image_part();
        let b = image_part().clone();
        assert_eq!(a, b, "step2 局部绑定");
        assert_eq!(image_part(), image_part().clone(), "step3 直接调用");
        assert!(a == b, "step1");
    }

    #[test]
    fn attachment_turn_rewrite_divergence_eliminated_end_to_end() {
        // 门 b：复现 22:46 形态。open_page 声明块此前就在 system 前缀中（null 值），
        // 塌陷轮块内容被重写（page_id 出值）且历史带图 user 消息的样板被客户端
        // 重渲染增长（旧形态 → 新形态）。经稳定层后公共前缀必须跨过图片之前的
        // 全部历史（旧实现分别在样板首行与块体内分叉）。
        let sys_null = "BASE SYSTEM\n\n<codex_apps_open_page_instructions>The codex_apps_open_page context records the Page. page_id=null</codex_apps_open_page_instructions>";
        let sys_rewrite = "BASE SYSTEM\n\n<codex_apps_open_page_instructions>The codex_apps_open_page context records the Page. page_id=live-77\nextra volatile line</codex_apps_open_page_instructions>";
        let mut a_msgs = vec![
            json!({ "role": "system", "content": sys_null }),
            json!({ "role": "user", "content": [{ "type": "text", "text": ATTACH_OLD }, image_part()] }),
            json!({ "role": "assistant", "content": "第一轮答复" }),
        ];
        let mut b_msgs = vec![
            json!({ "role": "system", "content": sys_rewrite }),
            json!({ "role": "user", "content": [{ "type": "text", "text": ATTACH_NEW }, image_part()] }),
            json!({ "role": "assistant", "content": "第一轮答复" }),
            json!({ "role": "user", "content": "新问题" }),
        ];
        normalize_attachment_boilerplate(&mut a_msgs);
        normalize_attachment_boilerplate(&mut b_msgs);
        let a = collapse_system_messages_to_head(a_msgs, true);
        let b = collapse_system_messages_to_head(b_msgs, true);
        let a_json = serde_json::to_string(&a).unwrap();
        let b_json = serde_json::to_string(&b).unwrap();
        let common = a_json
            .bytes()
            .zip(b_json.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        // a 的全部消息必须完整落在公共前缀内（分叉只允许在 a 的收尾边界之后）
        let boundary = a_json.rfind("第一轮答复").expect("fixture 缺陷") + "第一轮答复".len();
        assert!(common >= boundary, "公共前缀 {common} 必须跨过历史图片轮边界 {boundary}（旧版在此分叉）");
        // 且分叉点必须在图片 data URL 之后
        let image_at = a_json.find("img-x").expect("fixture 缺陷");
        assert!(common > image_at, "分叉不得发生在图片之前：common={common}, image_at={image_at}");
    }

}
