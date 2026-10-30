//! WorkBuddy（国内版，copilot.tencent.com）与 WorkBuddy AI（国际版，www.workbuddy.ai）。
//!
//! - 登录：浏览器登录（申请一个 state，打开官方登录页，轮询 token 与账号信息）、导入本机 WorkBuddy
//!   桌面端的登录（新版把 token 加密保存，导入不了时提示改用浏览器登录）、粘贴 API Key。
//!   refresh token 会轮换，新的必须写回；导入本机登录的账号不替桌面端刷新（会把桌面端挤下线），
//!   只重读文件。
//! - 聊天：`/v2/chat/completions` 基本是 OpenAI Chat，只收流式、请求体 gzip。几处要改写：
//!   工具名只能是 `[A-Za-z0-9_-]`（回来时映射回原名）、schema 根部要是 object、历史推理放
//!   `reasoning` 字段。上游会拒绝带第三方客户端系统提示的请求，所以客户端的系统提示放进第一条
//!   user 消息，system 用默认的一句。
//! - 请求头要像官方 CLI：版本号、`X-Conversation-ID`（同一会话不变）、`X-Conversation-Request-ID`
//!   （同一次用户提问的整轮工具循环不变，上游按它计请求次数）。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

use crate::gateway::subscriptions::b_shared::{
    self, ChatRelay, OpenAiChatSse, Sse, content_text, decode_stream, gzip_stored, object_root,
    sha256_hex,
};
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const APP_VERSION: &str = "5.9.3";
const CLI_VERSION: &str = "2.165.0";
/// 只用于打开登录页。
const LOGIN_PAGE_VERSION: &str = "2.0.0";
const DEFAULT_SYSTEM: &str = "You are a helpful assistant.";
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;

pub struct WorkBuddy {
    id: &'static str,
    name: &'static str,
    endpoint: &'static str,
    platform: &'static str,
    auth_file: &'static str,
    app_name: &'static str,
    default_models: &'static [&'static str],
    note: &'static str,
}

pub static WORKBUDDY: WorkBuddy = WorkBuddy {
    id: "workbuddy",
    name: "WorkBuddy",
    endpoint: "https://copilot.tencent.com",
    platform: "workbuddy",
    auth_file: "workbuddy-desktop.info",
    app_name: "WorkBuddy",
    default_models: &[
        "hy3",
        "hy3-x",
        "deepseek-v4.1-flash",
        "deepseek-v4-pro",
        "glm-5.3",
        "glm-5.2",
        "kimi-k3-1",
        "kimi-k2.7",
        "minimax-m3",
    ],
    note: "需要 WorkBuddy 账号且有有效套餐（国内版，copilot.tencent.com；免费模型也要求有套餐）；在浏览器里登录，或粘贴 API Key。",
};

pub static WORKBUDDY_AI: WorkBuddy = WorkBuddy {
    id: "workbuddy-ai",
    name: "WorkBuddy AI",
    endpoint: "https://www.workbuddy.ai",
    platform: "workbuddy-ai",
    auth_file: "workbuddy-desktop-ai.info",
    app_name: "workbuddy-ai",
    default_models: &[
        "default-model",
        "balanced-model",
        "primary-model",
        "gpt-5.5",
        "gpt-5.3-codex",
        "gemini-3.1-pro",
        "glm-5.3",
        "kimi-k3",
        "minimax-m3",
    ],
    note: "需要 WorkBuddy AI 国际版账号且有有效套餐（www.workbuddy.ai）；在浏览器里登录，或粘贴 API Key。",
};

// ---------------------------------------------------------------------------
// 请求头
// ---------------------------------------------------------------------------

fn stainless_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "MacOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn stainless_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "x32",
        other => other,
    }
}

fn api_key_of(credentials: &Value) -> Option<&str> {
    credentials
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
}

fn get<'a>(credentials: &'a Value, key: &str) -> &'a str {
    credentials
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// 每个进程一份随机盐：会话 / 轮次 id 只要求同一会话稳定，不需要跨进程一致。
fn salt() -> &'static str {
    static SALT: OnceLock<String> = OnceLock::new();
    SALT.get_or_init(|| b_shared::random_hex(16))
}

fn id_for(kind: &str, parts: &str) -> String {
    sha256_hex(format!("{}|{kind}|{parts}", salt()).as_bytes())[..32].to_string()
}

/// 消息的签名：文本直接取，非文本片段取哈希。
fn signature(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|part| match part.get("text").and_then(Value::as_str) {
                Some(text) => text.to_string(),
                None => format!(
                    "[{}:{}]",
                    part.get("type").and_then(Value::as_str).unwrap_or("part"),
                    &sha256_hex(part.to_string().as_bytes())[..8]
                ),
            })
            .collect(),
        _ => String::new(),
    }
}

/// (会话 id, 轮次 id)。会话按第一条 user 消息；轮次按最后一条用户亲口说的 user 消息
/// （前一条不是工具结果，也不是带工具调用的 assistant）。都按账号区分。
fn conversation_ids(request: &Value, who: &str) -> (Option<String>, String) {
    let messages: Vec<&Value> = request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|message| {
            !matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer")
            )
        })
        .collect();
    let first = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"));
    let Some(first) = first else {
        return (None, b_shared::random_hex(16));
    };
    let base = format!("account:{who}|first:{}", signature(first));
    let conversation = id_for("conversation", &base);
    let turn = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(index, message)| {
            if message.get("role").and_then(Value::as_str) != Some("user") {
                return false;
            }
            let previous = index.checked_sub(1).map(|at| messages[at]);
            !previous.is_some_and(|previous| {
                previous.get("role").and_then(Value::as_str) == Some("tool")
                    || previous
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|calls| !calls.is_empty())
            })
        })
        .map(|(index, message)| id_for("turn", &format!("{base}|u{index}:{}", signature(message))))
        .unwrap_or_else(|| b_shared::random_hex(16));
    (Some(conversation), turn)
}

impl WorkBuddy {
    fn user_agent(&self) -> String {
        format!(
            "WorkBuddy/{APP_VERSION} {}/{APP_VERSION} CLI/{CLI_VERSION}",
            self.app_name
        )
    }

    fn host(&self) -> &'static str {
        self.endpoint.trim_start_matches("https://")
    }

    /// 账号与客户端的那组头（聊天、模型配置都带）。
    fn sign_headers(&self, credentials: &Value) -> anyhow::Result<Vec<(&'static str, String)>> {
        let mut headers = Vec::new();
        if let Some(key) = api_key_of(credentials) {
            let tail: String = key
                .chars()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            headers.push(("authorization", format!("Bearer {key}")));
            headers.push(("x-api-key", key.to_string()));
            headers.push(("x-user-id", format!("anonymous_{tail}")));
        } else {
            let access = get(credentials, "access");
            if access.is_empty() {
                return Err(fail(401, format!("{} 凭据缺失，请重新登录", self.name)));
            }
            headers.push(("authorization", format!("Bearer {access}")));
            headers.push(("x-user-id", get(credentials, "uid").to_string()));
            let domain = get(credentials, "domain");
            headers.push((
                "x-domain",
                if domain.is_empty() {
                    self.host().to_string()
                } else {
                    domain.to_string()
                },
            ));
            let enterprise = get(credentials, "enterprise_id");
            if !enterprise.is_empty() {
                headers.push(("x-enterprise-id", enterprise.to_string()));
                headers.push(("x-tenant-id", enterprise.to_string()));
            }
        }
        headers.push(("x-product", "SaaS".into()));
        headers.push(("x-ide-type", "WorkBuddy".into()));
        headers.push(("user-agent", self.user_agent()));
        headers.push(("x-requested-with", "XMLHttpRequest".into()));
        headers.push(("accept", "application/json".into()));
        Ok(headers)
    }

    fn chat_headers(
        &self,
        credentials: &Value,
        request: &Value,
    ) -> anyhow::Result<Vec<(&'static str, String)>> {
        let mut headers = self.sign_headers(credentials)?;
        let who = format!(
            "{}|{}|{}",
            get(credentials, "uid"),
            get(credentials, "enterprise_id"),
            api_key_of(credentials).unwrap_or_default()
        );
        let (conversation, turn) = conversation_ids(request, &who);
        let message_id = b_shared::random_hex(16);
        headers.extend([
            ("x-agent-intent", "craft".to_string()),
            ("x-agent-purpose", "conversation".to_string()),
            ("x-ide-name", "WorkBuddy".to_string()),
            ("x-ide-version", APP_VERSION.to_string()),
            ("x-agent-type", "main".to_string()),
            ("x-agent-runtime", "local-agent".to_string()),
            ("x-stainless-retry-count", "0".to_string()),
            ("x-stainless-timeout", "1200".to_string()),
            ("x-stainless-lang", "js".to_string()),
            ("x-stainless-package-version", "6.25.0".to_string()),
            ("x-stainless-os", stainless_os().to_string()),
            ("x-stainless-arch", stainless_arch().to_string()),
            ("x-stainless-runtime", "node".to_string()),
            ("x-stainless-runtime-version", "v22.21.1".to_string()),
            ("x-conversation-message-id", message_id.clone()),
            ("x-request-id", message_id),
            ("x-conversation-request-id", turn),
            ("content-type", "application/json".to_string()),
            ("content-encoding", "gzip".to_string()),
        ]);
        if let Some(conversation) = conversation {
            headers.push(("x-conversation-id", conversation));
        }
        Ok(headers)
    }
}

// ---------------------------------------------------------------------------
// 账号接口（{code, msg, data} 信封）
// ---------------------------------------------------------------------------

/// 信封里的错误码（可能在好几个位置）。
fn error_code(value: &Value) -> String {
    for pointer in [
        "/error/internal_code",
        "/error/data/code",
        "/error/code",
        "/code",
        "/data/code",
    ] {
        match value.pointer(pointer) {
            Some(Value::Number(number)) if number.as_u64() != Some(0) => return number.to_string(),
            Some(Value::String(text)) if !text.is_empty() && text != "0" => return text.clone(),
            _ => {}
        }
    }
    // 「12345:…」形式的错误信息。
    let message =
        b_shared::str_at(value, &["/error/message", "msg", "message"]).unwrap_or_default();
    let digits: String = message.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() == 5 && message[digits.len()..].starts_with(':') {
        return digits;
    }
    String::new()
}

fn envelope_message(value: &Value) -> String {
    b_shared::str_at(value, &["msg", "message", "/error/message"])
        .unwrap_or("未知错误")
        .to_string()
}

async fn call(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&Value>,
) -> anyhow::Result<(u16, Value)> {
    let mut request = http
        .request(method, url)
        .header("accept", "application/json")
        .timeout(Duration::from_secs(20));
    if let Some(body) = body {
        request = request.json(body);
    }
    for (key, value) in headers {
        request = request.header(*key, value);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// 登录 / 刷新答复里的 token 字段 → 凭据片段。
fn token_fields(data: &Value) -> Value {
    let now = oauth::now_ms();
    let at = |at_key: &str, in_key: &str| -> Option<u64> {
        b_shared::u64_at(data, &[at_key])
            .filter(|at| *at > 0)
            .map(b_shared::to_ms)
            .or_else(|| b_shared::u64_at(data, &[in_key]).map(|seconds| now + seconds * 1000))
    };
    json!({
        "access": b_shared::str_at(data, &["accessToken"]),
        "refresh": b_shared::str_at(data, &["refreshToken"]),
        "expires_ms": at("expiresAt", "expiresIn"),
        "refresh_expires_ms": at("refreshExpiresAt", "refreshExpiresIn"),
        "domain": b_shared::str_at(data, &["domain"]),
    })
}

/// 本机桌面端登录文件的位置。
fn local_auth_path(file: &str) -> std::path::PathBuf {
    let home = b_shared::home();
    let base = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        home.join("AppData").join("Local")
    } else {
        home.join(".local").join("share")
    };
    base.join("CodeBuddyExtension")
        .join("Data")
        .join("Public")
        .join("auth")
        .join(file)
}

/// 解析桌面端的登录文件 → 凭据和账号名。
fn parse_local_auth(value: &Value) -> anyhow::Result<(Value, String)> {
    let auth = value.get("auth").context("登录文件里没有 auth")?;
    if auth
        .get("accessToken")
        .is_some_and(|token| token.get("$wbEncrypted").is_some())
    {
        anyhow::bail!("WorkBuddy 新版把登录加密保存了，无法导入，请改用浏览器登录");
    }
    let access = b_shared::str_at(auth, &["accessToken"])
        .context("登录文件里没有 token，请先在 WorkBuddy 里登录")?;
    let account = value.get("account").cloned().unwrap_or(Value::Null);
    let uid = b_shared::str_at(&account, &["uid"]).context("登录文件里没有账号 id")?;
    let mut credentials = token_fields(auth);
    credentials["access"] = json!(access);
    credentials["uid"] = json!(uid);
    credentials["enterprise_id"] = json!(b_shared::str_at(&account, &["enterpriseId"]));
    credentials["source"] = json!("local");
    let name = b_shared::str_at(&account, &["nickname", "phoneNumber", "uid"])
        .unwrap_or(uid)
        .to_string();
    Ok((credentials, name))
}

// ---------------------------------------------------------------------------
// 请求体
// ---------------------------------------------------------------------------

/// 工具名只能是 `[A-Za-z0-9_-]`：其余字符换成 `_`，重名加 `_2`、`_3`。返回 原名 → 新名。
fn tool_names(request: &Value) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut used = std::collections::HashSet::new();
    let names = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str));
    for name in names {
        if out.contains_key(name) {
            continue;
        }
        let clean: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let clean = if clean.is_empty() {
            "tool".to_string()
        } else {
            clean
        };
        let mut candidate = clean.clone();
        let mut suffix = 2;
        while !used.insert(candidate.clone()) {
            candidate = format!("{clean}_{suffix}");
            suffix += 1;
        }
        out.insert(name.to_string(), candidate);
    }
    out
}

fn renamed<'a>(names: &'a HashMap<String, String>, name: &'a str) -> &'a str {
    names.get(name).map(String::as_str).unwrap_or(name)
}

/// 工具参数：缺失保持不变，不是对象换成空 object，根部联合类型 / 没写 type 的展开成 object。
fn tool_parameters(parameters: Option<&Value>) -> Option<Value> {
    let parameters = parameters?;
    if !parameters.is_object() {
        return Some(json!({ "type": "object", "properties": {} }));
    }
    let union = ["anyOf", "oneOf", "allOf"]
        .iter()
        .any(|key| parameters.get(*key).is_some());
    if union || parameters.get("type").is_none() {
        return Some(object_root(parameters));
    }
    Some(parameters.clone())
}

/// Chat 请求 → WorkBuddy 的请求体。`names` 是 原名 → 上游名。
fn workbuddy_body(request: &Value, names: &HashMap<String, String>) -> Value {
    let mut body = request.clone();
    let object = body.as_object_mut().expect("请求体是对象");
    object.insert("stream".into(), json!(true));
    object.insert("stream_options".into(), json!({ "include_usage": true }));
    if let Some(max) = object.remove("max_completion_tokens")
        && !object.contains_key("max_tokens")
    {
        object.insert("max_tokens".into(), max);
    }

    // tool_choice：none 时连工具一起去掉；对象写法改成字符串。
    let mut drop_tools = false;
    if let Some(choice) = object.remove("tool_choice") {
        let kind = choice
            .as_str()
            .map(str::to_string)
            .or_else(|| {
                choice
                    .get("type")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        match kind.as_str() {
            "none" => drop_tools = true,
            "auto" | "required" => {
                object.insert("tool_choice".into(), json!(kind));
            }
            "function" => {
                let name = choice.pointer("/function/name").and_then(Value::as_str);
                let value = name
                    .map(|name| renamed(names, name).to_string())
                    .unwrap_or_else(|| "auto".into());
                object.insert("tool_choice".into(), json!(value));
            }
            _ => {}
        }
    }
    if drop_tools {
        object.remove("tools");
        object.remove("functions");
    } else if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools.iter_mut() {
            let Some(function) = tool.get_mut("function").and_then(Value::as_object_mut) else {
                continue;
            };
            if let Some(name) = function.get("name").and_then(Value::as_str) {
                let name = renamed(names, name).to_string();
                function.insert("name".into(), json!(name));
            }
            if let Some(parameters) = tool_parameters(function.get("parameters")) {
                function.insert("parameters".into(), parameters);
            }
        }
    }

    let model = object
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let raw = object
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut instructions = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for message in raw {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
            .to_string();
        match role.as_str() {
            "system" | "developer" => {
                let text = content_text(message.get("content"));
                if !text.trim().is_empty() {
                    instructions.push(text);
                }
            }
            "assistant" => {
                let mut message = message.clone();
                let reasoning = b_shared::str_at(&message, &["reasoning_content", "reasoning"])
                    .map(str::to_string)
                    .or_else(|| {
                        let text: String = message
                            .get("reasoning_details")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|detail| {
                                b_shared::str_at(detail, &["text", "summary"]).map(str::to_string)
                            })
                            .collect();
                        (!text.is_empty()).then_some(text)
                    });
                if let Some(map) = message.as_object_mut() {
                    map.remove("reasoning_content");
                    map.remove("reasoning_details");
                    map.remove("reasoning");
                    if let Some(reasoning) = reasoning {
                        map.insert("reasoning".into(), json!(reasoning));
                    }
                }
                for call in message
                    .get_mut("tool_calls")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        let name = renamed(names, name).to_string();
                        call["function"]["name"] = json!(name);
                    }
                }
                messages.push(message);
            }
            "tool" => {
                let mut message = message.clone();
                if let Some(name) = message.get("name").and_then(Value::as_str) {
                    let name = renamed(names, name).to_string();
                    message["name"] = json!(name);
                }
                messages.push(message);
            }
            _ => {
                // gemini 系模型：工具结果后面紧跟 user 时要隔一条空 assistant。
                if model.starts_with("gemini")
                    && messages
                        .last()
                        .and_then(|last| last.get("role"))
                        .and_then(Value::as_str)
                        == Some("tool")
                {
                    messages.push(json!({ "role": "assistant", "content": "" }));
                }
                messages.push(message.clone());
            }
        }
    }
    if !instructions.is_empty() {
        let prefix = format!(
            "<instructions>\n{}\n</instructions>",
            instructions.join("\n\n")
        );
        match messages
            .iter_mut()
            .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        {
            Some(first) => match first.get_mut("content") {
                Some(Value::Array(parts)) => {
                    parts.insert(
                        0,
                        json!({ "type": "text", "text": format!("{prefix}\n\n") }),
                    );
                }
                Some(content) => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    *content = json!(format!("{prefix}\n\n{text}"));
                }
                None => first["content"] = json!(prefix),
            },
            None => messages.insert(0, json!({ "role": "user", "content": prefix })),
        }
    }
    messages.insert(0, json!({ "role": "system", "content": DEFAULT_SYSTEM }));
    object.insert("messages".into(), json!(messages));
    body
}

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

fn failure(vendor: &str, status: u16, body: &str, www_authenticate: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = error_code(&value);
    let message = b_shared::error_message(body);
    let lower = message.to_ascii_lowercase();
    let (status, label) =
        if code == "14018" || lower.contains("credits exhausted") || message.contains("额度已用尽")
        {
            (429, "额度已用完")
        } else if code == "11140" {
            (403, "账号已被限制，请到官方 App 里申诉")
        } else if code == "11128"
            || lower.contains("unapproved channel")
            || lower.contains("illegal api invocation")
        {
            (400, "上游拒绝了这个请求的来源")
        } else if code == "12153" || lower.contains("invalid_grant") {
            (401, "登录已过期，请重新登录")
        } else if status == 401 && (www_authenticate.contains("invalid_token") || code.is_empty()) {
            (401, "登录已失效，请重新登录")
        } else if status == 429 || lower.contains("frequency limit") {
            (429, "额度用完或请求太频繁")
        } else if (400..600).contains(&status) {
            (status, "请求失败")
        } else {
            (502, "请求失败")
        };
    let code = if code.is_empty() {
        String::new()
    } else {
        format!("[{code}] ")
    };
    fail(
        status,
        format!("{vendor} {label}（{status}）：{code}{message}"),
    )
}

/// `/v3/config` → `cli` agent 能用的模型。
fn parse_config_models(value: &Value) -> Vec<String> {
    let data = value.get("data").unwrap_or(value);
    data.get("agents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|agent| agent.get("name").and_then(Value::as_str) == Some("cli"))
        .and_then(|agent| agent.get("models"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            model
                .as_str()
                .or_else(|| model.get("id").and_then(Value::as_str))
                .map(str::to_string)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Subscription
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl Subscription for WorkBuddy {
    fn id(&self) -> &'static str {
        self.id
    }

    fn name(&self) -> &'static str {
        self.name
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        self.note
    }

    async fn begin_login(
        &self,
        http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser | LoginMethod::DeviceCode => {
                let headers = [
                    ("user-agent", format!("WorkBuddy/{APP_VERSION}")),
                    ("x-no-authorization", "true".to_string()),
                    ("x-no-user-id", "true".to_string()),
                    ("x-no-enterprise-id", "true".to_string()),
                    ("x-no-department-info", "true".to_string()),
                ];
                let (status, value) = call(
                    http,
                    reqwest::Method::POST,
                    &format!(
                        "{}/v2/plugin/auth/state?platform={}",
                        self.endpoint, self.platform
                    ),
                    &headers,
                    Some(&json!({})),
                )
                .await?;
                if !error_code(&value).is_empty() || !(200..300).contains(&status) {
                    anyhow::bail!("{} 无法开始登录：{}", self.name, envelope_message(&value));
                }
                let state =
                    b_shared::str_at(&value, &["/data/state"]).context("没有拿到登录 state")?;
                let page = b_shared::str_at(&value, &["/data/authUrl"])
                    .filter(|url| url.starts_with("https://"))
                    .context("没有拿到登录页地址")?;
                let separator = if page.contains('?') { '&' } else { '?' };
                let url = format!(
                    "{page}{separator}{}",
                    oauth::query(&[
                        ("version", LOGIN_PAGE_VERSION),
                        ("loginSessionId", &b_shared::random_hex(16)),
                    ])
                );
                Ok(LoginStart {
                    open_url: Some(url),
                    user_code: None,
                    message: format!("在浏览器里登录 {}（扫码或账号登录都行）。", self.name),
                    state: json!({ "method": "browser", "state": state }),
                })
            }
            LoginMethod::ImportLocal => {
                let path = local_auth_path(self.auth_file);
                let value = oauth::read_local_json(&path)?;
                let (credentials, account) = parse_local_auth(&value)?;
                Ok(LoginStart {
                    message: format!("已读取本机 {} 的登录。", self.name),
                    state: json!({ "method": "local", "credentials": credentials, "account": account }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let key = input.trim().trim_start_matches("Bearer ").trim();
                if key.is_empty() {
                    anyhow::bail!("请粘贴 {} 的 API Key", self.name);
                }
                Ok(LoginStart {
                    message: "已收到 API Key。".into(),
                    state: json!({ "method": "api_key", "api_key": key }),
                    ..LoginStart::default()
                })
            }
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
            "local" => Ok(LoginDone {
                credentials: state.get("credentials").cloned().unwrap_or(Value::Null),
                account: get(state, "account").to_string(),
            }),
            "api_key" => {
                let key = get(state, "api_key");
                let tail: String = key
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                Ok(LoginDone {
                    credentials: json!({ "api_key": key, "source": "api_key" }),
                    account: format!("API Key …{tail}"),
                })
            }
            _ => {
                let login_state = oauth::encode(get(state, "state"));
                let token_url =
                    format!("{}/v2/plugin/auth/token?state={login_state}", self.endpoint);
                let token = oauth::poll_until(Duration::from_secs(1), || {
                    let http = http.clone();
                    let token_url = token_url.clone();
                    async move {
                        let (status, value) =
                            call(&http, reqwest::Method::GET, &token_url, &[], None).await?;
                        let code = error_code(&value);
                        let data = value.get("data").filter(|data| data.is_object());
                        if code.is_empty()
                            && let Some(data) = data
                            && b_shared::str_at(data, &["accessToken"]).is_some()
                        {
                            return Ok(Some(data.clone()));
                        }
                        if code == "11217" || matches!(status, 200 | 408 | 429) || status >= 500 {
                            return Ok(None);
                        }
                        anyhow::bail!("登录失败：{}", envelope_message(&value))
                    }
                })
                .await?;
                let mut credentials = token_fields(&token);
                credentials["source"] = json!("browser");
                let access = get(&credentials, "access").to_string();
                let domain = get(&credentials, "domain").to_string();
                let domain = if domain.is_empty() {
                    self.host().to_string()
                } else {
                    domain
                };
                let account_url = format!(
                    "{}/v2/plugin/login/account?state={login_state}",
                    self.endpoint
                );
                let who = oauth::poll_until(Duration::from_secs(1), || {
                    let http = http.clone();
                    let account_url = account_url.clone();
                    let headers = vec![
                        ("authorization", format!("Bearer {access}")),
                        ("x-domain", domain.clone()),
                        ("x-no-user-id", "true".to_string()),
                        ("x-no-enterprise-id", "true".to_string()),
                    ];
                    async move {
                        let (status, value) =
                            call(&http, reqwest::Method::GET, &account_url, &headers, None).await?;
                        let code = error_code(&value);
                        if code.is_empty()
                            && let Some(data) = value.get("data").filter(|data| data.is_object())
                        {
                            return Ok(Some(data.clone()));
                        }
                        if code == "12151" || status == 429 || status >= 500 {
                            return Ok(None);
                        }
                        anyhow::bail!("读取账号信息失败：{}", envelope_message(&value))
                    }
                })
                .await?;
                let uid = b_shared::str_at(&who, &["uid"])
                    .unwrap_or_default()
                    .to_string();
                let mut enterprise = b_shared::str_at(&who, &["enterpriseId"]).map(str::to_string);
                let kind = b_shared::str_at(&who, &["type"]).unwrap_or("personal");
                if enterprise.is_none() && kind != "personal" {
                    let headers = vec![
                        ("authorization", format!("Bearer {access}")),
                        ("x-domain", domain.clone()),
                    ];
                    if let Ok((_, list)) = call(
                        http,
                        reqwest::Method::GET,
                        &format!("{}/v2/plugin/accounts", self.endpoint),
                        &headers,
                        None,
                    )
                    .await
                    {
                        let mine: Vec<&Value> = list
                            .pointer("/data/accounts")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter(|item| b_shared::str_at(item, &["uid"]) == Some(uid.as_str()))
                            .collect();
                        let same_type: Vec<&&Value> = mine
                            .iter()
                            .filter(|item| b_shared::str_at(item, &["type"]) == Some(kind))
                            .collect();
                        let last: Vec<&&Value> = mine
                            .iter()
                            .filter(|item| {
                                item.get("lastLogin").and_then(Value::as_bool) == Some(true)
                            })
                            .collect();
                        let pick = if same_type.len() == 1 {
                            Some(*same_type[0])
                        } else if last.len() == 1 {
                            Some(*last[0])
                        } else if mine.len() == 1 {
                            Some(mine[0])
                        } else {
                            None
                        };
                        enterprise = pick
                            .and_then(|item| b_shared::str_at(item, &["enterpriseId"]))
                            .map(str::to_string);
                    }
                }
                credentials["uid"] = json!(uid);
                credentials["enterprise_id"] = json!(enterprise);
                let nickname = b_shared::str_at(&who, &["nickname", "phoneNumber"]).unwrap_or(&uid);
                let account = match b_shared::str_at(&who, &["enterpriseName"]) {
                    Some(team) => format!("{nickname} · {team}"),
                    None => nickname.to_string(),
                };
                Ok(LoginDone {
                    credentials,
                    account,
                })
            }
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
        let expires = credentials.get("expires_ms").and_then(Value::as_u64);
        if get(credentials, "source") == "local" {
            // 不替桌面端刷新（会把桌面端挤下线）：重读文件，跟着桌面端的登录走。
            if let Ok(value) = oauth::read_local_json(&local_auth_path(self.auth_file))
                && let Ok((fresh, _)) = parse_local_auth(&value)
                && get(&fresh, "access") != get(credentials, "access")
            {
                return Ok(Some(fresh));
            }
            if expires.is_some_and(|expires| expires <= oauth::now_ms()) {
                return Err(fail(
                    401,
                    format!(
                        "本机 {} 的登录已过期，请在 {} 里重新登录",
                        self.name, self.name
                    ),
                ));
            }
            return Ok(None);
        }
        if expires.is_none() || !oauth::expiring(credentials, "expires_ms", REFRESH_AHEAD_MS) {
            return Ok(None);
        }
        let refresh = get(credentials, "refresh");
        let refresh_dead = credentials
            .get("refresh_expires_ms")
            .and_then(Value::as_u64)
            .is_some_and(|at| at <= oauth::now_ms());
        if refresh.is_empty() || refresh_dead {
            if expires.is_some_and(|expires| expires > oauth::now_ms()) {
                return Ok(None);
            }
            return Err(fail(401, format!("{} 登录已过期，请重新登录", self.name)));
        }
        let domain = get(credentials, "domain");
        let mut headers = vec![
            ("user-agent", format!("WorkBuddy/{APP_VERSION}")),
            ("x-refresh-token", refresh.to_string()),
            ("x-auth-refresh-source", "plugin".to_string()),
            (
                "x-domain",
                if domain.is_empty() {
                    self.host().to_string()
                } else {
                    domain.to_string()
                },
            ),
        ];
        let enterprise = get(credentials, "enterprise_id");
        if !enterprise.is_empty() {
            headers.push(("x-enterprise-id", enterprise.to_string()));
            headers.push(("x-tenant-id", enterprise.to_string()));
        }
        let result = call(
            http,
            reqwest::Method::POST,
            &format!("{}/v2/plugin/auth/token/refresh", self.endpoint),
            &headers,
            Some(&json!({})),
        )
        .await;
        match result {
            Ok((status, value))
                if (200..300).contains(&status)
                    && error_code(&value).is_empty()
                    && b_shared::str_at(&value, &["/data/accessToken"]).is_some() =>
            {
                let data = value.get("data").cloned().unwrap_or(Value::Null);
                let renewed = token_fields(&data);
                let mut fresh = credentials.clone();
                for key in [
                    "access",
                    "refresh",
                    "expires_ms",
                    "refresh_expires_ms",
                    "domain",
                ] {
                    if !renewed[key].is_null() {
                        fresh[key] = renewed[key].clone();
                    }
                }
                Ok(Some(fresh))
            }
            Ok((status, value))
                if status == 401
                    || error_code(&value) == "12153"
                    || envelope_message(&value).contains("invalid_grant") =>
            {
                Err(fail(401, format!("{} 登录已过期，请重新登录", self.name)))
            }
            // 刷新服务抖动但旧 token 还能用：先接着用。
            _ if expires.is_some_and(|expires| expires > oauth::now_ms()) => Ok(None),
            Ok((status, value)) => Err(fail(
                502,
                format!(
                    "{} 刷新登录失败（{status}）：{}",
                    self.name,
                    envelope_message(&value)
                ),
            )),
            Err(error) => Err(fail(502, format!("{} 刷新登录失败：{error:#}", self.name))),
        }
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let defaults = || {
            self.default_models
                .iter()
                .map(|model| model.to_string())
                .collect()
        };
        // 只有平台签发的 key（ck_ / pt_）才查得到配置。
        if let Some(key) = api_key_of(credentials)
            && !(key.starts_with("ck_") || key.starts_with("pt_"))
        {
            return Ok(defaults());
        }
        let headers = self.sign_headers(credentials)?;
        let models = match call(
            http,
            reqwest::Method::GET,
            &format!("{}/v3/config", self.endpoint),
            &headers,
            None,
        )
        .await
        {
            Ok((status, value)) if (200..300).contains(&status) => parse_config_models(&value),
            _ => Vec::new(),
        };
        Ok(if models.is_empty() {
            defaults()
        } else {
            models
        })
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let names = tool_names(&request);
        let body = workbuddy_body(&request, &names);
        let headers = self.chat_headers(credentials, &request)?;
        let mut builder = http
            .post(format!("{}/v2/chat/completions", self.endpoint))
            .body(gzip_stored(body.to_string().as_bytes()));
        for (key, value) in &headers {
            builder = builder.header(*key, value);
        }
        let response = builder
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 {}：{error}", self.name)))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let www = response
                .headers()
                .get("www-authenticate")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let text = response.text().await.unwrap_or_default();
            return Err(failure(self.name, status, &text, &www));
        }
        // 有的错误（额度用完）以 200 + JSON 回来。
        let is_json = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"));
        if is_json {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(self.name, 502, &text, ""));
        }
        let mut relay = ChatRelay::new(&model);
        relay.rename = names
            .into_iter()
            .filter(|(original, clean)| original != clean)
            .map(|(original, clean)| (clean, original))
            .collect();
        Ok(decode_stream(
            response,
            Sse::new(OpenAiChatSse {
                relay,
                vendor: self.name,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::b_shared::run_decoder;
    use crate::gateway::subscriptions::chat::aggregate;

    fn status(error: &anyhow::Error) -> u16 {
        error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap()
            .status
    }

    fn request() -> Value {
        json!({
            "model": "glm-5.3",
            "max_completion_tokens": 100,
            "tool_choice": { "type": "function", "function": { "name": "fs.read" } },
            "messages": [
                { "role": "developer", "content": "Codex CLI is an open source project led by OpenAI." },
                { "role": "user", "content": "读文件" },
                { "role": "assistant", "content": "", "reasoning_content": "想", "tool_calls": [
                    { "id": "c1", "type": "function", "function": { "name": "fs.read", "arguments": "{}" } }
                ]},
                { "role": "tool", "tool_call_id": "c1", "name": "fs.read", "content": "内容" },
                { "role": "user", "content": [{ "type": "text", "text": "继续" }] }
            ],
            "tools": [
                { "type": "function", "function": { "name": "fs.read", "parameters": { "anyOf": [{ "type": "object", "properties": { "p": {} } }] } } },
                { "type": "function", "function": { "name": "fs_read", "parameters": "bad" } }
            ]
        })
    }

    #[test]
    fn rewrites_body() {
        let names = tool_names(&request());
        assert_eq!(names["fs.read"], "fs_read");
        assert_eq!(names["fs_read"], "fs_read_2");
        let body = workbuddy_body(&request(), &names);
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 100);
        assert!(body.get("max_completion_tokens").is_none());
        assert_eq!(body["tool_choice"], "fs_read");
        assert_eq!(body["tools"][0]["function"]["name"], "fs_read");
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(
            body["tools"][1]["function"]["parameters"],
            json!({ "type": "object", "properties": {} })
        );
        let messages = body["messages"].as_array().unwrap();
        // system 用默认一句，客户端的系统提示挪进第一条 user。
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], DEFAULT_SYSTEM);
        assert!(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .starts_with("<instructions>\nCodex CLI")
        );
        assert!(messages[1]["content"].as_str().unwrap().ends_with("读文件"));
        assert_eq!(messages[2]["reasoning"], "想");
        assert!(messages[2].get("reasoning_content").is_none());
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "fs_read");
        assert_eq!(messages[3]["name"], "fs_read");
        assert_eq!(messages[4]["role"], "user");
        // gemini：工具结果后插一条空 assistant。
        let mut gemini = request();
        gemini["model"] = json!("gemini-3.1-pro");
        let body = workbuddy_body(&gemini, &names);
        assert_eq!(body["messages"][4]["role"], "assistant");
        // tool_choice none 去掉工具。
        let mut none = request();
        none["tool_choice"] = json!("none");
        let body = workbuddy_body(&none, &names);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    #[test]
    fn conversation_ids_are_stable_per_turn() {
        let mut request = request();
        let (conversation, turn) = conversation_ids(&request, "u|e|");
        // 同一轮里追加工具结果：轮次不变。
        request["messages"].as_array_mut().unwrap().push(json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c2", "function": { "name": "x", "arguments": "{}" } }] }));
        request["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "role": "tool", "tool_call_id": "c2", "content": "y" }));
        let (again, same_turn) = conversation_ids(&request, "u|e|");
        assert_eq!(conversation, again);
        assert_eq!(turn, same_turn);
        // 紧跟工具结果的 user（比如工具返回的图片）仍算这一轮。
        request["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "role": "user", "content": "图片" }));
        assert_eq!(conversation_ids(&request, "u|e|").1, turn);
        // 模型答完、用户再说一句：换轮次，会话不变。
        request["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "role": "assistant", "content": "好了" }));
        request["messages"]
            .as_array_mut()
            .unwrap()
            .push(json!({ "role": "user", "content": "再来" }));
        let (again, next_turn) = conversation_ids(&request, "u|e|");
        assert_eq!(conversation, again);
        assert_ne!(turn, next_turn);
        // 换账号：都不同。
        let (other, _) = conversation_ids(&request, "v|e|");
        assert_ne!(conversation, other);
        assert_eq!(conversation.unwrap().len(), 32);
    }

    #[test]
    fn headers_follow_the_cli() {
        let creds = json!({ "access": "t", "uid": "u", "enterprise_id": "ent", "domain": "" });
        let headers = WORKBUDDY_AI.chat_headers(&creds, &request()).unwrap();
        let get = |key: &str| {
            headers
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.clone())
        };
        assert_eq!(
            get("user-agent").unwrap(),
            "WorkBuddy/5.9.3 workbuddy-ai/5.9.3 CLI/2.165.0"
        );
        assert_eq!(get("x-domain").unwrap(), "www.workbuddy.ai");
        assert_eq!(get("x-tenant-id").unwrap(), "ent");
        assert_eq!(get("content-encoding").unwrap(), "gzip");
        assert_eq!(get("x-request-id"), get("x-conversation-message-id"));
        assert!(get("x-conversation-id").is_some());
        let keyed = WORKBUDDY
            .sign_headers(&json!({ "api_key": "ck_12345678abcdefgh" }))
            .unwrap();
        assert!(
            keyed
                .iter()
                .any(|(key, value)| *key == "x-user-id" && value == "anonymous_abcdefgh")
        );
        assert!(!keyed.iter().any(|(key, _)| *key == "x-domain"));
        assert!(WORKBUDDY.sign_headers(&json!({})).is_err());
    }

    #[test]
    fn parses_local_auth_file() {
        let plain = json!({
            "auth": { "accessToken": "a", "refreshToken": "r", "expiresAt": 4_102_444_800_000_u64, "domain": "d" },
            "account": { "uid": "u1", "nickname": "小明", "enterpriseId": "e1" }
        });
        let (credentials, name) = parse_local_auth(&plain).unwrap();
        assert_eq!(name, "小明");
        assert_eq!(credentials["access"], "a");
        assert_eq!(credentials["expires_ms"], 4_102_444_800_000_u64);
        assert_eq!(credentials["enterprise_id"], "e1");
        assert_eq!(credentials["source"], "local");
        let in_seconds = json!({ "auth": { "accessToken": "a", "expiresIn": 3600 }, "account": { "uid": "u1" } });
        let (credentials, name) = parse_local_auth(&in_seconds).unwrap();
        assert_eq!(name, "u1");
        assert!(credentials["expires_ms"].as_u64().unwrap() > oauth::now_ms() + 3_500_000);
        let encrypted = json!({
            "auth": { "accessToken": { "$wbEncrypted": 1, "envelope": "x" } },
            "account": { "uid": "u1" }
        });
        assert!(
            parse_local_auth(&encrypted)
                .unwrap_err()
                .to_string()
                .contains("加密")
        );
    }

    #[test]
    fn maps_errors_and_codes() {
        assert_eq!(
            error_code(&json!({ "error": { "message": "14018:Credits exhausted" } })),
            "14018"
        );
        assert_eq!(error_code(&json!({ "code": 0, "data": {} })), "");
        assert_eq!(
            status(&failure(
                "W",
                400,
                r#"{"error":{"code":"14018","message":"x"}}"#,
                ""
            )),
            429
        );
        assert_eq!(
            status(&failure(
                "W",
                403,
                r#"{"code":11140,"msg":"restricted"}"#,
                ""
            )),
            403
        );
        assert_eq!(
            status(&failure(
                "W",
                400,
                r#"{"error":{"message":"Illegal API invocation from an unapproved channel"}}"#,
                ""
            )),
            400
        );
        assert_eq!(
            status(&failure(
                "W",
                401,
                "",
                "Bearer realm=\"copilot\", error=\"invalid_token\""
            )),
            401
        );
        assert_eq!(
            status(&failure(
                "W",
                429,
                r#"{"error":{"message":"usage exceeds frequency limit"}}"#,
                ""
            )),
            429
        );
    }

    #[test]
    fn parses_config_models() {
        let config = json!({ "code": 0, "data": {
            "agents": [{ "name": "ide", "models": ["x"] }, { "name": "cli", "models": ["glm-5.3", { "id": "hy3" }] }],
            "models": []
        }});
        assert_eq!(parse_config_models(&config), vec!["glm-5.3", "hy3"]);
        assert!(parse_config_models(&json!({})).is_empty());
    }

    #[test]
    fn stream_maps_tool_names_back() {
        let mut relay = ChatRelay::new("glm-5.3");
        relay.rename.insert("fs_read".into(), "fs.read".into());
        let events = [
            json!({ "choices": [{ "delta": { "reasoning_content": "想" } }] }),
            json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "c9", "function": { "name": "fs_read", "arguments": "{}" } }] } }] }),
            json!({ "choices": [{ "delta": {}, "finish_reason": "tool_calls" }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } }),
        ]
        .map(|event| format!("data: {event}\n\n"));
        let mut parts: Vec<&[u8]> = events.iter().map(|event| event.as_bytes()).collect();
        parts.push(b"data: [DONE]\n\n");
        let (text, error) = run_decoder(
            Sse::new(OpenAiChatSse {
                relay,
                vendor: "WorkBuddy",
            }),
            &parts,
        );
        assert!(error.is_none());
        let completion = aggregate(&text, "glm-5.3").unwrap();
        assert_eq!(
            completion["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "fs.read"
        );
        assert_eq!(
            completion["choices"][0]["message"]["reasoning_content"],
            "想"
        );
    }

    #[test]
    fn token_fields_prefer_absolute_times() {
        let fields = token_fields(
            &json!({ "accessToken": "a", "expiresAt": 1_900_000_000, "expiresIn": 5 }),
        );
        assert_eq!(fields["expires_ms"], 1_900_000_000_000_u64);
        let fields = token_fields(&json!({ "accessToken": "a" }));
        assert!(fields["expires_ms"].is_null());
    }
}
