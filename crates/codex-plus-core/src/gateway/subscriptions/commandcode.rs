//! Command Code 套餐（commandcode.ai）。
//!
//! 账号就是一把 Command Code 的 API Key（不过期，不用刷新）：
//! - 浏览器登录：CLI 自己的授权页，Studio 把为本机批准的 key POST 回本机回调；
//! - 导入本机 CLI 的登录：`~/.commandcode/auth.json`；
//! - 粘贴 key。
//!
//! 请求分两路：
//! - Go 以外的套餐走 Provider API（`/provider/v1`）：Claude 模型只有 Anthropic Messages，
//!   其余模型是标准 Chat Completions（直接透传）；
//! - Go 套餐没有 Provider API，只能走 CLI 自己的 `POST /alpha/generate`（逐行 JSON），
//!   这里把 Chat 请求转成它的格式、把逐行事件转回 Chat SSE。套餐未知时 Provider API 回
//!   「Go plan doesn't include API access」也会改走这一路。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::bail;
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::gateway::subscriptions::chat::{SseReader, Usage, passthrough};
use crate::gateway::subscriptions::editor_kit::{
    self, AnthropicEvents, Emitter, LineReader, arguments_object, content_text, reasoning_effort,
    reasoning_text, text_of,
};
use crate::gateway::subscriptions::oauth::{LOGIN_TIMEOUT, now_ms, query};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const API: &str = "https://api.commandcode.ai";
const STUDIO: &str = "https://commandcode.ai";
const CLI_VERSION: &str = "1.72.2";
/// `/alpha/generate` 的 max_tokens：没给时的默认值和上限。
const GO_MAX_TOKENS: u64 = 64_000;
const GO_MAX_OUTPUT: u64 = 200_000;
/// 套餐多久重新读一次。
const PLAN_TTL_MS: u64 = 12 * 3600 * 1000;
const STUDIO_ORIGINS: &[&str] = &["https://commandcode.ai", "https://staging.commandcode.ai"];

pub struct CommandCode;

pub static COMMAND_CODE: CommandCode = CommandCode;

fn base() -> String {
    format!("{API}/provider/v1")
}

/* —————————————— 凭据 —————————————— */

fn api_key(credentials: &Value) -> anyhow::Result<String> {
    text_of(credentials, "apiKey")
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Command Code 凭据里没有 API Key，请重新登录"))
}

fn plan_of(credentials: &Value) -> String {
    text_of(credentials, "plan").unwrap_or_default().to_string()
}

fn is_go(credentials: &Value) -> bool {
    plan_of(credentials) == "Go"
}

/// `~/.commandcode/auth.json`。
fn cli_auth_path() -> std::path::PathBuf {
    editor_kit::home().join(".commandcode").join("auth.json")
}

/// CLI 登录文件里的 key 和用户名。
fn parse_cli_auth(value: &Value) -> anyhow::Result<(String, String)> {
    let key = text_of(value, "apiKey")
        .ok_or_else(|| anyhow::anyhow!("Command Code CLI 没有登录，请先运行 commandcode login"))?;
    let user = text_of(value, "userName")
        .or_else(|| text_of(value, "userId"))
        .unwrap_or_default();
    Ok((key.to_string(), user.to_string()))
}

/// planId → 套餐名（按前缀，顺序不能乱：`individual-goat` 不能被当成 Go）。
fn plan_name(plan_id: &str) -> &'static str {
    let id = plan_id.trim().to_ascii_lowercase().replace('_', "-");
    const TABLE: &[(&str, &str)] = &[
        ("individual-provider", "Provider"),
        ("individual-pro-v1", "Pro"),
        ("individual-goat", "GOAT"),
        ("individual-ultra", "Ultra"),
        ("individual-max", "Max"),
        ("individual-pro", "Pro"),
        ("individual-go", "Go"),
        ("teams-pro", "Teams Pro"),
    ];
    TABLE
        .iter()
        .find(|(prefix, _)| id.starts_with(prefix))
        .map(|(_, name)| *name)
        .unwrap_or("")
}

/// `/alpha/billing/subscriptions` 的答复 → 套餐名；读失败（`success:false`）为 None，没有套餐为 `No plan`。
fn parse_plan(value: &Value) -> Option<String> {
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let data = value.get("data").filter(|data| data.is_object());
    let Some(data) = data else {
        return Some("No plan".to_string());
    };
    let status = text_of(data, "status").unwrap_or_default();
    let plan_id = text_of(data, "planId").unwrap_or_default();
    if plan_id.is_empty() || matches!(status, "canceled" | "incomplete_expired") {
        return Some("No plan".to_string());
    }
    let name = plan_name(plan_id);
    Some(if name.is_empty() {
        plan_id.to_string()
    } else {
        name.to_string()
    })
}

async fn account_json(http: &reqwest::Client, path: &str, key: &str) -> anyhow::Result<Value> {
    let response = http
        .get(format!("{API}{path}"))
        .bearer_auth(key)
        .header("accept", "application/json")
        .timeout(Duration::from_secs(25))
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let (status, message) = failure(status, &text);
        return Err(fail(
            if matches!(status, 401 | 403) {
                401
            } else {
                status
            },
            format!("Command Code {path}：{message}"),
        ));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// whoami → 显示名。key 失效时报 401。
async fn whoami(http: &reqwest::Client, key: &str) -> anyhow::Result<String> {
    let mut last = None;
    for attempt in 0..3 {
        match account_json(http, "/alpha/whoami", key).await {
            Ok(value) => {
                let user = value.get("user").cloned().unwrap_or(Value::Null);
                return Ok(["userName", "email", "id"]
                    .iter()
                    .find_map(|field| text_of(&user, field))
                    .unwrap_or_default()
                    .to_string());
            }
            Err(error) => {
                let refused = error
                    .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                    .is_some_and(|error| error.status == 401);
                if refused {
                    return Err(fail(
                        401,
                        "Command Code 不认这个 API Key（已撤销或已退出登录）",
                    ));
                }
                last = Some(error);
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
    // 新 key 偶尔会拿到 500：账号名留空，不算登录失败。
    let _ = last;
    Ok(String::new())
}

async fn read_plan(http: &reqwest::Client, key: &str) -> Option<String> {
    let value = account_json(http, "/alpha/billing/subscriptions", key)
        .await
        .ok()?;
    parse_plan(&value)
}

async fn finish_with_key(
    http: &reqwest::Client,
    key: &str,
    source: &str,
    fallback_name: &str,
) -> anyhow::Result<LoginDone> {
    let name = whoami(http, key).await?;
    let plan = read_plan(http, key).await;
    let account = if name.is_empty() {
        fallback_name.to_string()
    } else {
        name
    };
    let mut credentials = json!({
        "apiKey": key,
        "source": source,
        "account": account,
        "planCheckedAt": now_ms(),
        // key 不过期。
        "expiresAt": 0,
    });
    if let Some(plan) = plan {
        credentials["plan"] = json!(plan);
    }
    Ok(LoginDone {
        credentials,
        account,
    })
}

/* —————————————— 浏览器登录：本机回调 —————————————— */

struct BrowserLogin {
    listener: tokio::net::TcpListener,
    state: String,
}

fn cors_origin(origin: Option<&str>) -> &'static str {
    origin
        .and_then(|origin| STUDIO_ORIGINS.iter().find(|item| **item == origin))
        .copied()
        .unwrap_or(STUDIO_ORIGINS[0])
}

/// 回调里的结果：Ok(key, userName) 或 Err(说明)。
fn callback_outcome(
    form: &std::collections::BTreeMap<String, String>,
    state: &str,
) -> Result<(String, String), String> {
    if form.get("state").map(String::as_str) != Some(state) {
        return Err("state 不一致".to_string());
    }
    if let Some(error) = form.get("error").filter(|error| !error.is_empty()) {
        if error == "access_denied" {
            return Err("你在 Command Code 页面上拒绝了授权".to_string());
        }
        return Err(form
            .get("error_description")
            .filter(|text| !text.is_empty())
            .cloned()
            .unwrap_or_else(|| error.clone()));
    }
    let key = form
        .get("apiKey")
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .ok_or_else(|| "Command Code 没有发回 API Key".to_string())?;
    let user = form
        .get("userName")
        .or_else(|| form.get("userId"))
        .cloned()
        .unwrap_or_default();
    Ok((key, user))
}

async fn wait_browser(login: BrowserLogin) -> anyhow::Result<(String, String)> {
    let BrowserLogin { listener, state } = login;
    let outcome = tokio::time::timeout(LOGIN_TIMEOUT, async {
        loop {
            let (mut stream, request) = editor_kit::next_request(&listener).await?;
            let origin = cors_origin(request.header("origin"));
            let mut headers = vec![
                ("access-control-allow-origin", origin),
                ("access-control-allow-methods", "GET, POST, OPTIONS"),
                ("access-control-allow-headers", "Content-Type"),
            ];
            if request
                .header("access-control-request-private-network")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"))
            {
                headers.push(("access-control-allow-private-network", "true"));
            }
            if request.method == "OPTIONS" {
                editor_kit::respond(&mut stream, "204 No Content", &headers, "", b"").await;
                continue;
            }
            if request.path != "/callback" {
                editor_kit::respond(&mut stream, "404 Not Found", &headers, "", b"").await;
                continue;
            }
            if request.method != "POST" {
                editor_kit::respond(&mut stream, "405 Method Not Allowed", &headers, "", b"").await;
                continue;
            }
            let json_request = request
                .header("content-type")
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
            let form = request.form();
            if form.get("state").map(String::as_str) != Some(state.as_str()) {
                editor_kit::respond(
                    &mut stream,
                    "403 Forbidden",
                    &headers,
                    "text/html; charset=utf-8",
                    &editor_kit::page("这个登录页已失效，请在 Codex++ 最新打开的页面里授权。"),
                )
                .await;
                continue;
            }
            let outcome = callback_outcome(&form, &state);
            if json_request {
                let body = match &outcome {
                    Ok(_) => json!({ "success": true, "error": "" }),
                    Err(message) => json!({ "success": false, "error": message }),
                };
                let status = if outcome.is_ok() {
                    "200 OK"
                } else {
                    "400 Bad Request"
                };
                editor_kit::respond(
                    &mut stream,
                    status,
                    &headers,
                    "application/json",
                    body.to_string().as_bytes(),
                )
                .await;
            } else {
                let location = format!("/callback/complete?ok={}", u8::from(outcome.is_ok()));
                headers.push(("location", location.as_str()));
                editor_kit::respond(&mut stream, "303 See Other", &headers, "", b"").await;
            }
            return anyhow::Ok(outcome);
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有在 Command Code 页面上授权"))??;
    // 浏览器接着会来取完成页：再留一会儿。
    let ok = outcome.is_ok();
    tokio::spawn(async move {
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let Ok((mut stream, request)) = editor_kit::next_request(&listener).await else {
                    break;
                };
                let text = if ok {
                    "登录成功，可以回到 Codex++ 了。"
                } else {
                    "登录没有完成，请回到 Codex++ 重新登录。"
                };
                let status = if request.path.starts_with("/callback/complete") {
                    "200 OK"
                } else {
                    "404 Not Found"
                };
                editor_kit::respond(
                    &mut stream,
                    status,
                    &[],
                    "text/html; charset=utf-8",
                    &editor_kit::page(text),
                )
                .await;
            }
        })
        .await;
    });
    outcome.map_err(|message| anyhow::anyhow!("登录失败：{message}"))
}

/* —————————————— 失败 —————————————— */

/// 上游的失败 → (状态码, 说明)。
fn failure(status: u16, text: &str) -> (u16, String) {
    let parsed: Option<Value> = serde_json::from_str(text).ok();
    let mut message = parsed
        .as_ref()
        .and_then(|value| {
            value
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .or_else(|| match value.get("error") {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(other @ Value::Object(_)) => Some(other.to_string()),
                    _ => None,
                })
        })
        .filter(|text| !text.trim().is_empty())
        .unwrap_or_else(|| text.trim().to_string());
    if message.is_empty() {
        message = reqwest::StatusCode::from_u16(status)
            .ok()
            .and_then(|code| code.canonical_reason())
            .unwrap_or("error")
            .to_string();
    }
    let mut message: String = message.chars().take(600).collect();
    let mut status = status;
    let lower = message.to_ascii_lowercase();
    if let Some(rest) = message
        .split_once("MODEL_NOT_IN_PLAN:")
        .map(|(_, rest)| rest.trim().to_string())
    {
        status = 403;
        message = format!("Model not in plan: {rest}");
    } else if lower.contains("model_not_in_plan") {
        status = 403;
    } else if lower.contains("premium_credits_exhausted") || lower.contains("insufficient credits")
    {
        status = 402;
        let rest = message
            .replace("PREMIUM_CREDITS_EXHAUSTED:", "")
            .trim()
            .to_string();
        let credit_words = ["credit", "额度", "余额", "limit", "quota"];
        message = if credit_words
            .iter()
            .any(|word| rest.to_ascii_lowercase().contains(word))
        {
            rest
        } else {
            format!("out of credits: {rest}")
        };
    } else if (message.contains("RATE_LIMITED")
        || lower.contains("usage limit")
        || lower.contains("window_limit"))
        && (status < 400 || status == 500)
    {
        status = 429;
    }
    if !(400..600).contains(&status) {
        status = 502;
    }
    (status, message)
}

/// Provider API 回绝 Go 套餐的话。
fn go_refused(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("go plan doesn") && lower.contains("include api access")
}

/* —————————————— 模型 —————————————— */

/// Provider API 的接口：Chat / Anthropic Messages。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Endpoint {
    Chat,
    Messages,
}

fn endpoints() -> &'static Mutex<HashMap<String, Endpoint>> {
    static ENDPOINTS: OnceLock<Mutex<HashMap<String, Endpoint>>> = OnceLock::new();
    ENDPOINTS.get_or_init(Default::default)
}

fn endpoint_for(model: &str) -> Endpoint {
    if let Some(endpoint) = endpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(model)
    {
        return *endpoint;
    }
    if model.starts_with("claude-") || model.starts_with("anthropic/") {
        Endpoint::Messages
    } else {
        Endpoint::Chat
    }
}

/// `supported_endpoints` → 走哪一路（有 chat 走 chat）。
fn endpoint_of(model: &Value) -> Option<Endpoint> {
    let list = model.get("supported_endpoints")?.as_array()?;
    let names: Vec<String> = list
        .iter()
        .filter_map(Value::as_str)
        .map(|item| {
            let item = item.trim_end_matches('/');
            item.strip_prefix("/v1").unwrap_or(item).to_string()
        })
        .collect();
    if names.iter().any(|name| name == "/chat/completions") {
        Some(Endpoint::Chat)
    } else if names.iter().any(|name| name == "/messages") {
        Some(Endpoint::Messages)
    } else {
        None
    }
}

const PREMIUM: &[&str] = &[
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-haiku-4-5-20251001",
    "gpt-5.6-terra",
    "gpt-6-astra",
    "gpt-6.1-sol",
    "gpt-6-sol",
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.3-codex",
    "gpt-5.4-mini",
    "google/gemini-3.5-flash",
    "google/gemini-3.1-flash-lite",
    "sakana/fugu-ultra",
    "meta/muse-spark-1.1",
];
const GOAT_ONLY: &[&str] = &[
    "meta/muse-spark-1.2",
    "meta/muse-spark-1.3",
    "xai/grok-4.6",
    "xai/grok-4.7",
    "xiaomi/mimo-v2.6-pro-ultraspeed",
    "google/gemini-3.7-flash",
    "google/gemini-3.8-flash",
    "gpt-5.6-sol",
    "claude-sonnet-5-5",
    "claude-haiku-5-5",
];
const PRO_BLOCKED: &[&str] = &[
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-opus-4-5-20251101",
    "gpt-6-astra",
    "gpt-6.1-sol",
    "sakana/fugu-ultra",
];

/// 这个套餐用不了的模型。
fn blocked(plan: &str, model: &str) -> bool {
    match plan {
        "Go" => PREMIUM.contains(&model) || GOAT_ONLY.contains(&model),
        "GOAT" => PREMIUM.contains(&model),
        "Pro" => PRO_BLOCKED.contains(&model),
        "Provider" => model == "gpt-6.1-sol",
        _ => false,
    }
}

/// 画图、语音等不能对话的模型（Go 列表里去掉）。
fn not_chat(model: &str) -> bool {
    let lower = model.to_ascii_lowercase();
    [
        "image",
        "imagen",
        "imagine",
        "dall-e",
        "flux",
        "seedream",
        "cogview",
        "stable-diffusion",
        "sdxl",
        "wanx",
        "kolors",
        "hidream",
        "embed",
        "-tts",
        "audio",
        "-live",
        "robotics",
        "computer-use",
        "deep-research",
        "transcribe",
        "realtime",
        "moderation",
        "whisper",
        "sora",
    ]
    .iter()
    .any(|word| lower.contains(word))
}

/// `/models` 的答复 → (模型 id, 接口)。
fn parse_models(value: &Value) -> Vec<(String, Option<Endpoint>)> {
    let list = value
        .get("data")
        .and_then(Value::as_array)
        .filter(|list| !list.is_empty())
        .or_else(|| value.get("models").and_then(Value::as_array));
    list.into_iter()
        .flatten()
        .filter_map(|model| {
            let id = text_of(model, "id")?;
            Some((id.to_string(), endpoint_of(model)))
        })
        .collect()
}

fn plan_models(plan: &str, listed: Vec<(String, Option<Endpoint>)>) -> Vec<String> {
    listed
        .into_iter()
        .map(|(id, _)| id)
        .filter(|id| !blocked(plan, id))
        .filter(|id| plan != "Go" || !not_chat(id))
        .collect()
}

const DEFAULT_MODELS: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-5-5",
    "gpt-6-sol",
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-v4-flash",
    "moonshotai/Kimi-K3",
    "zai-org/GLM-5.3",
    "MiniMaxAI/MiniMax-M3",
];

/// Go 套餐的模型和它们的思考档位（拉不到列表时的兜底，也用来把档位对到最近一档）。
const GO_MODELS: &[(&str, &[&str])] = &[
    ("gpt-6-luna", EFF5),
    ("gpt-5.6-luna", EFF5),
    ("deepseek/deepseek-v4-pro", &["high", "max"]),
    ("deepseek/deepseek-v4-flash", &["high", "max"]),
    ("deepseek/deepseek-v4-flash-vision-exp", &["high", "max"]),
    ("deepseek/deepseek-v4-flash-fast", &["low", "high", "max"]),
    ("deepseek/deepseek-v4.1-flash", &["low", "high", "max"]),
    ("deepseek/deepseek-v4.1-flash-fast", &["low", "high", "max"]),
    ("moonshotai/Kimi-K3", &["low", "high", "max"]),
    ("moonshotai/Kimi-K2.7-Code", &[]),
    ("moonshotai/Kimi-K2.6", &[]),
    ("z-ai/glm-5.3-flash", &["low", "high", "max"]),
    ("zai-org/GLM-5.3", &["low", "high", "max"]),
    ("zai-org/GLM-5.2", &["high", "max"]),
    ("MiniMaxAI/MiniMax-M3", &["low", "medium", "high"]),
    ("MiniMaxAI/MiniMax-M2.7", &[]),
    ("xiaomi/mimo-v2.6-pro", &[]),
    ("xiaomi/mimo-v2.6-flash", &[]),
    ("Qwen/Qwen3.8-Max", &["low", "medium", "xhigh"]),
    ("Qwen/Qwen3.8-Flash", &["low", "medium", "xhigh"]),
    ("Qwen/Qwen3.7-Plus", &[]),
    ("stepfun/Step-5-Preview", &["low", "medium", "high"]),
    ("tencent/hy4-preview", &["low", "medium", "high"]),
    ("google/gemini-3.6-flash", &["low", "medium", "high"]),
    ("google/gemini-3.5-flash-lite", &["low", "medium", "high"]),
    ("xai/grok-4.5", &["low", "medium", "high"]),
];
const EFF5: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const EFFORT_ORDER: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// 把请求的档位对到模型有的最近一档（平局取高的）；模型不在表里原样给，没有档位的模型不给。
fn fit_effort(model: &str, effort: &str) -> Option<String> {
    let Some((_, levels)) = GO_MODELS.iter().find(|(id, _)| *id == model) else {
        return Some(effort.to_string());
    };
    if levels.is_empty() {
        return None;
    }
    let effort = if effort == "ultra" && !levels.contains(&"ultra") {
        "max"
    } else {
        effort
    };
    let rank = |level: &str| EFFORT_ORDER.iter().position(|item| *item == level);
    let wanted = rank(effort)? as i64;
    levels
        .iter()
        .filter_map(|level| rank(level).map(|position| (position as i64, *level)))
        .min_by_key(|(position, _)| ((position - wanted).abs(), -position))
        .map(|(_, level)| level.to_string())
}

/// 能关思考（`reasoning_effort: "off"`）的模型。
const THINK_OFF: &[&str] = &[
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-v4-flash",
    "deepseek/deepseek-v4-flash-vision-exp",
    "deepseek/deepseek-v4.1-flash",
    "deepseek/deepseek-v4.1-flash-fast",
];

/* —————————————— Provider API —————————————— */

/// 发给 Provider API 的 Chat 请求：思考关闭按模型改成 `off` 或去掉，要求带 usage。
fn provider_chat_body(request: &Value) -> Value {
    let mut body = request.clone();
    body["stream"] = json!(true);
    body["stream_options"] = json!({ "include_usage": true });
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(effort) = reasoning_effort(request)
        && matches!(effort.as_str(), "none" | "off")
        && let Some(object) = body.as_object_mut()
    {
        if THINK_OFF.contains(&model.as_str()) {
            object.insert("reasoning_effort".to_string(), json!("off"));
        } else {
            object.remove("reasoning_effort");
        }
    }
    body
}

async fn provider_chat(
    http: &reqwest::Client,
    key: &str,
    request: &Value,
) -> anyhow::Result<Result<ChatStream, String>> {
    let response = http
        .post(format!("{}/chat/completions", base()))
        .bearer_auth(key)
        .header("x-api-key", key)
        .header("accept", "text/event-stream")
        .json(&provider_chat_body(request))
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Command Code：{error}")))?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(Ok(passthrough(response)));
    }
    let text = response.text().await.unwrap_or_default();
    if go_refused(&text) {
        return Ok(Err(text));
    }
    let (status, message) = failure(status, &text);
    Err(fail(
        status,
        format!("Command Code 返回 {status}：{message}"),
    ))
}

async fn provider_messages(
    http: &reqwest::Client,
    key: &str,
    request: &Value,
    model: &str,
) -> anyhow::Result<Result<ChatStream, String>> {
    let body = editor_kit::anthropic_request(request, true)?;
    let response = http
        .post(format!("{}/messages", base()))
        .bearer_auth(key)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Command Code：{error}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let text = response.text().await.unwrap_or_default();
        if go_refused(&text) {
            return Ok(Err(text));
        }
        let (status, message) = failure(status, &text);
        return Err(fail(
            status,
            format!("Command Code 返回 {status}：{message}"),
        ));
    }
    Ok(Ok(editor_kit::spawn_chat(
        model,
        move |mut out| async move {
            let mut bytes = response.bytes_stream();
            let mut reader = SseReader::default();
            let mut state = AnthropicEvents::default();
            while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        out.fail(fail(502, format!("Command Code 的流断了：{error}")))
                            .await;
                        return;
                    }
                };
                for (_, data) in reader.push(&chunk) {
                    if !messages_event(&mut out, &mut state, &data).await {
                        return;
                    }
                }
            }
            if let Some((_, data)) = reader.finish()
                && !messages_event(&mut out, &mut state, &data).await
            {
                return;
            }
            if !state.ended {
                cut_short(&mut out, |out| state.finish(&mut out.sse)).await;
            }
        },
    )))
}

/// 处理一个 Messages 事件；返回 false 表示该停了。
async fn messages_event(out: &mut Emitter, state: &mut AnthropicEvents, data: &str) -> bool {
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return true;
    };
    match state.push(&mut out.sse, &event) {
        Ok(bytes) => out.send(bytes).await && !state.ended,
        Err(error) => {
            out.fail(error).await;
            false
        }
    }
}

/// 流在结束行之前断了：说过话就补一句说明正常收尾，没说过话就报 502。
async fn cut_short(out: &mut Emitter, finish: impl FnOnce(&mut Emitter) -> Vec<u8>) {
    if out.started() {
        let note = out
            .sse
            .text("\n\n(回复被截断：Command Code 在说完之前关闭了连接，可以让它接着说。)");
        out.send(note).await;
        let bytes = finish(out);
        out.send(bytes).await;
    } else {
        out.fail(fail(502, "Command Code 的回复在说完之前就断了"))
            .await;
    }
}

/* —————————————— Go：/alpha/generate —————————————— */

fn session_id() -> &'static str {
    static SESSION: OnceLock<String> = OnceLock::new();
    SESSION.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// 工作目录名（`x-project-slug`）：Latin-1 以外的字符做百分号编码，空的用 root。
fn project_slug(dir: &str) -> String {
    let name = dir
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default();
    if name.is_empty() {
        return "root".to_string();
    }
    name.chars()
        .map(|ch| {
            if (ch as u32) < 0x100 {
                ch.to_string()
            } else {
                let mut buffer = [0_u8; 4];
                ch.encode_utf8(&mut buffer)
                    .bytes()
                    .map(|byte| format!("%{byte:02X}"))
                    .collect()
            }
        })
        .collect()
}

/// UTC 日期 `YYYY-MM-DD`。
fn today() -> String {
    let days = (now_ms() / 86_400_000) as i64;
    // 公历换算（Howard Hinnant 的 civil_from_days）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Chat 请求 → `/alpha/generate` 的请求体。
fn generate_body(request: &Value, working_dir: &str) -> Value {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let names = editor_kit::tool_names(&messages);
    let mut system = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    for message in &messages {
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => {
                let text = content_text(message);
                if !text.is_empty() {
                    system.push(text);
                }
            }
            "assistant" => {
                let mut parts = Vec::new();
                let reasoning = reasoning_text(message);
                if !reasoning.is_empty() {
                    parts.push(json!({ "type": "reasoning", "text": reasoning }));
                }
                let text = content_text(message);
                if !text.is_empty() {
                    parts.push(json!({ "type": "text", "text": text }));
                }
                for call in message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    parts.push(json!({
                        "type": "tool-call",
                        "toolCallId": call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "toolName": call.pointer("/function/name").and_then(Value::as_str).unwrap_or_default(),
                        "input": arguments_object(call.pointer("/function/arguments")),
                    }));
                }
                if !parts.is_empty() {
                    out.push(json!({ "role": "assistant", "content": parts }));
                }
            }
            "tool" => {
                let id = message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let part = json!({
                    "type": "tool-result",
                    "toolCallId": id,
                    "toolName": names.get(id).map(String::as_str).unwrap_or("unknown"),
                    "output": { "type": "text", "value": content_text(message) },
                });
                // 连着的 tool 消息合成一条。
                match out.last_mut() {
                    Some(last) if last.get("role").and_then(Value::as_str) == Some("tool") => {
                        if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut)
                        {
                            content.push(part);
                        }
                    }
                    _ => out.push(json!({ "role": "tool", "content": [part] })),
                }
            }
            _ => {
                let mut parts = Vec::new();
                match message.get("content") {
                    Some(Value::String(text)) if !text.is_empty() => {
                        parts.push(json!({ "type": "text", "text": text }));
                    }
                    Some(Value::Array(items)) => {
                        for item in items {
                            match item.get("type").and_then(Value::as_str) {
                                Some("text") => {
                                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                                        parts.push(json!({ "type": "text", "text": text }));
                                    }
                                }
                                Some("image_url") => {
                                    let url = item
                                        .pointer("/image_url/url")
                                        .or_else(|| item.get("image_url"))
                                        .and_then(Value::as_str)
                                        .unwrap_or_default();
                                    let mut part = json!({ "type": "image", "image": url });
                                    if let Some(mime) = url
                                        .strip_prefix("data:")
                                        .and_then(|rest| rest.split([';', ',']).next())
                                        .filter(|mime| !mime.is_empty())
                                    {
                                        part["mimeType"] = json!(mime);
                                    }
                                    parts.push(part);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
                if !parts.is_empty() {
                    out.push(json!({ "role": "user", "content": parts }));
                }
            }
        }
    }
    let tools: Vec<Value> = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            Some(json!({
                "name": function.get("name")?.as_str()?,
                "description": function.get("description").and_then(Value::as_str).unwrap_or_default(),
                "input_schema": function.get("parameters").cloned().unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
            }))
        })
        .collect();
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let max_tokens = request
        .get("max_completion_tokens")
        .or_else(|| request.get("max_tokens"))
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(GO_MAX_TOKENS)
        .min(GO_MAX_OUTPUT);
    let mut params = json!({
        "model": model,
        "messages": out,
        "tools": tools,
        "system": system.join("\n\n"),
        "max_tokens": max_tokens,
        "stream": true,
    });
    if let Some(temperature) = request.get("temperature").and_then(Value::as_f64) {
        params["temperature"] = json!(temperature);
    }
    if let Some(effort) = reasoning_effort(request)
        .filter(|effort| !matches!(effort.as_str(), "none" | "off"))
        .and_then(|effort| fit_effort(model, &effort))
    {
        params["reasoning_effort"] = json!(effort);
    }
    json!({
        "config": {
            "workingDir": working_dir,
            "date": today(),
            "environment": platform(),
            "structure": [],
            "isGitRepo": false,
            "currentBranch": "",
            "mainBranch": "",
            "gitStatus": "",
            "recentCommits": [],
        },
        "memory": null,
        "taste": null,
        "skills": null,
        "permissionMode": "standard",
        "params": params,
    })
}

/// `/alpha/generate` 逐行事件 → Chat SSE 的状态。
#[derive(Debug, Default)]
struct GenerateEvents {
    cache_write: u64,
    finished: bool,
}

impl GenerateEvents {
    /// 处理一行；上游报错时返回带状态码的错误。
    fn push(&mut self, out: &mut Emitter, line: &str) -> anyhow::Result<Vec<u8>> {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return Ok(Vec::new());
        };
        let sse = &mut out.sse;
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "text-delta" => Ok(sse.text(
                event
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )),
            "reasoning-delta" => Ok(sse.reasoning(
                event
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )),
            "tool-call" => {
                if event.get("providerExecuted").and_then(Value::as_bool) == Some(true) {
                    return Ok(Vec::new());
                }
                let id = text_of(&event, "toolCallId")
                    .map(str::to_string)
                    .unwrap_or_else(editor_kit::new_call_id);
                let name = text_of(&event, "toolName").unwrap_or_default();
                let arguments = match event.get("input").or_else(|| event.get("args")) {
                    Some(Value::String(text)) => text.clone(),
                    Some(value) if !value.is_null() => value.to_string(),
                    _ => "{}".to_string(),
                };
                Ok(sse.tool_call(&id, name, &arguments))
            }
            "cache-write-tokens" => {
                if let Some(value) = event.get("cacheWriteTokens").and_then(Value::as_u64) {
                    self.cache_write = self.cache_write.max(value);
                }
                Ok(Vec::new())
            }
            "finish" => {
                self.finished = true;
                let reason = match event.get("finishReason").and_then(Value::as_str) {
                    Some("tool-calls") => Some("tool_calls"),
                    Some("length") => Some("length"),
                    Some("content-filter") => Some("content_filter"),
                    _ => None,
                };
                let usage = event.get("totalUsage").map(|usage| {
                    let get =
                        |pointer: &str| usage.pointer(pointer).and_then(Value::as_u64).unwrap_or(0);
                    Usage {
                        prompt_tokens: get("/inputTokens"),
                        completion_tokens: get("/outputTokens"),
                        cached_tokens: get("/inputTokenDetails/cacheReadTokens"),
                        reasoning_tokens: get("/outputTokenDetails/reasoningTokens"),
                    }
                });
                Ok(sse.finish(reason, usage))
            }
            "abort" => {
                self.finished = true;
                Ok(sse.finish(Some("stop"), None))
            }
            "error" => {
                let (status, message) = generate_error(event.get("error").unwrap_or(&Value::Null));
                Err(fail(status, format!("Command Code：{message}")))
            }
            _ => Ok(Vec::new()),
        }
    }
}

/// error 行 → (状态码, 说明)。`message` 可能是 `"429 {json}"`。
fn generate_error(error: &Value) -> (u16, String) {
    let (message, status) = match error {
        Value::String(text) => (text.clone(), None),
        other => (
            other
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("上游返回错误")
                .to_string(),
            other
                .get("statusCode")
                .and_then(Value::as_u64)
                .map(|code| code as u16),
        ),
    };
    if let Some((head, rest)) = message.split_once(' ')
        && let Ok(code) = head.parse::<u16>()
        && (100..600).contains(&code)
    {
        return failure(code, rest);
    }
    failure(status.unwrap_or(500), &message)
}

async fn generate(
    http: &reqwest::Client,
    key: &str,
    request: &Value,
    model: &str,
) -> anyhow::Result<ChatStream> {
    let working_dir = editor_kit::home().to_string_lossy().to_string();
    let body = generate_body(request, &working_dir);
    let response = http
        .post(format!("{API}/alpha/generate"))
        .bearer_auth(key)
        .header("content-type", "application/json")
        .header("user-agent", "cli")
        .header("x-command-code-version", CLI_VERSION)
        .header("x-cli-environment", "production")
        .header("x-project-slug", project_slug(&working_dir))
        .header("x-taste-learning", "false")
        .header("x-session-id", session_id())
        .json(&body)
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Command Code：{error}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let text = response.text().await.unwrap_or_default();
        let (status, message) = failure(status, &text);
        return Err(fail(
            status,
            format!("Command Code 返回 {status}：{message}"),
        ));
    }
    Ok(editor_kit::spawn_chat(model, move |mut out| async move {
        let mut bytes = response.bytes_stream();
        let mut lines = LineReader::default();
        let mut state = GenerateEvents::default();
        while let Some(chunk) = bytes.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    out.fail(fail(502, format!("Command Code 的流断了：{error}")))
                        .await;
                    return;
                }
            };
            for line in lines.push(&chunk) {
                if !generate_line(&mut out, &mut state, &line).await {
                    return;
                }
            }
        }
        if let Some(line) = lines.finish()
            && !generate_line(&mut out, &mut state, &line).await
        {
            return;
        }
        if !state.finished {
            cut_short(&mut out, |out| out.sse.finish(None, None)).await;
        }
    }))
}

async fn generate_line(out: &mut Emitter, state: &mut GenerateEvents, line: &str) -> bool {
    match state.push(out, line) {
        Ok(bytes) => out.send(bytes).await && !state.finished,
        Err(error) => {
            out.fail(error).await;
            false
        }
    }
}

/* —————————————— Subscription —————————————— */

#[async_trait::async_trait]
impl Subscription for CommandCode {
    fn id(&self) -> &'static str {
        "commandcode"
    }

    fn name(&self) -> &'static str {
        "Command Code"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要 Command Code 付费套餐（Pro / GOAT / Max / Ultra / Go / Teams Pro），可用 commandcode login 登录后导入，或粘贴 API Key。"
    }

    fn detect_local(&self) -> Option<String> {
        let value = editor_kit::read_json_file(&cli_auth_path()).ok()?;
        parse_cli_auth(&value).ok().map(|(_, user)| user)
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser => {
                let listener = editor_kit::listen_first(&[0]).await?;
                let port = listener.local_addr()?.port();
                let state = editor_kit::b64url(&editor_kit::random_bytes(24));
                let callback = format!("http://127.0.0.1:{port}/callback");
                let url = format!(
                    "{STUDIO}/studio/auth/cli?{}",
                    query(&[
                        ("callback", &callback),
                        ("state", &state),
                        ("mode", "redirect")
                    ])
                );
                let session = editor_kit::park(self.id(), BrowserLogin { listener, state });
                Ok(LoginStart {
                    open_url: Some(url),
                    message: "请在浏览器里登录 Command Code 并批准这台电脑的 key。".to_string(),
                    state: json!({ "method": "browser", "session": session }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::ImportLocal => {
                let (key, user) = parse_cli_auth(&editor_kit::read_json_file(&cli_auth_path())?)?;
                Ok(LoginStart {
                    message: "已读取 Command Code CLI 的登录。".to_string(),
                    state: json!({ "method": "cli", "apiKey": key, "userName": user }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let key = input.trim();
                if key.is_empty() {
                    bail!("请粘贴 Command Code 的 API Key（commandcode.ai/settings/keys）");
                }
                Ok(LoginStart {
                    message: "正在校验 API Key。".to_string(),
                    state: json!({ "method": "paste", "apiKey": key }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::DeviceCode => bail!("Command Code 不支持设备码登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        match text_of(state, "method").unwrap_or_default() {
            "browser" => {
                let session = text_of(state, "session").unwrap_or_default();
                let login = editor_kit::unpark::<BrowserLogin>(self.id(), session)
                    .ok_or_else(|| anyhow::anyhow!("这次登录已失效，请重新开始"))?;
                let (key, user) = wait_browser(login).await?;
                finish_with_key(http, &key, "browser", &user).await
            }
            "cli" => {
                let key = text_of(state, "apiKey").unwrap_or_default();
                let user = text_of(state, "userName").unwrap_or_default();
                finish_with_key(http, key, "cli", user).await
            }
            "paste" => {
                let key = text_of(state, "apiKey").unwrap_or_default();
                finish_with_key(http, key, "paste", "").await
            }
            other => bail!("不认识的登录状态：{other}"),
        }
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        let mut updated = credentials.clone();
        let mut changed = false;
        // 从 CLI 导入的：CLI 重新登录后换了 key，跟着换。
        if text_of(credentials, "source") == Some("cli")
            && let Ok(value) = editor_kit::read_json_file(&cli_auth_path())
            && let Ok((key, _)) = parse_cli_auth(&value)
            && Some(key.as_str()) != text_of(credentials, "apiKey")
        {
            updated["apiKey"] = json!(key);
            updated["planCheckedAt"] = json!(0);
            changed = true;
        }
        let checked = editor_kit::millis_of(&updated, "planCheckedAt");
        if checked + PLAN_TTL_MS <= now_ms() {
            let key = api_key(&updated)?;
            if let Some(plan) = read_plan(http, &key).await {
                updated["plan"] = json!(plan);
            }
            // 读失败也记下时间，别每个请求都等这个慢接口。
            updated["planCheckedAt"] = json!(now_ms());
            changed = true;
        }
        Ok(changed.then_some(updated))
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let key = api_key(credentials)?;
        let plan = plan_of(credentials);
        let go = plan == "Go";
        let mut request = http
            .get(format!("{}/models", base()))
            .header("accept", "application/json")
            .timeout(Duration::from_secs(15));
        // Go 的 key 没有 Provider API：不带 key 也能列。
        if go {
            request = request.header("user-agent", "node");
        } else {
            request = request.bearer_auth(&key).header("x-api-key", &key);
        }
        let listed = match request.send().await {
            Ok(response) if response.status().is_success() => response
                .json::<Value>()
                .await
                .map(|value| parse_models(&value))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if listed.is_empty() {
            return Ok(if go {
                GO_MODELS.iter().map(|(id, _)| id.to_string()).collect()
            } else {
                DEFAULT_MODELS.iter().map(|id| id.to_string()).collect()
            });
        }
        {
            let mut map = endpoints()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (id, endpoint) in &listed {
                if let Some(endpoint) = endpoint {
                    map.insert(id.clone(), *endpoint);
                }
            }
        }
        Ok(plan_models(&plan, listed))
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let key = api_key(credentials)?;
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if is_go(credentials) {
            return generate(http, &key, &request, &model).await;
        }
        let attempt = match endpoint_for(&model) {
            Endpoint::Messages => provider_messages(http, &key, &request, &model).await?,
            Endpoint::Chat => provider_chat(http, &key, &request).await?,
        };
        match attempt {
            Ok(stream) => Ok(stream),
            // 套餐其实是 Go：改走 CLI 的接口。
            Err(_) => generate(http, &key, &request, &model).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::chat::aggregate;
    use crate::gateway::subscriptions::editor_kit::spawn_chat;

    #[test]
    fn plan_names_follow_prefix_order() {
        assert_eq!(plan_name("individual-goat-v2"), "GOAT");
        assert_eq!(plan_name("individual_ultra"), "Ultra");
        assert_eq!(plan_name("individual-go"), "Go");
        assert_eq!(plan_name("individual-pro-v1-annual"), "Pro");
        assert_eq!(plan_name("enterprise"), "");
        let value = json!({"success":true,"data":{"status":"active","planId":"individual-goat","currentPeriodEnd":"2026-10-28T16:16:42.000Z"}});
        assert_eq!(parse_plan(&value).as_deref(), Some("GOAT"));
        assert_eq!(
            parse_plan(&json!({"success":true,"data":null})).as_deref(),
            Some("No plan")
        );
        assert_eq!(
            parse_plan(
                &json!({"success":true,"data":{"status":"canceled","planId":"individual-pro"}})
            )
            .as_deref(),
            Some("No plan")
        );
        assert_eq!(parse_plan(&json!({"success":false,"error":"db"})), None);
    }

    #[test]
    fn cli_auth_file_parses() {
        let value = json!({"apiKey":" user_abc ","userId":"u1","userName":"octo","keyName":"cli"});
        assert_eq!(
            parse_cli_auth(&value).unwrap(),
            ("user_abc".to_string(), "octo".to_string())
        );
        assert!(parse_cli_auth(&json!({"apiKey":"  "})).is_err());
    }

    #[test]
    fn failures_map_to_statuses() {
        let (status, message) = failure(
            403,
            r#"{"error":{"type":"payment","message":"PREMIUM_CREDITS_EXHAUSTED: Monthly limit hit"}}"#,
        );
        assert_eq!(status, 402);
        assert_eq!(message, "Monthly limit hit");
        let (status, message) = failure(
            403,
            r#"{"error":{"message":"PREMIUM_CREDITS_EXHAUSTED: nope"}}"#,
        );
        assert_eq!((status, message.as_str()), (402, "out of credits: nope"));
        assert_eq!(failure(429, "").1, "Too Many Requests");
        let (status, message) = failure(403, "MODEL_NOT_IN_PLAN: gpt-6-sol");
        assert_eq!(
            (status, message.as_str()),
            (403, "Model not in plan: gpt-6-sol")
        );
        assert_eq!(
            failure(500, r#"{"error":{"message":"boom"}}"#),
            (500, "boom".to_string())
        );
        assert_eq!(failure(500, "RATE_LIMITED: slow").0, 429);
        assert_eq!(failure(200, "weird").0, 502);
        assert_eq!(
            generate_error(
                &json!({"message":"429 {\"error\":{\"message\":\"RATE_LIMITED: slow down\"}}"})
            ),
            (429, "RATE_LIMITED: slow down".to_string())
        );
        assert_eq!(
            generate_error(&json!({"message":"Unauthorized","statusCode":401})).0,
            401
        );
        assert!(go_refused(
            r#"{"error":{"type":"forbidden","message":"Your Go plan doesn't include API access. Upgrade"}}"#
        ));
    }

    #[test]
    fn models_follow_plan_and_endpoints() {
        let value = json!({"object":"list","data":[
            {"id":"claude-sonnet-5","supported_endpoints":["/messages"]},
            {"id":"gpt-6-sol","supported_endpoints":["/chat/completions","/responses"]},
            {"id":"deepseek/deepseek-v4-pro","supported_endpoints":["/v1/chat/completions/"]},
            {"id":"vendor/image-gen"},
        ]});
        let listed = parse_models(&value);
        assert_eq!(listed[0].1, Some(Endpoint::Messages));
        assert_eq!(listed[2].1, Some(Endpoint::Chat));
        assert_eq!(listed[3].1, None);
        assert_eq!(
            plan_models("Go", listed.clone()),
            vec!["deepseek/deepseek-v4-pro".to_string()]
        );
        assert_eq!(plan_models("Max", listed.clone()).len(), 4);
        assert_eq!(plan_models("GOAT", listed).len(), 2);
        assert_eq!(endpoint_for("claude-opus-9"), Endpoint::Messages);
        assert_eq!(endpoint_for("kimi"), Endpoint::Chat);
    }

    #[test]
    fn efforts_fit_nearest_level() {
        assert_eq!(
            fit_effort("deepseek/deepseek-v4-pro", "medium").as_deref(),
            Some("high")
        );
        assert_eq!(
            fit_effort("deepseek/deepseek-v4-pro", "ultra").as_deref(),
            Some("max")
        );
        // medium 与 low / high 等距时取高的。
        assert_eq!(
            fit_effort("moonshotai/Kimi-K3", "medium").as_deref(),
            Some("high")
        );
        assert_eq!(fit_effort("moonshotai/Kimi-K2.6", "high"), None);
        assert_eq!(fit_effort("unknown/model", "low").as_deref(), Some("low"));
    }

    #[test]
    fn provider_body_turns_thinking_off() {
        let body = provider_chat_body(
            &json!({"model":"deepseek/deepseek-v4-pro","reasoning_effort":"none","messages":[]}),
        );
        assert_eq!(body["reasoning_effort"], "off");
        assert_eq!(body["stream_options"]["include_usage"], true);
        let body = provider_chat_body(
            &json!({"model":"moonshotai/Kimi-K3","reasoning_effort":"off","messages":[]}),
        );
        assert!(body.get("reasoning_effort").is_none());
        let body =
            provider_chat_body(&json!({"model":"x","reasoning_effort":"high","messages":[]}));
        assert_eq!(body["reasoning_effort"], "high");
    }

    #[test]
    fn generate_body_maps_messages_and_tools() {
        let request = json!({
            "model": "deepseek/deepseek-v4-pro",
            "reasoning_effort": "low",
            "max_tokens": 999_999,
            "temperature": 0.3,
            "messages": [
                { "role": "system", "content": "sys" },
                { "role": "developer", "content": [{ "type": "text", "text": "dev" }] },
                { "role": "user", "content": [
                    { "type": "text", "text": "look" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
                ] },
                { "role": "assistant", "content": "ok", "reasoning_content": "think", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "read", "arguments": "{\"path\":\"a\"}" } },
                    { "id": "c2", "type": "function", "function": { "name": "list", "arguments": "oops" } },
                ] },
                { "role": "tool", "tool_call_id": "c1", "content": "A" },
                { "role": "tool", "tool_call_id": "c2", "content": [{ "type": "text", "text": "B" }] },
                { "role": "tool", "tool_call_id": "zz", "content": "C" },
            ],
            "tools": [
                { "type": "function", "function": { "name": "read", "description": "Read", "parameters": { "type": "object", "properties": { "path": { "type": "string" } } } } },
                { "type": "function", "function": { "name": "list" } },
            ],
        });
        let body = generate_body(&request, "/Users/me/项目");
        let params = &body["params"];
        assert_eq!(params["system"], "sys\n\ndev");
        assert_eq!(params["max_tokens"], 200_000);
        assert_eq!(params["reasoning_effort"], "high");
        assert_eq!(params["temperature"], 0.3);
        assert_eq!(params["stream"], true);
        let messages = params["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["content"][1]["mimeType"], "image/png");
        assert_eq!(messages[1]["content"][0]["type"], "reasoning");
        assert_eq!(messages[1]["content"][2]["input"]["path"], "a");
        assert_eq!(messages[1]["content"][3]["input"], json!({}));
        let results = messages[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[1]["toolName"], "list");
        assert_eq!(results[1]["output"]["value"], "B");
        assert_eq!(results[2]["toolName"], "unknown");
        assert_eq!(params["tools"][1]["input_schema"]["type"], "object");
        assert_eq!(params["tools"][1]["description"], "");
        assert_eq!(body["config"]["workingDir"], "/Users/me/项目");
        assert_eq!(project_slug("/Users/me/项目"), "%E9%A1%B9%E7%9B%AE");
        assert_eq!(project_slug("/x/café"), "café");
        assert_eq!(project_slug("/"), "root");
        assert_eq!(today().len(), 10);
    }

    async fn run_lines(lines: &'static [&'static str]) -> Vec<anyhow::Result<Vec<u8>>> {
        let stream = spawn_chat("m", move |mut out| async move {
            let mut state = GenerateEvents::default();
            for line in lines {
                if !generate_line(&mut out, &mut state, line).await {
                    return;
                }
            }
            if !state.finished {
                cut_short(&mut out, |out| out.sse.finish(None, None)).await;
            }
        });
        stream.collect().await
    }

    #[tokio::test]
    async fn generate_lines_become_chat_sse() {
        let items = run_lines(&[
            r#"{"type":"start"}"#,
            r#"{"type":"reasoning-delta","text":"thinking"}"#,
            r#"{"type":"text-delta","text":"hi"}"#,
            r#"{"type":"tool-call","toolCallId":"c1","toolName":"read","input":{"path":"a"}}"#,
            r#"{"type":"cache-write-tokens","cacheWriteTokens":4}"#,
            r#"{"type":"finish","finishReason":"tool-calls","totalUsage":{"inputTokens":3,"outputTokens":1,"inputTokenDetails":{"cacheReadTokens":2}}}"#,
        ])
        .await;
        let text: String = items
            .into_iter()
            .map(|item| String::from_utf8(item.unwrap()).unwrap())
            .collect();
        let completion = aggregate(&text, "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["content"], "hi");
        assert_eq!(message["reasoning_content"], "thinking");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a\"}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            completion["usage"]["prompt_tokens_details"]["cached_tokens"],
            2
        );
    }

    #[tokio::test]
    async fn generate_errors_keep_status() {
        let items =
            run_lines(&[r#"{"type":"error","error":{"message":"Unauthorized","statusCode":401}}"#])
                .await;
        let error = items[0].as_ref().unwrap_err();
        let status = error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap()
            .status;
        assert_eq!(status, 401);

        // 什么都没说就断了：502。
        let items = run_lines(&[r#"{"type":"start"}"#]).await;
        let error = items[0].as_ref().unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                .unwrap()
                .status,
            502
        );

        // 说过话再断：补一句说明正常收尾。
        let items = run_lines(&[r#"{"type":"text-delta","text":"half"}"#]).await;
        let text: String = items
            .into_iter()
            .map(|item| String::from_utf8(item.unwrap()).unwrap())
            .collect();
        assert!(text.contains("回复被截断"));
        assert!(text.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn callback_outcomes() {
        let mut form = std::collections::BTreeMap::new();
        form.insert("state".to_string(), "s".to_string());
        form.insert("apiKey".to_string(), "k".to_string());
        form.insert("userName".to_string(), "octo".to_string());
        assert_eq!(
            callback_outcome(&form, "s"),
            Ok(("k".to_string(), "octo".to_string()))
        );
        assert!(callback_outcome(&form, "t").is_err());
        form.insert("error".to_string(), "access_denied".to_string());
        assert!(callback_outcome(&form, "s").unwrap_err().contains("拒绝"));
        assert_eq!(
            cors_origin(Some("https://staging.commandcode.ai")),
            "https://staging.commandcode.ai"
        );
        assert_eq!(
            cors_origin(Some("https://evil.example")),
            "https://commandcode.ai"
        );
    }
}
