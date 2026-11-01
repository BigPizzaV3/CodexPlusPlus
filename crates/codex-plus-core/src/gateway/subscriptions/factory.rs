//! Factory（Droid 订阅）。
//!
//! - 登录：WorkOS 设备码（和 droid 一样的 client id）、导入本机 Droid 登录（`~/.factory/auth.json`，
//!   或环境变量 `FACTORY_API_KEY`）、粘贴 Factory API Key（`fk-…`）。access token 是 JWT，
//!   过期前用 refresh token 换新（WorkOS 会轮换 refresh token，新的必须写回）。
//! - 聊天：每个模型走 droid 给它用的那种接口——Claude 走 Anthropic Messages（`/api/llm/a/v1`），
//!   GPT / Grok 走 OpenAI Responses（`/api/llm/o/v1/responses`），其余开源模型走 Chat Completions。
//!   订阅只接受 droid 发出的请求：系统提示第一行必须是 droid 的身份句。
//! - Anthropic 路由拒收根部带 anyOf / oneOf / allOf 的工具 schema：这类工具把参数包一层
//!   `{"arguments": …}` 发上去，回来时再解包。

use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

use crate::anthropic_bridge::{
    AnthropicRequestOptions, chat_completions_to_anthropic_messages, effort_to_anthropic_effort,
    effort_to_thinking_budget, inject_prompt_cache, model_supports_adaptive_thinking,
};
use crate::gateway::subscriptions::b_shared::{
    self, AnthropicRelay, ChatRelay, OpenAiChatSse, ResponsesRelay, Sse, content_text,
    decode_stream, system_texts,
};
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const WORKOS: &str = "https://api.workos.com/user_management";
const API: &str = "https://api.factory.ai";
const API_EU: &str = "https://api.eu.factory.ai";
/// droid 的公开 OAuth client id。
const CLIENT_ID: &str = "client_01HNM792M5G5G1A2THWPXKFMXB";
const VERSION: &str = "0.242.0";
/// 订阅只接受 droid 的请求：系统提示第一行必须是这句。
const DROID_LINE: &str = "You are Droid, an AI software engineering agent built by Factory.";
/// droid 发往 OpenAI 上游时带的固定组织头（不是凭据）。
const OPENAI_PLATFORM: &str = "org-bHuLtG1fGmYk5YaOihAAXFBw";
const FAST_BETA: &str = "fast-mode-2026-02-01";
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;
const DEFAULT_MAX_TOKENS: u64 = 32_000;

/// Factory 没有模型列表接口：droid 自带的模型表（不含暂不支持的 Gemini 路由）。
const DEFAULT_MODELS: &[&str] = &[
    "claude-opus-5-5",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
    "claude-opus-5",
    "claude-sonnet-5",
    "claude-fable-5.1",
    "claude-opus-4-8",
    "claude-sonnet-4-6",
    "claude-opus-5-5-fast",
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-6-astra",
    "gpt-5.6-sol",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.5-fast",
    "grok-4.7",
    "glm-5.3",
    "glm-5.3-flash",
    "glm-5.2",
    "kimi-k3",
    "deepseek-v4.1-flash",
    "qwen3.8-max",
    "minimax-m3",
    "minimax-m2.7",
    "mistral-large-4",
    "nemotron-3-ultra",
];

pub struct Factory;

pub static FACTORY: Factory = Factory;

/// 上游接口格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    Anthropic,
    Responses,
    Chat,
    Gemini,
}

/// 模型走哪种接口、`x-api-provider` 填什么（和 droid 的模型表一致）。
fn route(model: &str) -> (Wire, &'static str) {
    let model = model.to_ascii_lowercase();
    if model.starts_with("claude-") {
        (Wire::Anthropic, "anthropic")
    } else if model.starts_with("minimax-m2") {
        (Wire::Anthropic, "fireworks")
    } else if model.starts_with("gpt-") {
        (Wire::Responses, "openai")
    } else if model.starts_with("grok-") {
        (Wire::Responses, "xai")
    } else if model.starts_with("gemini-") {
        (Wire::Gemini, "google")
    } else if model.starts_with("glm-5.2") || model.starts_with("nemotron-") {
        (Wire::Chat, "baseten")
    } else if model.starts_with("mistral-") {
        (Wire::Chat, "mistral")
    } else {
        (Wire::Chat, "fireworks")
    }
}

/// 整个进程一个会话 id（droid 也是一个进程一个）。
fn session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

// ---------------------------------------------------------------------------
// 凭据
// ---------------------------------------------------------------------------

fn api_key_of(credentials: &Value) -> Option<&str> {
    credentials
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
}

fn token_of(credentials: &Value) -> anyhow::Result<String> {
    api_key_of(credentials)
        .or_else(|| credentials.get("access").and_then(Value::as_str))
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Factory 凭据缺失，请重新登录"))
}

/// 管理类接口（whoami / org）的根地址：EU 组织走 EU。
fn base(credentials: &Value) -> &'static str {
    if credentials.get("region").and_then(Value::as_str) == Some("eu") {
        API_EU
    } else {
        API
    }
}

/// 模型请求的根地址：whoami 给了私有部署地址时用它。
fn llm_base(credentials: &Value) -> String {
    match credentials
        .get("llm_host")
        .and_then(Value::as_str)
        .map(|host| host.trim().trim_end_matches('/'))
        .filter(|host| !host.is_empty())
    {
        Some(host) if host.starts_with("http://") || host.starts_with("https://") => {
            host.to_string()
        }
        Some(host) => format!("https://{host}"),
        None => base(credentials).to_string(),
    }
}

/// droid 的公共请求头。
fn factory_headers(credentials: &Value, token: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("authorization", format!("Bearer {token}")),
        ("x-factory-client", "cli".to_string()),
        ("x-client-version", VERSION.to_string()),
        ("user-agent", format!("factory-cli/{VERSION}")),
    ];
    if api_key_of(credentials).is_none()
        && let Some(org) = credentials
            .get("org_id")
            .and_then(Value::as_str)
            .filter(|org| !org.is_empty())
    {
        headers.push(("x-factory-org-id", org.to_string()));
    }
    headers
}

/// WorkOS 的表单请求：返回 (状态码, JSON)。400 + `error` 在轮询时是正常应答。
async fn workos(
    http: &reqwest::Client,
    path: &str,
    form: &[(&str, &str)],
) -> anyhow::Result<(u16, Value)> {
    let response = http
        .post(format!("{WORKOS}{path}"))
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(oauth::query(form))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("无法连接 Factory 的登录服务（WorkOS）")?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// 用 refresh token 换新（`organization_id` 只在把 token 放进某个组织时带）。
async fn renew(
    http: &reqwest::Client,
    refresh: &str,
    organization: Option<&str>,
) -> anyhow::Result<(String, String)> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", CLIENT_ID),
    ];
    if let Some(organization) = organization {
        form.push(("organization_id", organization));
    }
    let (status, value) = workos(http, "/authenticate", &form).await?;
    let access = value.get("access_token").and_then(Value::as_str);
    match access {
        Some(access) if status == 200 => {
            let refresh = value
                .get("refresh_token")
                .and_then(Value::as_str)
                .unwrap_or(refresh);
            Ok((access.to_string(), refresh.to_string()))
        }
        _ => {
            let detail = b_shared::str_at(&value, &["error_description", "error", "message"])
                .unwrap_or("未知错误")
                .to_string();
            // 4xx（429 除外）：refresh token 已作废，要重新登录。
            let status = if (400..500).contains(&status) && status != 429 {
                401
            } else {
                502
            };
            Err(fail(status, format!("Factory 登录刷新失败：{detail}")))
        }
    }
}

/// GET 一个管理接口（JSON）。
async fn get_json(
    http: &reqwest::Client,
    url: &str,
    headers: &[(&'static str, String)],
) -> anyhow::Result<Value> {
    let mut request = http.get(url).timeout(Duration::from_secs(20));
    for (key, value) in headers {
        request = request.header(*key, value);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    oauth::read_json(response).await
}

/// whoami：组织、区域、私有部署地址、邮箱。只带最少的头（和 droid 一样）。
async fn whoami(http: &reqwest::Client, token: &str, org: Option<&str>) -> anyhow::Result<Value> {
    let mut headers = vec![
        ("authorization", format!("Bearer {token}")),
        ("x-factory-whoami-extended", "true".to_string()),
    ];
    if let Some(org) = org.filter(|org| !org.is_empty()) {
        headers.push(("x-factory-org-id", org.to_string()));
    }
    get_json(http, &format!("{API}/api/cli/whoami"), &headers).await
}

/// WorkOS 登录拿到 token 之后：补组织、查 whoami，整理成凭据。
async fn signed_in(
    http: &reqwest::Client,
    mut access: String,
    mut refresh: String,
    organization: Option<String>,
    email: Option<String>,
) -> anyhow::Result<LoginDone> {
    let claims = oauth::jwt_claims(&access);
    let mut org = organization
        .filter(|org| !org.is_empty())
        .or_else(|| b_shared::str_at(&claims, &["org_id"]).map(str::to_string));
    if org.is_none() && b_shared::str_at(&claims, &["external_org_id"]).is_none() {
        // token 没放进组织：取第一个组织，带 organization_id 刷新一次。
        let orgs = get_json(
            http,
            &format!("{API}/api/cli/org"),
            &factory_headers(&json!({}), &access),
        )
        .await
        .unwrap_or(Value::Null);
        if let Some(first) = b_shared::str_at(&orgs, &["/workosOrgIds/0"]).map(str::to_string) {
            let (fresh, rotated) = renew(http, &refresh, Some(&first)).await?;
            access = fresh;
            refresh = rotated;
            org = Some(first);
        }
    }
    let who = whoami(http, &access, org.as_deref())
        .await
        .unwrap_or(Value::Null);
    let claims = oauth::jwt_claims(&access);
    let account = b_shared::str_at(&who, &["email"])
        .map(str::to_string)
        .or(email)
        .or_else(|| b_shared::str_at(&claims, &["email", "sub"]).map(str::to_string))
        .unwrap_or_else(|| "Factory".to_string());
    let credentials = json!({
        "access": access,
        "refresh": refresh,
        "expires_ms": oauth::jwt_expires_ms(&access).unwrap_or(0),
        "org_id": b_shared::str_at(&who, &["orgId"]).map(str::to_string).or(org),
        "region": b_shared::str_at(&who, &["region"]),
        "llm_host": b_shared::str_at(&who, &["premBaseHostV2"]),
        "email": account,
        "source": "workos",
    });
    Ok(LoginDone {
        credentials,
        account,
    })
}

/// API Key 登录：whoami 只为了拿区域和私有部署地址，失败也能用。
async fn api_key_login(http: &reqwest::Client, key: &str) -> anyhow::Result<LoginDone> {
    let who = match whoami(http, key, None).await {
        Ok(who) => who,
        Err(error) => {
            let status = error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .map(|error| error.status);
            if status == Some(401) {
                return Err(fail(401, "Factory API Key 无效"));
            }
            Value::Null
        }
    };
    let suffix: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let account = b_shared::str_at(&who, &["email"])
        .map(str::to_string)
        .unwrap_or_else(|| format!("API Key …{suffix}"));
    Ok(LoginDone {
        credentials: json!({
            "api_key": key,
            "region": b_shared::str_at(&who, &["region"]),
            "llm_host": b_shared::str_at(&who, &["premBaseHostV2"]),
            "email": account,
            "source": "api_key",
        }),
        account,
    })
}

/// 本机 Droid 的配置目录（`$FACTORY_HOME_OVERRIDE/.factory` 优先）。
fn factory_dir() -> std::path::PathBuf {
    std::env::var_os("FACTORY_HOME_OVERRIDE")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(b_shared::home)
        .join(".factory")
}

/// 解析 droid 的 `auth.json`：(access, refresh)。
fn parse_droid_auth(value: &Value) -> anyhow::Result<(String, String)> {
    let access = b_shared::str_at(
        value,
        &["access_token", "accessToken", "/tokens/access_token"],
    )
    .context("Droid 的登录文件里没有 access token")?;
    let refresh = b_shared::str_at(
        value,
        &["refresh_token", "refreshToken", "/tokens/refresh_token"],
    )
    .unwrap_or_default();
    Ok((access.to_string(), refresh.to_string()))
}

/// 读本机 Droid 登录：先看 `auth.json`，再看 `FACTORY_API_KEY`。
fn read_local() -> anyhow::Result<Value> {
    let dir = factory_dir();
    let path = dir.join("auth.json");
    if path.exists() {
        let value = oauth::read_local_json(&path)?;
        let (access, refresh) = parse_droid_auth(&value)?;
        return Ok(json!({ "access": access, "refresh": refresh }));
    }
    if let Some(key) = std::env::var("FACTORY_API_KEY")
        .ok()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        return Ok(json!({ "api_key": key }));
    }
    if dir.join("auth.encrypted").exists() {
        anyhow::bail!("本机 Droid 的登录是加密保存的，无法导入，请改用设备码登录");
    }
    anyhow::bail!(
        "没找到 {}，请先运行 droid 登录，或改用设备码登录",
        path.display()
    )
}

// ---------------------------------------------------------------------------
// 请求体
// ---------------------------------------------------------------------------

/// 系统提示前面加上 droid 的身份句（已经有了就不重复）。
fn with_droid_line(text: &str) -> String {
    if text.trim().is_empty() {
        DROID_LINE.to_string()
    } else if text.starts_with(DROID_LINE) {
        text.to_string()
    } else {
        format!("{DROID_LINE}\n{text}")
    }
}

/// 根部是 anyOf / oneOf / allOf 的 schema。
fn union_root(schema: &Value) -> bool {
    ["anyOf", "oneOf", "allOf"]
        .iter()
        .any(|key| schema.get(*key).is_some())
}

/// 包进 `properties.arguments` 后，原 schema 里的本地 `$ref` 要改指向。
fn rewrite_refs(value: &mut Value, root: bool) {
    match value {
        Value::Object(map) => {
            if !root
                && map
                    .get("$id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
            {
                return;
            }
            for (key, child) in map.iter_mut() {
                match key.as_str() {
                    "$ref" | "$dynamicRef" => {
                        if let Value::String(reference) = child
                            && let Some(pointer) = reference.strip_prefix('#')
                            && (pointer.is_empty() || pointer.starts_with('/'))
                        {
                            *reference = format!("#/properties/arguments{pointer}");
                        }
                    }
                    "const" | "default" | "examples" | "enum" => {}
                    _ => rewrite_refs(child, false),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                rewrite_refs(item, false);
            }
        }
        _ => {}
    }
}

/// Anthropic 请求里根部是联合类型的工具：参数包一层 `arguments`，历史里的 tool_use 同步改。
/// 返回被包装的工具名。
fn wrap_union_tools(body: &mut Value) -> HashSet<String> {
    let mut names = HashSet::new();
    for tool in body
        .get_mut("tools")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(schema) = tool.get("input_schema").filter(|schema| union_root(schema)) else {
            continue;
        };
        let mut inner = schema.clone();
        rewrite_refs(&mut inner, true);
        let mut wrapped = json!({
            "type": "object",
            "properties": { "arguments": inner },
            "required": ["arguments"],
            "additionalProperties": false,
        });
        if let Some(dialect) = schema.get("$schema") {
            wrapped["$schema"] = dialect.clone();
        }
        tool["input_schema"] = wrapped;
        if let Some(name) = tool.get("name").and_then(Value::as_str) {
            names.insert(name.to_string());
        }
    }
    if names.is_empty() {
        return names;
    }
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
            let is_wrapped = block.get("type").and_then(Value::as_str) == Some("tool_use")
                && block
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| names.contains(name));
            if is_wrapped {
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                block["input"] = json!({ "arguments": input });
            }
        }
    }
    names
}

/// Chat → Anthropic Messages（droid 身份句、思考、快速模式、联合 schema 包装）。
fn anthropic_body(request: &Value, model: &str) -> anyhow::Result<(Value, HashSet<String>)> {
    let options = AnthropicRequestOptions {
        max_tokens: DEFAULT_MAX_TOKENS,
        ..AnthropicRequestOptions::default()
    };
    let mut body = chat_completions_to_anthropic_messages(request.clone(), &options)?;
    body["model"] = json!(model);
    body["stream"] = json!(true);
    let system = body
        .get("system")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut blocks = vec![json!({ "type": "text", "text": DROID_LINE })];
    let rest = system
        .strip_prefix(DROID_LINE)
        .unwrap_or(&system)
        .trim_start();
    if !rest.is_empty() {
        blocks.push(json!({ "type": "text", "text": rest }));
    }
    body["system"] = json!(blocks);

    if let Some(effort) = b_shared::reasoning_effort(request)
        && !matches!(effort.as_str(), "none" | "off" | "disabled")
    {
        if model_supports_adaptive_thinking(model)
            && let Some(level) = effort_to_anthropic_effort(&effort)
        {
            body["thinking"] = json!({ "type": "adaptive" });
            body["output_config"] = json!({ "effort": level });
        } else if let Some(budget) = effort_to_thinking_budget(&effort) {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
            let max = body.get("max_tokens").and_then(Value::as_u64).unwrap_or(0);
            if max <= budget {
                body["max_tokens"] = json!(budget + 4096);
            }
        }
        // 开着思考时 Anthropic 只接受默认采样参数。
        if let Some(object) = body.as_object_mut() {
            object.remove("temperature");
            object.remove("top_p");
        }
    }
    if model.ends_with("-fast") {
        body["speed"] = json!("fast");
    }
    let wrapped = wrap_union_tools(&mut body);
    inject_prompt_cache(&mut body);
    Ok((body, wrapped))
}

/// 消息内容 → Responses 的输入片段。
fn responses_parts(content: Option<&Value>, assistant: bool) -> Vec<Value> {
    let text_type = if assistant {
        "output_text"
    } else {
        "input_text"
    };
    match content {
        Some(Value::String(text)) if !text.is_empty() => {
            vec![json!({ "type": text_type, "text": text })]
        }
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| match part.get("type").and_then(Value::as_str) {
                Some("image_url") if !assistant => {
                    let url = part
                        .pointer("/image_url/url")
                        .or_else(|| part.get("image_url"))
                        .and_then(Value::as_str)?;
                    Some(json!({ "type": "input_image", "image_url": url }))
                }
                _ => {
                    let text = part.get("text").and_then(Value::as_str)?;
                    Some(json!({ "type": text_type, "text": text }))
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Chat → OpenAI Responses（无状态：store=false）。
fn responses_body(request: &Value, model: &str, upstream: &str) -> Value {
    let mut input = Vec::new();
    for message in request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let content = message.get("content");
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {}
            "assistant" => {
                let parts = responses_parts(content, true);
                if !parts.is_empty() {
                    input.push(json!({ "type": "message", "role": "assistant", "content": parts }));
                }
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let arguments = match call.pointer("/function/arguments") {
                        Some(Value::String(text)) => text.clone(),
                        Some(other) => other.to_string(),
                        None => "{}".to_string(),
                    };
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "name": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
                        "arguments": arguments,
                    }));
                }
            }
            "tool" => input.push(json!({
                "type": "function_call_output",
                "call_id": message.get("tool_call_id").and_then(Value::as_str).unwrap_or_default(),
                "output": content_text(content),
            })),
            _ => {
                let parts = responses_parts(content, false);
                if !parts.is_empty() {
                    input.push(json!({ "type": "message", "role": "user", "content": parts }));
                }
            }
        }
    }
    let mut body = json!({
        "model": model,
        "instructions": with_droid_line(&system_texts(request).join("\n\n")),
        "input": input,
        "stream": true,
        "store": false,
    });
    let tools: Vec<Value> = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            Some(json!({
                "type": "function",
                "name": function.get("name")?,
                "description": function.get("description").cloned().unwrap_or(json!("")),
                "parameters": function.get("parameters").cloned().unwrap_or(json!({ "type": "object", "properties": {} })),
            }))
        })
        .collect();
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        match request.get("tool_choice") {
            Some(Value::String(choice)) => body["tool_choice"] = json!(choice),
            Some(choice) => {
                if let Some(name) = choice.pointer("/function/name") {
                    body["tool_choice"] = json!({ "type": "function", "name": name });
                }
            }
            None => {}
        }
        if let Some(parallel) = request.get("parallel_tool_calls") {
            body["parallel_tool_calls"] = parallel.clone();
        }
    }
    if let Some(effort) = b_shared::reasoning_effort(request) {
        body["reasoning"] = json!({ "effort": effort, "summary": "auto" });
    }
    if let Some(max) = b_shared::max_tokens(request) {
        body["max_output_tokens"] = json!(max);
    }
    // OpenAI 的推理模型带 temperature / top_p 会 400。
    if upstream != "openai" {
        for key in ["temperature", "top_p"] {
            if let Some(value) = request.get(key) {
                body[key] = value.clone();
            }
        }
    }
    if model.ends_with("-fast") {
        body["service_tier"] = json!("priority");
    }
    body
}

/// Chat → Chat：首条 system 加 droid 身份句，要上 usage。
fn chat_body(request: &Value) -> Value {
    let mut body = request.clone();
    let mut messages = body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first_is_system = messages
        .first()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        .is_some_and(|role| matches!(role, "system" | "developer"));
    if first_is_system {
        let text = content_text(messages[0].get("content"));
        messages[0] = json!({ "role": "system", "content": with_droid_line(&text) });
    } else {
        messages.insert(0, json!({ "role": "system", "content": DROID_LINE }));
    }
    body["messages"] = json!(messages);
    body["stream"] = json!(true);
    body["stream_options"] = json!({ "include_usage": true });
    body
}

/// Factory 的错误体 → 带状态码的失败。
fn failure(status: u16, body: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let message = b_shared::str_at(
        &value,
        &[
            "/errors/0/message",
            "/detail/message",
            "/error/message",
            "detail",
            "error",
            "message",
            "msg",
        ],
    )
    .map(str::to_string)
    .unwrap_or_else(|| body.chars().take(300).collect());
    let code = b_shared::str_at(&value, &["code", "/error/code"]).unwrap_or_default();
    let lower = message.to_ascii_lowercase();
    let status = if status == 403 && code == "content_policy_rejected" {
        400
    } else if status == 401 {
        401
    } else if status == 402
        || status == 429
        || lower.contains("usage limit")
        || lower.contains("quota")
        || lower.contains("insufficient")
    {
        429
    } else {
        status
    };
    fail(status, format!("Factory 返回 {status}：{message}"))
}

#[async_trait::async_trait]
impl Subscription for Factory {
    fn id(&self) -> &'static str {
        "factory"
    }

    fn name(&self) -> &'static str {
        "Factory"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::DeviceCode,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要 Factory（Droid）付费订阅；可用设备码登录，或先在 Droid 里登录后导入，也可粘贴 fk- 开头的 API Key。"
    }

    fn detect_local(&self) -> Option<String> {
        // 只认 droid 的登录文件，不把环境变量里的 FACTORY_API_KEY 当成登录。
        let value = oauth::read_local_json(&factory_dir().join("auth.json")).ok()?;
        parse_droid_auth(&value).ok().map(|_| String::new())
    }

    async fn begin_login(
        &self,
        http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::DeviceCode => {
                let answer = oauth::post_form(
                    http,
                    &format!("{WORKOS}/authorize/device"),
                    &[("client_id", CLIENT_ID)],
                )
                .await?;
                let device_code = b_shared::str_at(&answer, &["device_code"])
                    .context("Factory 没有返回设备码")?;
                let url =
                    b_shared::str_at(&answer, &["verification_uri_complete", "verification_uri"])
                        .context("Factory 没有返回验证地址")?;
                Ok(LoginStart {
                    open_url: Some(url.to_string()),
                    user_code: b_shared::str_at(&answer, &["user_code"]).map(str::to_string),
                    message: "在浏览器里确认验证码并登录 Factory。".into(),
                    state: json!({
                        "method": "device",
                        "device_code": device_code,
                        "interval": b_shared::u64_at(&answer, &["interval"]).unwrap_or(5),
                    }),
                })
            }
            LoginMethod::ImportLocal => Ok(LoginStart {
                message: "已读取本机 Droid 的登录。".into(),
                state: json!({ "method": "local", "local": read_local()? }),
                ..LoginStart::default()
            }),
            LoginMethod::Paste => {
                let key = input.trim().trim_start_matches("Bearer ").trim();
                if !key.starts_with("fk-") {
                    anyhow::bail!("请粘贴 fk- 开头的 Factory API Key");
                }
                Ok(LoginStart {
                    message: "正在校验 API Key。".into(),
                    state: json!({ "method": "paste", "api_key": key }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Browser => anyhow::bail!("Factory 请用设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        match state
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "device" => {
                let device_code = state
                    .get("device_code")
                    .and_then(Value::as_str)
                    .context("缺少设备码")?
                    .to_string();
                let interval = state.get("interval").and_then(Value::as_u64).unwrap_or(5);
                let answer = oauth::poll_until(Duration::from_secs(interval), || {
                    let http = http.clone();
                    let device_code = device_code.clone();
                    async move {
                        let (status, value) = workos(
                            &http,
                            "/authenticate",
                            &[
                                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                                ("device_code", &device_code),
                                ("client_id", CLIENT_ID),
                            ],
                        )
                        .await?;
                        if status == 200 && value.get("access_token").is_some() {
                            return Ok(Some(value));
                        }
                        match value.get("error").and_then(Value::as_str) {
                            Some("authorization_pending" | "slow_down") => Ok(None),
                            Some("expired_token") => anyhow::bail!("验证码已过期，请重新登录"),
                            Some("access_denied") => anyhow::bail!("登录被拒绝"),
                            _ if status >= 500 => Ok(None),
                            _ => anyhow::bail!(
                                "Factory 登录失败：{}",
                                b_shared::str_at(
                                    &value,
                                    &["error_description", "error", "message"]
                                )
                                .unwrap_or("未知错误")
                            ),
                        }
                    }
                })
                .await?;
                signed_in(
                    http,
                    b_shared::str_at(&answer, &["access_token"])
                        .unwrap_or_default()
                        .to_string(),
                    b_shared::str_at(&answer, &["refresh_token"])
                        .unwrap_or_default()
                        .to_string(),
                    b_shared::str_at(&answer, &["organization_id"]).map(str::to_string),
                    b_shared::str_at(&answer, &["/user/email"]).map(str::to_string),
                )
                .await
            }
            "local" => {
                let local = state.get("local").cloned().unwrap_or(Value::Null);
                if let Some(key) = api_key_of(&local) {
                    return api_key_login(http, key).await;
                }
                let mut access = b_shared::str_at(&local, &["access"])
                    .unwrap_or_default()
                    .to_string();
                let mut refresh = b_shared::str_at(&local, &["refresh"])
                    .unwrap_or_default()
                    .to_string();
                let expired = oauth::jwt_expires_ms(&access)
                    .is_some_and(|expires| expires <= oauth::now_ms() + REFRESH_AHEAD_MS);
                if expired && !refresh.is_empty() {
                    (access, refresh) = renew(http, &refresh, None).await?;
                }
                signed_in(http, access, refresh, None, None).await
            }
            "paste" => {
                let key = state
                    .get("api_key")
                    .and_then(Value::as_str)
                    .context("缺少 API Key")?;
                api_key_login(http, key).await
            }
            _ => anyhow::bail!("未知的登录方式"),
        }
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if api_key_of(credentials).is_some() {
            return Ok(None);
        }
        let expires = credentials
            .get("expires_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if expires > 0 && !oauth::expiring(credentials, "expires_ms", REFRESH_AHEAD_MS) {
            return Ok(None);
        }
        let Some(refresh) = credentials
            .get("refresh")
            .and_then(Value::as_str)
            .filter(|refresh| !refresh.is_empty())
        else {
            return Ok(None);
        };
        match renew(http, refresh, None).await {
            Ok((access, rotated)) => {
                let mut fresh = credentials.clone();
                fresh["expires_ms"] = json!(oauth::jwt_expires_ms(&access).unwrap_or(0));
                fresh["access"] = json!(access);
                fresh["refresh"] = json!(rotated);
                Ok(Some(fresh))
            }
            // 刷新服务抖动但旧 token 还没过期：先接着用。
            Err(error)
                if expires > oauth::now_ms()
                    && error
                        .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                        .is_none_or(|error| error.status != 401) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    async fn models(
        &self,
        _http: &reqwest::Client,
        _credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        Ok(DEFAULT_MODELS
            .iter()
            .map(|model| model.to_string())
            .collect())
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let token = token_of(credentials)?;
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let (wire, upstream) = route(&model);
        let root = llm_base(credentials);
        let mut headers = factory_headers(credentials, &token);
        headers.push(("x-api-provider", upstream.to_string()));
        headers.push(("x-session-id", session_id().to_string()));
        headers.push(("x-assistant-message-id", uuid::Uuid::new_v4().to_string()));
        headers.push(("x-provider-routing-source", "registry_default".to_string()));
        headers.push(("content-type", "application/json".to_string()));
        let mut wrapped = HashSet::new();
        let (url, body) = match wire {
            Wire::Anthropic => {
                let (body, names) = anthropic_body(&request, &model)?;
                wrapped = names;
                headers.push(("anthropic-version", "2023-06-01".to_string()));
                headers.push(("x-api-key", "placeholder".to_string()));
                if model.ends_with("-fast") {
                    headers.push(("anthropic-beta", FAST_BETA.to_string()));
                }
                (format!("{root}/api/llm/a/v1/messages"), body)
            }
            Wire::Responses => {
                if upstream == "openai" {
                    headers.push(("openai-platform", OPENAI_PLATFORM.to_string()));
                }
                (
                    format!("{root}/api/llm/o/v1/responses"),
                    responses_body(&request, &model, upstream),
                )
            }
            Wire::Chat => (
                format!("{root}/api/llm/o/v1/chat/completions"),
                chat_body(&request),
            ),
            Wire::Gemini => {
                return Err(fail(400, "Factory 的 Gemini 模型暂不支持，请换别的模型"));
            }
        };
        let mut builder = http.post(&url).body(body.to_string());
        for (key, value) in &headers {
            builder = builder.header(*key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 Factory：{error}")))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(status, &text));
        }
        Ok(match wire {
            Wire::Anthropic => {
                let mut relay = AnthropicRelay::new(&model, "Factory");
                relay.wrapped = wrapped;
                decode_stream(response, Sse::new(relay))
            }
            Wire::Responses => {
                decode_stream(response, Sse::new(ResponsesRelay::new(&model, "Factory")))
            }
            _ => decode_stream(
                response,
                Sse::new(OpenAiChatSse {
                    relay: ChatRelay::new(&model),
                    vendor: "Factory",
                }),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "model": "claude-opus-5-5",
            "reasoning_effort": "high",
            "temperature": 0.2,
            "messages": [
                { "role": "system", "content": "Be brief." },
                { "role": "user", "content": "改一下" },
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "edit", "arguments": "{\"path\":\"a\"}" } },
                    { "id": "call_2", "type": "function", "function": { "name": "ls", "arguments": "{}" } }
                ]},
                { "role": "tool", "tool_call_id": "call_1", "content": "ok" },
                { "role": "tool", "tool_call_id": "call_2", "content": "a b" }
            ],
            "tools": [
                { "type": "function", "function": { "name": "edit", "description": "Edit", "parameters": {
                    "anyOf": [{ "$ref": "#/$defs/A" }], "$defs": { "A": { "type": "object" } }
                }}},
                { "type": "function", "function": { "name": "ls", "parameters": { "type": "object", "properties": {} } } }
            ]
        })
    }

    #[test]
    fn routes_models_to_wires() {
        assert_eq!(route("claude-opus-5-5"), (Wire::Anthropic, "anthropic"));
        assert_eq!(route("minimax-m2.7"), (Wire::Anthropic, "fireworks"));
        assert_eq!(route("gpt-5.5-fast"), (Wire::Responses, "openai"));
        assert_eq!(route("grok-4.7"), (Wire::Responses, "xai"));
        assert_eq!(route("glm-5.2"), (Wire::Chat, "baseten"));
        assert_eq!(route("glm-5.3"), (Wire::Chat, "fireworks"));
        assert_eq!(route("gemini-3.5-flash").0, Wire::Gemini);
        assert!(
            DEFAULT_MODELS
                .iter()
                .all(|model| route(model).0 != Wire::Gemini)
        );
    }

    #[test]
    fn anthropic_body_adds_identity_thinking_and_wraps_unions() {
        let (body, wrapped) = anthropic_body(&request(), "claude-opus-5-5").unwrap();
        assert_eq!(body["system"][0]["text"], DROID_LINE);
        assert_eq!(body["system"][1]["text"], "Be brief.");
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert!(body.get("temperature").is_none());
        assert!(wrapped.contains("edit"));
        assert!(!wrapped.contains("ls"));
        let edit = &body["tools"][0]["input_schema"];
        assert_eq!(edit["required"], json!(["arguments"]));
        assert_eq!(
            edit["properties"]["arguments"]["anyOf"][0]["$ref"],
            "#/properties/arguments/$defs/A"
        );
        // 历史里的 tool_use 也包一层；工具结果并成一条 user。
        let messages = body["messages"].as_array().unwrap();
        let assistant = &messages[1];
        assert_eq!(
            assistant["content"][0]["input"],
            json!({ "arguments": { "path": "a" } })
        );
        assert_eq!(assistant["content"][1]["input"], json!({}));
        let results = &messages[2]["content"];
        assert_eq!(results[0]["type"], "tool_result");
        assert_eq!(results[1]["tool_use_id"], "call_2");
    }

    #[test]
    fn fast_models_set_speed() {
        let (body, _) = anthropic_body(&request(), "claude-opus-5-5-fast").unwrap();
        assert_eq!(body["speed"], "fast");
        let body = responses_body(&request(), "gpt-5.5-fast", "openai");
        assert_eq!(body["service_tier"], "priority");
    }

    #[test]
    fn responses_body_maps_messages_and_tools() {
        let body = responses_body(&request(), "gpt-5.5", "openai");
        assert_eq!(body["instructions"], format!("{DROID_LINE}\nBe brief."));
        assert_eq!(body["store"], false);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(body["tools"][1]["name"], "ls");
        assert_eq!(body["reasoning"]["effort"], "high");
        // OpenAI 上游去掉 temperature，xAI 保留。
        assert!(body.get("temperature").is_none());
        assert_eq!(
            responses_body(&request(), "grok-4.7", "xai")["temperature"],
            0.2
        );
    }

    #[test]
    fn chat_body_prefixes_identity() {
        let body = chat_body(&request());
        assert_eq!(
            body["messages"][0]["content"],
            format!("{DROID_LINE}\nBe brief.")
        );
        let bare = chat_body(&json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        assert_eq!(bare["messages"][0]["content"], DROID_LINE);
        assert_eq!(bare["stream_options"]["include_usage"], true);
        assert_eq!(with_droid_line(DROID_LINE), DROID_LINE);
    }

    #[test]
    fn maps_errors() {
        let status = |error: anyhow::Error| {
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status
        };
        assert_eq!(
            status(failure(
                403,
                r#"{"code":"content_policy_rejected","message":"no"}"#
            )),
            400
        );
        assert_eq!(status(failure(401, "{}")), 401);
        assert_eq!(
            status(failure(403, r#"{"detail":"Usage limit reached"}"#)),
            429
        );
        assert!(
            failure(500, r#"{"errors":[{"message":"boom"}]}"#)
                .to_string()
                .contains("boom")
        );
    }

    #[test]
    fn reads_droid_auth_and_bases() {
        let (access, refresh) =
            parse_droid_auth(&json!({ "access_token": "a", "refresh_token": "r" })).unwrap();
        assert_eq!((access.as_str(), refresh.as_str()), ("a", "r"));
        assert!(parse_droid_auth(&json!({ "accessToken": "a" })).is_ok());
        assert!(parse_droid_auth(&json!({})).is_err());
        let eu = json!({ "region": "eu", "llm_host": "llm.example.com/" });
        assert_eq!(base(&eu), API_EU);
        assert_eq!(llm_base(&eu), "https://llm.example.com");
        assert_eq!(llm_base(&json!({})), API);
        let headers = factory_headers(&json!({ "org_id": "org_1" }), "t");
        assert!(
            headers
                .iter()
                .any(|(key, value)| *key == "x-factory-org-id" && value == "org_1")
        );
        let keyed = factory_headers(&json!({ "api_key": "fk-1", "org_id": "org_1" }), "fk-1");
        assert!(!keyed.iter().any(|(key, _)| *key == "x-factory-org-id"));
    }

    #[test]
    fn refresh_skips_fresh_and_api_key_credentials() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let http = reqwest::Client::new();
        runtime.block_on(async {
            let fresh =
                json!({ "access": "a", "refresh": "r", "expires_ms": oauth::now_ms() + 3_600_000 });
            assert!(FACTORY.refresh(&http, &fresh).await.unwrap().is_none());
            let keyed = json!({ "api_key": "fk-1" });
            assert!(FACTORY.refresh(&http, &keyed).await.unwrap().is_none());
            // 没有 refresh token 就不刷新。
            let stale = json!({ "access": "a", "expires_ms": 1 });
            assert!(FACTORY.refresh(&http, &stale).await.unwrap().is_none());
        });
    }
}
