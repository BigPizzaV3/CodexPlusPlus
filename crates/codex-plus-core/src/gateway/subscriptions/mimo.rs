//! Xiaomi MiMo：MiMo 桌面端的订阅（会员）。
//!
//! - 登录：小米账号的长轮询登录（打开小米的登录页，扫码或输密码都行；本机轮询拿到 passToken）。
//!   拿到 passToken 后从 MiMo 服务器的 `/user/xiaomi/me` 出发手动跟随重定向（经小米的 serviceLogin
//!   回到 MiMo 的 `/sts` 种下会话 Cookie），会话 Cookie 就是聊天凭据。
//! - 会话按桌面端的习惯一天续一次：快到期时用 passToken 重新走一遍；passToken 失效就要重新登录。
//! - 聊天：`{base}/route/chat/completions`，OpenAI Chat 兼容，原样透传；带上会话 Cookie 和桌面端的标识头。
//! - 没有会员的账号上游回 403 `membership_required`（开放平台的 Token Plan 暂不支持）。

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

use crate::gateway::subscriptions::b_shared;
use crate::gateway::subscriptions::chat::passthrough;
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const HOSTS: &[(&str, &str)] = &[
    ("SGP", "https://mimo-server-sgp.xiaomimimo.com/api"),
    ("RU", "https://mimo-server-ru.xiaomimimo.com/api"),
    ("IN", "https://mimo-server-in.xiaomimimo.com/api"),
];
const DEFAULT_REGION: &str = "SGP";
const ACCOUNT: &str = "https://account.xiaomi.com";
const APP_VERSION: &str = "26.929.292248";
const SOURCE: &str = "mimocode-cli-free";
const UA: &str = "miNative PC/Normal Windows_NT/10.0.26100 SDKV/1.0.0 DEVT/PC DEVS/Windows APP/miaccount_desktop APPV/0.1.0";
/// 桌面端一天重新登录一次。
const RENEW_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
const RENEW_AHEAD_MS: u64 = 10 * 60 * 1000;
/// 小米登录页最多等这么久。
const SIGN_IN_LIFE_MS: u64 = 10 * 60 * 1000;
const MODELS: &[&str] = &["mimo-pro", "mimo-flash"];

pub struct Mimo;

pub static MIMO: Mimo = Mimo;

fn base_of(region: &str) -> Option<&'static str> {
    HOSTS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(region))
        .map(|(_, base)| *base)
}

fn default_base() -> &'static str {
    base_of(DEFAULT_REGION).unwrap_or(HOSTS[0].1)
}

/// 登录要自己跟随重定向（每一跳都要收 Cookie），用不跟随重定向的 client。
fn manual_client() -> anyhow::Result<reqwest::Client> {
    crate::http_client::build_with_relay_proxy(
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30)),
        None,
    )
}

/// 小米账号接口的答复以 `&&&START&&&` 开头。
fn account_json(text: &str) -> Option<Value> {
    serde_json::from_str(text.trim().trim_start_matches("&&&START&&&")).ok()
}

// ---------------------------------------------------------------------------
// 够用的 Cookie 罐
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Cookie {
    name: String,
    value: String,
    domain: String,
    host_only: bool,
    path: String,
    secure: bool,
}

#[derive(Debug, Default)]
struct Jar(Vec<Cookie>);

impl Jar {
    fn put(&mut self, cookie: Cookie, gone: bool) {
        self.0.retain(|old| {
            !(old.name == cookie.name && old.domain == cookie.domain && old.path == cookie.path)
        });
        if !gone {
            self.0.push(cookie);
        }
    }

    fn seed(&mut self, host: &str, name: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        self.put(
            Cookie {
                name: name.into(),
                value: value.into(),
                domain: host.to_ascii_lowercase(),
                host_only: false,
                path: "/".into(),
                secure: false,
            },
            false,
        );
    }

    /// 收下答复的 `Set-Cookie`。
    fn take<'a>(&mut self, lines: impl Iterator<Item = &'a str>, url: &reqwest::Url) {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        for line in lines {
            let mut parts = line.split(';');
            let Some((name, value)) = parts.next().and_then(|pair| pair.split_once('=')) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let path = url.path();
            let mut cookie = Cookie {
                name: name.into(),
                value: value.trim().into(),
                domain: host.clone(),
                host_only: true,
                path: match path.rfind('/') {
                    Some(at) if at > 0 => path[..at].to_string(),
                    _ => "/".into(),
                },
                secure: false,
            };
            let mut gone = false;
            for attribute in parts {
                let (key, value) = attribute.split_once('=').unwrap_or((attribute, ""));
                let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
                match key.as_str() {
                    "domain" if !value.is_empty() => {
                        let domain = value.trim_start_matches('.').to_ascii_lowercase();
                        if host == domain || host.ends_with(&format!(".{domain}")) {
                            cookie.domain = domain;
                            cookie.host_only = false;
                        }
                    }
                    "path" if value.starts_with('/') => cookie.path = value.into(),
                    "secure" => cookie.secure = true,
                    "max-age" if value.parse::<i64>().is_ok_and(|age| age <= 0) => gone = true,
                    "expires" => {
                        if httpdate::parse_http_date(value)
                            .is_ok_and(|at| at <= std::time::SystemTime::now())
                        {
                            gone = true;
                        }
                    }
                    _ => {}
                }
            }
            self.put(cookie, gone);
        }
    }

    fn matching(&self, url: &reqwest::Url) -> Vec<&Cookie> {
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let path = url.path();
        self.0
            .iter()
            .filter(|cookie| {
                if cookie.secure && url.scheme() != "https" {
                    return false;
                }
                let domain_ok = if cookie.host_only {
                    host == cookie.domain
                } else {
                    host == cookie.domain || host.ends_with(&format!(".{}", cookie.domain))
                };
                let prefix = if cookie.path.ends_with('/') {
                    cookie.path.clone()
                } else {
                    format!("{}/", cookie.path)
                };
                domain_ok
                    && (cookie.path == "/" || path == cookie.path || path.starts_with(&prefix))
            })
            .collect()
    }

    fn header(&self, url: &reqwest::Url) -> String {
        self.matching(url)
            .iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// 会话 Cookie → Cookie 头：`serviceToken`、`userId`、`cUserId` 在前。
fn cookie_header(cookies: &Value) -> String {
    let Some(map) = cookies.as_object() else {
        return String::new();
    };
    let first = ["serviceToken", "userId", "cUserId"];
    let mut parts: Vec<String> = first
        .iter()
        .filter_map(|key| {
            map.get(*key)
                .and_then(Value::as_str)
                .map(|value| format!("{key}={value}"))
        })
        .collect();
    for (key, value) in map {
        if !first.contains(&key.as_str())
            && let Some(value) = value.as_str()
        {
            parts.push(format!("{key}={value}"));
        }
    }
    parts.join("; ")
}

// ---------------------------------------------------------------------------
// 会话
// ---------------------------------------------------------------------------

/// 小米账号（续会话要用的部分）。
#[derive(Debug, Clone, PartialEq)]
struct Account {
    user_id: String,
    c_user_id: String,
    pass_token: String,
    device_id: String,
}

impl Account {
    fn from_credentials(credentials: &Value) -> anyhow::Result<Self> {
        let get = |key: &str| {
            credentials
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let account = Self {
            user_id: get("user_id"),
            c_user_id: get("c_user_id"),
            pass_token: get("pass_token"),
            device_id: get("device_id"),
        };
        if account.user_id.is_empty() || account.pass_token.is_empty() {
            return Err(fail(401, "Xiaomi MiMo 凭据不完整，请重新登录"));
        }
        Ok(account)
    }
}

/// 从 MiMo 服务器的 `/user/xiaomi/me` 出发跟随重定向，拿会话 Cookie 和账号信息。
async fn session(
    client: &reqwest::Client,
    account: &Account,
    base: &str,
) -> anyhow::Result<(BTreeMap<String, String>, Value)> {
    let mut jar = Jar::default();
    let account_host = "account.xiaomi.com";
    for (name, value) in [
        ("userId", account.user_id.as_str()),
        ("passToken", account.pass_token.as_str()),
        ("cUserId", account.c_user_id.as_str()),
        ("deviceId", account.device_id.as_str()),
        ("pass_ua", "pc"),
        ("uLocale", "zh_CN"),
    ] {
        jar.seed(account_host, name, value);
    }
    let base_url = reqwest::Url::parse(base).context("MiMo 服务器地址不对")?;
    let mut url = reqwest::Url::parse(&format!("{}/user/xiaomi/me", base.trim_end_matches('/')))?;
    let mut hops = 0;
    let response = loop {
        hops += 1;
        if hops > 10 {
            anyhow::bail!("Xiaomi MiMo 登录：重定向次数过多");
        }
        let mut request = client
            .get(url.clone())
            .header("user-agent", UA)
            .header("x-client-version", APP_VERSION);
        let cookie = jar.header(&url);
        if !cookie.is_empty() {
            request = request.header("cookie", cookie);
        }
        let response = request.send().await.context("无法连接 Xiaomi MiMo")?;
        jar.take(
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|value| value.to_str().ok()),
            &url,
        );
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let Some(location) = location.filter(|_| (300..400).contains(&status)) else {
            break response;
        };
        let mut next = url.join(&location).context("重定向地址不对")?;
        // 服务器回跳写的是 http://，它的会话 Cookie 只走 https。
        if next.scheme() == "http"
            && base_url.scheme() == "https"
            && next.host_str() == base_url.host_str()
        {
            let _ = next.set_scheme("https");
        }
        url = next;
    };
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let Ok(me) = serde_json::from_str::<Value>(&text) else {
        // 落到了小米的登录页：passToken 已失效。
        return Err(fail(401, "Xiaomi MiMo 的小米账号登录已失效，请重新登录"));
    };
    let code = me.get("code").and_then(Value::as_i64);
    if status == 403 || matches!(code, Some(403 | 46109)) {
        return Err(fail(403, "Xiaomi MiMo 不服务这个小米账号所在的地区"));
    }
    if code != Some(0)
        || b_shared::str_at(&me, &["/data/userId"]).is_none()
            && me.pointer("/data/userId").and_then(Value::as_u64).is_none()
    {
        return Err(fail(401, "Xiaomi MiMo 的小米账号登录已失效，请重新登录"));
    }
    let cookies: BTreeMap<String, String> = jar
        .matching(&base_url)
        .into_iter()
        .map(|cookie| (cookie.name.clone(), cookie.value.clone()))
        .collect();
    if cookies.is_empty() {
        anyhow::bail!("Xiaomi MiMo 没有建立会话");
    }
    Ok((cookies, me.get("data").cloned().unwrap_or(Value::Null)))
}

fn credentials(
    account: &Account,
    region: &str,
    base: &str,
    cookies: &BTreeMap<String, String>,
) -> Value {
    json!({
        "user_id": account.user_id,
        "c_user_id": account.c_user_id,
        "pass_token": account.pass_token,
        "device_id": account.device_id,
        "region": region,
        "base": base,
        "cookies": cookies,
        "expires_ms": oauth::now_ms() + RENEW_AFTER_MS,
    })
}

fn base_from(credentials: &Value) -> String {
    credentials
        .get("base")
        .and_then(Value::as_str)
        .filter(|base| base.starts_with("https://"))
        .map(str::to_string)
        .or_else(|| {
            credentials
                .get("region")
                .and_then(Value::as_str)
                .and_then(base_of)
                .map(str::to_string)
        })
        .unwrap_or_else(|| default_base().to_string())
}

/// 用 passToken 重新建立会话。
async fn renew(credentials: &Value) -> anyhow::Result<Value> {
    let account = Account::from_credentials(credentials)?;
    let base = base_from(credentials);
    let region = credentials
        .get("region")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_REGION);
    let (cookies, _) = session(&manual_client()?, &account, &base).await?;
    Ok(self::credentials(&account, region, &base, &cookies))
}

/// 聊天请求体：`mimo-auto` 是桌面端的默认，按它代表的 `mimo-pro` 问。
fn chat_body(request: &Value) -> Value {
    let mut body = request.clone();
    if body.get("model").and_then(Value::as_str) == Some("mimo-auto") {
        body["model"] = json!("mimo-pro");
    }
    body["stream"] = json!(true);
    body
}

fn failure(status: u16, body: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = b_shared::str_at(&value, &["/error/code"]).unwrap_or_default();
    let message = b_shared::error_message(body);
    let (status, label) = if code == "membership_required" {
        (403, "账号没有开通 MiMo 会员或会员已到期")
    } else if status == 401 {
        (401, "会话已失效，请重新登录")
    } else if status == 429 || message.to_ascii_lowercase().contains("quota") {
        (429, "额度已用完或请求太频繁")
    } else if (400..600).contains(&status) {
        (status, "请求失败")
    } else {
        (502, "请求失败")
    };
    fail(
        status,
        format!("Xiaomi MiMo {label}（{status}）：{message}"),
    )
}

async fn send_chat(
    http: &reqwest::Client,
    credentials: &Value,
    body: &Value,
) -> anyhow::Result<reqwest::Response> {
    let cookie = cookie_header(credentials.get("cookies").unwrap_or(&Value::Null));
    http.post(format!(
        "{}/route/chat/completions",
        base_from(credentials).trim_end_matches('/')
    ))
    .header("cookie", cookie)
    .header("x-mimo-source", SOURCE)
    .header("user-agent", UA)
    .header("x-client-version", APP_VERSION)
    .header("content-type", "application/json")
    .header("accept", "text/event-stream")
    .body(body.to_string())
    .send()
    .await
    .map_err(|error| fail(502, format!("无法连接 Xiaomi MiMo：{error}")))
}

/// 会话失效的答复：401，或 200 却不是流（被重定向到了登录页）。
fn session_lapsed(response: &reqwest::Response) -> bool {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    status == 401 || (status == 200 && content_type.contains("text/html"))
}

#[async_trait::async_trait]
impl Subscription for Mimo {
    fn id(&self) -> &'static str {
        "mimo"
    }

    fn name(&self) -> &'static str {
        "Xiaomi MiMo"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[LoginMethod::Browser]
    }

    fn note(&self) -> &'static str {
        "需要开通 Xiaomi MiMo 会员的小米账号；在浏览器打开小米登录页，扫码或输入账号密码登录。"
    }

    async fn begin_login(
        &self,
        _http: &reqwest::Client,
        method: LoginMethod,
        _input: &str,
    ) -> anyhow::Result<LoginStart> {
        if !matches!(method, LoginMethod::Browser | LoginMethod::DeviceCode) {
            anyhow::bail!("Xiaomi MiMo 只支持用小米账号登录");
        }
        let client = manual_client()?;
        let base = default_base();
        // MiMo 服务器把没有会话的请求转去小米登录，从这次跳转里读 sid 和回调地址。
        let mut sid = "mimosgp".to_string();
        let mut callback = format!("{base}/sts");
        if let Ok(response) = client
            .get(format!("{base}/user/xiaomi/me"))
            .header("user-agent", UA)
            .header("x-client-version", APP_VERSION)
            .send()
            .await
            && let Some(location) = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
            && let Ok(url) = reqwest::Url::parse(base).and_then(|base| base.join(location))
        {
            for (key, value) in url.query_pairs() {
                match key.as_ref() {
                    "sid" if !value.is_empty() => sid = value.to_string(),
                    "callback" if !value.is_empty() => callback = value.to_string(),
                    _ => {}
                }
            }
        }
        let device = format!("pc_{}", b_shared::random_hex(16));
        let qs = format!("%3Fsid%3D{}%26_json%3Dtrue", oauth::encode(&sid));
        let url = format!(
            "{ACCOUNT}/longPolling/loginUrl?{}",
            oauth::query(&[
                ("_group", "DEFAULT"),
                ("_qrsize", "240"),
                ("qs", &qs),
                ("callback", &callback),
                ("_hasLogo", "false"),
                ("sid", &sid),
                ("serviceParam", ""),
                ("_locale", "en_US"),
            ])
        );
        let response = client
            .get(url)
            .header("user-agent", UA)
            .header(
                "cookie",
                format!("deviceId={device}; pass_ua=pc; uLocale=zh_CN"),
            )
            .send()
            .await
            .context("无法连接小米账号服务")?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        let ticket = account_json(&text)
            .filter(|value| value.get("code").and_then(Value::as_i64) == Some(0))
            .with_context(|| format!("小米账号服务返回 {status}，无法开始登录"))?;
        let login_url = b_shared::str_at(&ticket, &["loginUrl"]).context("没有拿到登录页")?;
        let poll = b_shared::str_at(&ticket, &["lp"]).context("没有拿到轮询地址")?;
        let life = b_shared::u64_at(&ticket, &["timeout"])
            .filter(|seconds| *seconds > 0)
            .map(|seconds| (seconds * 1000).min(SIGN_IN_LIFE_MS))
            .unwrap_or(SIGN_IN_LIFE_MS);
        Ok(LoginStart {
            open_url: Some(login_url.to_string()),
            user_code: None,
            message: "在打开的小米登录页里扫码或输入账号密码。".into(),
            state: json!({
                "poll": poll,
                "device": device,
                "until": oauth::now_ms() + life,
            }),
        })
    }

    async fn finish_login(
        &self,
        _http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone> {
        let state = &start.state;
        let poll = state
            .get("poll")
            .and_then(Value::as_str)
            .context("缺少轮询地址")?;
        let device = state
            .get("device")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let until = state.get("until").and_then(Value::as_u64).unwrap_or(0);
        let client = manual_client()?;
        let answer = loop {
            let now = oauth::now_ms();
            if now >= until {
                anyhow::bail!("小米登录页已过期，请重新登录");
            }
            let wait = Duration::from_millis((until - now).clamp(1000, 70_000));
            let result = client
                .get(poll)
                .header("user-agent", UA)
                .header(
                    "cookie",
                    format!("deviceId={device}; pass_ua=pc; uLocale=zh_CN"),
                )
                .timeout(wait)
                .send()
                .await;
            if let Ok(response) = result
                && response.status().is_success()
                && let Some(value) = account_json(&response.text().await.unwrap_or_default())
                && b_shared::str_at(&value, &["passToken"]).is_some()
                && (b_shared::str_at(&value, &["userId"]).is_some()
                    || value.get("userId").and_then(Value::as_u64).is_some())
            {
                break value;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        let id_of = |key: &str| {
            b_shared::str_at(&answer, &[key])
                .map(str::to_string)
                .or_else(|| {
                    answer
                        .get(key)
                        .and_then(Value::as_u64)
                        .map(|id| id.to_string())
                })
                .unwrap_or_default()
        };
        let mut account = Account {
            user_id: id_of("userId"),
            c_user_id: id_of("cUserId"),
            pass_token: b_shared::str_at(&answer, &["passToken"])
                .unwrap_or_default()
                .to_string(),
            device_id: device.to_string(),
        };
        let mut region = DEFAULT_REGION.to_string();
        let mut base = default_base().to_string();
        let (mut cookies, mut me) = session(&client, &account, &base).await.map_err(|error| {
            anyhow::anyhow!("小米账号已登录，但 MiMo 服务器没有接受：{error:#}")
        })?;
        // 别的地区的账号由那个地区的服务器服务。
        let home = b_shared::str_at(&me, &["region"])
            .unwrap_or_default()
            .to_ascii_uppercase();
        if let Some(other) = base_of(&home).filter(|other| !home.is_empty() && *other != base)
            && let Ok((other_cookies, other_me)) = session(&client, &account, other).await
        {
            cookies = other_cookies;
            me = other_me;
            region = home;
            base = other.to_string();
        }
        if let Some(user) = b_shared::str_at(&me, &["userId"])
            .map(str::to_string)
            .or_else(|| {
                me.get("userId")
                    .and_then(Value::as_u64)
                    .map(|id| id.to_string())
            })
        {
            account.user_id = user;
        }
        let name = b_shared::str_at(&me, &["userName", "nickName", "nickname", "name"])
            .map(str::to_string)
            .unwrap_or_else(|| account.user_id.clone());
        Ok(LoginDone {
            credentials: credentials(&account, &region, &base, &cookies),
            account: name,
        })
    }

    async fn refresh(
        &self,
        _http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !oauth::expiring(credentials, "expires_ms", RENEW_AHEAD_MS) {
            return Ok(None);
        }
        let expires = credentials
            .get("expires_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        match renew(credentials).await {
            Ok(fresh) => Ok(Some(fresh)),
            Err(error)
                if error
                    .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                    .is_some_and(|error| matches!(error.status, 401 | 403)) =>
            {
                Err(error)
            }
            // 网络抖动但旧会话还在有效期内：先接着用。
            Err(_) if expires > oauth::now_ms() => Ok(None),
            Err(error) => Err(fail(502, format!("Xiaomi MiMo 续会话失败：{error:#}"))),
        }
    }

    async fn models(
        &self,
        _http: &reqwest::Client,
        _credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        // MiMo 没有模型列表接口。
        Ok(MODELS.iter().map(|model| model.to_string()).collect())
    }

    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream> {
        let body = chat_body(&request);
        let mut response = send_chat(http, credentials, &body).await?;
        if session_lapsed(&response) {
            // 会话提前失效：用 passToken 续一次再问（新会话下次刷新时再写回）。
            let fresh = renew(credentials).await?;
            response = send_chat(http, &fresh, &body).await?;
        }
        let status = response.status().as_u16();
        if !(200..300).contains(&status) || session_lapsed(&response) {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(if status == 200 { 401 } else { status }, &text));
        }
        Ok(passthrough(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> reqwest::Url {
        reqwest::Url::parse(text).unwrap()
    }

    #[test]
    fn jar_follows_cookie_rules() {
        let mut jar = Jar::default();
        jar.seed("account.xiaomi.com", "passToken", "p");
        let sts = url("https://mimo-server-sgp.xiaomimimo.com/api/sts?x=1");
        jar.take(
            [
                "serviceToken=abc; Path=/; Secure; HttpOnly",
                "userId=42; Domain=.xiaomimimo.com; Path=/",
                "old=1; Max-Age=0",
                "foreign=1; Domain=example.com",
            ]
            .into_iter(),
            &sts,
        );
        let api = url("https://mimo-server-sgp.xiaomimimo.com/api/route/chat/completions");
        let header = jar.header(&api);
        assert!(header.contains("serviceToken=abc"));
        assert!(header.contains("userId=42"));
        assert!(!header.contains("old="));
        assert!(!header.contains("passToken"));
        // 域 Cookie 对子域也生效；Secure 的不走 http。
        let other = url("http://x.xiaomimimo.com/");
        assert_eq!(jar.header(&other), "userId=42");
        // 不能替别的域设 Cookie：留在本主机。
        assert!(
            jar.0
                .iter()
                .any(|cookie| cookie.name == "foreign" && cookie.host_only)
        );
        assert!(
            jar.header(&url("https://account.xiaomi.com/pass/serviceLogin"))
                .contains("passToken=p")
        );
        // 默认 path 是请求路径的目录。
        let mut jar = Jar::default();
        jar.take(["a=1"].into_iter(), &url("https://h.com/x/y/z"));
        assert_eq!(jar.0[0].path, "/x/y");
        assert_eq!(jar.header(&url("https://h.com/x/y/w")), "a=1");
        assert_eq!(jar.header(&url("https://h.com/other")), "");
    }

    #[test]
    fn cookie_header_orders_session_first() {
        let cookies = json!({ "a": "1", "cUserId": "c", "serviceToken": "s", "userId": "u" });
        assert_eq!(
            cookie_header(&cookies),
            "serviceToken=s; userId=u; cUserId=c; a=1"
        );
        assert_eq!(cookie_header(&Value::Null), "");
    }

    #[test]
    fn credentials_round_trip() {
        let account = Account {
            user_id: "42".into(),
            c_user_id: "c".into(),
            pass_token: "p".into(),
            device_id: "pc_1".into(),
        };
        let cookies = BTreeMap::from([("serviceToken".to_string(), "s".to_string())]);
        let creds = credentials(&account, "RU", base_of("RU").unwrap(), &cookies);
        assert_eq!(Account::from_credentials(&creds).unwrap(), account);
        assert_eq!(
            base_from(&creds),
            "https://mimo-server-ru.xiaomimimo.com/api"
        );
        assert!(!oauth::expiring(&creds, "expires_ms", RENEW_AHEAD_MS));
        assert_eq!(
            base_from(&json!({ "region": "in" })),
            "https://mimo-server-in.xiaomimimo.com/api"
        );
        assert_eq!(base_from(&json!({})), default_base());
        assert!(Account::from_credentials(&json!({ "user_id": "1" })).is_err());
    }

    #[test]
    fn parses_account_answers_and_body() {
        let value = account_json("&&&START&&&{\"code\":0,\"lp\":\"https://x\"}").unwrap();
        assert_eq!(value["lp"], "https://x");
        assert!(account_json("<html>").is_none());
        let body = chat_body(&json!({ "model": "mimo-auto", "messages": [] }));
        assert_eq!(body["model"], "mimo-pro");
        assert_eq!(body["stream"], true);
        assert_eq!(
            chat_body(&json!({ "model": "mimo-flash" }))["model"],
            "mimo-flash"
        );
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
                r#"{"error":{"code":"membership_required","message":"未开通会员"}}"#
            )),
            403
        );
        assert_eq!(status(failure(401, "")), 401);
        assert_eq!(status(failure(429, "{}")), 429);
        assert_eq!(status(failure(500, "{}")), 500);
    }
}
