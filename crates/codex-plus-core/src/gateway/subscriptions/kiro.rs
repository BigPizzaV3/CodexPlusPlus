//! Kiro（AWS CodeWhisperer 系）订阅。
//!
//! - 对话：`POST https://runtime.<区域>.kiro.dev/generateAssistantResponse`，请求是
//!   `conversationState`（历史 + 当前消息 + 工具），答复是 AWS event-stream 二进制帧
//!   （这里手写帧解析：prelude、headers、payload、CRC32 校验）。
//! - 登录：导入本机 Kiro IDE（`~/.aws/sso/cache/kiro-auth-token.json`）或 kiro-cli
//!   （`data.sqlite3` 的 `auth_kv`）的登录；浏览器登录走 Kiro 自己的登录页
//!   （Google / GitHub 直接换 token；Builder ID / IAM Identity Center 再跳 AWS 授权码 + PKCE）。
//! - 刷新：社交登录走 Kiro 的 `/refreshToken`，AWS 登录走 OIDC `/token`（refresh_token）。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::gateway::subscriptions::chat::{ChatSse, Usage};
use crate::gateway::subscriptions::editor_kit::{
    self, Emitter, arguments_object, content_text, reasoning_effort, text_of,
};
use crate::gateway::subscriptions::oauth::{LOGIN_TIMEOUT, expiring, now_ms, query};
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const PORTAL: &str = "https://app.kiro.dev";
const AUTH_SERVICE: &str = "https://prod.us-east-1.auth.desktop.kiro.dev";
/// Kiro 登录页会把浏览器送回的本机端口（和 IDE 一样）。
const CALLBACK_PORTS: &[u16] = &[
    3128, 4649, 6588, 8008, 9091, 49153, 50153, 51153, 52153, 53153,
];
const SCOPES: &[&str] = &[
    "codewhisperer:completions",
    "codewhisperer:analysis",
    "codewhisperer:conversations",
    "codewhisperer:transformations",
    "codewhisperer:taskassist",
];
const DESKTOP_UA: &str = "Kiro-Desktop/0.2.13 (darwin; arm64)";
/// Builder ID 没有自己的 profile，Kiro 的客户端用这个服务 profile。
const BUILDER_ID_PROFILE: &str =
    "arn:aws:codewhisperer:us-east-1:638616132270:profile/AAAACCCCXXXX";
const BUILDER_ID_START: &str = "https://view.awsapps.com/start";
/// 快过期前多久刷新。
const REFRESH_AHEAD_MS: u64 = 5 * 60 * 1000;
const NO_RESULT: &str = "Tool use was interrupted and did not produce a result.";
const MAX_TOOL_RESULT: usize = 250_000;
const MAX_FRAME: usize = 16 * 1024 * 1024;

pub struct Kiro;

pub static KIRO: Kiro = Kiro;

fn oidc(region: &str) -> String {
    format!("https://oidc.{region}.amazonaws.com")
}

fn runtime(region: &str) -> String {
    format!("https://runtime.{region}.kiro.dev")
}

fn management(region: &str) -> String {
    format!("https://management.{region}.kiro.dev")
}

fn ide_user_agent() -> String {
    let seed = format!("codex-plus:{}", editor_kit::home().display());
    format!(
        "KiroIDE-1.1.70-{}",
        editor_kit::hex(&editor_kit::sha256(seed.as_bytes()))
    )
}

/* —————————————— 时间 —————————————— */

/// 公历日期 → 自 1970-01-01 的天数（Howard Hinnant 的 days_from_civil）。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// ISO-8601 时间（`2026-10-30T12:00:00.123Z` / `+08:00`）→ 毫秒；解析不了为 0。
fn parse_iso_ms(text: &str) -> u64 {
    let text = text.trim();
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        number(0..4),
        number(5..7),
        number(8..10),
        number(11..13),
        number(14..16),
        number(17..19),
    ) else {
        return 0;
    };
    let mut rest = &text[19..];
    let mut millis = 0_i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
        rest = &fraction[digits.len()..];
        let padded = format!("{digits:0<3}");
        millis = padded[..3].parse().unwrap_or(0);
    }
    let offset = match rest.chars().next() {
        Some('+') | Some('-') => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            let hours = digits
                .get(0..2)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            let minutes = digits
                .get(2..4)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            sign * (hours * 3600 + minutes * 60)
        }
        _ => 0,
    };
    let seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    if seconds < 0 {
        return 0;
    }
    (seconds * 1000 + millis) as u64
}

/* —————————————— 凭据 —————————————— */

fn region_of(credentials: &Value) -> String {
    text_of(credentials, "region")
        .unwrap_or("us-east-1")
        .to_string()
}

/// 对话 / 模型接口的区域：profileArn 的第 4 段；没有时按登录区域选。
fn api_region(credentials: &Value) -> String {
    if let Some(region) = text_of(credentials, "profileArn")
        .and_then(|arn| arn.split(':').nth(3))
        .filter(|region| !region.is_empty())
    {
        return region.to_string();
    }
    if region_of(credentials).starts_with("eu-") {
        "eu-central-1".to_string()
    } else {
        "us-east-1".to_string()
    }
}

fn access_token(credentials: &Value) -> anyhow::Result<String> {
    text_of(credentials, "accessToken")
        .map(str::to_string)
        .ok_or_else(|| fail(401, "Kiro 凭据里没有 access token，请重新登录"))
}

fn is_builder_id(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("builderid")
}

/// IDE 的 `kiro-auth-token.json`（加上 `<clientIdHash>.json` 里的客户端注册）→ 凭据。
fn parse_ide_token(token: &Value, registration: Option<&Value>) -> anyhow::Result<Value> {
    let access = text_of(token, "accessToken")
        .ok_or_else(|| anyhow::anyhow!("Kiro 的登录文件里没有 accessToken，请先在 Kiro 里登录"))?;
    let client_id = registration
        .and_then(|value| text_of(value, "clientId"))
        .unwrap_or_default();
    let client_secret = registration
        .and_then(|value| text_of(value, "clientSecret"))
        .unwrap_or_default();
    let social = text_of(token, "authMethod")
        .is_some_and(|method| method.eq_ignore_ascii_case("social"))
        || client_id.is_empty();
    let provider = text_of(token, "provider").unwrap_or_default();
    let mut credentials = json!({
        "accessToken": access,
        "refreshToken": text_of(token, "refreshToken").unwrap_or_default(),
        "expiresAt": text_of(token, "expiresAt").map(parse_iso_ms).unwrap_or(0),
        "authMethod": if social { "social" } else { "idc" },
        "provider": provider,
        "region": text_of(token, "region").unwrap_or("us-east-1"),
        "source": "ide",
    });
    if let Some(arn) = text_of(token, "profileArn") {
        credentials["profileArn"] = json!(arn);
    } else if is_builder_id(provider) {
        credentials["profileArn"] = json!(BUILDER_ID_PROFILE);
    }
    if !social {
        credentials["clientId"] = json!(client_id);
        credentials["clientSecret"] = json!(client_secret);
    }
    Ok(credentials)
}

/// kiro-cli `auth_kv` 里的一条 token（snake_case）→ 凭据。
fn parse_cli_token(kind: &str, token: &Value, registration: Option<&Value>) -> Option<Value> {
    let access = text_of(token, "access_token")?;
    let social = kind == "social";
    let start_url = text_of(token, "start_url").unwrap_or_default();
    let builder = !social && (start_url.is_empty() || start_url == BUILDER_ID_START);
    let mut credentials = json!({
        "accessToken": access,
        "refreshToken": text_of(token, "refresh_token").unwrap_or_default(),
        "expiresAt": text_of(token, "expires_at").map(parse_iso_ms).unwrap_or(0),
        "authMethod": if social { "social" } else { "idc" },
        "provider": if social { "" } else if builder { "BuilderId" } else { "Enterprise" },
        "region": text_of(token, "region").unwrap_or("us-east-1"),
        "source": "cli",
    });
    if let Some(arn) = text_of(token, "profile_arn") {
        credentials["profileArn"] = json!(arn);
    } else if builder {
        credentials["profileArn"] = json!(BUILDER_ID_PROFILE);
    }
    if !social {
        credentials["clientId"] = json!(
            registration
                .and_then(|value| text_of(value, "client_id"))
                .unwrap_or_default()
        );
        credentials["clientSecret"] = json!(
            registration
                .and_then(|value| text_of(value, "client_secret"))
                .unwrap_or_default()
        );
    }
    Some(credentials)
}

fn ide_dir() -> std::path::PathBuf {
    editor_kit::home().join(".aws").join("sso").join("cache")
}

fn read_ide() -> anyhow::Result<Value> {
    let token = editor_kit::read_json_file(&ide_dir().join("kiro-auth-token.json"))?;
    let registration = text_of(&token, "clientIdHash")
        .and_then(|hash| editor_kit::read_json_file(&ide_dir().join(format!("{hash}.json"))).ok());
    parse_ide_token(&token, registration.as_ref())
}

fn cli_db() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| editor_kit::home().join(".local").join("share"))
        .join("kiro-cli")
        .join("data.sqlite3")
}

fn read_cli() -> anyhow::Result<Value> {
    let path = cli_db();
    let db =
        rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("没找到 {}，请先用 kiro-cli login 登录", path.display()))?;
    let get = |key: &str| -> Option<Value> {
        let text: String = db
            .query_row("SELECT value FROM auth_kv WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .ok()?;
        serde_json::from_str(&text).ok()
    };
    for kind in ["social", "odic"] {
        let Some(token) = get(&format!("kirocli:{kind}:token")) else {
            continue;
        };
        let registration = get("kirocli:odic:device-registration");
        if let Some(credentials) = parse_cli_token(kind, &token, registration.as_ref()) {
            return Ok(credentials);
        }
    }
    bail!("kiro-cli 没有登录（或用的是公司自己的身份提供商，暂不支持）")
}

/// 刷新 token。
async fn refresh_token(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<Value> {
    let refresh = text_of(credentials, "refreshToken")
        .ok_or_else(|| fail(401, "Kiro 的登录没有 refresh token，请重新登录"))?;
    let region = region_of(credentials);
    let social = text_of(credentials, "authMethod") != Some("idc");
    let response = if social {
        http.post(format!(
            "https://prod.{region}.auth.desktop.kiro.dev/refreshToken"
        ))
        .header("user-agent", DESKTOP_UA)
        .header("accept", "application/json")
        .json(&json!({ "refreshToken": refresh }))
        .timeout(Duration::from_secs(30))
        .send()
        .await
    } else {
        http.post(format!("{}/token", oidc(&region)))
            .header("accept", "application/json")
            .json(&json!({
                "clientId": text_of(credentials, "clientId").unwrap_or_default(),
                "clientSecret": text_of(credentials, "clientSecret").unwrap_or_default(),
                "refreshToken": refresh,
                "grantType": "refresh_token",
            }))
            .timeout(Duration::from_secs(30))
            .send()
            .await
    }
    .map_err(|error| fail(502, format!("无法连接 Kiro 登录服务：{error}")))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if matches!(status, 400 | 401 | 403) {
        return Err(fail(
            401,
            format!("Kiro 的登录已失效（{status}），请重新登录"),
        ));
    }
    if !(200..300).contains(&status) {
        return Err(fail(
            502,
            format!(
                "刷新 Kiro 登录失败（{status}）：{}",
                text.chars().take(300).collect::<String>()
            ),
        ));
    }
    let value: Value = serde_json::from_str(&text).context("Kiro 刷新答复不是 JSON")?;
    Ok(apply_tokens(credentials, &value))
}

/// 把 token 答复（accessToken、refreshToken、expiresIn、profileArn）并进凭据。
fn apply_tokens(credentials: &Value, tokens: &Value) -> Value {
    let mut updated = credentials.clone();
    if let Some(access) = text_of(tokens, "accessToken") {
        updated["accessToken"] = json!(access);
    }
    if let Some(refresh) = text_of(tokens, "refreshToken") {
        updated["refreshToken"] = json!(refresh);
    }
    let expires_in = tokens
        .get("expiresIn")
        .and_then(Value::as_u64)
        .unwrap_or(3600);
    updated["expiresAt"] = json!(now_ms() + expires_in * 1000);
    if let Some(arn) = text_of(tokens, "profileArn") {
        updated["profileArn"] = json!(arn);
    }
    updated
}

async fn management_get(
    http: &reqwest::Client,
    region: &str,
    token: &str,
    method: &str,
    params: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let response = http
        .get(format!("{}/{method}?{}", management(region), query(params)))
        .bearer_auth(token)
        .header("accept", "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        return Err(fail(
            status,
            format!(
                "Kiro {method} 返回 {status}：{}",
                text.chars().take(300).collect::<String>()
            ),
        ));
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// 没有 profileArn 时去列一个（Builder ID 用固定的服务 profile）。
async fn profile_of(http: &reqwest::Client, credentials: &Value) -> anyhow::Result<String> {
    if let Some(arn) = text_of(credentials, "profileArn") {
        return Ok(arn.to_string());
    }
    if is_builder_id(text_of(credentials, "provider").unwrap_or_default()) {
        return Ok(BUILDER_ID_PROFILE.to_string());
    }
    let token = access_token(credentials)?;
    let mut last = None;
    for region in ["us-east-1", "eu-central-1"] {
        let response = http
            .post(format!("{}/List-Available-Profiles", management(region)))
            .bearer_auth(&token)
            .header("accept", "application/json")
            .json(&json!({}))
            .timeout(Duration::from_secs(20))
            .send()
            .await;
        match response {
            Ok(response) => {
                let status = response.status().as_u16();
                let text = response.text().await.unwrap_or_default();
                if (200..300).contains(&status) {
                    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                    if let Some(arn) = value
                        .get("profiles")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .find_map(|profile| text_of(profile, "arn"))
                    {
                        return Ok(arn.to_string());
                    }
                } else if status == 403
                    && text.contains("Builder ID")
                    && text_of(credentials, "authMethod") == Some("idc")
                {
                    return Ok(BUILDER_ID_PROFILE.to_string());
                } else {
                    last = Some(format!(
                        "{status}：{}",
                        text.chars().take(200).collect::<String>()
                    ));
                }
            }
            Err(error) => last = Some(error.to_string()),
        }
    }
    bail!(
        "Kiro 没有这个账号的 profile（{}）",
        last.unwrap_or_default()
    )
}

/// 补齐 profileArn，读账号邮箱，得到登录结果。
async fn finish_credentials(
    http: &reqwest::Client,
    mut credentials: Value,
) -> anyhow::Result<LoginDone> {
    if expiring(&credentials, "expiresAt", REFRESH_AHEAD_MS) {
        credentials = refresh_token(http, &credentials).await?;
    }
    let profile = profile_of(http, &credentials).await?;
    credentials["profileArn"] = json!(profile);
    let token = access_token(&credentials)?;
    let limits = management_get(
        http,
        &api_region(&credentials),
        &token,
        "Get-Usage-Limits",
        &[
            ("origin", "KIRO_CLI"),
            ("profileArn", &profile),
            ("resourceType", "CREDIT"),
            ("isEmailRequired", "true"),
        ],
    )
    .await
    .unwrap_or(Value::Null);
    let email = limits
        .pointer("/userInfo/email")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(plan) = limits
        .pointer("/subscriptionInfo/subscriptionTitle")
        .and_then(Value::as_str)
    {
        credentials["plan"] = json!(plan);
    }
    let provider = text_of(&credentials, "provider")
        .unwrap_or_default()
        .to_string();
    let account = match (email.is_empty(), provider.is_empty()) {
        (true, _) => "Kiro".to_string(),
        (false, true) => email,
        (false, false) => format!("{email} ({provider})"),
    };
    credentials["account"] = json!(account);
    Ok(LoginDone {
        credentials,
        account,
    })
}

/* —————————————— 浏览器登录 —————————————— */

struct BrowserLogin {
    listener: tokio::net::TcpListener,
    port: u16,
    state: String,
    verifier: String,
}

/// 跳到 AWS 之后的现场。
struct AwsStage {
    region: String,
    client_id: String,
    client_secret: String,
    provider: &'static str,
    state: String,
    verifier: String,
}

async fn sign_in_post(http: &reqwest::Client, url: &str, body: &Value) -> anyhow::Result<Value> {
    let response = http
        .post(url)
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .header("user-agent", ide_user_agent())
        .json(body)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let detail = ["error_description", "message", "error"]
            .iter()
            .find_map(|key| text_of(&value, key))
            .map(str::to_string)
            .unwrap_or_else(|| text.chars().take(200).collect());
        bail!("登录被拒绝（{status}）：{detail}");
    }
    serde_json::from_str(&text).context("登录答复不是 JSON")
}

fn valid_region(region: &str) -> bool {
    let parts: Vec<&str> = region.split('-').collect();
    parts.len() >= 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_alphanumeric()))
        && parts
            .last()
            .is_some_and(|part| part.chars().all(|ch| ch.is_ascii_digit()))
}

async fn wait_browser(http: &reqwest::Client, login: BrowserLogin) -> anyhow::Result<Value> {
    let BrowserLogin {
        listener,
        port,
        state,
        verifier,
    } = login;
    let redirect = format!("http://localhost:{port}");
    let aws_redirect = format!("http://127.0.0.1:{port}/oauth/callback");
    let mut aws: Option<AwsStage> = None;
    let html = "text/html; charset=utf-8";
    tokio::time::timeout(LOGIN_TIMEOUT, async {
        loop {
            let (mut stream, request) = editor_kit::next_request(&listener).await?;
            if request.path != "/oauth/callback" && request.path != "/signin/callback" {
                editor_kit::respond(&mut stream, "404 Not Found", &[], "", b"").await;
                continue;
            }
            let params = &request.query;
            let got = params.get("state").map(String::as_str).unwrap_or_default();
            let from_aws = aws.as_ref().is_some_and(|stage| stage.state == got);
            if got != state && !from_aws {
                editor_kit::respond(
                    &mut stream,
                    "404 Not Found",
                    &[],
                    html,
                    &editor_kit::page("这个登录页不属于这次登录，请回到 Codex++ 重新开始。"),
                )
                .await;
                continue;
            }
            if let Some(error) = params.get("error").filter(|error| !error.is_empty()) {
                let detail = params
                    .get("error_description")
                    .cloned()
                    .unwrap_or_else(|| error.clone());
                editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page("登录没有完成，请回到 Codex++ 重新登录。")).await;
                bail!("登录失败：{detail}");
            }
            let code = params.get("code").cloned().unwrap_or_default();
            if from_aws {
                let stage = aws.as_ref().expect("checked");
                if request.path != "/oauth/callback" || code.is_empty() {
                    editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page("AWS 没有返回授权码，请重新登录。")).await;
                    bail!("AWS 没有返回授权码");
                }
                let tokens = sign_in_post(
                    http,
                    &format!("{}/token", oidc(&stage.region)),
                    &json!({
                        "clientId": stage.client_id,
                        "clientSecret": stage.client_secret,
                        "grantType": "authorization_code",
                        "redirectUri": aws_redirect,
                        "code": code,
                        "codeVerifier": stage.verifier,
                    }),
                )
                .await;
                let page = if tokens.is_ok() { "登录成功，可以回到 Codex++ 了。" } else { "登录没有完成，请回到 Codex++ 重新登录。" };
                editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page(page)).await;
                let base = json!({
                    "authMethod": "idc",
                    "provider": stage.provider,
                    "region": stage.region,
                    "clientId": stage.client_id,
                    "clientSecret": stage.client_secret,
                    "source": "browser",
                });
                let mut credentials = apply_tokens(&base, &tokens?);
                if is_builder_id(stage.provider) {
                    credentials["profileArn"] = json!(BUILDER_ID_PROFILE);
                }
                return anyhow::Ok(credentials);
            }
            let option = params.get("login_option").cloned().unwrap_or_default();
            match option.as_str() {
                "google" | "github" => {
                    if code.is_empty() {
                        editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page("Kiro 没有返回授权码，请重新登录。")).await;
                        bail!("Kiro 登录页没有返回授权码");
                    }
                    let tokens = sign_in_post(
                        http,
                        &format!("{AUTH_SERVICE}/oauth/token"),
                        &json!({
                            "code": code,
                            "code_verifier": verifier,
                            "redirect_uri": format!("{redirect}{}?login_option={option}", request.path),
                        }),
                    )
                    .await;
                    let page = if tokens.is_ok() { "登录成功，可以回到 Codex++ 了。" } else { "登录没有完成，请回到 Codex++ 重新登录。" };
                    editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page(page)).await;
                    let base = json!({
                        "authMethod": "social",
                        "provider": if option == "google" { "Google" } else { "Github" },
                        "region": "us-east-1",
                        "source": "browser",
                    });
                    return Ok(apply_tokens(&base, &tokens?));
                }
                "builderid" | "awsidc" | "internal" => {
                    let issuer = params.get("issuer_url").cloned().unwrap_or_default();
                    let region = params.get("idc_region").cloned().unwrap_or_default();
                    if !issuer.starts_with("https://") || !valid_region(&region) {
                        editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page("Kiro 登录页没有给出有效的 AWS 登录地址。")).await;
                        bail!("Kiro 登录页没有给出有效的 AWS 登录地址");
                    }
                    let registration = sign_in_post(
                        http,
                        &format!("{}/client/register", oidc(&region)),
                        &json!({
                            "clientName": "Kiro IDE",
                            "clientType": "public",
                            "scopes": SCOPES,
                            "grantTypes": ["authorization_code", "refresh_token"],
                            "redirectUris": ["http://127.0.0.1/oauth/callback"],
                            "issuerUrl": issuer,
                        }),
                    )
                    .await?;
                    let (aws_verifier, challenge) = editor_kit::pkce(48);
                    let stage = AwsStage {
                        region: region.clone(),
                        client_id: text_of(&registration, "clientId").unwrap_or_default().to_string(),
                        client_secret: text_of(&registration, "clientSecret").unwrap_or_default().to_string(),
                        provider: match option.as_str() {
                            "builderid" => "BuilderId",
                            "awsidc" => "Enterprise",
                            _ => "Internal",
                        },
                        state: editor_kit::b64url(&editor_kit::random_bytes(24)),
                        verifier: aws_verifier,
                    };
                    if stage.client_id.is_empty() {
                        bail!("AWS 没有注册登录客户端");
                    }
                    let scopes = SCOPES.join(",");
                    let location = format!(
                        "{}/authorize?{}",
                        oidc(&region),
                        query(&[
                            ("response_type", "code"),
                            ("client_id", &stage.client_id),
                            ("redirect_uri", &aws_redirect),
                            ("scopes", &scopes),
                            ("state", &stage.state),
                            ("code_challenge", &challenge),
                            ("code_challenge_method", "S256"),
                        ])
                    );
                    editor_kit::respond(&mut stream, "302 Found", &[("location", location.as_str())], "", b"").await;
                    aws = Some(stage);
                }
                other => {
                    editor_kit::respond(&mut stream, "200 OK", &[], html, &editor_kit::page("这种登录方式暂不支持，请换 Google、GitHub 或 Builder ID。")).await;
                    bail!("Kiro 登录页返回了不支持的登录方式：{other}");
                }
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成 Kiro 登录"))?
}

/* —————————————— AWS event-stream —————————————— */

fn crc32_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0_u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = if value & 1 != 0 {
                    0xEDB8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            *slot = value;
        }
        table
    })
}

/// CRC-32/IEEE（zlib.crc32）。
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let table = crc32_table();
    !bytes.iter().fold(!0_u32, |crc, byte| {
        table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8)
    })
}

/// 一帧：字符串头（其余类型的头只跳过）和 payload。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Frame {
    pub headers: Vec<(String, String)>,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// 解析 headers 段。
fn parse_headers(mut bytes: &[u8]) -> anyhow::Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let take = |bytes: &mut &[u8], count: usize| -> anyhow::Result<Vec<u8>> {
        if bytes.len() < count {
            bail!("event-stream 帧头被截断");
        }
        let (head, rest) = bytes.split_at(count);
        *bytes = rest;
        Ok(head.to_vec())
    };
    while !bytes.is_empty() {
        let name_len = take(&mut bytes, 1)?[0] as usize;
        let name = String::from_utf8_lossy(&take(&mut bytes, name_len)?).to_string();
        let kind = take(&mut bytes, 1)?[0];
        match kind {
            0 | 1 => {}
            2 => {
                take(&mut bytes, 1)?;
            }
            3 => {
                take(&mut bytes, 2)?;
            }
            4 => {
                take(&mut bytes, 4)?;
            }
            5 | 8 => {
                take(&mut bytes, 8)?;
            }
            9 => {
                take(&mut bytes, 16)?;
            }
            6 | 7 => {
                let length = take(&mut bytes, 2)?;
                let length = u16::from_be_bytes([length[0], length[1]]) as usize;
                let value = take(&mut bytes, length)?;
                if kind == 7 {
                    out.push((name, String::from_utf8_lossy(&value).to_string()));
                }
            }
            other => bail!("event-stream 帧头类型未知：{other}"),
        }
    }
    Ok(out)
}

/// 喂字节、吐整帧；`verify` 时校验两个 CRC。
#[derive(Debug)]
pub(crate) struct EventStreamReader {
    buffer: Vec<u8>,
    verify: bool,
}

impl Default for EventStreamReader {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            verify: true,
        }
    }
}

impl EventStreamReader {
    pub fn push(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<Frame>> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        loop {
            if self.buffer.len() < 12 {
                break;
            }
            let word = |at: usize| {
                u32::from_be_bytes([
                    self.buffer[at],
                    self.buffer[at + 1],
                    self.buffer[at + 2],
                    self.buffer[at + 3],
                ])
            };
            let total = word(0) as usize;
            let header_len = word(4) as usize;
            if !(16..=MAX_FRAME).contains(&total) || header_len > total - 16 {
                bail!("event-stream 帧长度不对（{total} / {header_len}）");
            }
            if self.verify && crc32(&self.buffer[..8]) != word(8) {
                bail!("event-stream 帧的 prelude CRC 不对");
            }
            if self.buffer.len() < total {
                break;
            }
            let frame: Vec<u8> = self.buffer.drain(..total).collect();
            let message_crc = u32::from_be_bytes([
                frame[total - 4],
                frame[total - 3],
                frame[total - 2],
                frame[total - 1],
            ]);
            if self.verify && crc32(&frame[..total - 4]) != message_crc {
                bail!("event-stream 帧的 CRC 不对");
            }
            let headers = parse_headers(&frame[12..12 + header_len])?;
            let payload = frame[12 + header_len..total - 4].to_vec();
            frames.push(Frame { headers, payload });
        }
        Ok(frames)
    }

    /// 流结束时还剩半帧：断流。
    pub fn finish(&self) -> anyhow::Result<()> {
        if self.buffer.is_empty() {
            Ok(())
        } else {
            bail!("event-stream 在帧中间断开（unexpected EOF）")
        }
    }
}

/// 编一帧（只用字符串头）；测试用。
#[cfg(test)]
pub(crate) fn encode_frame(headers: &[(&str, &str)], payload: &[u8]) -> Vec<u8> {
    let mut head = Vec::new();
    for (name, value) in headers {
        head.push(name.len() as u8);
        head.extend_from_slice(name.as_bytes());
        head.push(7);
        head.extend_from_slice(&(value.len() as u16).to_be_bytes());
        head.extend_from_slice(value.as_bytes());
    }
    let total = 12 + head.len() + payload.len() + 4;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(head.len() as u32).to_be_bytes());
    let prelude_crc = crc32(&out);
    out.extend_from_slice(&prelude_crc.to_be_bytes());
    out.extend_from_slice(&head);
    out.extend_from_slice(payload);
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

/* —————————————— 请求 —————————————— */

#[derive(Debug, Clone, PartialEq)]
struct ToolResult {
    id: String,
    text: String,
    error: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct ToolUse {
    id: String,
    name: String,
    input: Value,
}

#[derive(Debug, Clone, PartialEq)]
enum Turn {
    User {
        text: String,
        images: Vec<Value>,
        results: Vec<ToolResult>,
    },
    Assistant {
        text: String,
        uses: Vec<ToolUse>,
    },
}

fn join_text(left: &mut String, right: &str) {
    if right.is_empty() {
        return;
    }
    if !left.is_empty() {
        left.push_str("\n\n");
    }
    left.push_str(right);
}

/// 工具名：Kiro 只认 `[A-Za-z0-9_-]{1,64}`，不合法的换成 `_` 截短再接哈希。
fn safe_tool_name(name: &str) -> String {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    if valid {
        return name.to_string();
    }
    let cleaned: String = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .take(51)
        .collect();
    let digest = editor_kit::hex(&editor_kit::sha256(name.as_bytes()));
    format!("{cleaned}_{}", &digest[..12])
}

/// 工具调用 id：Kiro 只认 `[A-Za-z0-9_.:-]{1,64}`。
fn safe_tool_id(id: &str) -> String {
    let valid = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | ':' | '-'));
    if valid {
        return id.to_string();
    }
    let digest = editor_kit::b64url(&editor_kit::sha256(id.as_bytes()));
    format!("t_{}", &digest[..32.min(digest.len())])
}

/// 工具 schema：根上补 `type: object`，`["object","null"]` 收成 `object`。
fn clean_schema(schema: Option<&Value>) -> Value {
    let Some(Value::Object(object)) = schema else {
        return json!({ "type": "object", "properties": {} });
    };
    if object.is_empty() {
        return json!({ "type": "object", "properties": {} });
    }
    let mut object = object.clone();
    match object.get("type") {
        None => {
            object.insert("type".to_string(), json!("object"));
        }
        Some(Value::Array(types))
            if types.iter().any(|item| item == "object")
                && types.iter().all(|item| item == "object" || item == "null") =>
        {
            object.insert("type".to_string(), json!("object"));
        }
        _ => {}
    }
    Value::Object(object)
}

/// data URL 图片 → Kiro 的图片。
fn kiro_image(url: &str) -> Option<Value> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.split(';').next()?;
    let format = mime.strip_prefix("image/")?;
    let format = if format == "jpg" { "jpeg" } else { format };
    Some(json!({ "format": format, "source": { "bytes": data } }))
}

/// Chat 消息 → 规整后的轮次（同角色合并、首尾是用户、工具结果配对）。
fn turns_of(messages: &[Value]) -> (String, Vec<Turn>) {
    let mut system = String::new();
    let mut turns: Vec<Turn> = Vec::new();
    let push = |turns: &mut Vec<Turn>, turn: Turn| match (turns.last_mut(), turn) {
        (
            Some(Turn::User {
                text,
                images,
                results,
            }),
            Turn::User {
                text: more,
                images: more_images,
                results: more_results,
            },
        ) => {
            join_text(text, &more);
            images.extend(more_images);
            results.extend(more_results);
        }
        (
            Some(Turn::Assistant { text, uses }),
            Turn::Assistant {
                text: more,
                uses: more_uses,
            },
        ) => {
            join_text(text, &more);
            uses.extend(more_uses);
        }
        (_, turn) => turns.push(turn),
    };
    for message in messages {
        match message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user")
        {
            "system" | "developer" => join_text(&mut system, &content_text(message)),
            "assistant" => {
                let uses = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|call| ToolUse {
                        id: safe_tool_id(
                            call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        ),
                        name: safe_tool_name(
                            call.pointer("/function/name")
                                .and_then(Value::as_str)
                                .unwrap_or_default(),
                        ),
                        input: arguments_object(call.pointer("/function/arguments")),
                    })
                    .collect::<Vec<_>>();
                let text = content_text(message);
                // 只有思考、没有文字也没有工具调用的 assistant 丢掉。
                if text.trim().is_empty() && uses.is_empty() {
                    continue;
                }
                push(&mut turns, Turn::Assistant { text, uses });
            }
            "tool" => {
                let mut text = content_text(message);
                if text.chars().count() > MAX_TOOL_RESULT {
                    text = text.chars().take(MAX_TOOL_RESULT).collect::<String>() + "\n… (cut)";
                }
                if text.is_empty() {
                    text = "(no output)".to_string();
                }
                let result = ToolResult {
                    id: safe_tool_id(
                        message
                            .get("tool_call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ),
                    text,
                    error: false,
                };
                push(
                    &mut turns,
                    Turn::User {
                        text: String::new(),
                        images: Vec::new(),
                        results: vec![result],
                    },
                );
            }
            _ => {
                let images = message
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|part| part.get("type").and_then(Value::as_str) == Some("image_url"))
                    .filter_map(|part| {
                        let url = part
                            .pointer("/image_url/url")
                            .or_else(|| part.get("image_url"))
                            .and_then(Value::as_str)?;
                        kiro_image(url)
                    })
                    .collect();
                push(
                    &mut turns,
                    Turn::User {
                        text: content_text(message),
                        images,
                        results: Vec::new(),
                    },
                );
            }
        }
    }
    // 开头必须是用户：去掉开头的 assistant 和只有工具结果的用户。
    while let Some(first) = turns.first() {
        let drop = match first {
            Turn::Assistant { .. } => true,
            Turn::User { text, images, .. } => text.trim().is_empty() && images.is_empty(),
        };
        if !drop {
            break;
        }
        turns.remove(0);
    }
    if let Some(Turn::User { results, .. }) = turns.first_mut() {
        results.clear();
    }
    // 结尾必须是用户。
    if !matches!(turns.last(), Some(Turn::User { .. })) {
        turns.push(Turn::User {
            text: String::new(),
            images: Vec::new(),
            results: Vec::new(),
        });
    }
    // 工具结果只留上一条 assistant 调用过的，缺的补「被中断」。
    for index in 0..turns.len() {
        let called: Vec<String> = match index.checked_sub(1).map(|previous| &turns[previous]) {
            Some(Turn::Assistant { uses, .. }) => uses.iter().map(|item| item.id.clone()).collect(),
            _ => Vec::new(),
        };
        if let Turn::User {
            text,
            images,
            results,
        } = &mut turns[index]
        {
            let mut seen = HashSet::new();
            results.retain(|result| called.contains(&result.id) && seen.insert(result.id.clone()));
            for id in &called {
                if !results.iter().any(|result| &result.id == id) {
                    results.push(ToolResult {
                        id: id.clone(),
                        text: NO_RESULT.to_string(),
                        error: true,
                    });
                }
            }
            if text.trim().is_empty() && results.is_empty() && images.is_empty() {
                *text = "Please proceed with the task.".to_string();
            }
        }
    }
    (system, turns)
}

/// 开思考：只有 Claude 系和 auto，按档位给预算。
fn thinking_budget(model: &str, effort: Option<&str>) -> Option<u64> {
    let effort = effort?;
    if !(model.to_ascii_lowercase().contains("claude") || model == "auto") {
        return None;
    }
    match effort {
        "none" | "off" => None,
        "minimal" | "low" => Some(10_000),
        "medium" => Some(20_000),
        "high" => Some(30_000),
        _ => Some(50_000),
    }
}

/// Chat 请求 → Kiro 请求体。返回 (请求体, Kiro 工具名 → 原名, 是否开了思考)。
fn build_request(
    request: &Value,
    profile_arn: Option<&str>,
) -> (Value, HashMap<String, String>, bool) {
    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("auto");
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let (mut system, mut turns) = turns_of(&messages);
    let effort = reasoning_effort(request);
    let budget = thinking_budget(model, effort.as_deref());
    if let Some(budget) = budget {
        system = format!(
            "<thinking_mode>enabled</thinking_mode><max_thinking_length>{budget}</max_thinking_length>\n{system}"
        );
    }
    // 系统提示放进第一条用户消息开头。
    if !system.trim().is_empty()
        && let Some(Turn::User { text, .. }) = turns.first_mut()
    {
        *text = if text.is_empty() {
            system.clone()
        } else {
            format!("{system}\n\n{text}")
        };
    }
    // 图片只留在最近一条带图的用户消息上。
    let last_image = turns
        .iter()
        .rposition(|turn| matches!(turn, Turn::User { images, .. } if !images.is_empty()));
    // 工具：调用方给的 + 历史里用过但没声明的。
    let choice_none = request.get("tool_choice").and_then(Value::as_str) == Some("none");
    let mut names = HashMap::new();
    let mut specs = Vec::new();
    let mut declared = HashSet::new();
    if !choice_none {
        for tool in request
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(function) = tool.get("function") else {
                continue;
            };
            let Some(name) = function.get("name").and_then(Value::as_str) else {
                continue;
            };
            let safe = safe_tool_name(name);
            names.insert(safe.clone(), name.to_string());
            if !declared.insert(safe.clone()) {
                continue;
            }
            let description = function
                .get("description")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or(name);
            specs.push(json!({ "toolSpecification": {
                "name": safe,
                "description": description,
                "inputSchema": { "json": clean_schema(function.get("parameters")) },
            } }));
        }
    }
    for turn in &turns {
        if let Turn::Assistant { uses, .. } = turn {
            for item in uses {
                if declared.insert(item.name.clone()) {
                    specs.push(json!({ "toolSpecification": {
                        "name": item.name,
                        "description": "Tool",
                        "inputSchema": { "json": { "type": "object", "properties": {} } },
                    } }));
                }
            }
        }
    }
    let last = turns.len() - 1;
    let user_message = |index: usize, text: &str, images: &[Value], results: &[ToolResult]| {
        let mut message = json!({ "content": text, "modelId": model, "origin": "KIRO_CLI" });
        if Some(index) == last_image && !images.is_empty() {
            message["images"] = json!(images);
        }
        let mut context = serde_json::Map::new();
        if !results.is_empty() {
            context.insert(
                "toolResults".to_string(),
                json!(
                    results
                        .iter()
                        .map(|result| json!({
                            "toolUseId": result.id,
                            "status": if result.error { "error" } else { "success" },
                            "content": [{ "text": result.text }],
                        }))
                        .collect::<Vec<_>>()
                ),
            );
        }
        if index == last && !specs.is_empty() {
            context.insert("tools".to_string(), json!(specs));
        }
        if !context.is_empty() {
            message["userInputMessageContext"] = Value::Object(context);
        }
        message
    };
    let mut history = Vec::new();
    let mut current = Value::Null;
    for (index, turn) in turns.iter().enumerate() {
        match turn {
            Turn::User {
                text,
                images,
                results,
            } => {
                let message = user_message(index, text, images, results);
                if index == last {
                    current = message;
                } else {
                    history.push(json!({ "userInputMessage": message }));
                }
            }
            Turn::Assistant { text, uses } => {
                let mut message = json!({ "content": text });
                if !uses.is_empty() {
                    message["toolUses"] = json!(uses
                        .iter()
                        .map(|item| json!({ "toolUseId": item.id, "name": item.name, "input": item.input }))
                        .collect::<Vec<_>>());
                }
                history.push(json!({ "assistantResponseMessage": message }));
            }
        }
    }
    let mut state = json!({
        "chatTriggerType": "MANUAL",
        "agentTaskType": "vibe",
        "conversationId": uuid::Uuid::new_v4().to_string(),
        "currentMessage": { "userInputMessage": current },
    });
    if !history.is_empty() {
        state["history"] = json!(history);
    }
    let mut body = json!({ "conversationState": state, "agentMode": "vibe" });
    if let Some(arn) = profile_arn.filter(|arn| !arn.is_empty()) {
        body["profileArn"] = json!(arn);
    }
    (body, names, budget.is_some())
}

/* —————————————— 答复 —————————————— */

/// 开思考时文本开头的 `<thinking>…</thinking>` 拆成推理（标签可能跨分片）。
#[derive(Debug, Default)]
struct ThinkingSplitter {
    /// 0：还没确定开头；1：在思考里；2：思考已结束。
    stage: u8,
    buffer: String,
}

const OPEN_TAG: &str = "<thinking>";
const CLOSE_TAG: &str = "</thinking>";

impl ThinkingSplitter {
    /// 返回 (推理, 文本)。
    fn push(&mut self, text: &str) -> (String, String) {
        self.buffer.push_str(text);
        let mut reasoning = String::new();
        let mut visible = String::new();
        loop {
            match self.stage {
                0 => {
                    let trimmed = self.buffer.trim_start();
                    if trimmed.starts_with(OPEN_TAG) {
                        let start = self.buffer.len() - trimmed.len() + OPEN_TAG.len();
                        self.buffer = self.buffer[start..].to_string();
                        self.stage = 1;
                        continue;
                    }
                    if OPEN_TAG.starts_with(trimmed) {
                        break; // 还不够判断
                    }
                    self.stage = 2;
                }
                1 => {
                    if let Some(position) = self.buffer.find(CLOSE_TAG) {
                        reasoning.push_str(&self.buffer[..position]);
                        let rest = self.buffer[position + CLOSE_TAG.len()..]
                            .trim_start_matches(['\n', '\r'])
                            .to_string();
                        self.buffer = rest;
                        self.stage = 2;
                        continue;
                    }
                    // 留下可能是半个结束标签的尾巴。
                    let mut keep = self.buffer.len().saturating_sub(CLOSE_TAG.len() - 1);
                    while !self.buffer.is_char_boundary(keep) {
                        keep -= 1;
                    }
                    reasoning.push_str(&self.buffer[..keep]);
                    self.buffer = self.buffer[keep..].to_string();
                    break;
                }
                _ => {
                    visible.push_str(&self.buffer);
                    self.buffer.clear();
                    break;
                }
            }
        }
        (reasoning, visible)
    }

    fn finish(&mut self) -> (String, String) {
        let rest = std::mem::take(&mut self.buffer);
        match self.stage {
            1 => (rest, String::new()),
            _ => (String::new(), rest),
        }
    }
}

fn input_limits() -> &'static Mutex<HashMap<String, u64>> {
    static LIMITS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    LIMITS.get_or_init(Default::default)
}

/// 错误信息 → 状态码（还没输出内容时用）。
fn error_status(kind: &str, message: &str) -> u16 {
    let text = format!("{kind} {message}");
    let lower = text.to_ascii_lowercase();
    if text.contains("CONTENT_LENGTH_EXCEEDS_THRESHOLD") || lower.contains("input is too long") {
        return 400;
    }
    if text.contains("INSUFFICIENT_MODEL_CAPACITY") {
        return 503;
    }
    if text.contains("MONTHLY_REQUEST_COUNT") || text.contains("USAGE_LIMIT") {
        return 429;
    }
    let quota = [
        "throttl",
        "quota",
        "insufficient",
        "balance",
        "credit",
        "billing",
        "exceeded",
        "rate limit",
        "ratelimit",
        "usage limit",
        "limit reached",
        "too many requests",
        "overloaded",
    ];
    if quota.iter().any(|word| lower.contains(word)) {
        return 429;
    }
    if lower.contains("accessdenied") || lower.contains("unauthorized") || lower.contains("expired")
    {
        return 401;
    }
    502
}

/// event-stream 帧 → Chat SSE。
struct KiroEvents {
    names: HashMap<String, String>,
    splitter: Option<ThinkingSplitter>,
    /// 工具调用 id → (已发出的参数, 是否已经开过)。
    tools: HashMap<String, String>,
    stop: Option<&'static str>,
    chars: usize,
    context_percent: Option<f64>,
    input_limit: u64,
    prompt_estimate: u64,
    token_usage: Option<Usage>,
}

impl KiroEvents {
    fn new(
        names: HashMap<String, String>,
        thinking: bool,
        input_limit: u64,
        prompt_estimate: u64,
    ) -> Self {
        Self {
            names,
            splitter: thinking.then(ThinkingSplitter::default),
            tools: HashMap::new(),
            stop: None,
            chars: 0,
            context_percent: None,
            input_limit,
            prompt_estimate,
            token_usage: None,
        }
    }

    fn text(&mut self, sse: &mut ChatSse, text: &str) -> Vec<u8> {
        self.chars += text.chars().count();
        let (reasoning, visible) = match &mut self.splitter {
            Some(splitter) => splitter.push(text),
            None => (String::new(), text.to_string()),
        };
        let mut out = sse.reasoning(&reasoning);
        out.extend(sse.text(&visible));
        out
    }

    /// 开始工具调用前把缓冲的思考 / 文本吐出来。
    fn flush(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        let Some(splitter) = &mut self.splitter else {
            return Vec::new();
        };
        let (reasoning, visible) = splitter.finish();
        splitter.stage = 2;
        let mut out = sse.reasoning(&reasoning);
        out.extend(sse.text(&visible));
        out
    }

    fn push(&mut self, sse: &mut ChatSse, frame: &Frame) -> anyhow::Result<Vec<u8>> {
        let message_type = frame.header(":message-type").unwrap_or("event");
        let payload: Value = serde_json::from_slice(&frame.payload).unwrap_or(Value::Null);
        if matches!(message_type, "exception" | "error") {
            let kind = frame
                .header(":exception-type")
                .or_else(|| frame.header(":error-code"))
                .unwrap_or("error");
            let message = payload
                .get("message")
                .or_else(|| payload.get("Message"))
                .and_then(Value::as_str)
                .unwrap_or("Kiro 返回错误");
            let label = if kind.to_ascii_lowercase().contains("throttl") {
                format!("rate limited: {message}")
            } else {
                format!("{kind}: {message}")
            };
            return Err(fail(error_status(kind, message), format!("Kiro {label}")));
        }
        let event = frame.header(":event-type").unwrap_or_default();
        match event {
            "assistantResponseEvent" => {
                let text = payload
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                Ok(self.text(sse, text))
            }
            "reasoningContentEvent" => {
                let text = payload
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.chars += text.chars().count();
                Ok(sse.reasoning(text))
            }
            "toolUseEvent" => {
                let mut out = self.flush(sse);
                let id = text_of(&payload, "toolUseId")
                    .map(str::to_string)
                    .unwrap_or_else(editor_kit::new_call_id);
                let input = match payload.get("input") {
                    Some(Value::String(text)) => text.clone(),
                    Some(value @ Value::Object(object)) if !object.is_empty() => value.to_string(),
                    _ => String::new(),
                };
                self.chars += input.chars().count();
                if let Some(sent) = self.tools.get_mut(&id) {
                    sent.push_str(&input);
                    out.extend(sse.tool_arguments(&id, &input));
                } else {
                    let name = text_of(&payload, "name").unwrap_or_default();
                    let name = self
                        .names
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string());
                    self.tools.insert(id.clone(), input.clone());
                    out.extend(sse.tool_call(&id, &name, &input));
                }
                Ok(out)
            }
            "metadataEvent" | "messageMetadataEvent" => {
                if let Some(reason) = payload.get("stopReason").and_then(Value::as_str) {
                    self.stop = Some(match reason.to_ascii_lowercase().as_str() {
                        "max_tokens" => "length",
                        "content_filtered" => "content_filter",
                        _ => "stop",
                    });
                }
                if let Some(usage) = payload.get("tokenUsage").filter(|usage| usage.is_object()) {
                    let get = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
                    let input = usage
                        .get("uncachedInputTokens")
                        .and_then(Value::as_u64)
                        .filter(|value| *value > 0)
                        .unwrap_or_else(|| get("inputTokens"));
                    let cached = get("cacheReadInputTokens");
                    self.token_usage = Some(Usage {
                        prompt_tokens: input + cached + get("cacheWriteInputTokens"),
                        completion_tokens: get("outputTokens"),
                        cached_tokens: cached,
                        reasoning_tokens: 0,
                    });
                }
                Ok(Vec::new())
            }
            "contextUsageEvent" => {
                self.context_percent = payload
                    .get("contextUsagePercentage")
                    .and_then(Value::as_f64);
                Ok(Vec::new())
            }
            "error"
            | "throttlingError"
            | "validationError"
            | "serviceUnavailableError"
            | "internalServerException" => {
                let message = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Kiro 返回错误");
                Err(fail(
                    error_status(event, message),
                    format!("Kiro {event}: {message}"),
                ))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn usage(&self) -> Usage {
        if let Some(usage) = self.token_usage {
            return usage;
        }
        let prompt = match self.context_percent {
            Some(percent) if self.input_limit > 0 => {
                (percent / 100.0 * self.input_limit as f64) as u64
            }
            _ => self.prompt_estimate,
        };
        Usage {
            prompt_tokens: prompt,
            completion_tokens: (self.chars as u64).div_ceil(4),
            ..Usage::default()
        }
    }

    fn finish(&mut self, sse: &mut ChatSse) -> Vec<u8> {
        let mut out = self.flush(sse);
        let reason = if sse.has_tool_calls() {
            Some("tool_calls")
        } else {
            self.stop
        };
        let usage = self.usage();
        out.extend(sse.finish(reason, Some(usage)));
        out
    }
}

async fn send_chat(
    http: &reqwest::Client,
    region: &str,
    token: &str,
    body: &Value,
) -> anyhow::Result<reqwest::Response> {
    let agent = format!(
        "aws-sdk-rust/1.0.0 ua/2.1 os/other lang/rust api/codewhispererstreaming#1.28.3 m/E app/AmazonQ-For-CLI md/appVersion-1.28.3-{}",
        uuid::Uuid::new_v4().simple()
    );
    http.post(format!("{}/generateAssistantResponse", runtime(region)))
        .bearer_auth(token)
        .header("amz-sdk-invocation-id", uuid::Uuid::new_v4().to_string())
        .header("amz-sdk-request", "attempt=1; max=1")
        .header("x-amz-user-agent", &agent)
        .header("user-agent", &agent)
        .header("x-amzn-codewhisperer-optout", "true")
        .header("x-amzn-kiro-agent-mode", "vibe")
        .header("content-type", "application/json")
        .header("accept", "application/vnd.amazon.eventstream")
        .json(body)
        .send()
        .await
        .map_err(|error| fail(502, format!("无法连接 Kiro：{error}")))
}

async fn http_failure(response: reqwest::Response) -> anyhow::Error {
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let message = ["message", "Message"]
        .iter()
        .find_map(|key| text_of(&value, key))
        .map(str::to_string)
        .unwrap_or_else(|| text.chars().take(300).collect());
    let reason = text_of(&value, "reason").unwrap_or_default();
    let mapped = match error_status(reason, &message) {
        502 => status,
        mapped => mapped,
    };
    let mapped = if (400..600).contains(&mapped) {
        mapped
    } else {
        502
    };
    fail(mapped, format!("Kiro 返回 {status}：{message}"))
}

/* —————————————— Subscription —————————————— */

#[async_trait::async_trait]
impl Subscription for Kiro {
    fn id(&self) -> &'static str {
        "kiro"
    }

    fn name(&self) -> &'static str {
        "Kiro"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[LoginMethod::ImportLocal, LoginMethod::Browser]
    }

    fn note(&self) -> &'static str {
        "需要 Kiro 账号（免费或付费套餐，额度按 Kiro 的规则计）：可先在 Kiro IDE 或 kiro-cli 里登录后导入，或在浏览器里用 Google / GitHub / Builder ID 登录。"
    }

    fn detect_local(&self) -> Option<String> {
        read_ide().or_else(|_| read_cli()).ok().map(|_| String::new())
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        _input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::ImportLocal => {
                let credentials =
                    read_ide().or_else(|ide_error| read_cli().map_err(|_| ide_error))?;
                Ok(LoginStart {
                    message: "已读取本机 Kiro 的登录。".to_string(),
                    state: json!({ "method": "import", "credentials": credentials }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Browser => {
                let listener = editor_kit::listen_first(CALLBACK_PORTS).await?;
                let port = listener.local_addr()?.port();
                let state = editor_kit::b64url(&editor_kit::random_bytes(24));
                let (verifier, challenge) = editor_kit::pkce(48);
                let url = format!(
                    "{PORTAL}/signin?{}",
                    query(&[
                        ("state", &state),
                        ("code_challenge", &challenge),
                        ("code_challenge_method", "S256"),
                        ("redirect_uri", &format!("http://localhost:{port}")),
                        ("redirect_from", "KiroIDE"),
                    ])
                );
                let session = editor_kit::park(
                    self.id(),
                    BrowserLogin {
                        listener,
                        port,
                        state,
                        verifier,
                    },
                );
                Ok(LoginStart {
                    open_url: Some(url),
                    message: "请在浏览器里用 Google、GitHub、Builder ID 或 IAM Identity Center 登录 Kiro。".to_string(),
                    state: json!({ "method": "browser", "session": session }),
                    ..LoginStart::default()
                })
            }
            _ => bail!("Kiro 只支持导入本机登录和浏览器登录"),
        }
    }

    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let credentials = match text_of(state, "method").unwrap_or_default() {
            "import" => state.get("credentials").cloned().unwrap_or(Value::Null),
            "browser" => {
                let session = text_of(state, "session").unwrap_or_default();
                let login = editor_kit::unpark::<BrowserLogin>(self.id(), session)
                    .ok_or_else(|| anyhow::anyhow!("这次登录已失效，请重新开始"))?;
                wait_browser(http, login).await?
            }
            other => bail!("不认识的登录状态：{other}"),
        };
        finish_credentials(http, credentials).await
    }

    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !expiring(credentials, "expiresAt", REFRESH_AHEAD_MS) {
            return Ok(None);
        }
        // 从 IDE / kiro-cli 导入的：它们可能已经自己刷新过，先看本机的新 token。
        let local = match text_of(credentials, "source") {
            Some("ide") => read_ide().ok(),
            Some("cli") => read_cli().ok(),
            _ => None,
        };
        if let Some(local) = local.filter(|local| !expiring(local, "expiresAt", REFRESH_AHEAD_MS)) {
            let mut updated = credentials.clone();
            for key in ["accessToken", "refreshToken", "expiresAt"] {
                updated[key] = local[key].clone();
            }
            return Ok(Some(updated));
        }
        let mut updated = refresh_token(http, credentials).await?;
        if text_of(&updated, "profileArn").is_none()
            && let Ok(profile) = profile_of(http, &updated).await
        {
            updated["profileArn"] = json!(profile);
        }
        Ok(Some(updated))
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let token = access_token(credentials)?;
        let profile = text_of(credentials, "profileArn")
            .unwrap_or_default()
            .to_string();
        let mut params = vec![("origin", "KIRO_CLI")];
        if !profile.is_empty() {
            params.push(("profileArn", profile.as_str()));
        }
        let listed = management_get(
            http,
            &api_region(credentials),
            &token,
            "List-Available-Models",
            &params,
        )
        .await;
        let Ok(value) = listed else {
            return Ok(vec!["auto".to_string()]);
        };
        let models = parse_models(&value);
        {
            let mut limits = input_limits()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (id, limit) in &models {
                if *limit > 0 {
                    limits.insert(id.clone(), *limit);
                }
            }
        }
        let ids: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        Ok(if ids.is_empty() {
            vec!["auto".to_string()]
        } else {
            ids
        })
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let mut token = access_token(credentials)?;
        let model = request
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("auto")
            .to_string();
        let (body, names, thinking) = build_request(&request, text_of(credentials, "profileArn"));
        let region = api_region(credentials);
        let mut response = send_chat(http, &region, &token, &body).await?;
        if response.status().as_u16() == 403 && text_of(credentials, "refreshToken").is_some() {
            // token 可能已被撤销：刷新一次再试。
            let fresh = refresh_token(http, credentials).await?;
            token = access_token(&fresh)?;
            response = send_chat(http, &region, &token, &body).await?;
        }
        if !response.status().is_success() {
            return Err(http_failure(response).await);
        }
        let input_limit = input_limits()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&model)
            .copied()
            .unwrap_or(0);
        let prompt_estimate = (body.to_string().chars().count() as u64) / 4;
        let state = KiroEvents::new(names, thinking, input_limit, prompt_estimate);
        Ok(editor_kit::spawn_chat(&model, move |out| {
            stream_reply(out, response, state)
        }))
    }
}

/// `List-Available-Models` → (模型 id, 输入上限)，默认模型排第一。
fn parse_models(value: &Value) -> Vec<(String, u64)> {
    let default = value
        .pointer("/defaultModel/modelId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut models: Vec<(String, u64)> = value
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| {
            let id = text_of(model, "modelId")?;
            let limit = model
                .pointer("/tokenLimits/maxInputTokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            Some((id.to_string(), limit))
        })
        .collect();
    if let Some(position) = models.iter().position(|(id, _)| id == default) {
        let item = models.remove(position);
        models.insert(0, item);
    }
    models
}

async fn stream_reply(mut out: Emitter, response: reqwest::Response, mut state: KiroEvents) {
    let mut bytes = response.bytes_stream();
    let mut reader = EventStreamReader::default();
    while let Some(chunk) = bytes.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                out.fail(fail(502, format!("Kiro 的流断了：{error}"))).await;
                return;
            }
        };
        let frames = match reader.push(&chunk) {
            Ok(frames) => frames,
            Err(error) => {
                out.fail(fail(502, format!("{error:#}"))).await;
                return;
            }
        };
        for frame in frames {
            match state.push(&mut out.sse, &frame) {
                Ok(bytes) => {
                    if !out.send(bytes).await {
                        return;
                    }
                }
                Err(error) => {
                    out.fail(error).await;
                    return;
                }
            }
        }
    }
    if let Err(error) = reader.finish() {
        out.fail(fail(502, format!("{error:#}"))).await;
        return;
    }
    let bytes = state.finish(&mut out.sse);
    out.send(bytes).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::subscriptions::SubscriptionError;

    fn status_of(error: &anyhow::Error) -> u16 {
        error
            .downcast_ref::<SubscriptionError>()
            .map(|error| error.status)
            .unwrap_or(0)
    }
    use crate::gateway::subscriptions::chat::aggregate;
    use base64::Engine;

    fn b64(text: &str) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .unwrap()
    }

    #[test]
    fn crc32_matches_zlib() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        // AWS 公开的空消息向量。
        let empty = encode_frame(&[], b"");
        assert_eq!(editor_kit::hex(&empty), "000000100000000005c248eb7d98c8ff");
    }

    #[test]
    fn decodes_recorded_frames_with_valid_crc() {
        let mut bytes = b64(
            "AAAAfAAAAFw73c94DTptZXNzYWdlLXR5cGUHAAVldmVudAs6ZXZlbnQtdHlwZQcAFmFzc2lzdGFudFJlc3BvbnNlRXZlbnQNOmNvbnRlbnQtdHlwZQcAEGFwcGxpY2F0aW9uL2pzb257ImNvbnRlbnQiOiJIaSJ9CfZvUQ==",
        );
        bytes.extend(b64("AAAAfQAAADE1vvstDTptZXNzYWdlLXR5cGUHAAVldmVudAs6ZXZlbnQtdHlwZQcADHRvb2xVc2VFdmVudHsidG9vbFVzZUlkIjoidG9vbHVzZV94IiwibmFtZSI6InJlYWQiLCJpbnB1dCI6IntcInBhdGhcIjoifYDelGc="));
        bytes.extend(b64("AAAAdAAAADE4rplcDTptZXNzYWdlLXR5cGUHAAVldmVudAs6ZXZlbnQtdHlwZQcADHRvb2xVc2VFdmVudHsidG9vbFVzZUlkIjoidG9vbHVzZV94IiwibmFtZSI6InJlYWQiLCJzdG9wIjp0cnVlfeXssUs="));
        let mut reader = EventStreamReader::default();
        // 一个字节一个字节地喂，测半帧。
        let mut frames = Vec::new();
        for byte in &bytes {
            frames.extend(reader.push(&[*byte]).unwrap());
        }
        reader.finish().unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(
            frames[0].header(":event-type"),
            Some("assistantResponseEvent")
        );
        assert_eq!(frames[0].header(":content-type"), Some("application/json"));
        assert_eq!(frames[0].payload, br#"{"content":"Hi"}"#);
        assert_eq!(frames[2].header(":event-type"), Some("toolUseEvent"));

        let exception = b64(
            "AAAAbwAAAEAInH5lDTptZXNzYWdlLXR5cGUHAAlleGNlcHRpb24POmV4Y2VwdGlvbi10eXBlBwATVGhyb3R0bGluZ0V4Y2VwdGlvbnsibWVzc2FnZSI6IlRvbyBtYW55IHJlcXVlc3RzIn3r1DaX",
        );
        let frames = EventStreamReader::default().push(&exception).unwrap();
        assert_eq!(
            frames[0].header(":exception-type"),
            Some("ThrottlingException")
        );
    }

    #[test]
    fn frames_round_trip_and_reject_bad_crc() {
        let frame = encode_frame(
            &[
                (":message-type", "event"),
                (":event-type", "assistantResponseEvent"),
            ],
            br#"{"content":"x"}"#,
        );
        let decoded = EventStreamReader::default().push(&frame).unwrap();
        assert_eq!(decoded[0].payload, br#"{"content":"x"}"#);
        let mut broken = frame.clone();
        let last = broken.len() - 5;
        broken[last] ^= 0xFF;
        assert!(EventStreamReader::default().push(&broken).is_err());
        let mut reader = EventStreamReader::default();
        reader.push(&frame[..20]).unwrap();
        assert!(reader.finish().is_err());
        // 其它类型的头跳过。
        let mut head = vec![2_u8];
        head.extend_from_slice(b"ok");
        head.push(0);
        head.push(4);
        head.extend_from_slice(b"size");
        head.push(4);
        head.extend_from_slice(&7_u32.to_be_bytes());
        head.push(1);
        head.extend_from_slice(b"s");
        head.push(7);
        head.extend_from_slice(&2_u16.to_be_bytes());
        head.extend_from_slice(b"hi");
        assert_eq!(
            parse_headers(&head).unwrap(),
            vec![("s".to_string(), "hi".to_string())]
        );
    }

    #[test]
    fn iso_times_parse() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse_iso_ms("2026-10-30T12:00:00.5Z"), 1_793_361_600_500);
        assert_eq!(parse_iso_ms("2026-10-30T20:00:00+08:00"), 1_793_361_600_000);
        assert_eq!(parse_iso_ms("garbage"), 0);
    }

    #[test]
    fn imports_ide_and_cli_tokens() {
        let token = json!({
            "accessToken": "aoa", "refreshToken": "aor", "expiresAt": "2026-10-30T12:00:00.000Z",
            "authMethod": "IdC", "provider": "BuilderId", "region": "us-east-1", "clientIdHash": "h",
        });
        let registration = json!({ "clientId": "cid", "clientSecret": "sec" });
        let credentials = parse_ide_token(&token, Some(&registration)).unwrap();
        assert_eq!(credentials["authMethod"], "idc");
        assert_eq!(credentials["clientId"], "cid");
        assert_eq!(credentials["profileArn"], BUILDER_ID_PROFILE);
        assert_eq!(credentials["expiresAt"], 1_793_361_600_000_u64);

        let social = json!({ "accessToken": "a", "refreshToken": "r", "authMethod": "social", "provider": "Google", "profileArn": "arn:aws:codewhisperer:eu-central-1:1:profile/X" });
        let credentials = parse_ide_token(&social, None).unwrap();
        assert_eq!(credentials["authMethod"], "social");
        assert!(credentials.get("clientId").is_none());
        assert_eq!(api_region(&credentials), "eu-central-1");
        assert!(parse_ide_token(&json!({}), None).is_err());

        let cli = json!({ "access_token": "a", "refresh_token": "r", "expires_at": "2026-10-30T12:00:00Z", "region": "us-east-1" });
        let credentials = parse_cli_token(
            "odic",
            &cli,
            Some(&json!({ "client_id": "c", "client_secret": "s" })),
        )
        .unwrap();
        assert_eq!(credentials["provider"], "BuilderId");
        assert_eq!(credentials["clientSecret"], "s");
        assert_eq!(credentials["source"], "cli");
        assert!(parse_cli_token("social", &json!({}), None).is_none());
    }

    #[test]
    fn token_expiry_is_checked() {
        let fresh = apply_tokens(
            &json!({ "refreshToken": "old" }),
            &json!({ "accessToken": "a", "expiresIn": 3600 }),
        );
        assert_eq!(fresh["refreshToken"], "old");
        assert!(!expiring(&fresh, "expiresAt", REFRESH_AHEAD_MS));
        let soon = apply_tokens(
            &fresh,
            &json!({ "accessToken": "b", "refreshToken": "new", "expiresIn": 60 }),
        );
        assert_eq!(soon["refreshToken"], "new");
        assert!(expiring(&soon, "expiresAt", REFRESH_AHEAD_MS));
    }

    #[test]
    fn names_and_ids_are_made_safe() {
        assert_eq!(safe_tool_name("read_file"), "read_file");
        let long = safe_tool_name("mcp.server/tool name");
        assert!(long.starts_with("mcp_server_tool_name_"));
        assert!(
            long.chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        );
        assert!(safe_tool_name(&"x".repeat(80)).len() <= 64);
        assert_eq!(safe_tool_id("call_abc"), "call_abc");
        let id = safe_tool_id("call_x\nfc_y");
        assert!(id.starts_with("t_") && id.len() == 34);
        assert_eq!(
            clean_schema(None),
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(
            clean_schema(Some(&json!({ "properties": {} })))["type"],
            "object"
        );
        assert_eq!(
            clean_schema(Some(&json!({ "type": ["object", "null"] })))["type"],
            "object"
        );
        assert_eq!(
            clean_schema(Some(&json!({ "type": "string" })))["type"],
            "string"
        );
    }

    #[test]
    fn request_carries_history_tools_and_results() {
        let request = json!({
            "model": "claude-sonnet-4.5",
            "reasoning_effort": "high",
            "messages": [
                { "role": "system", "content": "be brief" },
                { "role": "assistant", "content": "stray" },
                { "role": "user", "content": [
                    { "type": "text", "text": "look" },
                    { "type": "image_url", "image_url": { "url": "data:image/png;base64,QUJD" } },
                ] },
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "read file", "arguments": "{\"p\":\"a\"}" } },
                ] },
                { "role": "assistant", "content": "", "tool_calls": [
                    { "id": "call_2", "type": "function", "function": { "name": "list", "arguments": "[]" } },
                ] },
                { "role": "tool", "tool_call_id": "call_1", "content": "A" },
                { "role": "tool", "tool_call_id": "zzz", "content": "orphan" },
                { "role": "assistant", "content": "done" },
            ],
            "tools": [
                { "type": "function", "function": { "name": "read file", "description": "", "parameters": { "properties": { "p": { "type": "string" } } } } },
            ],
        });
        let (body, names, thinking) = build_request(
            &request,
            Some("arn:aws:codewhisperer:us-east-1:1:profile/P"),
        );
        assert!(thinking);
        assert_eq!(
            body["profileArn"],
            "arn:aws:codewhisperer:us-east-1:1:profile/P"
        );
        let state = &body["conversationState"];
        let history = state["history"].as_array().unwrap();
        // user, assistant(两次调用合并), user(工具结果), assistant(done)；最后补了一条空用户。
        assert_eq!(history.len(), 4);
        let first = &history[0]["userInputMessage"];
        assert!(first["content"].as_str().unwrap().starts_with("<thinking_mode>enabled</thinking_mode><max_thinking_length>30000</max_thinking_length>\nbe brief\n\nlook"));
        assert_eq!(first["images"][0]["format"], "png");
        assert_eq!(first["origin"], "KIRO_CLI");
        let uses = history[1]["assistantResponseMessage"]["toolUses"]
            .as_array()
            .unwrap();
        assert_eq!(uses.len(), 2);
        let safe = safe_tool_name("read file");
        assert_eq!(uses[0]["name"], safe);
        assert_eq!(uses[0]["input"]["p"], "a");
        assert_eq!(uses[1]["input"], json!({}));
        let results = history[2]["userInputMessage"]["userInputMessageContext"]["toolResults"]
            .as_array()
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["toolUseId"], "call_1");
        assert_eq!(results[0]["status"], "success");
        assert_eq!(results[1]["toolUseId"], "call_2");
        assert_eq!(results[1]["status"], "error");
        assert_eq!(history[3]["assistantResponseMessage"]["content"], "done");
        let current = &state["currentMessage"]["userInputMessage"];
        assert_eq!(current["content"], "Please proceed with the task.");
        assert_eq!(current["modelId"], "claude-sonnet-4.5");
        let tools = current["userInputMessageContext"]["tools"]
            .as_array()
            .unwrap();
        // 声明的 read file + 历史里用过但没声明的 list。
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["toolSpecification"]["description"], "read file");
        assert_eq!(
            tools[0]["toolSpecification"]["inputSchema"]["json"]["type"],
            "object"
        );
        assert_eq!(tools[1]["toolSpecification"]["name"], "list");
        assert_eq!(names.get(&safe).map(String::as_str), Some("read file"));
    }

    #[test]
    fn single_message_has_no_history() {
        let (body, _, thinking) = build_request(
            &json!({ "model": "deepseek-3.2", "reasoning_effort": "high", "messages": [{ "role": "user", "content": "hi" }] }),
            None,
        );
        assert!(!thinking);
        assert!(body["conversationState"].get("history").is_none());
        assert!(body.get("profileArn").is_none());
        assert_eq!(
            body["conversationState"]["currentMessage"]["userInputMessage"]["content"],
            "hi"
        );
    }

    #[test]
    fn thinking_tags_split_across_chunks() {
        let mut splitter = ThinkingSplitter::default();
        let mut reasoning = String::new();
        let mut text = String::new();
        for piece in [
            "<thin",
            "king>I think",
            " hard</thi",
            "nking>\n\nAnswer",
            " here",
        ] {
            let (r, t) = splitter.push(piece);
            reasoning.push_str(&r);
            text.push_str(&t);
        }
        let (r, t) = splitter.finish();
        reasoning.push_str(&r);
        text.push_str(&t);
        assert_eq!(reasoning, "I think hard");
        assert_eq!(text, "Answer here");

        let mut plain = ThinkingSplitter::default();
        assert_eq!(plain.push("Hello"), (String::new(), "Hello".to_string()));
    }

    fn event(kind: &str, payload: Value) -> Frame {
        Frame {
            headers: vec![
                (":message-type".to_string(), "event".to_string()),
                (":event-type".to_string(), kind.to_string()),
            ],
            payload: payload.to_string().into_bytes(),
        }
    }

    #[test]
    fn events_become_chat_sse() {
        let mut names = HashMap::new();
        names.insert(safe_tool_name("read file"), "read file".to_string());
        let mut state = KiroEvents::new(names, true, 200_000, 10);
        let mut sse = ChatSse::new("m");
        let frames = [
            event(
                "assistantResponseEvent",
                json!({ "content": "<thinking>hmm</thinking>" }),
            ),
            event("assistantResponseEvent", json!({ "content": "OK" })),
            event(
                "toolUseEvent",
                json!({ "toolUseId": "tooluse_x", "name": safe_tool_name("read file"), "input": "{\"p\":" }),
            ),
            event(
                "toolUseEvent",
                json!({ "toolUseId": "tooluse_x", "name": safe_tool_name("read file"), "input": "1}" }),
            ),
            event(
                "toolUseEvent",
                json!({ "toolUseId": "tooluse_x", "name": safe_tool_name("read file"), "stop": true }),
            ),
            event("metadataEvent", json!({ "stopReason": "END_TURN" })),
            event(
                "contextUsageEvent",
                json!({ "contextUsagePercentage": 11.0 }),
            ),
            event("meteringEvent", json!({ "unit": "credit", "usage": 0.09 })),
        ];
        let mut out = Vec::new();
        for frame in &frames {
            out.extend(state.push(&mut sse, frame).unwrap());
        }
        out.extend(state.finish(&mut sse));
        let completion = aggregate(&String::from_utf8(out).unwrap(), "m").unwrap();
        let message = &completion["choices"][0]["message"];
        assert_eq!(message["reasoning_content"], "hmm");
        assert_eq!(message["content"], "OK");
        assert_eq!(message["tool_calls"][0]["id"], "tooluse_x");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "read file");
        assert_eq!(
            message["tool_calls"][0]["function"]["arguments"],
            "{\"p\":1}"
        );
        assert_eq!(completion["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(completion["usage"]["prompt_tokens"], 22_000);

        let mut state = KiroEvents::new(HashMap::new(), false, 0, 10);
        let mut sse = ChatSse::new("m");
        let mut out = state
            .push(
                &mut sse,
                &event("assistantResponseEvent", json!({ "content": "abcd" })),
            )
            .unwrap();
        out.extend(state.push(&mut sse, &event("metadataEvent", json!({ "stopReason": "MAX_TOKENS", "tokenUsage": { "inputTokens": 9, "outputTokens": 2 } }))).unwrap());
        out.extend(state.finish(&mut sse));
        let completion = aggregate(&String::from_utf8(out).unwrap(), "m").unwrap();
        assert_eq!(completion["choices"][0]["finish_reason"], "length");
        assert_eq!(completion["usage"]["prompt_tokens"], 9);
    }

    #[test]
    fn exceptions_keep_status() {
        let mut state = KiroEvents::new(HashMap::new(), false, 0, 0);
        let mut sse = ChatSse::new("m");
        let frame = Frame {
            headers: vec![
                (":message-type".to_string(), "exception".to_string()),
                (
                    ":exception-type".to_string(),
                    "ThrottlingException".to_string(),
                ),
            ],
            payload: br#"{"message":"Too many requests"}"#.to_vec(),
        };
        let error = state.push(&mut sse, &frame).unwrap_err();
        assert_eq!(status_of(&error), 429);
        assert!(error.to_string().contains("rate limited"));
        assert_eq!(
            error_status("ValidationException", "Input is too long."),
            400
        );
        assert_eq!(error_status("", "MONTHLY_REQUEST_COUNT reached"), 429);
        assert_eq!(error_status("", "INSUFFICIENT_MODEL_CAPACITY"), 503);
        assert_eq!(error_status("InternalServerException", "boom"), 502);
    }

    #[test]
    fn models_put_default_first() {
        let value = json!({
            "defaultModel": { "modelId": "auto" },
            "models": [
                { "modelId": "claude-sonnet-4.5", "tokenLimits": { "maxInputTokens": 1_000_000 } },
                { "modelId": "auto" },
            ],
        });
        assert_eq!(
            parse_models(&value),
            vec![
                ("auto".to_string(), 0),
                ("claude-sonnet-4.5".to_string(), 1_000_000)
            ]
        );
        assert!(valid_region("us-east-1"));
        assert!(!valid_region("evil.com/x"));
    }
}
