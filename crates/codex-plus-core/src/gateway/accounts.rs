//! 在网关里直接登录订阅账号：ChatGPT（Codex 订阅）、ChatGPT API、GitHub Copilot、SuperGrok。
//!
//! 走与 Codex CLI 相同的浏览器 PKCE 登录：本机 1455 端口接回调，换到的 token 只存在 gateway.json，
//! 与 Codex 自己的登录是两次独立的登录（各自的 refresh token），互不影响。

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::official_login::{GATEWAY_OWNED_MARKER, OFFICIAL_OAUTH_CLIENT_ID};
use crate::settings::{RelayMode, RelayProfile, RelayProtocol};

const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
/// Codex CLI 登记的回调地址，端口固定。
const CALLBACK_PORT: u16 = 1455;
const CALLBACK_PATH: &str = "/auth/callback";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// 回调监听上每条连接最多等这么久收请求（测试里缩短，免得空连接拖慢测试）。
const CALLBACK_READ_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(5)
};

/// 一次登录拿到的账号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatGptAccount {
    pub email: String,
    pub plan: String,
    pub account_id: String,
    /// auth.json 结构，带网关所有的标记。
    pub auth_contents: String,
}

/// PKCE：verifier 用两个随机 UUID 拼成（64 个 [0-9a-f] 字符），challenge 是它的 SHA-256。
pub fn pkce_pair() -> (String, String) {
    let verifier = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn redirect_uri() -> String {
    format!("http://localhost:{CALLBACK_PORT}{CALLBACK_PATH}")
}

/// 浏览器要打开的登录地址。
pub fn chatgpt_authorize_url(challenge: &str, state: &str) -> String {
    let params = [
        ("response_type", "code".to_string()),
        ("client_id", OFFICIAL_OAUTH_CLIENT_ID.to_string()),
        ("redirect_uri", redirect_uri()),
        ("scope", "openid profile email offline_access".to_string()),
        ("code_challenge", challenge.to_string()),
        ("code_challenge_method", "S256".to_string()),
        ("id_token_add_organizations", "true".to_string()),
        ("codex_cli_simplified_flow", "true".to_string()),
        ("state", state.to_string()),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{AUTHORIZE_URL}?{query}")
}

/// 解析回调请求行里的 code / state / error（ChatGPT 的回调路径）。
pub fn parse_callback(request_line: &str) -> Option<Result<(String, String), String>> {
    parse_callback_at(request_line, CALLBACK_PATH)
}

fn parse_callback_at(
    request_line: &str,
    callback_path: &str,
) -> Option<Result<(String, String), String>> {
    callback_params(request_line, callback_path).map(|result| {
        result.map(|params| {
            (
                params.get("code").cloned().unwrap_or_default(),
                params.get("state").cloned().unwrap_or_default(),
            )
        })
    })
}

/// 回调请求行里的全部查询参数；带 error 或缺 code 时返回错误，路径不对返回 None。
fn callback_params(
    request_line: &str,
    callback_path: &str,
) -> Option<Result<BTreeMap<String, String>, String>> {
    callback_query(request_line, callback_path).map(callback_result)
}

/// 回调请求行里的查询参数（不做校验）；路径不对返回 None。
fn callback_query(request_line: &str, callback_path: &str) -> Option<BTreeMap<String, String>> {
    let target = request_line.split_whitespace().nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != callback_path {
        return None;
    }
    let mut params = BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        params.entry(decode(key)).or_insert_with(|| decode(value));
    }
    Some(params)
}

/// 带 error 或缺 code 时返回错误。
fn callback_result(params: BTreeMap<String, String>) -> Result<BTreeMap<String, String>, String> {
    if let Some(error) = params
        .get("error_description")
        .or_else(|| params.get("error"))
        .filter(|error| !error.is_empty())
    {
        return Err(error.clone());
    }
    if params.get("code").is_none_or(|code| code.is_empty()) {
        return Err("回调里没有授权码".to_string());
    }
    Ok(params)
}

fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => out.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    out.push(byte);
                    index += 2;
                } else {
                    out.push(b'%');
                }
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn jwt_claims(token: &str) -> Value {
    token
        .split('.')
        .nth(1)
        .and_then(|payload| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload.as_bytes())
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null)
}

/// 换 token 的响应 → 账号。
pub fn account_from_tokens(tokens: &Value) -> anyhow::Result<ChatGptAccount> {
    let field = |key: &str| {
        tokens
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let access_token = field("access_token");
    let refresh_token = field("refresh_token");
    let id_token = field("id_token");
    if access_token.is_empty() || refresh_token.is_empty() {
        bail!("登录响应里缺少 token");
    }
    let claims = jwt_claims(&id_token);
    let auth = claims
        .get("https://api.openai.com/auth")
        .cloned()
        .unwrap_or(Value::Null);
    let account_id = auth
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let plan = auth
        .get("chatgpt_plan_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let auth_contents = serde_json::to_string_pretty(&json!({
        "auth_mode": "chatgpt",
        GATEWAY_OWNED_MARKER: true,
        "tokens": {
            "id_token": id_token,
            "access_token": access_token,
            "refresh_token": refresh_token,
            "account_id": account_id,
        }
    }))?;
    Ok(ChatGptAccount {
        email,
        plan,
        account_id,
        auth_contents,
    })
}

/// 登录得到的账号做成网关供应商（官方通道，Responses 协议，没有地址和 Key）。
pub fn chatgpt_relay(account: &ChatGptAccount, models: &[String]) -> RelayProfile {
    let plan = match account.plan.as_str() {
        "" => String::new(),
        plan => format!(" {}", plan[..1].to_uppercase() + &plan[1..]),
    };
    let name = if account.email.is_empty() {
        format!("ChatGPT{plan}")
    } else {
        format!("ChatGPT{plan} · {}", account.email)
    };
    RelayProfile {
        name,
        relay_mode: RelayMode::Official,
        protocol: RelayProtocol::Responses,
        base_url: String::new(),
        api_key: String::new(),
        auth_contents: account.auth_contents.clone(),
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        ..RelayProfile::default()
    }
}

/// 两份 ChatGPT 登录凭据（auth.json 结构）是不是同一个人的同一个账号。
/// Team / Business 工作区的成员共用 chatgpt_account_id，只比它会把另一个成员的登录覆盖掉，
/// 所以还要比 ID token 里的用户（sub，没有再看 chatgpt_user_id、邮箱）。
/// 两边都读不出用户时只能按账号 id 认；只有一边读得出时当作不同的人。
pub fn same_chatgpt_login(left: &str, right: &str) -> bool {
    let account_id = crate::official_login::chatgpt_account_id_from_auth_contents;
    let left_account = account_id(left);
    if left_account.is_empty() || left_account != account_id(right) {
        return false;
    }
    chatgpt_login_user(left) == chatgpt_login_user(right)
}

/// 凭据里 ID token 标识的用户；读不出时为 None。
fn chatgpt_login_user(auth_contents: &str) -> Option<String> {
    let value: Value = serde_json::from_str(auth_contents).ok()?;
    let claims = jwt_claims(value.pointer("/tokens/id_token")?.as_str()?);
    [
        claims.get("sub"),
        claims.pointer("/https:~1~1api.openai.com~1auth/chatgpt_user_id"),
        claims.get("email"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(str::trim)
    .find(|user| !user.is_empty())
    .map(str::to_string)
}

/// 跑一次 ChatGPT 浏览器登录：在 1455 端口等回调，换 token。`open_browser` 负责打开登录页。
pub async fn chatgpt_login(open_browser: impl FnOnce(&str)) -> anyhow::Result<ChatGptAccount> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .await
        .with_context(|| {
            format!("本机端口 {CALLBACK_PORT} 被占用（可能 Codex 正在登录），请稍后再试")
        })?;
    let (verifier, challenge) = pkce_pair();
    let state = uuid::Uuid::new_v4().simple().to_string();
    open_browser(&chatgpt_authorize_url(&challenge, &state));
    let code = tokio::time::timeout(
        LOGIN_TIMEOUT,
        wait_for_callback(&listener, CALLBACK_PATH, &state),
    )
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成浏览器登录"))??;
    exchange_code(&code, &verifier).await
}

async fn wait_for_callback(
    listener: &tokio::net::TcpListener,
    callback_path: &str,
    state: &str,
) -> anyhow::Result<String> {
    let params = wait_for_callback_params(listener, callback_path, state).await?;
    Ok(params.get("code").cloned().unwrap_or_default())
}

async fn wait_for_callback_params(
    listener: &tokio::net::TcpListener,
    callback_path: &str,
    state: &str,
) -> anyhow::Result<BTreeMap<String, String>> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let mut buffer = vec![0_u8; 8192];
        // 浏览器常预先连上却不发请求：读不到就丢掉这条连接，别把后面的回调堵住。
        let read = tokio::time::timeout(CALLBACK_READ_TIMEOUT, stream.read(&mut buffer))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(0);
        if read == 0 {
            continue;
        }
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        let line = request.lines().next().unwrap_or_default();
        let Some(params) = callback_query(line, callback_path) else {
            let _ = stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                )
                .await;
            continue;
        };
        // state 不一致多半是之前放弃的登录页（旧标签页）回调过来的：告诉浏览器这页作废，
        // 接着等这次登录的回调，不结束整个登录。
        if params.get("state").map(String::as_str) != Some(state) {
            respond_callback_page(
                &mut stream,
                "这个登录页已失效（state 不一致），请在 Codex++ 最新打开的登录页里完成登录。",
            )
            .await;
            continue;
        }
        let result = callback_result(params);
        let page = match &result {
            Ok(_) => "登录成功，可以回到 Codex++ 了。",
            Err(_) => "登录没有完成，请回到 Codex++ 重新登录。",
        };
        respond_callback_page(&mut stream, page).await;
        return result.map_err(|error| anyhow::anyhow!("登录失败：{error}"));
    }
}

async fn respond_callback_page(stream: &mut tokio::net::TcpStream, page: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Codex++</title><body style=\"font-family:system-ui;padding:48px;text-align:center\"><h2>{page}</h2></body>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn exchange_code(code: &str, verifier: &str) -> anyhow::Result<ChatGptAccount> {
    let token_url = std::env::var("CODEX_PLUS_OFFICIAL_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| crate::official_login::OFFICIAL_OAUTH_TOKEN_URL.to_string());
    let client = reqwest::Client::builder()
        .user_agent(format!("CodexPlusPlus/{}", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .build()?;
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &redirect_uri()),
            ("client_id", OFFICIAL_OAUTH_CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .context("换取登录 token 失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("换取登录 token 失败：HTTP {}", status.as_u16());
    }
    let value: Value = serde_json::from_str(&text).context("登录响应不是 JSON")?;
    account_from_tokens(&value)
}

/* —————————————— GitHub Copilot —————————————— */

/// VS Code Copilot 的 OAuth App（Copilot 的 token 接口只认它这类白名单 App）。
pub const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_USER_URL: &str = "https://api.github.com/copilot_internal/user";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const COPILOT_DEFAULT_API: &str = "https://api.githubcopilot.com";
/// 凭据里标识账号类型的字段。
pub const ACCOUNT_KIND_FIELD: &str = "codex_plus_account";
/// Copilot 短期 token 到期前多久换新。
const COPILOT_REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;

/// Copilot 请求要带的编辑器标识（与 VS Code Copilot Chat 一致）。
pub fn copilot_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Copilot-Integration-Id", "vscode-chat"),
        ("Editor-Version", "vscode/1.95.0"),
        ("Editor-Plugin-Version", "copilot-chat/0.26.7"),
        ("User-Agent", "GitHubCopilotChat/0.26.7"),
        ("Openai-Intent", "conversation-panel"),
        ("X-Github-Api-Version", "2025-04-01"),
    ]
}

/// 设备码登录的第一步：拿到用户要在浏览器里输入的验证码。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u64,
    pub expires_in: u64,
}

fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent("GitHubCopilotChat/0.26.7")
        .timeout(Duration::from_secs(30))
        .build()?)
}

pub async fn copilot_device_code() -> anyhow::Result<DeviceCode> {
    let response = http_client()?
        .post(GITHUB_DEVICE_CODE_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[("client_id", COPILOT_CLIENT_ID), ("scope", "read:user")])
        .send()
        .await
        .context("连不上 GitHub")?;
    let value: Value = response.json().await.context("GitHub 返回的不是 JSON")?;
    parse_device_code(&value)
}

pub fn parse_device_code(value: &Value) -> anyhow::Result<DeviceCode> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let code = DeviceCode {
        device_code: text("device_code"),
        user_code: text("user_code"),
        verification_uri: text("verification_uri"),
        interval: value
            .get("interval")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .max(1),
        expires_in: value
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(900),
    };
    if code.device_code.is_empty() || code.user_code.is_empty() {
        bail!(
            "GitHub 没有返回设备码：{}",
            value
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or("未知错误")
        );
    }
    Ok(code)
}

/// 轮询结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePoll {
    Token(String),
    Pending,
    SlowDown,
    Failed(String),
}

pub fn parse_device_poll(value: &Value) -> DevicePoll {
    if let Some(token) = value
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
    {
        return DevicePoll::Token(token.to_string());
    }
    match value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "authorization_pending" => DevicePoll::Pending,
        "slow_down" => DevicePoll::SlowDown,
        "expired_token" => DevicePoll::Failed("验证码过期了，请重新登录".to_string()),
        "access_denied" => DevicePoll::Failed("在 GitHub 上取消了授权".to_string()),
        other => DevicePoll::Failed(format!("GitHub 登录失败：{other}")),
    }
}

/// 等用户在浏览器里输入验证码，拿到 GitHub token。
pub async fn copilot_wait_for_github_token(code: &DeviceCode) -> anyhow::Result<String> {
    let client = http_client()?;
    let mut interval = code.interval;
    let deadline = std::time::Instant::now() + Duration::from_secs(code.expires_in.max(60));
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let response = client
            .post(GITHUB_ACCESS_TOKEN_URL)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&[
                ("client_id", COPILOT_CLIENT_ID),
                ("device_code", code.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await;
        let Ok(response) = response else { continue };
        let Ok(value) = response.json::<Value>().await else {
            continue;
        };
        match parse_device_poll(&value) {
            DevicePoll::Token(token) => return Ok(token),
            DevicePoll::Pending => {}
            DevicePoll::SlowDown => interval += 5,
            DevicePoll::Failed(message) => bail!(message),
        }
    }
    bail!("登录超时：验证码已过期")
}

/// Copilot 的短期 token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopilotToken {
    pub token: String,
    pub expires_at_ms: u64,
    pub api_base: String,
    /// 订阅档位（GitHub 返回的 `sku`，如 `free_limited_copilot`、`plus_monthly_subscriber_quota`），没有时为空。
    pub sku: String,
}

pub fn parse_copilot_token(value: &Value) -> anyhow::Result<CopilotToken> {
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if token.is_empty() {
        bail!("这个 GitHub 账号没有可用的 Copilot 订阅");
    }
    let api_base = value
        .pointer("/endpoints/api")
        .and_then(Value::as_str)
        .map(|base| base.trim().trim_end_matches('/').to_string())
        .filter(|base| base.starts_with("https://") && base.contains("githubcopilot.com"))
        .unwrap_or_else(|| COPILOT_DEFAULT_API.to_string());
    Ok(CopilotToken {
        token,
        expires_at_ms: value.get("expires_at").and_then(Value::as_u64).unwrap_or(0) * 1000,
        api_base,
        sku: value
            .get("sku")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

/// Copilot 账号信息（含本月高级请求额度）：`api.github.com/copilot_internal/user`。
pub async fn copilot_user(relay: &RelayProfile) -> anyhow::Result<Value> {
    let github_token = account_kind(relay)
        .and_then(|(_, value)| {
            value
                .get("github_token")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|token| !token.is_empty())
        .context("Copilot 登录信息不完整，请重新登录")?;
    let mut request = http_client()?
        .get(COPILOT_USER_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("token {github_token}"),
        )
        .header(reqwest::header::ACCEPT, "application/json");
    for (key, value) in copilot_headers() {
        request = request.header(key, value);
    }
    let response = request.send().await.context("连不上 GitHub")?;
    if !response.status().is_success() {
        bail!("查询 Copilot 额度失败：HTTP {}", response.status().as_u16());
    }
    Ok(response.json().await?)
}

pub async fn copilot_exchange(github_token: &str) -> anyhow::Result<CopilotToken> {
    let mut request = http_client()?
        .get(COPILOT_TOKEN_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("token {github_token}"),
        )
        .header(reqwest::header::ACCEPT, "application/json");
    for (key, value) in copilot_headers() {
        request = request.header(key, value);
    }
    let response = request.send().await.context("连不上 GitHub")?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 401 {
        bail!("GitHub 登录已失效，请重新登录 Copilot");
    }
    if !status.is_success() {
        bail!(
            "换取 Copilot token 失败：HTTP {}（这个账号可能没有 Copilot 订阅）",
            status.as_u16()
        );
    }
    parse_copilot_token(&value)
}

async fn github_login_name(github_token: &str) -> String {
    let Ok(client) = http_client() else {
        return String::new();
    };
    let Ok(response) = client
        .get(GITHUB_USER_URL)
        .header(
            reqwest::header::AUTHORIZATION,
            format!("token {github_token}"),
        )
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
    else {
        return String::new();
    };
    response
        .json::<Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("login")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Copilot 账号可用的对话模型。
pub async fn copilot_models(token: &CopilotToken) -> Vec<String> {
    let Ok(client) = http_client() else {
        return Vec::new();
    };
    let mut request = client
        .get(format!("{}/models", token.api_base))
        .bearer_auth(&token.token);
    for (key, value) in copilot_headers() {
        request = request.header(key, value);
    }
    let Ok(response) = request.send().await else {
        return Vec::new();
    };
    let Ok(value) = response.json::<Value>().await else {
        return Vec::new();
    };
    parse_copilot_models(&value)
}

pub fn parse_copilot_models(value: &Value) -> Vec<String> {
    let mut models = Vec::new();
    for model in value
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = model.get("id").and_then(Value::as_str) else {
            continue;
        };
        let chat = model
            .pointer("/capabilities/type")
            .and_then(Value::as_str)
            .is_none_or(|kind| kind == "chat");
        let picker = model
            .get("model_picker_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if chat && picker && !models.iter().any(|known: &String| known == id) {
            models.push(id.to_string());
        }
    }
    models
}

/// Copilot 供应商：Chat Completions 协议，地址是 Copilot 的 API（`#` 表示不再补 `/v1`），Key 是短期 token。
pub fn copilot_relay(
    login: &str,
    github_token: &str,
    token: &CopilotToken,
    models: &[String],
) -> RelayProfile {
    let auth = json!({
        ACCOUNT_KIND_FIELD: "copilot",
        "login": login,
        "github_token": github_token,
        "copilot_token": token.token,
        "expires_at_ms": token.expires_at_ms,
        "api_base": token.api_base,
        "sku": token.sku,
    });
    RelayProfile {
        name: if login.is_empty() {
            "GitHub Copilot".to_string()
        } else {
            format!("Copilot · {login}")
        },
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        base_url: format!("{}#", token.api_base),
        api_key: token.token.clone(),
        auth_contents: auth.to_string(),
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        custom_headers: copilot_headers()
            .into_iter()
            .map(|(key, value)| crate::settings::RelayHeaderKeyValue {
                key: key.to_string(),
                value: value.to_string(),
            })
            .collect(),
        ..RelayProfile::default()
    }
}

/// 完成 Copilot 登录：拿 GitHub token → 换 Copilot token → 读模型。
pub async fn copilot_finish(code: &DeviceCode) -> anyhow::Result<RelayProfile> {
    let github_token = copilot_wait_for_github_token(code).await?;
    let token = copilot_exchange(&github_token).await?;
    let login = github_login_name(&github_token).await;
    let models = copilot_models(&token).await;
    Ok(copilot_relay(&login, &github_token, &token, &models))
}

/* —————————————— xAI Grok（SuperGrok 订阅） —————————————— */

/// Grok CLI 的公开 OAuth 客户端（没有 secret）。
pub const GROK_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const GROK_AUTHORIZE_URL: &str = "https://auth.x.ai/oauth2/authorize";
const GROK_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const GROK_SCOPES: &str = "openid profile email offline_access grok-cli:access api:access";
/// 用订阅额度的推理代理（api.x.ai 对 OAuth token 只给免费额度）。
pub const GROK_PROXY_BASE: &str = "https://cli-chat-proxy.grok.com/v1";
const GROK_CALLBACK_PORT: u16 = 56121;
const GROK_REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;
const GROK_DEFAULT_MODELS: &[&str] =
    &["grok-4.6", "grok-build-0.1", "grok-4.3", "grok-code-fast-1"];

/// 冒充的 Grok CLI 版本兜底值。xAI 会抬高最低版本（低于时回 426 Upgrade Required），
/// 平时联网取官方稳定版，取不到或比这个旧时才用它。
const GROK_CLIENT_VERSION: &str = "1.0.70";
/// 官方安装脚本读的稳定渠道指针，正文第一行是版本号；x.ai 不通时脚本回退到 GCS 镜像。
const GROK_STABLE_CHANNEL_URLS: &[&str] = &[
    "https://x.ai/cli/stable",
    "https://storage.googleapis.com/grok-build-public-artifacts/cli/stable",
];
const GROK_VERSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const GROK_VERSION_RETRY: Duration = Duration::from_secs(60 * 60);

struct GrokVersionCache {
    /// 最近一次取到的版本号；查询失败时保留上次的值。
    version: Option<String>,
    checked_at: std::time::Instant,
    ok: bool,
}

static GROK_VERSION_CACHE: std::sync::Mutex<Option<GrokVersionCache>> = std::sync::Mutex::new(None);

/// 推理代理要求的客户端标识，缺了会回 426。
pub fn grok_proxy_headers() -> Vec<(&'static str, String)> {
    vec![
        ("X-XAI-Token-Auth", "xai-grok-cli".to_string()),
        (
            "x-authenticateresponse",
            "authenticate-response".to_string(),
        ),
        ("x-grok-client-mode", "headless".to_string()),
        ("x-grok-client-version", grok_client_version()),
        ("x-grok-client-identifier", "grok-shell".to_string()),
    ]
}

fn grok_client_version() -> String {
    let fetched = GROK_VERSION_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().and_then(|cache| cache.version.clone()));
    pick_grok_client_version(fetched.as_deref())
}

/// 联网值和兜底常量取较新的那个；联网值格式不对就不用。
fn pick_grok_client_version(fetched: Option<&str>) -> String {
    let fallback = parse_grok_version(GROK_CLIENT_VERSION).expect("兜底版本号格式正确");
    let (major, minor, patch) = fetched
        .and_then(parse_grok_version)
        .filter(|version| *version > fallback)
        .unwrap_or(fallback);
    format!("{major}.{minor}.{patch}")
}

/// 只认 `X.Y.Z`，预发布后缀、HTML 错误页之类一律不用。
fn parse_grok_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.lines().next()?.trim();
    let mut parts = value.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u64>().ok())
            .flatten()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// 缓存过期才联网，并发请求共用一次查询。查询失败只影响版本号来源，不挡请求本身。
async fn refresh_grok_client_version() {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOCK.get_or_init(Default::default).lock().await;
    let due = match GROK_VERSION_CACHE.lock() {
        Ok(cache) => cache.as_ref().is_none_or(|cache| {
            let ttl = if cache.ok {
                GROK_VERSION_TTL
            } else {
                GROK_VERSION_RETRY
            };
            cache.checked_at.elapsed() >= ttl
        }),
        Err(_) => false,
    };
    if !due {
        return;
    }
    let fetched = fetch_grok_stable_version().await;
    if let Ok(mut cache) = GROK_VERSION_CACHE.lock() {
        let previous = cache.take().and_then(|cache| cache.version);
        *cache = Some(GrokVersionCache {
            ok: fetched.is_some(),
            version: fetched.or(previous),
            checked_at: std::time::Instant::now(),
        });
    }
}

async fn fetch_grok_stable_version() -> Option<String> {
    for url in GROK_STABLE_CHANNEL_URLS {
        let Ok(client) = crate::http_client::proxied_client_for_url_with_timeouts(
            "",
            url,
            Duration::from_secs(3),
            Duration::from_secs(5),
        ) else {
            continue;
        };
        let body = match client.get(*url).send().await {
            Ok(response) if response.status().is_success() => response.text().await.ok(),
            _ => None,
        };
        if let Some((major, minor, patch)) = body.as_deref().and_then(parse_grok_version) {
            return Some(format!("{major}.{minor}.{patch}"));
        }
    }
    None
}

/// 与官方 CLI 一致的 User-Agent：`grok-shell/<版本> (<系统>; <架构>)`。
fn grok_user_agent() -> String {
    let arch = match std::env::consts::ARCH {
        "x86" => "386",
        other => other,
    };
    format!(
        "grok-shell/{} ({}; {arch})",
        grok_client_version(),
        std::env::consts::OS
    )
}

/// 登录时这些头已存进供应商配置，版本号升级后老账号仍带旧值。
/// 每次请求前用当前值覆盖同名头，用户自加的其他头保留。
fn sync_grok_proxy_headers(relay: &mut RelayProfile) {
    relay.user_agent = grok_user_agent();
    let current = grok_proxy_headers();
    relay.custom_headers.retain(|header| {
        !current
            .iter()
            .any(|(key, _)| key.eq_ignore_ascii_case(header.key.trim()))
    });
    relay
        .custom_headers
        .extend(
            current
                .into_iter()
                .map(|(key, value)| crate::settings::RelayHeaderKeyValue {
                    key: key.to_string(),
                    value,
                }),
        );
}

pub fn grok_authorize_url(redirect_uri: &str, challenge: &str, state: &str) -> String {
    let params = [
        ("response_type", "code".to_string()),
        ("client_id", GROK_CLIENT_ID.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("scope", GROK_SCOPES.to_string()),
        ("code_challenge", challenge.to_string()),
        ("code_challenge_method", "S256".to_string()),
        ("state", state.to_string()),
        ("nonce", uuid::Uuid::new_v4().simple().to_string()),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{GROK_AUTHORIZE_URL}?{query}")
}

/// Grok 的 token：access / refresh 与到期时间。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
    pub email: String,
}

pub fn parse_grok_tokens(value: &Value, previous_refresh: &str) -> anyhow::Result<GrokTokens> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let access_token = text("access_token");
    if access_token.is_empty() {
        bail!(
            "Grok 登录失败：{}",
            value
                .get("error_description")
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("没有拿到 token")
        );
    }
    let refresh_token = Some(text("refresh_token"))
        .filter(|token| !token.is_empty())
        .unwrap_or_else(|| previous_refresh.to_string());
    let expires_in = value
        .get("expires_in")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    let email = jwt_claims(&text("id_token"))
        .get("email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(GrokTokens {
        access_token,
        refresh_token,
        expires_at_ms: now_ms() + expires_in * 1000,
        email,
    })
}

async fn grok_token_request(form: &[(&str, &str)]) -> anyhow::Result<Value> {
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .post(GROK_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .context("连不上 auth.x.ai")?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() && value.get("access_token").is_none() {
        let reason = value
            .get("error_description")
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        bail!("Grok 登录失败：HTTP {} {reason}", status.as_u16());
    }
    Ok(value)
}

/// 浏览器登录 Grok（PKCE，本机回调；默认端口被占时换一个空闲端口）。
pub async fn grok_login(open_browser: impl FnOnce(&str)) -> anyhow::Result<GrokTokens> {
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", GROK_CALLBACK_PORT)).await {
        Ok(listener) => listener,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("无法在本机监听登录回调")?,
    };
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let (verifier, challenge) = pkce_pair();
    let state = uuid::Uuid::new_v4().simple().to_string();
    open_browser(&grok_authorize_url(&redirect_uri, &challenge, &state));
    let code = tokio::time::timeout(
        LOGIN_TIMEOUT,
        wait_for_callback(&listener, "/callback", &state),
    )
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成浏览器登录"))??;
    let value = grok_token_request(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", GROK_CLIENT_ID),
        ("code_verifier", &verifier),
    ])
    .await?;
    parse_grok_tokens(&value, "")
}

/// SuperGrok 订阅的额度（Grok CLI 的 `/usage` 读的同一个接口）。`relay` 要先经过 `prepare_relay`。
pub async fn grok_billing(relay: &RelayProfile) -> anyhow::Result<Value> {
    let mut request = http_client()?
        .get(format!("{GROK_PROXY_BASE}/billing?format=credits"))
        .bearer_auth(&relay.api_key)
        .header(reqwest::header::USER_AGENT, grok_user_agent())
        .header(reqwest::header::ACCEPT, "application/json");
    for (key, value) in grok_proxy_headers() {
        request = request.header(key, value);
    }
    let response = request.send().await.context("连不上 SuperGrok")?;
    if !response.status().is_success() {
        bail!(
            "查询 SuperGrok 额度失败：HTTP {}",
            response.status().as_u16()
        );
    }
    Ok(response.json().await?)
}

/// SuperGrok 账号可用的模型；读不到时用常见的几个。
pub async fn grok_models(access_token: &str) -> Vec<String> {
    refresh_grok_client_version().await;
    let fetched = async {
        let mut request = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?
            .get(format!("{GROK_PROXY_BASE}/models-v2"))
            .bearer_auth(access_token);
        for (key, value) in grok_proxy_headers() {
            request = request.header(key, value);
        }
        let value: Value = request.send().await.ok()?.json().await.ok()?;
        Some(parse_grok_models(&value))
    }
    .await
    .unwrap_or_default();
    if fetched.is_empty() {
        GROK_DEFAULT_MODELS
            .iter()
            .map(|model| model.to_string())
            .collect()
    } else {
        fetched
    }
}

/// 模型目录的字段名各版本不一，`data` / `models` 里的 `id` / `name` / `model` 都认。
pub fn parse_grok_models(value: &Value) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    for key in ["data", "models"] {
        for item in value
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let id = item.as_str().or_else(|| {
                ["id", "name", "model"]
                    .iter()
                    .find_map(|field| item.get(*field).and_then(Value::as_str))
            });
            if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty())
                && !models.iter().any(|known| known == id)
            {
                models.push(id.to_string());
            }
        }
    }
    models
}

/// SuperGrok 供应商：Responses 协议，地址是订阅推理代理，Key 是 OAuth access token。
pub fn grok_relay(tokens: &GrokTokens, models: &[String]) -> RelayProfile {
    let auth = json!({
        ACCOUNT_KIND_FIELD: "grok",
        "email": tokens.email,
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "expires_at_ms": tokens.expires_at_ms,
    });
    RelayProfile {
        name: if tokens.email.is_empty() {
            "SuperGrok".to_string()
        } else {
            format!("SuperGrok · {}", tokens.email)
        },
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        base_url: GROK_PROXY_BASE.to_string(),
        api_key: tokens.access_token.clone(),
        auth_contents: auth.to_string(),
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        custom_headers: grok_proxy_headers()
            .into_iter()
            .map(|(key, value)| crate::settings::RelayHeaderKeyValue {
                key: key.to_string(),
                value,
            })
            .collect(),
        ..RelayProfile::default()
    }
}

fn grok_from_auth(relay: &RelayProfile, value: &Value) -> Option<RelayProfile> {
    let expires = value
        .get("expires_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let token = value
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if token.is_empty() || expires <= now_ms() + GROK_REFRESH_AHEAD_MS {
        return None;
    }
    let mut updated = relay.clone();
    updated.api_key = token.to_string();
    updated.base_url = GROK_PROXY_BASE.to_string();
    sync_grok_proxy_headers(&mut updated);
    Some(updated)
}

async fn prepare_grok(relay: &RelayProfile, value: &Value) -> anyhow::Result<RelayProfile> {
    refresh_grok_client_version().await;
    if let Some(ready) = grok_from_auth(relay, value) {
        return Ok(ready);
    }
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOCK.get_or_init(Default::default).lock().await;
    let store = crate::gateway::store::GatewayStore::default();
    let latest = store
        .load()
        .ok()
        .and_then(|gateway| {
            gateway
                .provider(&relay.id)
                .and_then(|provider| account_kind(&provider.relay))
                .map(|(_, value)| value)
        })
        .unwrap_or_else(|| value.clone());
    if let Some(ready) = grok_from_auth(relay, &latest) {
        return Ok(ready);
    }
    let refresh = latest
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if refresh.is_empty() {
        bail!("SuperGrok 登录已过期，请重新登录");
    }
    let response = grok_token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", GROK_CLIENT_ID),
        ("refresh_token", &refresh),
    ])
    .await
    .context("SuperGrok 登录已失效，请重新登录")?;
    grok_finish_refresh(&store, relay, &latest, &response, &refresh)
}

/// 刷新响应写回凭据。refresh token 可能已经轮换：先落盘再看响应能不能用，
/// 响应缺 access token 时也要把新的 refresh token 存下，否则下次刷新只剩 refresh_token_reused。
fn grok_finish_refresh(
    store: &crate::gateway::store::GatewayStore,
    relay: &RelayProfile,
    latest: &Value,
    response: &Value,
    refresh: &str,
) -> anyhow::Result<RelayProfile> {
    let parsed = parse_grok_tokens(response, refresh);
    let mut auth = latest.clone();
    match &parsed {
        Ok(tokens) => {
            auth["access_token"] = json!(tokens.access_token);
            auth["refresh_token"] = json!(tokens.refresh_token);
            auth["expires_at_ms"] = json!(tokens.expires_at_ms);
        }
        Err(_) => {
            if let Some(rotated) = response
                .get("refresh_token")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
            {
                auth["refresh_token"] = json!(rotated);
            }
        }
    }
    if auth != *latest {
        let auth_contents = auth.to_string();
        persist_account_auth(store, relay, "grok", |stored| {
            stored.auth_contents = auth_contents;
        });
    }
    let tokens = parsed?;
    let mut updated = relay.clone();
    updated.auth_contents = auth.to_string();
    updated.api_key = tokens.access_token.clone();
    updated.base_url = GROK_PROXY_BASE.to_string();
    sync_grok_proxy_headers(&mut updated);
    Ok(updated)
}

/// 刷新得到的凭据写回 gateway.json：后台刷新 token 和界面命令可能同时改，带锁读改写免得互相覆盖。
/// 写不进去要留痕——refresh token 可能已经轮换，没存下来下次就得重新登录。日志里不写凭据。
fn persist_account_auth(
    store: &crate::gateway::store::GatewayStore,
    relay: &RelayProfile,
    kind: &str,
    apply: impl FnOnce(&mut RelayProfile),
) {
    let result = store.update(|gateway| {
        if let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == relay.id)
        {
            apply(&mut provider.relay);
        }
        Ok(())
    });
    if let Err(error) = result {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "gateway.account_token_persist_failed",
            json!({
                "relayId": relay.id,
                "kind": kind,
                "error": error.to_string(),
            }),
        );
    }
}

/* —————————————— ChatGPT API（用 ChatGPT 登录） —————————————— */
//
// OpenAI 给本机运行的开源应用开放的「用 ChatGPT 登录」：ChatGPT 套餐的额度直接用在
// api.openai.com/v1/responses 上，请求保持 Agent 自己的指令，不必装成 Codex CLI。
// 首次登录时应用以 dynamic_agent_client 向 OpenAI 注册，回调里带回签发的 client_id，
// 之后换 token、刷新、注销都用它；每个账号各自保存。与 ChatGPT（Codex 订阅）互不影响。

const SIWC_ISSUER: &str = "https://auth.openai.com";
const SIWC_AUTHORIZE_URL: &str = "https://auth.openai.com/api/accounts/authorize";
const SIWC_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/oauth/token";
/// 请求发往的 API，也是 token 的 resource。
pub const SIWC_API: &str = "https://api.openai.com/v1";
const SIWC_DYNAMIC_CLIENT: &str = "dynamic_agent_client";
/// 授权页上显示的应用名。
const SIWC_AGENT_NAME: &str = "Codex++";
/// 允许 token 直接发给 api.openai.com 的授权范围；没批下来说明账号不符合条件。
const SIWC_DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const SIWC_SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const SIWC_REFRESH_AHEAD_MS: u64 = 3 * 60 * 1000;
/// refresh token 永久失效的错误码：只能重新登录。
const SIWC_DEAD_CODES: &[&str] = &[
    "invalid_grant",
    "invalid_refresh_token",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_invalidated",
    "refresh_token_reused",
];
/// 本机标识的文件名（放在网关状态目录里，生成一次后不变）。
const SIWC_HOST_FILE: &str = "chatgpt-api-host";

/// 本机在 OpenAI 那里的标识（`urn:uuid:` 形式），生成一次后保存，之后每次登录都用同一个。
pub fn siwc_host_id(dir: &Path) -> anyhow::Result<String> {
    let path = dir.join(SIWC_HOST_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim();
        if id.starts_with("urn:uuid:") {
            return Ok(id.to_string());
        }
    }
    let id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, format!("{id}\n"))?;
    Ok(id)
}

pub fn siwc_authorize_url(
    redirect_uri: &str,
    state: &str,
    nonce: &str,
    challenge: &str,
    host_id: &str,
) -> String {
    let params = [
        ("client_id", SIWC_DYNAMIC_CLIENT),
        ("agent_name_hint", SIWC_AGENT_NAME),
        ("ext_agent_host_id", host_id),
        ("response_type", "code"),
        ("redirect_uri", redirect_uri),
        ("resource", SIWC_API),
        ("scope", SIWC_SCOPES),
        ("state", state),
        ("nonce", nonce),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ];
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{SIWC_AUTHORIZE_URL}?{query}")
}

/// 一个 ChatGPT API 账号的凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiwcAccount {
    pub client_id: String,
    pub host_id: String,
    pub subject: String,
    pub email: String,
    pub plan: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
}

/// 换 token / 刷新的响应 → 凭据。`nonce` 只在登录时给：校验 ID token 是这次登录的。
pub fn siwc_from_tokens(
    tokens: &Value,
    client_id: &str,
    host_id: &str,
    nonce: Option<&str>,
) -> anyhow::Result<SiwcAccount> {
    let text = |key: &str| {
        tokens
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let access_token = text("access_token");
    let refresh_token = text("refresh_token");
    if access_token.is_empty() || refresh_token.is_empty() {
        bail!("OpenAI 没有返回 token");
    }
    let expires_in = tokens
        .get("expires_in")
        .and_then(Value::as_f64)
        .filter(|value| *value > 0.0)
        .context("OpenAI 返回的 token 没有有效期")?;
    let scopes = text("scope");
    if !siwc_has_direct_scope(&scopes) {
        bail!(
            "OpenAI 没有授权通过 API 使用这个套餐（{SIWC_DIRECT_SCOPE}），这个 ChatGPT 账号可能不符合条件"
        );
    }
    let id_token = text("id_token");
    let claims = jwt_claims(&id_token);
    if let Some(nonce) = nonce {
        if id_token.is_empty() {
            bail!("OpenAI 没有返回 ID token");
        }
        siwc_check_id(&claims, client_id, nonce)?;
    }
    let claim = |value: &Value, key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let plan = siwc_plan(&claims, &access_token);
    Ok(SiwcAccount {
        client_id: client_id.to_string(),
        host_id: host_id.to_string(),
        subject: claim(&claims, "sub"),
        email: claim(&claims, "email"),
        plan,
        access_token,
        refresh_token,
        expires_at_ms: now_ms() + (expires_in * 1000.0) as u64,
    })
}

/// 套餐类型：先看 ID token，再看 access token；都没有为空。
fn siwc_plan(id_claims: &Value, access_token: &str) -> String {
    [id_claims, &jwt_claims(access_token)]
        .iter()
        .find_map(|value| {
            value
                .pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
                .and_then(Value::as_str)
                .filter(|plan| !plan.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// 授权范围里有没有直连 API 的那一项。
fn siwc_has_direct_scope(scopes: &str) -> bool {
    scopes
        .split_whitespace()
        .any(|scope| scope == SIWC_DIRECT_SCOPE)
}

/// ID token 必须是 OpenAI 签发、给这个 client、带着这次登录的 nonce、没过期。
fn siwc_check_id(claims: &Value, client_id: &str, nonce: &str) -> anyhow::Result<()> {
    if !claims.is_object() {
        bail!("OpenAI 返回的 ID token 读不了");
    }
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if issuer.trim_end_matches('/') != SIWC_ISSUER {
        bail!("ID token 不是 OpenAI 签发的（{issuer}）");
    }
    let audience_ok = match claims.get("aud") {
        Some(Value::String(audience)) => audience == client_id,
        Some(Value::Array(audiences)) => audiences.iter().any(|audience| audience == client_id),
        _ => false,
    };
    if !audience_ok {
        bail!("ID token 是给别的应用的");
    }
    if claims.get("nonce").and_then(Value::as_str) != Some(nonce) {
        bail!("ID token 不属于这次登录");
    }
    if let Some(exp) = claims.get("exp").and_then(Value::as_u64)
        && exp * 1000 + 60_000 < now_ms()
    {
        bail!("ID token 已过期");
    }
    Ok(())
}

async fn siwc_token_request(form: &[(&str, &str)]) -> Result<Value, (String, String)> {
    let token_url = std::env::var("CODEX_PLUS_SIWC_TOKEN_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| SIWC_TOKEN_URL.to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| (String::new(), error.to_string()))?;
    let response = client
        .post(token_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(|error| (String::new(), format!("连不上 auth.openai.com：{error}")))?;
    let status = response.status();
    let value: Value = response.json().await.unwrap_or(Value::Null);
    if status.is_success() {
        return Ok(value);
    }
    // 错误码可能在 error（字符串或对象）或 code 里。
    let code = value
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| value.get("error").and_then(Value::as_str))
        .or_else(|| value.pointer("/error/code").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let message = value
        .get("error_description")
        .and_then(Value::as_str)
        .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
        .unwrap_or_default();
    Err((
        code.clone(),
        format!("HTTP {} {code} {message}", status.as_u16())
            .trim()
            .to_string(),
    ))
}

/// 浏览器登录 ChatGPT API：先试 1455 端口（与 OpenAI 自己的应用一致），被占用就换空闲端口。
pub async fn chatgpt_api_login(
    state_dir: &Path,
    open_browser: impl FnOnce(&str),
) -> anyhow::Result<SiwcAccount> {
    let host_id = siwc_host_id(state_dir)?;
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", CALLBACK_PORT)).await {
        Ok(listener) => listener,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("无法在本机监听登录回调")?,
    };
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
    let (verifier, challenge) = pkce_pair();
    let state = uuid::Uuid::new_v4().simple().to_string();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    open_browser(&siwc_authorize_url(
        &redirect_uri,
        &state,
        &nonce,
        &challenge,
        &host_id,
    ));
    let params = tokio::time::timeout(
        LOGIN_TIMEOUT,
        wait_for_callback_params(&listener, CALLBACK_PATH, &state),
    )
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成浏览器登录"))??;
    let client_id = params
        .get("client_id")
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .context("OpenAI 没有完成应用注册（回调里没有 client_id），请重新登录")?;
    let code = params.get("code").cloned().unwrap_or_default();
    let value = siwc_token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", &client_id),
        ("code", &code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect_uri),
        ("resource", SIWC_API),
    ])
    .await
    .map_err(|(_, message)| anyhow::anyhow!("换取登录 token 失败：{message}"))?;
    let account = siwc_from_tokens(&value, &client_id, &host_id, Some(&nonce))?;
    if account.email.is_empty() && account.subject.is_empty() {
        bail!("OpenAI 没有说明登录的是哪个账号");
    }
    Ok(account)
}

/// 账号可用的模型：OpenAI 给这个 token 列出的模型，再补上 Codex 为 ChatGPT 套餐列出的
/// （本机 Codex 的模型缓存里有，目录里没列也能用）；目录里标为隐藏的不加。
pub async fn chatgpt_api_models(access_token: &str, codex_home: &Path) -> Vec<String> {
    let fetched = async {
        let value: Value = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .ok()?
            .get(format!("{SIWC_API}/models"))
            .bearer_auth(access_token)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;
        Some(value)
    }
    .await
    .unwrap_or(Value::Null);
    let codex = std::fs::read_to_string(codex_home.join("models_cache.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or(Value::Null);
    merge_siwc_models(&fetched, &codex)
}

/// 两种目录格式都认：Codex 式的 `models[].slug`（带 visibility）和 OpenAI 式的 `data[].id`。
pub fn merge_siwc_models(listed: &Value, codex: &Value) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    let mut hidden: Vec<String> = Vec::new();
    for item in listed
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(slug) = item
            .get("slug")
            .and_then(Value::as_str)
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        match item.get("visibility").and_then(Value::as_str) {
            Some(visibility) if !visibility.is_empty() && visibility != "list" => {
                hidden.push(slug.to_string())
            }
            _ if !models.iter().any(|known| known == slug) => models.push(slug.to_string()),
            _ => {}
        }
    }
    for id in listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
    {
        if !models.iter().any(|known| known == id) {
            models.push(id.to_string());
        }
    }
    for item in codex
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(slug) = item
            .get("slug")
            .and_then(Value::as_str)
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        let listed_in_codex = item
            .get("visibility")
            .and_then(Value::as_str)
            .is_none_or(|visibility| visibility == "list");
        if listed_in_codex
            && !slug.contains('/')
            && !hidden.iter().any(|known| known == slug)
            && !models.iter().any(|known| known == slug)
        {
            models.push(slug.to_string());
        }
    }
    models
}

/// ChatGPT API 供应商：Responses 协议，地址是 api.openai.com，Key 是 access token。
pub fn chatgpt_api_relay(account: &SiwcAccount, models: &[String]) -> RelayProfile {
    let auth = json!({
        ACCOUNT_KIND_FIELD: "chatgpt-api",
        "client_id": account.client_id,
        "host_id": account.host_id,
        "subject": account.subject,
        "email": account.email,
        "plan": account.plan,
        "access_token": account.access_token,
        "refresh_token": account.refresh_token,
        "expires_at_ms": account.expires_at_ms,
        "api_base": SIWC_API,
    });
    let plan = match account.plan.as_str() {
        "" => String::new(),
        plan => format!(" {}", plan[..1].to_uppercase() + &plan[1..]),
    };
    let who = if account.email.is_empty() {
        &account.subject
    } else {
        &account.email
    };
    RelayProfile {
        name: if who.is_empty() {
            format!("ChatGPT API{plan}")
        } else {
            format!("ChatGPT API{plan} · {who}")
        },
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        base_url: SIWC_API.to_string(),
        api_key: account.access_token.clone(),
        auth_contents: auth.to_string(),
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        ..RelayProfile::default()
    }
}

fn chatgpt_api_from_auth(relay: &RelayProfile, value: &Value) -> Option<RelayProfile> {
    let expires = value
        .get("expires_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let token = value
        .get("access_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if token.is_empty() || expires <= now_ms() + SIWC_REFRESH_AHEAD_MS {
        return None;
    }
    // 刷新时记下的授权范围里没有直连那一项：这个 token 发不到 API，下次再刷新、报清楚原因。
    if let Some(scopes) = value.get("scope").and_then(Value::as_str)
        && !siwc_has_direct_scope(scopes)
    {
        return None;
    }
    let mut updated = relay.clone();
    updated.api_key = token.to_string();
    updated.base_url = value
        .get("api_base")
        .and_then(Value::as_str)
        .filter(|base| !base.is_empty())
        .unwrap_or(SIWC_API)
        .to_string();
    Some(updated)
}

async fn prepare_chatgpt_api(relay: &RelayProfile, value: &Value) -> anyhow::Result<RelayProfile> {
    if let Some(ready) = chatgpt_api_from_auth(relay, value) {
        return Ok(ready);
    }
    // refresh token 每次刷新都会轮换：并发请求只刷新一次。
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOCK.get_or_init(Default::default).lock().await;
    let store = crate::gateway::store::GatewayStore::default();
    let latest = store
        .load()
        .ok()
        .and_then(|gateway| {
            gateway
                .provider(&relay.id)
                .and_then(|provider| account_kind(&provider.relay))
                .map(|(_, value)| value)
        })
        .unwrap_or_else(|| value.clone());
    if let Some(ready) = chatgpt_api_from_auth(relay, &latest) {
        return Ok(ready);
    }
    let text = |key: &str| {
        latest
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let (client_id, refresh) = (text("client_id"), text("refresh_token"));
    if client_id.is_empty() || refresh.is_empty() {
        bail!("ChatGPT API 登录信息不完整，请重新登录");
    }
    let response = siwc_token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", &client_id),
        ("refresh_token", &refresh),
        ("resource", SIWC_API),
    ])
    .await;
    let response = match response {
        Ok(value) => value,
        Err((code, _)) if SIWC_DEAD_CODES.contains(&code.as_str()) => {
            bail!("ChatGPT API 登录已过期，请重新登录")
        }
        Err((code, message)) if code == "invalid_client" => {
            bail!("OpenAI 已不认这次登录注册的应用（{message}），请重新登录")
        }
        Err((_, message)) => bail!("刷新 ChatGPT API 登录失败：{message}"),
    };
    siwc_finish_refresh(&store, relay, &latest, &response)
}

/// 刷新响应合进已存凭据。没返回 refresh_token / scope 时沿用旧的；
/// 没返回有效期时读 access token 自带的 exp，再没有就记为已过期（下次重新刷新）。
fn siwc_merge_refresh(latest: &Value, response: &Value) -> Value {
    let text = |key: &str| {
        response
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    };
    let mut auth = latest.clone();
    for key in ["access_token", "refresh_token", "scope"] {
        if let Some(value) = text(key) {
            auth[key] = json!(value);
        }
    }
    if let Some(access_token) = text("access_token") {
        let expires_at_ms = response
            .get("expires_in")
            .and_then(Value::as_f64)
            .filter(|value| *value > 0.0)
            .map(|seconds| now_ms() + (seconds * 1000.0) as u64)
            .or_else(|| {
                jwt_claims(access_token)
                    .get("exp")
                    .and_then(Value::as_u64)
                    .map(|exp| exp * 1000)
            })
            .unwrap_or(0);
        auth["expires_at_ms"] = json!(expires_at_ms);
        let plan = siwc_plan(
            &jwt_claims(text("id_token").unwrap_or_default()),
            access_token,
        );
        if !plan.is_empty() {
            auth["plan"] = json!(plan);
        }
    }
    auth
}

/// 刷新请求一到服务端，旧 refresh token 就作废了：新凭据先落盘，再校验能不能用。
/// 校验不过只是这次请求失败，新 refresh token 已经存下，下次还能接着刷新。
fn siwc_finish_refresh(
    store: &crate::gateway::store::GatewayStore,
    relay: &RelayProfile,
    latest: &Value,
    response: &Value,
) -> anyhow::Result<RelayProfile> {
    let auth = siwc_merge_refresh(latest, response);
    let mut updated = relay.clone();
    updated.auth_contents = auth.to_string();
    if auth != *latest {
        let auth_contents = updated.auth_contents.clone();
        persist_account_auth(store, relay, "chatgpt-api", |stored| {
            stored.auth_contents = auth_contents;
        });
    }
    if response
        .get("access_token")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        bail!("刷新 ChatGPT API 登录失败：OpenAI 没有返回 token");
    }
    if let Some(scopes) = auth.get("scope").and_then(Value::as_str)
        && !siwc_has_direct_scope(scopes)
    {
        bail!(
            "OpenAI 没有授权通过 API 使用这个套餐（{SIWC_DIRECT_SCOPE}），这个 ChatGPT 账号可能不符合条件"
        );
    }
    chatgpt_api_from_auth(&updated, &auth)
        .context("刷新 ChatGPT API 登录失败：新 token 的有效期太短")
}

/// 删除 ChatGPT API 账号时让 OpenAI 吊销它的 refresh token（按发现文档里的吊销地址）。
pub async fn chatgpt_api_revoke(relay: &RelayProfile) -> anyhow::Result<()> {
    let Some((kind, value)) = account_kind(relay) else {
        return Ok(());
    };
    if kind != "chatgpt-api" {
        return Ok(());
    }
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let discovery: Value = client
        .get(format!("{SIWC_ISSUER}/.well-known/openid-configuration"))
        .send()
        .await?
        .json()
        .await?;
    let endpoint = discovery
        .get("revocation_endpoint")
        .and_then(Value::as_str)
        .filter(|endpoint| endpoint.starts_with("https://"))
        .context("OpenAI 没有给出吊销地址")?;
    let status = client
        .post(endpoint)
        .form(&[
            ("token", text("refresh_token")),
            ("token_type_hint", "refresh_token"),
            ("client_id", text("client_id")),
        ])
        .send()
        .await?
        .status();
    if !status.is_success() {
        bail!("注销 ChatGPT API 登录失败：HTTP {}", status.as_u16());
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

/// 账号类供应商的类型（`copilot` / `grok` / `chatgpt-api`）；普通供应商为 None。
/// 订阅账号的套餐，如「ChatGPT Pro」「Copilot Pro+」「SuperGrok」；不是订阅账号或读不出来时为空。
/// 只读登录时存下的凭据，不发请求。
pub fn account_plan(relay: &RelayProfile) -> String {
    if crate::official_login::member_uses_official_login_transport(relay) {
        let value: Value = serde_json::from_str(&relay.auth_contents).unwrap_or(Value::Null);
        let id_token = value
            .pointer("/tokens/id_token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let plan = jwt_claims(id_token)
            .pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        return with_plan("ChatGPT", &plan);
    }
    let Some((kind, value)) = account_kind(relay) else {
        return String::new();
    };
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    match kind.as_str() {
        "chatgpt-api" => with_plan("ChatGPT", &text("plan")),
        "copilot" => copilot_plan_label(&text("sku"), &text("api_base")),
        "grok" => "SuperGrok".to_string(),
        _ => String::new(),
    }
}

/// `pro` → 「ChatGPT Pro」；没有套餐时只写产品名。
fn with_plan(product: &str, plan: &str) -> String {
    let plan = plan.trim();
    if plan.is_empty() {
        return product.to_string();
    }
    let mut chars = plan.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    format!("{product} {first}{}", chars.as_str())
}

/// Copilot 的 `sku` → 套餐名。旧账号没存 `sku` 时按 API 地址（individual / business / enterprise）推断。
pub fn copilot_plan_label(sku: &str, api_base: &str) -> String {
    let sku = sku.to_ascii_lowercase();
    let label = if sku.contains("enterprise") {
        "Copilot Enterprise"
    } else if sku.contains("business") {
        "Copilot Business"
    } else if sku.contains("plus") {
        "Copilot Pro+"
    } else if sku.contains("educational") {
        "Copilot Pro"
    } else if sku.contains("free") {
        "Copilot Free"
    } else if !sku.is_empty() {
        "Copilot Pro"
    } else if api_base.contains(".enterprise.") {
        "Copilot Enterprise"
    } else if api_base.contains(".business.") {
        "Copilot Business"
    } else {
        "Copilot"
    };
    label.to_string()
}

/// 账号名打码，与管理器的 `maskAccountName` 一致：邮箱只打本地部分；
/// 「ChatGPT / Copilot / SuperGrok … · 账号」里不是邮箱的账号整体打码。写进 Agent 配置、给 Agent 看的名字都要过它。
pub fn mask_account_name(name: &str) -> String {
    let masked = name
        .split(' ')
        .map(|word| match word.split_once('@') {
            Some((local, domain))
                if !local.is_empty() && domain.contains('.') && !domain.ends_with('.') =>
            {
                format!("{}@{domain}", mask_token(local))
            }
            _ => word.to_string(),
        })
        .collect::<Vec<_>>()
        .join(" ");
    let account_prefix = ["ChatGPT", "Copilot", "SuperGrok", "GitHub Copilot"]
        .iter()
        .any(|prefix| masked.starts_with(prefix));
    if let Some((head, rest)) = masked.split_once('·')
        && account_prefix
        && !rest.contains('@')
        && !rest.trim().is_empty()
    {
        let spaces = &rest[..rest.len() - rest.trim_start().len()];
        return format!("{head}·{spaces}{}", mask_token(rest.trim()));
    }
    masked
}

/// 4 个字符以内只留第一个；更长的留头两个、尾两个，中间固定 4 个星号（不暴露长度）。
fn mask_token(value: &str) -> String {
    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= 4 {
        return format!(
            "{}***",
            chars.first().map(char::to_string).unwrap_or_default()
        );
    }
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}****{tail}")
}

pub fn relay_account_kind(relay: &RelayProfile) -> Option<String> {
    account_kind(relay).map(|(kind, _)| kind)
}

fn account_kind(relay: &RelayProfile) -> Option<(String, Value)> {
    let value: Value = serde_json::from_str(&relay.auth_contents).ok()?;
    let kind = value.get(ACCOUNT_KIND_FIELD)?.as_str()?.to_string();
    Some((kind, value))
}

/// 请求前准备账号类供应商的凭据：Copilot 的短期 token 快过期时换新并写回 gateway.json。
/// 普通供应商原样返回。
pub async fn prepare_relay(relay: &RelayProfile) -> anyhow::Result<RelayProfile> {
    let Some((kind, value)) = account_kind(relay) else {
        return Ok(relay.clone());
    };
    match kind.as_str() {
        "copilot" => prepare_copilot(relay, &value).await,
        "grok" => prepare_grok(relay, &value).await,
        "chatgpt-api" => prepare_chatgpt_api(relay, &value).await,
        _ => Ok(relay.clone()),
    }
}

/// 从凭据里取出当前的 Copilot token 和地址（地址、Key 不单独落盘，读回来要从凭据补上）。
/// token 快过期时返回 None。
fn copilot_from_auth(relay: &RelayProfile, value: &Value) -> Option<RelayProfile> {
    let expires = value
        .get("expires_at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let token = value
        .get("copilot_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if token.is_empty() || expires <= now_ms() + COPILOT_REFRESH_AHEAD_MS {
        return None;
    }
    let api_base = value
        .get("api_base")
        .and_then(Value::as_str)
        .filter(|base| !base.is_empty())
        .unwrap_or(COPILOT_DEFAULT_API);
    let mut updated = relay.clone();
    updated.api_key = token.to_string();
    updated.base_url = format!("{api_base}#");
    Some(updated)
}

async fn prepare_copilot(relay: &RelayProfile, value: &Value) -> anyhow::Result<RelayProfile> {
    if let Some(ready) = copilot_from_auth(relay, value) {
        return Ok(ready);
    }
    // 串行换新：并发请求只换一次。
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    let _guard = LOCK.get_or_init(Default::default).lock().await;
    // 等锁期间别的请求可能已经换过了。
    let store = crate::gateway::store::GatewayStore::default();
    if let Some(latest) = store.load().ok().and_then(|gateway| {
        gateway
            .provider(&relay.id)
            .map(|provider| provider.relay.clone())
    }) && let Some((_, latest_value)) = account_kind(&latest)
        && let Some(ready) = copilot_from_auth(relay, &latest_value)
    {
        return Ok(ready);
    }
    let github_token = value
        .get("github_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if github_token.is_empty() {
        bail!("Copilot 登录信息不完整，请重新登录");
    }
    let token = copilot_exchange(github_token).await?;
    let mut updated = relay.clone();
    let mut auth = value.clone();
    auth["copilot_token"] = json!(token.token);
    auth["expires_at_ms"] = json!(token.expires_at_ms);
    auth["api_base"] = json!(token.api_base);
    if !token.sku.is_empty() {
        auth["sku"] = json!(token.sku);
    }
    updated.auth_contents = auth.to_string();
    updated.api_key = token.token.clone();
    updated.base_url = format!("{}#", token.api_base);
    persist_account_auth(&store, relay, "copilot", |stored| {
        stored.auth_contents = updated.auth_contents.clone();
        stored.api_key = updated.api_key.clone();
        stored.base_url = updated.base_url.clone();
    });
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_account_names_like_the_manager() {
        assert_eq!(
            mask_account_name("ChatGPT Pro · 2649340134@qq.com"),
            "ChatGPT Pro · 26****34@qq.com"
        );
        assert_eq!(
            mask_account_name("ChatGPT Plus · ab@x.io"),
            "ChatGPT Plus · a***@x.io"
        );
        assert_eq!(
            mask_account_name("Copilot Pro+ · octocat-dev"),
            "Copilot Pro+ · oc****ev"
        );
        assert_eq!(mask_account_name("DeepSeek"), "DeepSeek");
        assert_eq!(
            mask_account_name("Relay · team"),
            "Relay · team",
            "普通供应商名不动"
        );
    }

    fn jwt(payload: Value) -> String {
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.",
            engine.encode(br#"{"alg":"none"}"#),
            engine.encode(serde_json::to_vec(&payload).unwrap())
        )
    }

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let (verifier, challenge) = pkce_pair();
        assert_eq!(verifier.len(), 64);
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
        let url = chatgpt_authorize_url(&challenge, "st");
        assert!(url.starts_with("https://auth.openai.com/oauth/authorize?response_type=code"));
        assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
        assert!(url.contains("scope=openid%20profile%20email%20offline_access"));
        assert!(url.contains(&format!("code_challenge={challenge}")));
    }

    #[test]
    fn callback_parsing() {
        assert_eq!(
            parse_callback("GET /auth/callback?code=ab%2Fc&state=xyz HTTP/1.1"),
            Some(Ok(("ab/c".to_string(), "xyz".to_string())))
        );
        assert_eq!(
            parse_callback("GET /auth/callback?error=access_denied HTTP/1.1"),
            Some(Err("access_denied".to_string()))
        );
        assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1"), None);
    }

    #[test]
    fn tokens_become_a_gateway_owned_official_provider() {
        let tokens = json!({
            "access_token": "at",
            "refresh_token": "rt",
            "id_token": jwt(json!({
                "email": "me@example.com",
                "https://api.openai.com/auth": { "chatgpt_account_id": "acc-1", "chatgpt_plan_type": "pro" }
            }))
        });
        let account = account_from_tokens(&tokens).unwrap();
        assert_eq!(account.email, "me@example.com");
        assert_eq!(account.account_id, "acc-1");
        assert_eq!(account.plan, "pro");
        let relay = chatgpt_relay(
            &account,
            &["gpt-6.1-sol".to_string(), "gpt-6-luna".to_string()],
        );
        assert_eq!(relay.name, "ChatGPT Pro · me@example.com");
        assert!(crate::official_login::member_uses_official_login_transport(
            &relay
        ));
        let auth: Value = serde_json::from_str(&relay.auth_contents).unwrap();
        assert_eq!(auth[GATEWAY_OWNED_MARKER], true);
        assert_eq!(auth["tokens"]["account_id"], "acc-1");
        assert!(account_from_tokens(&json!({ "access_token": "x" })).is_err());
    }

    #[test]
    fn refreshed_tokens_are_written_back_to_the_gateway_store() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::gateway::store::GatewayStore::at(temp.path().join("gateway.json"));
        let account = account_from_tokens(
            &json!({ "access_token": "old-at", "refresh_token": "old-rt", "id_token": "" }),
        )
        .unwrap();
        let mut relay = chatgpt_relay(&account, &["gpt-6.1-sol".to_string()]);
        relay.id = "chatgpt".to_string();
        let mut gateway = crate::gateway::store::GatewaySettings::default();
        gateway
            .providers
            .push(crate::gateway::store::GatewayProvider {
                enabled: true,
                source: "login:chatgpt".to_string(),
                price: None,
                model_prices: Default::default(),
                model_windows: Default::default(),
                context_window: None,
                proxy: String::new(),
                upstream_new: Vec::new(),
                upstream_gone: Vec::new(),
                models_checked_at_ms: 0,
                relay,
                extra: Default::default(),
            });
        store.save(&gateway).unwrap();
        crate::official_login::persist_gateway_login(
            &store, "chatgpt", "new-at", "new-rt", "acc-9",
        );
        let saved = store.load().unwrap();
        let auth: Value =
            serde_json::from_str(&saved.provider("chatgpt").unwrap().relay.auth_contents).unwrap();
        assert_eq!(auth["tokens"]["access_token"], "new-at");
        assert_eq!(auth["tokens"]["refresh_token"], "new-rt");
        assert_eq!(auth["tokens"]["account_id"], "acc-9");
        assert_eq!(auth[GATEWAY_OWNED_MARKER], true, "标记保留");
    }

    #[test]
    fn copilot_device_flow_and_token_parsing() {
        let code = parse_device_code(&json!({ "device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://github.com/login/device", "interval": 5, "expires_in": 900 })).unwrap();
        assert_eq!(code.user_code, "ABCD-1234");
        assert!(parse_device_code(&json!({ "error": "x" })).is_err());
        assert_eq!(
            parse_device_poll(&json!({ "error": "authorization_pending" })),
            DevicePoll::Pending
        );
        assert_eq!(
            parse_device_poll(&json!({ "error": "slow_down" })),
            DevicePoll::SlowDown
        );
        assert_eq!(
            parse_device_poll(&json!({ "access_token": "gho_x" })),
            DevicePoll::Token("gho_x".to_string())
        );
        assert!(matches!(
            parse_device_poll(&json!({ "error": "access_denied" })),
            DevicePoll::Failed(_)
        ));

        let token = parse_copilot_token(&json!({ "token": "tid=1", "expires_at": 1_800_000_000, "endpoints": { "api": "https://api.individual.githubcopilot.com/" } })).unwrap();
        assert_eq!(token.api_base, "https://api.individual.githubcopilot.com");
        assert_eq!(token.expires_at_ms, 1_800_000_000_000);
        let untrusted = parse_copilot_token(
            &json!({ "token": "t", "endpoints": { "api": "https://evil.example" } }),
        )
        .unwrap();
        assert_eq!(
            untrusted.api_base, "https://api.githubcopilot.com",
            "只信任 githubcopilot.com"
        );
        assert!(parse_copilot_token(&json!({})).is_err());

        let models = parse_copilot_models(&json!({ "data": [
            { "id": "gpt-5.5", "model_picker_enabled": true, "capabilities": { "type": "chat" } },
            { "id": "text-embedding-3-small", "capabilities": { "type": "embeddings" } },
            { "id": "hidden", "model_picker_enabled": false },
            { "id": "claude-sonnet-5.5" }
        ]}));
        assert_eq!(models, vec!["gpt-5.5", "claude-sonnet-5.5"]);

        let relay = copilot_relay("octo", "gho_x", &token, &models);
        assert_eq!(relay.name, "Copilot · octo");
        assert_eq!(relay.protocol, RelayProtocol::ChatCompletions);
        assert_eq!(
            crate::protocol_proxy::chat_completions_url(&relay.base_url),
            "https://api.individual.githubcopilot.com/chat/completions"
        );
        assert!(
            relay
                .custom_headers
                .iter()
                .any(|header| header.key == "Copilot-Integration-Id")
        );
        assert_eq!(account_kind(&relay).unwrap().0, "copilot");
    }

    #[tokio::test]
    async fn fresh_copilot_tokens_are_used_as_is() {
        let token = CopilotToken {
            sku: String::new(),
            token: "fresh".to_string(),
            expires_at_ms: now_ms() + 3_600_000,
            api_base: "https://api.githubcopilot.com".to_string(),
        };
        let relay = copilot_relay("octo", "gho_x", &token, &[]);
        let mut stored = relay.clone();
        // 读回来时地址和 Key 不在（不单独落盘），要从凭据补上。
        stored.api_key.clear();
        stored.base_url = "https://api.openai.com/v1".to_string();
        let prepared = prepare_relay(&stored).await.unwrap();
        assert_eq!(prepared.api_key, "fresh");
        assert_eq!(prepared.base_url, "https://api.githubcopilot.com#");
        let plain = RelayProfile {
            api_key: "sk".to_string(),
            ..RelayProfile::default()
        };
        assert_eq!(prepare_relay(&plain).await.unwrap().api_key, "sk");
    }

    #[test]
    fn grok_login_and_relay() {
        let url = grok_authorize_url("http://127.0.0.1:56121/callback", "ch", "st");
        assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?response_type=code"));
        assert!(url.contains("client_id=b1a00492-073a-47ea-816f-4c329264a828"));
        assert!(url.contains(
            "scope=openid%20profile%20email%20offline_access%20grok-cli%3Aaccess%20api%3Aaccess"
        ));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback"));
        assert_eq!(
            parse_callback_at("GET /callback?code=c1&state=st HTTP/1.1", "/callback"),
            Some(Ok(("c1".to_string(), "st".to_string())))
        );

        let tokens = parse_grok_tokens(&json!({ "access_token": "at", "refresh_token": "rt", "expires_in": 3600, "id_token": jwt(json!({ "email": "g@x.ai" })) }), "").unwrap();
        assert_eq!(tokens.email, "g@x.ai");
        let rotated = parse_grok_tokens(&json!({ "access_token": "at2" }), "rt").unwrap();
        assert_eq!(
            rotated.refresh_token, "rt",
            "没有返回新的 refresh token 时沿用旧的"
        );
        assert!(parse_grok_tokens(&json!({ "error": "invalid_grant" }), "").is_err());

        assert_eq!(
            parse_grok_models(
                &json!({ "data": [{ "id": "grok-4.6" }, { "name": "grok-build-0.1" }] })
            ),
            vec!["grok-4.6", "grok-build-0.1"]
        );
        assert_eq!(
            parse_grok_models(&json!({ "models": ["grok-4.3"] })),
            vec!["grok-4.3"]
        );

        let relay = grok_relay(&tokens, &["grok-4.6".to_string()]);
        assert_eq!(relay.name, "SuperGrok · g@x.ai");
        assert_eq!(relay.protocol, RelayProtocol::Responses);
        assert_eq!(relay.base_url, GROK_PROXY_BASE);
        assert!(
            relay
                .custom_headers
                .iter()
                .any(|header| header.key == "X-XAI-Token-Auth" && header.value == "xai-grok-cli")
        );
        let mut stored = relay.clone();
        stored.api_key.clear();
        let ready = grok_from_auth(&stored, &account_kind(&relay).unwrap().1).unwrap();
        assert_eq!(ready.api_key, "at");

        // 老账号存的是旧版本号，请求前要换成当前值，用户自加的头保留
        for header in &mut stored.custom_headers {
            if header.key == "x-grok-client-version" {
                header.value = "1.0.5".to_string();
            }
        }
        stored
            .custom_headers
            .push(crate::settings::RelayHeaderKeyValue {
                key: "X-Custom".to_string(),
                value: "keep".to_string(),
            });
        let ready = grok_from_auth(&stored, &account_kind(&relay).unwrap().1).unwrap();
        let versions: Vec<_> = ready
            .custom_headers
            .iter()
            .filter(|header| header.key.eq_ignore_ascii_case("x-grok-client-version"))
            .map(|header| header.value.as_str())
            .collect();
        assert_eq!(versions, vec![grok_client_version()]);
        assert!(
            ready
                .user_agent
                .starts_with(&format!("grok-shell/{} (", grok_client_version()))
        );
        assert!(
            ready
                .custom_headers
                .iter()
                .any(|header| header.key == "X-Custom" && header.value == "keep")
        );
    }

    #[test]
    fn grok_client_version_prefers_newer_stable_and_falls_back_to_constant() {
        assert_eq!(parse_grok_version("1.0.70\n"), Some((1, 0, 70)));
        assert_eq!(parse_grok_version("  2.10.3\r\nextra"), Some((2, 10, 3)));
        for bad in [
            "",
            "1.0",
            "1.0.70.1",
            "1.0.70-rc1",
            "+1.0.70",
            "v1.0.70",
            "<html>",
        ] {
            assert_eq!(parse_grok_version(bad), None, "{bad:?} 不是合法版本号");
        }

        assert_eq!(pick_grok_client_version(Some("1.0.73")), "1.0.73");
        assert_eq!(pick_grok_client_version(Some("1.1.0")), "1.1.0");
        assert_eq!(
            pick_grok_client_version(Some("1.0.13")),
            GROK_CLIENT_VERSION,
            "联网值比兜底旧时用兜底"
        );
        assert_eq!(
            pick_grok_client_version(Some("garbage")),
            GROK_CLIENT_VERSION
        );
        assert_eq!(pick_grok_client_version(None), GROK_CLIENT_VERSION);
    }

    #[test]
    fn chatgpt_api_login_url_callback_and_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let host = siwc_host_id(temp.path()).unwrap();
        assert!(host.starts_with("urn:uuid:"));
        assert_eq!(
            siwc_host_id(temp.path()).unwrap(),
            host,
            "本机标识只生成一次"
        );

        let url = siwc_authorize_url(
            "http://127.0.0.1:1455/auth/callback",
            "st",
            "nn",
            "ch",
            &host,
        );
        assert!(url.starts_with(
            "https://auth.openai.com/api/accounts/authorize?client_id=dynamic_agent_client"
        ));
        assert!(url.contains("agent_name_hint=Codex%2B%2B"));
        assert!(url.contains("resource=https%3A%2F%2Fapi.openai.com%2Fv1"));
        assert!(url.contains("chatgpt.tokens.use.direct"));
        assert!(url.contains("nonce=nn"));

        // 回调里除了 code / state 还带回 OpenAI 签发的 client_id。
        let params = callback_params(
            "GET /auth/callback?code=c1&state=st&client_id=app_123 HTTP/1.1",
            "/auth/callback",
        )
        .unwrap()
        .unwrap();
        assert_eq!(params["client_id"], "app_123");

        let id_token = jwt(json!({
            "iss": "https://auth.openai.com", "aud": ["app_123"], "nonce": "nn",
            "sub": "user-1", "email": "me@example.com",
            "https://api.openai.com/auth": { "chatgpt_plan_type": "plus" }
        }));
        let tokens = json!({
            "access_token": "at", "refresh_token": "rt", "id_token": id_token, "expires_in": 3600,
            "scope": "openid email resource.invoke chatgpt.tokens.use.direct"
        });
        let account = siwc_from_tokens(&tokens, "app_123", &host, Some("nn")).unwrap();
        assert_eq!(account.email, "me@example.com");
        assert_eq!(account.plan, "plus");
        assert!(account.expires_at_ms > now_ms());
        assert!(
            siwc_from_tokens(&tokens, "app_123", &host, Some("other")).is_err(),
            "nonce 不符"
        );
        assert!(
            siwc_from_tokens(&tokens, "app_999", &host, Some("nn")).is_err(),
            "aud 不符"
        );
        let mut no_scope = tokens.clone();
        no_scope["scope"] = json!("openid email");
        let error = siwc_from_tokens(&no_scope, "app_123", &host, Some("nn")).unwrap_err();
        assert!(error.to_string().contains("不符合条件"), "{error}");

        let relay = chatgpt_api_relay(&account, &["gpt-6.1-sol".to_string()]);
        assert_eq!(relay.name, "ChatGPT API Plus · me@example.com");
        assert_eq!(relay.protocol, RelayProtocol::Responses);
        assert_eq!(relay.base_url, SIWC_API);
        assert!(!crate::official_login::member_uses_official_login_transport(&relay));
        assert_eq!(relay_account_kind(&relay).as_deref(), Some("chatgpt-api"));
        let mut stored = relay.clone();
        stored.api_key.clear();
        stored.base_url.clear();
        let ready = chatgpt_api_from_auth(&stored, &account_kind(&relay).unwrap().1).unwrap();
        assert_eq!(ready.api_key, "at");
        assert_eq!(ready.base_url, SIWC_API);
    }

    #[test]
    fn chatgpt_api_models_merge_the_listed_and_codex_catalogs() {
        let listed = json!({ "models": [
            { "slug": "gpt-6.1-sol", "visibility": "list" },
            { "slug": "gpt-old", "visibility": "hide" }
        ]});
        let codex = json!({ "models": [
            { "slug": "gpt-6-luna", "visibility": "list" },
            { "slug": "gpt-old", "visibility": "list" },
            { "slug": "gpt-6.1-sol" }
        ]});
        assert_eq!(
            merge_siwc_models(&listed, &codex),
            vec!["gpt-6.1-sol", "gpt-6-luna"]
        );
        assert_eq!(
            merge_siwc_models(&json!({ "data": [{ "id": "gpt-5.5" }] }), &Value::Null),
            vec!["gpt-5.5"]
        );
    }

    #[test]
    fn account_plan_reads_stored_credentials() {
        let copilot = |sku: &str, api: &str| copilot_plan_label(sku, api);
        assert_eq!(copilot("free_limited_copilot", ""), "Copilot Free");
        assert_eq!(copilot("plus_monthly_subscriber_quota", ""), "Copilot Pro+");
        assert_eq!(copilot("monthly_subscriber_quota", ""), "Copilot Pro");
        assert_eq!(copilot("free_educational_quota", ""), "Copilot Pro");
        assert_eq!(
            copilot("copilot_for_business_seat_quota", ""),
            "Copilot Business"
        );
        assert_eq!(
            copilot("", "https://api.enterprise.githubcopilot.com"),
            "Copilot Enterprise"
        );
        assert_eq!(
            copilot("", "https://api.individual.githubcopilot.com"),
            "Copilot"
        );

        let grok = grok_relay(
            &GrokTokens {
                access_token: "a".into(),
                refresh_token: "r".into(),
                expires_at_ms: 0,
                email: "g@x.ai".into(),
            },
            &[],
        );
        assert_eq!(account_plan(&grok), "SuperGrok");

        let account = account_from_tokens(&json!({
            "access_token": "a",
            "refresh_token": "r",
            "id_token": jwt(json!({
                "email": "me@example.com",
                "https://api.openai.com/auth": { "chatgpt_account_id": "acc", "chatgpt_plan_type": "pro" }
            })),
        }))
        .unwrap();
        assert_eq!(account_plan(&chatgpt_relay(&account, &[])), "ChatGPT Pro");
        assert_eq!(account_plan(&RelayProfile::default()), "");
    }

    /// 只有一个供应商的临时 gateway.json。
    fn store_with(relay: RelayProfile) -> (tempfile::TempDir, crate::gateway::store::GatewayStore) {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::gateway::store::GatewayStore::at(temp.path().join("gateway.json"));
        let mut gateway = crate::gateway::store::GatewaySettings::default();
        gateway
            .providers
            .push(crate::gateway::store::GatewayProvider {
                enabled: true,
                source: "login:test".to_string(),
                price: None,
                model_prices: Default::default(),
                model_windows: Default::default(),
                context_window: None,
                proxy: String::new(),
                upstream_new: Vec::new(),
                upstream_gone: Vec::new(),
                models_checked_at_ms: 0,
                relay,
                extra: Default::default(),
            });
        store.save(&gateway).unwrap();
        (temp, store)
    }

    fn stored_auth(store: &crate::gateway::store::GatewayStore, id: &str) -> Value {
        let gateway = store.load().unwrap();
        serde_json::from_str(&gateway.provider(id).unwrap().relay.auth_contents).unwrap()
    }

    fn expired_chatgpt_api_relay() -> RelayProfile {
        let mut relay = chatgpt_api_relay(
            &SiwcAccount {
                client_id: "app_123".to_string(),
                host_id: "urn:uuid:h".to_string(),
                subject: "user-1".to_string(),
                email: "me@example.com".to_string(),
                plan: "plus".to_string(),
                access_token: "old-at".to_string(),
                refresh_token: "old-rt".to_string(),
                expires_at_ms: 0,
            },
            &["gpt-6.1-sol".to_string()],
        );
        relay.id = "chatgpt-api".to_string();
        relay
    }

    #[test]
    fn chatgpt_api_refresh_keeps_missing_fields_and_persists_before_checks() {
        let relay = expired_chatgpt_api_relay();
        let latest = account_kind(&relay).unwrap().1;

        // 没返回 refresh_token / scope：沿用旧的，照常可用。
        let (_temp, store) = store_with(relay.clone());
        let ready = siwc_finish_refresh(
            &store,
            &relay,
            &latest,
            &json!({ "access_token": "at-2", "expires_in": 3600 }),
        )
        .unwrap();
        assert_eq!(ready.api_key, "at-2");
        let saved = stored_auth(&store, "chatgpt-api");
        assert_eq!(saved["access_token"], "at-2");
        assert_eq!(
            saved["refresh_token"], "old-rt",
            "没返回新的就沿用旧 refresh token"
        );

        // 有效期太短：这次失败，但轮换出来的 refresh token 已经存下。
        let (_temp, store) = store_with(relay.clone());
        let error = siwc_finish_refresh(
            &store,
            &relay,
            &latest,
            &json!({ "access_token": "at-3", "refresh_token": "rt-3", "expires_in": 1 }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("有效期太短"), "{error}");
        assert_eq!(stored_auth(&store, "chatgpt-api")["refresh_token"], "rt-3");

        // 没给有效期：读 access token 自带的 exp。
        let (_temp, store) = store_with(relay.clone());
        let exp = now_ms() / 1000 + 3600;
        let ready = siwc_finish_refresh(
            &store,
            &relay,
            &latest,
            &json!({ "access_token": jwt(json!({ "exp": exp })), "refresh_token": "rt-4" }),
        )
        .unwrap();
        assert!(!ready.api_key.is_empty());
        assert_eq!(
            stored_auth(&store, "chatgpt-api")["expires_at_ms"],
            exp * 1000
        );

        // 授权范围里没有直连那一项：报清楚原因，新 refresh token 仍然存下，存下的 token 不拿来直接用。
        let (_temp, store) = store_with(relay.clone());
        let error = siwc_finish_refresh(
            &store,
            &relay,
            &latest,
            &json!({ "access_token": "at-5", "refresh_token": "rt-5", "expires_in": 3600, "scope": "openid email" }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("不符合条件"), "{error}");
        let saved = stored_auth(&store, "chatgpt-api");
        assert_eq!(saved["refresh_token"], "rt-5");
        assert!(chatgpt_api_from_auth(&relay, &saved).is_none());

        // 只返回了新 refresh token：也要存下。
        let (_temp, store) = store_with(relay.clone());
        assert!(
            siwc_finish_refresh(&store, &relay, &latest, &json!({ "refresh_token": "rt-6" }))
                .is_err()
        );
        assert_eq!(stored_auth(&store, "chatgpt-api")["refresh_token"], "rt-6");
    }

    #[test]
    fn grok_refresh_persists_a_rotated_refresh_token_even_when_the_response_is_unusable() {
        let mut relay = grok_relay(
            &GrokTokens {
                access_token: "old-at".to_string(),
                refresh_token: "old-rt".to_string(),
                expires_at_ms: 0,
                email: "me@example.com".to_string(),
            },
            &[],
        );
        relay.id = "grok".to_string();
        let latest = account_kind(&relay).unwrap().1;

        let (_temp, store) = store_with(relay.clone());
        assert!(
            grok_finish_refresh(
                &store,
                &relay,
                &latest,
                &json!({ "refresh_token": "rt-2" }),
                "old-rt"
            )
            .is_err()
        );
        assert_eq!(stored_auth(&store, "grok")["refresh_token"], "rt-2");

        let (_temp, store) = store_with(relay.clone());
        let ready = grok_finish_refresh(
            &store,
            &relay,
            &latest,
            &json!({ "access_token": "at-3" }),
            "old-rt",
        )
        .unwrap();
        assert_eq!(ready.api_key, "at-3");
        let saved = stored_auth(&store, "grok");
        assert_eq!(saved["access_token"], "at-3");
        assert_eq!(saved["refresh_token"], "old-rt");
    }

    #[test]
    fn workspace_members_sharing_an_account_id_are_different_logins() {
        let login = |sub: &str, email: &str, account: &str| {
            let mut claims = json!({ "email": email, "https://api.openai.com/auth": { "chatgpt_account_id": account } });
            if !sub.is_empty() {
                claims["sub"] = json!(sub);
            }
            account_from_tokens(
                &json!({ "access_token": "at", "refresh_token": "rt", "id_token": jwt(claims) }),
            )
            .unwrap()
            .auth_contents
        };
        let alice = login("user-a", "a@team.com", "ws-1");
        assert!(same_chatgpt_login(
            &alice,
            &login("user-a", "a@team.com", "ws-1")
        ));
        assert!(
            !same_chatgpt_login(&alice, &login("user-b", "b@team.com", "ws-1")),
            "同一工作区的另一个成员"
        );
        assert!(!same_chatgpt_login(
            &alice,
            &login("user-a", "a@team.com", "ws-2")
        ));
        // 没有 sub 时按邮箱认。
        assert!(same_chatgpt_login(
            &login("", "a@team.com", "ws-1"),
            &login("", "a@team.com", "ws-1")
        ));
        assert!(!same_chatgpt_login(
            &login("", "a@team.com", "ws-1"),
            &login("", "b@team.com", "ws-1")
        ));
    }

    #[tokio::test]
    async fn callback_listener_skips_idle_sockets_and_stale_login_tabs() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let waiting = tokio::spawn(async move {
            wait_for_callback_params(&listener, CALLBACK_PATH, "fresh").await
        });
        // 浏览器预连接、一直不发请求的空连接不能卡住后面的回调。
        let _idle = tokio::net::TcpStream::connect(address).await.unwrap();
        let request = |query: &'static str| async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream
                .write_all(
                    format!("GET {CALLBACK_PATH}?{query} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes(),
                )
                .await
                .unwrap();
            let mut page = String::new();
            stream.read_to_string(&mut page).await.unwrap();
            page
        };
        // 之前放弃的登录页带着旧 state 回调：回一页说明，登录继续等。
        let stale = request("code=old&state=stale").await;
        assert!(stale.contains("已失效"), "{stale}");
        assert!(!waiting.is_finished());
        let fresh = request("code=c1&state=fresh").await;
        assert!(fresh.contains("登录成功"), "{fresh}");
        let params = waiting.await.unwrap().unwrap();
        assert_eq!(params["code"], "c1");
    }
}
