//! ZCode（智谱 Z.ai / BigModel 的编码客户端）：用账号下的 GLM Coding Plan API Key。
//!
//! - 登录：浏览器登录（ZCode CLI 的流程：开一个登录流程、打开授权页、轮询结果），拿到站点 token 后
//!   在账号的默认项目里找或建一把 `zcode-api-key`；导入本机 ZCode 客户端的登录（`~/.zcode/v2/credentials.json`，
//!   每个值都是 AES-256-GCM 密文，只读不写）；粘贴 `<id>.<secret>` 形式的 API Key。
//!   API Key 不会过期，没有刷新；导入本机登录的账号每次请求前重读文件。
//! - 聊天：Coding Plan 的 OpenAI 兼容接口（`/api/coding/paas/v4/chat/completions`），原样透传；
//!   推理开关用 `thinking`。额度用完时上游可能回 200 + JSON 错误体，要先看 content-type。
//! - 免费的 Start Plan（只有 ZCode 自己的 JWT、没有 API Key）暂不支持。

use std::time::Duration;

use anyhow::Context;
use base64::Engine;
use serde_json::{Value, json};

use crate::gateway::subscriptions::b_shared;
use crate::gateway::subscriptions::chat::passthrough;
use crate::gateway::subscriptions::oauth;
use crate::gateway::subscriptions::{
    ChatStream, LoginDone, LoginMethod, LoginStart, Subscription, fail,
};

const ZCODE: &str = "https://zcode.z.ai";
const APP_VERSION: &str = "3.16.3";
const KEY_NAME: &str = "zcode-api-key";
const DEFAULT_MODELS: &[&str] = &["glm-5.3", "glm-5.3-flash", "glm-5.2", "glm-5-turbo"];

pub struct ZCode;

pub static ZCODE_SUB: ZCode = ZCode;

/// 账号所在站点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    /// Z.ai（国际站）。
    Zai,
    /// BigModel（智谱国内站）。
    BigModel,
}

impl Site {
    fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("bigmodel") {
            Self::BigModel
        } else {
            Self::Zai
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Zai => "zai",
            Self::BigModel => "bigmodel",
        }
    }

    /// 业务接口根地址。
    fn root(self) -> &'static str {
        match self {
            Self::Zai => "https://api.z.ai",
            Self::BigModel => "https://bigmodel.cn",
        }
    }

    /// 模型接口根地址。
    fn api(self) -> &'static str {
        match self {
            Self::Zai => "https://api.z.ai",
            Self::BigModel => "https://open.bigmodel.cn",
        }
    }

    fn chat_url(self) -> String {
        format!("{}/api/coding/paas/v4/chat/completions", self.api())
    }

    fn provider_id(self) -> &'static str {
        match self {
            Self::Zai => "account:zai-individual-coding-plan",
            Self::BigModel => "account:bigmodel-individual-coding-plan",
        }
    }
}

fn user_agent() -> String {
    format!("ZCode/{APP_VERSION}")
}

fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

// ---------------------------------------------------------------------------
// 业务接口（{code, msg, data} 信封）
// ---------------------------------------------------------------------------

/// 调业务接口，返回 `data`。`auth` 原样放进 Authorization（Z.ai 带 Bearer，BigModel 是裸 token）。
async fn biz(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    auth: &str,
    body: Option<&Value>,
    extra: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let mut request = http
        .request(method, url)
        .header("accept", "application/json")
        .header("user-agent", user_agent())
        .timeout(Duration::from_secs(20));
    if !auth.is_empty() {
        request = request.header("authorization", auth);
    }
    for (key, value) in extra {
        request = request.header(*key, *value);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let code = value.get("code").cloned().unwrap_or(Value::Null);
    let ok_code = match &code {
        Value::Null => true,
        Value::Number(number) => matches!(number.as_u64(), Some(0 | 200)),
        Value::String(text) => matches!(text.as_str(), "0" | "200"),
        _ => false,
    };
    if !(200..300).contains(&status) || !ok_code {
        let message = b_shared::str_at(&value, &["msg", "message", "/error/message"])
            .map(str::to_string)
            .unwrap_or_else(|| text.chars().take(200).collect());
        let status = if (200..300).contains(&status) {
            502
        } else {
            status
        };
        return Err(fail(
            status,
            format!("ZCode 账号接口返回 {status}：{message}"),
        ));
    }
    Ok(value.get("data").cloned().unwrap_or(Value::Null))
}

/// 列表答复可能直接是数组，也可能包在 list / records / rows 里。
fn list_of(data: &Value) -> Vec<Value> {
    data.as_array()
        .or_else(|| {
            ["list", "records", "rows", "items"]
                .iter()
                .find_map(|key| data.get(*key).and_then(Value::as_array))
        })
        .cloned()
        .unwrap_or_default()
}

/// 个人项目：组织优先「默认机构」，项目优先「默认项目」，否则取第一个；跳过团队项目（type 2）。
fn personal_project(customer: &Value) -> Option<(String, String)> {
    let organizations = list_of(customer.get("organizations").unwrap_or(&Value::Null));
    let personal = |project: &&Value| b_shared::u64_at(project, &["projectType"]) != Some(2);
    let mut ordered: Vec<&Value> = organizations.iter().collect();
    ordered.sort_by_key(|org| {
        !b_shared::str_at(org, &["organizationName"]).is_some_and(|name| name.contains("默认机构"))
    });
    for org in ordered {
        let org_id = b_shared::str_at(org, &["organizationId"])?;
        let projects = list_of(org.get("projects").unwrap_or(&Value::Null));
        let mut candidates: Vec<&Value> = projects.iter().filter(personal).collect();
        candidates.sort_by_key(|project| {
            !b_shared::str_at(project, &["projectName"])
                .is_some_and(|name| name.contains("默认项目"))
        });
        if let Some(project) = candidates.first()
            && let Some(project_id) = b_shared::str_at(project, &["projectId"])
        {
            return Some((org_id.to_string(), project_id.to_string()));
        }
    }
    None
}

/// 找到或新建 `zcode-api-key`，返回完整的 `<id>.<secret>`。
async fn ensure_key(
    http: &reqwest::Client,
    site: Site,
    auth: &str,
    org: &str,
    project: &str,
) -> anyhow::Result<String> {
    let keys_url = format!(
        "{}/api/biz/v1/organization/{org}/projects/{project}/api_keys",
        site.root()
    );
    let list = biz(http, reqwest::Method::GET, &keys_url, auth, None, &[]).await?;
    let existing = list_of(&list)
        .into_iter()
        .find(|key| b_shared::str_at(key, &["name"]) == Some(KEY_NAME))
        .and_then(|key| b_shared::str_at(&key, &["apiKey"]).map(str::to_string));
    let id = match existing {
        Some(id) => id,
        None => {
            let created = biz(
                http,
                reqwest::Method::POST,
                &keys_url,
                auth,
                Some(&json!({ "name": KEY_NAME })),
                &[],
            )
            .await?;
            b_shared::str_at(&created, &["apiKey"])
                .context("没能创建 API Key")?
                .to_string()
        }
    };
    let secret = biz(
        http,
        reqwest::Method::GET,
        &format!("{keys_url}/copy/{id}"),
        auth,
        None,
        &[],
    )
    .await?;
    let secret = b_shared::str_at(&secret, &["secretKey"]).context("没能读取 API Key")?;
    Ok(format!("{id}.{secret}"))
}

/// 这把 key 有没有生效中的 Coding Plan：Some(套餐名)。
async fn coding_plan(
    http: &reqwest::Client,
    site: Site,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let data = biz(
        http,
        reqwest::Method::GET,
        &format!("{}/api/biz/subscription/list", site.api()),
        key,
        None,
        &[],
    )
    .await?;
    Ok(list_of(&data)
        .into_iter()
        .find(|item| b_shared::str_at(item, &["status"]) == Some("VALID"))
        .map(|item| {
            b_shared::str_at(&item, &["productName"])
                .unwrap_or("GLM Coding Plan")
                .to_string()
        }))
}

// ---------------------------------------------------------------------------
// 本机 ZCode 的登录
// ---------------------------------------------------------------------------

/// 本机数据目录：`setting.json` 的 `dataBaseDir` 是绝对路径时用它，否则 `~/.zcode/v2`。
fn data_dir() -> std::path::PathBuf {
    let default = b_shared::home().join(".zcode").join("v2");
    std::fs::read_to_string(default.join("setting.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|value| {
            value
                .get("dataBaseDir")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(".zcode").join("v2"))
        .unwrap_or(default)
}

/// 本机凭据的解密密钥种子。
fn credential_seed() -> String {
    if let Ok(secret) = std::env::var("ZCODE_CREDENTIAL_SECRET")
        && !secret.is_empty()
    {
        return secret;
    }
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let user = user.rsplit('\\').next().unwrap_or(&user).to_string();
    format!(
        "zcode-credential-fallback:{platform}:{}:{user}",
        b_shared::home().display()
    )
}

/// 解密 `enc:v1:<iv>.<tag>.<data>`（三段 base64url，AES-256-GCM，密钥 = sha256(种子)）。
fn decrypt(value: &str, seed: &str) -> anyhow::Result<String> {
    use aes_gcm::aead::consts::{U12, U16};
    use aes_gcm::aead::{Aead, KeyInit, generic_array::GenericArray};
    use aes_gcm::aes::Aes256;
    use sha2::{Digest, Sha256};
    let Some(rest) = value.strip_prefix("enc:v1:") else {
        return Ok(value.to_string());
    };
    let decode = |part: &str| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(part.trim_end_matches('=').as_bytes())
    };
    let mut parts = rest.split('.');
    let (Some(iv), Some(tag), Some(data)) = (parts.next(), parts.next(), parts.next()) else {
        anyhow::bail!("ZCode 凭据格式不对");
    };
    let (iv, tag, mut data) = (decode(iv)?, decode(tag)?, decode(data)?);
    data.extend_from_slice(&tag);
    let key = Sha256::digest(seed.as_bytes());
    let plain = match iv.len() {
        12 => aes_gcm::AesGcm::<Aes256, U12>::new(GenericArray::from_slice(&key))
            .decrypt(GenericArray::from_slice(&iv), data.as_slice()),
        16 => aes_gcm::AesGcm::<Aes256, U16>::new(GenericArray::from_slice(&key))
            .decrypt(GenericArray::from_slice(&iv), data.as_slice()),
        other => anyhow::bail!("ZCode 凭据的 IV 长度 {other} 不支持"),
    }
    .map_err(|_| anyhow::anyhow!("ZCode 凭据解密失败"))?;
    Ok(String::from_utf8(plain)?)
}

/// 本机登录里找到的东西。
#[derive(Debug, Default, PartialEq)]
struct LocalLogin {
    key: Option<(String, Site)>,
    /// (站点, token)
    site_token: Option<(Site, String)>,
    /// (组织, 项目, key id)：ZCode 3.16 只存 key 的位置。
    location: Option<(String, String, String)>,
    account: Option<String>,
}

/// 解析（已解密的）`credentials.json` 与 `config.json`。
fn parse_local(
    credentials: &serde_json::Map<String, Value>,
    config: &Value,
    seed: &str,
) -> LocalLogin {
    let read = |value: &Value| value.as_str().and_then(|text| decrypt(text, seed).ok());
    let mut login = LocalLogin::default();
    let mut keys: Vec<(String, Site)> = credentials
        .iter()
        .filter(|(name, _)| name.contains(":coding-plan:") && name.ends_with(":api-key"))
        .filter_map(|(name, value)| {
            let site = if name.contains(":bigmodel-") {
                Site::BigModel
            } else {
                Site::Zai
            };
            read(value)
                .filter(|key| !key.is_empty())
                .map(|key| (key, site))
        })
        .collect();
    keys.sort_by_key(|(_, site)| *site == Site::BigModel);
    login.key = keys.into_iter().next();
    if login.key.is_none() {
        for (provider, site) in [
            ("builtin:zai-coding-plan", Site::Zai),
            ("account:zai-individual-coding-plan", Site::Zai),
            ("builtin:bigmodel-coding-plan", Site::BigModel),
            ("account:bigmodel-individual-coding-plan", Site::BigModel),
        ] {
            if let Some(key) = config
                .pointer(&format!(
                    "/provider/{}/options/apiKey",
                    provider.replace('/', "~1")
                ))
                .and_then(Value::as_str)
                .and_then(|text| decrypt(text, seed).ok())
                .filter(|key| !key.is_empty())
            {
                login.key = Some((key, site));
                break;
            }
        }
    }
    for site in [Site::Zai, Site::BigModel] {
        let family = site.as_str();
        if login.site_token.is_none()
            && let Some(token) = credentials
                .get(&format!("oauth:{family}:access_token"))
                .and_then(read)
                .filter(|token| !token.is_empty())
        {
            login.site_token = Some((site, token));
        }
        if login.account.is_none()
            && let Some(info) = credentials
                .get(&format!("oauth:{family}:user_info"))
                .and_then(read)
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        {
            login.account = b_shared::str_at(&info, &["email", "name", "user_id", "id"])
                .map(|name| name.trim_end_matches("@phone.local").to_string());
        }
    }
    login.location = credentials
        .iter()
        .filter(|(name, _)| name.starts_with("project-key-location:v1:"))
        .filter_map(|(_, value)| read(value))
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .find_map(|value| {
            Some((
                b_shared::str_at(&value, &["organizationId"])?.to_string(),
                b_shared::str_at(&value, &["projectId"])?.to_string(),
                b_shared::str_at(&value, &["apiKeyId"])?.to_string(),
            ))
        });
    login
}

fn read_local_files() -> anyhow::Result<LocalLogin> {
    let dir = data_dir();
    let path = dir.join("credentials.json");
    let credentials = oauth::read_local_json(&path)
        .map_err(|_| anyhow::anyhow!("没找到 {}，请先在 ZCode 里登录", path.display()))?;
    let config = std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let map = credentials.as_object().cloned().unwrap_or_default();
    Ok(parse_local(&map, &config, &credential_seed()))
}

fn site_auth(site: Site, token: &str) -> String {
    match site {
        Site::Zai => format!("Bearer {token}"),
        Site::BigModel => token.to_string(),
    }
}

/// 本机登录 → (key, 站点)。只存了 key 位置时用站点 token 取一次。
async fn local_key(http: &reqwest::Client, login: &LocalLogin) -> anyhow::Result<(String, Site)> {
    if let Some((key, site)) = &login.key {
        return Ok((key.clone(), *site));
    }
    if let (Some((site, token)), Some((org, project, id))) = (&login.site_token, &login.location) {
        let data = biz(
            http,
            reqwest::Method::GET,
            &format!(
                "{}/api/biz/v1/organization/{org}/projects/{project}/api_keys/copy/{id}",
                site.root()
            ),
            &site_auth(*site, token),
            None,
            &[],
        )
        .await?;
        if let Some(secret) = b_shared::str_at(&data, &["secretKey"]) {
            return Ok((format!("{id}.{secret}"), *site));
        }
    }
    anyhow::bail!("本机 ZCode 没有 Coding Plan 的 API Key（只用免费的 Start Plan 时无法导入）")
}

// ---------------------------------------------------------------------------
// 聊天
// ---------------------------------------------------------------------------

/// Chat 请求 → Coding Plan 的 OpenAI 兼容请求：模型名小写，推理等级换成 `thinking` 开关。
fn chat_body(request: &Value) -> Value {
    let mut body = request.clone();
    if let Some(model) = body.get("model").and_then(Value::as_str) {
        let model = model.to_ascii_lowercase();
        body["model"] = json!(model);
    }
    let effort = b_shared::reasoning_effort(request);
    if let Some(object) = body.as_object_mut() {
        object.remove("reasoning_effort");
        object.remove("reasoning");
        if let Some(max) = object.remove("max_completion_tokens")
            && !object.contains_key("max_tokens")
        {
            object.insert("max_tokens".into(), max);
        }
        match effort.as_deref() {
            Some("none" | "off" | "disabled") => {
                object.insert("thinking".into(), json!({ "type": "disabled" }));
            }
            Some(_) => {
                object.insert("thinking".into(), json!({ "type": "enabled" }));
            }
            None => {}
        }
        object.insert("stream".into(), json!(true));
    }
    body
}

/// 智谱的错误体 → 带状态码的失败。
fn failure(status: u16, body: &str) -> anyhow::Error {
    let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let code = ["/error/code", "/code"]
        .iter()
        .find_map(|pointer| match value.pointer(pointer) {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        })
        .unwrap_or_default();
    let message = b_shared::error_message(body);
    let lower = body.to_ascii_lowercase();
    let (status, label) = match code.as_str() {
        "1113" | "1005" => (429, "Coding Plan 额度已用完"),
        "1302" | "1303" => (429, "请求太频繁"),
        "3008" | "3009" => (429, "并发数超限"),
        "3012" => (403, "账号被风控限制"),
        "1000" | "1001" | "1002" => (401, "API Key 无效"),
        _ if status == 401 => (401, "API Key 无效"),
        _ if status == 405 || lower.contains("unusual activity") => (403, "账号被风控限制"),
        _ if status == 429 => (429, "请求太频繁"),
        _ if (400..600).contains(&status) => (status, "请求失败"),
        _ => (502, "请求失败"),
    };
    fail(status, format!("ZCode {label}（{status}）：{message}"))
}

/// 模型配置 → 这个站点 Coding Plan 的内置模型（小写）。
fn models_from_config(config: &Value, site: Site) -> Vec<String> {
    let config = config.get("config").unwrap_or(config);
    let provider = site.provider_id();
    let mut out: Vec<String> = Vec::new();
    let rules = config.pointer("/providerConfigRules/providerRules");
    let rule = match rules {
        Some(Value::Array(items)) => items
            .iter()
            .find(|item| item.get("providerId").and_then(Value::as_str) == Some(provider))
            .cloned(),
        Some(Value::Object(map)) => map.get(provider).cloned(),
        _ => None,
    };
    for id in rule
        .as_ref()
        .and_then(|rule| rule.get("builtinModelIds"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let id = id.to_ascii_lowercase();
        if !out.contains(&id) {
            out.push(id);
        }
    }
    for item in config
        .pointer("/modelConfigRules/builtinProviderModelRules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if item.get("providerId").and_then(Value::as_str) != Some(provider)
            || item.pointer("/config/enabled").and_then(Value::as_bool) == Some(false)
        {
            continue;
        }
        if let Some(id) = item.get("modelId").and_then(Value::as_str) {
            let id = id.to_ascii_lowercase();
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// 远程模型配置：先取配置索引，再取内置供应商配置。
async fn remote_config(http: &reqwest::Client) -> anyhow::Result<Value> {
    let index = http
        .get(format!(
            "{ZCODE}/api/v1/client/configs?{}",
            oauth::query(&[("app_version", APP_VERSION), ("platform", &platform())])
        ))
        .header("user-agent", user_agent())
        .header("x-device-mid", uuid::Uuid::new_v4().to_string())
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    let index = oauth::read_json(index).await?;
    let url = b_shared::str_at(&index, &["/data/configs/builtin_provider_config_json"])
        .filter(|url| url.starts_with("https://"))
        .context("没有模型配置地址")?;
    let config = http
        .get(url)
        .header("user-agent", user_agent())
        .timeout(Duration::from_secs(15))
        .send()
        .await?;
    oauth::read_json(config).await
}

fn site_of(credentials: &Value) -> Site {
    Site::parse(
        credentials
            .get("site")
            .and_then(Value::as_str)
            .unwrap_or("zai"),
    )
}

fn key_of(credentials: &Value) -> anyhow::Result<String> {
    credentials
        .get("api_key")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| fail(401, "ZCode 凭据缺失，请重新登录"))
}

fn credentials(key: &str, site: Site, plan: Option<&str>, source: &str) -> Value {
    json!({
        "api_key": key,
        "site": site.as_str(),
        "plan": plan,
        "source": source,
        // API Key 不会过期。
        "expires_ms": null,
    })
}

#[async_trait::async_trait]
impl Subscription for ZCode {
    fn id(&self) -> &'static str {
        "zcode"
    }

    fn name(&self) -> &'static str {
        "ZCode"
    }

    fn login_methods(&self) -> &'static [LoginMethod] {
        &[
            LoginMethod::Browser,
            LoginMethod::ImportLocal,
            LoginMethod::Paste,
        ]
    }

    fn note(&self) -> &'static str {
        "需要智谱 GLM Coding Plan 套餐（Z.ai 或 BigModel）；可在浏览器登录 Z.ai，或先在 ZCode 里登录后导入，也可粘贴 Coding Plan 的 API Key（BigModel 账号前面加 bigmodel:）。"
    }

    async fn begin_login(
        &self,
        http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart> {
        match method {
            LoginMethod::Browser | LoginMethod::DeviceCode => {
                let site = Site::parse(input.trim());
                let poll = format!("Bearer {}", b_shared::random_hex(32));
                let device = uuid::Uuid::new_v4().to_string();
                let data = biz(
                    http,
                    reqwest::Method::POST,
                    &format!("{ZCODE}/api/v1/oauth/cli/init"),
                    &poll,
                    Some(&json!({ "provider": site.as_str() })),
                    &[("x-device-mid", &device)],
                )
                .await?;
                let flow = b_shared::str_at(&data, &["flow_id"]).context("没有拿到登录流程")?;
                let page = b_shared::str_at(&data, &["authorize_url"])
                    .filter(|url| url.starts_with("https://"))
                    .context("没有拿到授权页地址")?;
                let mut url = reqwest::Url::parse(page).context("授权页地址不对")?;
                let param = match site {
                    Site::Zai => "redirect_uri",
                    Site::BigModel => "redirect",
                };
                let landing = format!(
                    "{ZCODE}/app/oauth/login?{}",
                    oauth::query(&[
                        ("redirect", "zcode://oauth/callback"),
                        ("app_version", APP_VERSION)
                    ])
                );
                let pairs: Vec<(String, String)> = url
                    .query_pairs()
                    .filter(|(key, _)| key != param)
                    .map(|(key, value)| (key.to_string(), value.to_string()))
                    .collect();
                url.query_pairs_mut()
                    .clear()
                    .extend_pairs(pairs)
                    .append_pair(param, &landing);
                Ok(LoginStart {
                    open_url: Some(url.to_string()),
                    user_code: None,
                    message: "在浏览器里登录智谱账号并授权 ZCode。".into(),
                    state: json!({
                        "method": "browser",
                        "site": site.as_str(),
                        "flow": flow,
                        "poll": poll,
                        "device": device,
                        "interval": b_shared::u64_at(&data, &["poll_interval_sec"]).unwrap_or(2),
                    }),
                })
            }
            LoginMethod::ImportLocal => {
                let login = read_local_files()?;
                let (key, site) = local_key(http, &login).await?;
                Ok(LoginStart {
                    message: "已读取本机 ZCode 的登录。".into(),
                    state: json!({
                        "method": "local",
                        "api_key": key,
                        "site": site.as_str(),
                        "account": login.account,
                    }),
                    ..LoginStart::default()
                })
            }
            LoginMethod::Paste => {
                let text = input.trim();
                let (site, key) = match text.split_once(':') {
                    Some((prefix, key)) if prefix.eq_ignore_ascii_case("bigmodel") => {
                        (Some(Site::BigModel), key.trim())
                    }
                    Some((prefix, key)) if prefix.eq_ignore_ascii_case("zai") => {
                        (Some(Site::Zai), key.trim())
                    }
                    _ => (None, text),
                };
                if !key.contains('.') {
                    anyhow::bail!("请粘贴 Coding Plan 的 API Key（形如 xxxx.yyyy）");
                }
                Ok(LoginStart {
                    message: "正在校验 API Key。".into(),
                    state: json!({ "method": "paste", "api_key": key, "site": site.map(Site::as_str) }),
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
        let text = |key: &str| {
            state
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match text("method").as_str() {
            "local" => {
                let site = Site::parse(&text("site"));
                let key = text("api_key");
                let plan = coding_plan(http, site, &key).await.ok().flatten();
                let account = state
                    .get("account")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        format!("{} …{}", site.as_str(), &key[key.len().saturating_sub(4)..])
                    });
                Ok(LoginDone {
                    credentials: credentials(&key, site, plan.as_deref(), "local"),
                    account,
                })
            }
            "paste" => {
                let key = text("api_key");
                let sites = match state.get("site").and_then(Value::as_str) {
                    Some(site) => vec![Site::parse(site)],
                    None => vec![Site::Zai, Site::BigModel],
                };
                let mut last_error = None;
                for site in sites {
                    match coding_plan(http, site, &key).await {
                        Ok(Some(plan)) => {
                            return Ok(LoginDone {
                                credentials: credentials(&key, site, Some(&plan), "paste"),
                                account: format!("{} · {plan}", site.as_str()),
                            });
                        }
                        Ok(None) => {
                            last_error = Some(anyhow::anyhow!(
                                "这把 API Key 没有生效中的 GLM Coding Plan 套餐"
                            ))
                        }
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(last_error.unwrap_or_else(|| anyhow::anyhow!("API Key 校验失败")))
            }
            _ => {
                let site = Site::parse(&text("site"));
                let poll = text("poll");
                let device = text("device");
                let url = format!("{ZCODE}/api/v1/oauth/cli/poll/{}", text("flow"));
                let interval = state.get("interval").and_then(Value::as_u64).unwrap_or(2);
                let data = oauth::poll_until(Duration::from_secs(interval), || {
                    let http = http.clone();
                    let (url, poll, device) = (url.clone(), poll.clone(), device.clone());
                    async move {
                        match biz(&http, reqwest::Method::GET, &url, &poll, None, &[("x-device-mid", &device)]).await {
                            Ok(data) => match b_shared::str_at(&data, &["status"]) {
                                Some("ready") => Ok(Some(data)),
                                Some("failed") => anyhow::bail!("登录被拒绝"),
                                _ => Ok(None),
                            },
                            Err(error) => {
                                let status = error
                                    .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
                                    .map(|error| error.status)
                                    .unwrap_or(0);
                                if (400..500).contains(&status) && status != 408 && status != 429 {
                                    Err(error)
                                } else {
                                    Ok(None)
                                }
                            }
                        }
                    }
                })
                .await?;
                let token = match site {
                    Site::Zai => b_shared::str_at(&data, &["/zai/access_token"]),
                    Site::BigModel => b_shared::str_at(
                        &data,
                        &["/bigmodel/access_token", "/bigmodel/accessToken"],
                    ),
                }
                .context("登录结果里没有站点 token")?
                .to_string();
                let auth = match site {
                    Site::Zai => {
                        let login = biz(
                            http,
                            reqwest::Method::POST,
                            &format!("{}/api/auth/z/login", site.root()),
                            "",
                            Some(&json!({ "token": token })),
                            &[],
                        )
                        .await?;
                        format!(
                            "Bearer {}",
                            b_shared::str_at(&login, &["access_token"])
                                .context("没能换到业务 token")?
                        )
                    }
                    Site::BigModel => token,
                };
                let customer = biz(
                    http,
                    reqwest::Method::GET,
                    &format!("{}/api/biz/customer/getCustomerInfo", site.root()),
                    &auth,
                    None,
                    &[],
                )
                .await?;
                let (org, project) = personal_project(&customer).context("账号下没有可用的项目")?;
                let key = ensure_key(http, site, &auth, &org, &project).await?;
                let plan = coding_plan(http, site, &key).await?.ok_or_else(|| {
                    fail(
                        403,
                        "这个账号没有生效中的 GLM Coding Plan 套餐（ZCode 免费的 Start Plan 暂不支持）",
                    )
                })?;
                let account =
                    b_shared::str_at(&data, &["/user/email", "/user/name", "/user/user_id"])
                        .map(|name| name.trim_end_matches("@phone.local").to_string())
                        .unwrap_or_else(|| site.as_str().to_string());
                Ok(LoginDone {
                    credentials: credentials(&key, site, Some(&plan), "browser"),
                    account,
                })
            }
        }
    }

    async fn refresh(
        &self,
        _http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>> {
        // API Key 不过期；导入本机登录的账号跟着 ZCode 客户端走（重读文件，key 变了就换）。
        if credentials.get("source").and_then(Value::as_str) != Some("local") {
            return Ok(None);
        }
        let Ok(LocalLogin {
            key: Some((key, site)),
            ..
        }) = read_local_files()
        else {
            return Ok(None);
        };
        if credentials.get("api_key").and_then(Value::as_str) == Some(key.as_str()) {
            return Ok(None);
        }
        let plan = credentials.get("plan").and_then(Value::as_str);
        Ok(Some(self::credentials(&key, site, plan, "local")))
    }

    async fn models(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Vec<String>> {
        let site = site_of(credentials);
        if let Ok(config) = remote_config(http).await {
            let models = models_from_config(&config, site);
            if !models.is_empty() {
                return Ok(models);
            }
        }
        let local = std::fs::read_to_string(data_dir().join("provider_config.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .map(|config| models_from_config(&config, site))
            .unwrap_or_default();
        if !local.is_empty() {
            return Ok(local);
        }
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
        let key = key_of(credentials)?;
        let site = site_of(credentials);
        let response = http
            .post(site.chat_url())
            .header("authorization", format!("Bearer {key}"))
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .header("user-agent", user_agent())
            .header(
                "accept-language",
                if site == Site::BigModel {
                    "zh-CN,zh"
                } else {
                    "en-US,en"
                },
            )
            .body(chat_body(&request).to_string())
            .send()
            .await
            .map_err(|error| fail(502, format!("无法连接 ZCode：{error}")))?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(status, &text));
        }
        // 额度用完等错误可能以 200 + JSON 回来。
        let is_json = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"));
        if is_json {
            let text = response.text().await.unwrap_or_default();
            return Err(failure(502, &text));
        }
        Ok(passthrough(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(error: &anyhow::Error) -> u16 {
        error
            .downcast_ref::<crate::gateway::subscriptions::SubscriptionError>()
            .unwrap()
            .status
    }

    /// 测试用：按 ZCode 的格式加密。
    fn encrypt(plain: &str, seed: &str, iv: &[u8; 12]) -> String {
        use aes_gcm::aead::{Aead, KeyInit, generic_array::GenericArray};
        use sha2::{Digest, Sha256};
        let key = Sha256::digest(seed.as_bytes());
        let sealed = aes_gcm::Aes256Gcm::new(GenericArray::from_slice(&key))
            .encrypt(GenericArray::from_slice(iv), plain.as_bytes())
            .unwrap();
        let (data, tag) = sealed.split_at(sealed.len() - 16);
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        format!("enc:v1:{}.{}.{}", encode(iv), encode(tag), encode(data))
    }

    #[test]
    fn decrypts_local_values() {
        let seed = "zcode-credential-fallback:darwin:/Users/a:a";
        let sealed = encrypt("abc.def", seed, &[7; 12]);
        assert_eq!(decrypt(&sealed, seed).unwrap(), "abc.def");
        assert!(decrypt(&sealed, "wrong").is_err());
        assert_eq!(decrypt("plain", seed).unwrap(), "plain");
    }

    #[test]
    fn parses_local_login() {
        let seed = "s";
        let iv = [1; 12];
        let mut credentials = serde_json::Map::new();
        credentials.insert(
            "account-provider:coding-plan:account:bigmodel-individual-coding-plan:account:1:api-key".into(),
            json!(encrypt("big.key", seed, &iv)),
        );
        credentials.insert(
            "account-provider:coding-plan:account:zai-individual-coding-plan:account:2:api-key"
                .into(),
            json!(encrypt("zai.key", seed, &iv)),
        );
        credentials.insert(
            "oauth:zai:user_info".into(),
            json!(encrypt(
                r#"{"email":"138@phone.local","id":"u"}"#,
                seed,
                &iv
            )),
        );
        credentials.insert(
            "oauth:zai:access_token".into(),
            json!(encrypt("tok", seed, &iv)),
        );
        let login = parse_local(&credentials, &Value::Null, seed);
        // 两个站都有时优先 Z.ai。
        assert_eq!(login.key, Some(("zai.key".to_string(), Site::Zai)));
        assert_eq!(login.account.as_deref(), Some("138"));
        assert_eq!(login.site_token, Some((Site::Zai, "tok".to_string())));

        // 只有 config.json 里的 key。
        let config = json!({ "provider": { "builtin:bigmodel-coding-plan": { "options": { "apiKey": "cfg.key" } } } });
        let login = parse_local(&serde_json::Map::new(), &config, seed);
        assert_eq!(login.key, Some(("cfg.key".to_string(), Site::BigModel)));

        // 只存了 key 的位置。
        let mut located = serde_json::Map::new();
        located.insert(
            "project-key-location:v1:%5B%5D".into(),
            json!(encrypt(
                r#"{"organizationId":"o","projectId":"p","apiKeyId":"k"}"#,
                seed,
                &iv
            )),
        );
        let login = parse_local(&located, &Value::Null, seed);
        assert_eq!(login.key, None);
        assert_eq!(login.location, Some(("o".into(), "p".into(), "k".into())));
    }

    #[test]
    fn builds_chat_body() {
        let request = json!({
            "model": "GLM-5.2",
            "reasoning_effort": "none",
            "max_completion_tokens": 10,
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "type": "function", "function": { "name": "ls", "parameters": {} } }]
        });
        let body = chat_body(&request);
        assert_eq!(body["model"], "glm-5.2");
        assert_eq!(body["thinking"]["type"], "disabled");
        assert_eq!(body["max_tokens"], 10);
        assert!(body.get("reasoning_effort").is_none());
        assert_eq!(body["tools"][0]["function"]["name"], "ls");
        let mut high = request.clone();
        high["reasoning_effort"] = json!("high");
        assert_eq!(chat_body(&high)["thinking"]["type"], "enabled");
        assert!(
            chat_body(&json!({ "model": "glm-5.3" }))
                .get("thinking")
                .is_none()
        );
    }

    #[test]
    fn maps_errors() {
        assert_eq!(
            status(&failure(
                200,
                r#"{"error":{"code":"1113","message":"余额不足"}}"#
            )),
            429
        );
        assert_eq!(
            status(&failure(200, r#"{"code":1005,"msg":"exceed quota limit"}"#)),
            429
        );
        assert_eq!(status(&failure(429, r#"{"error":{"code":"3008"}}"#)), 429);
        assert_eq!(status(&failure(401, "{}")), 401);
        assert_eq!(status(&failure(405, "blocked")), 403);
        assert!(
            failure(200, r#"{"error":{"code":"1113","message":"余额不足"}}"#)
                .to_string()
                .contains("余额不足")
        );
    }

    #[test]
    fn picks_personal_project_and_models() {
        let customer = json!({ "organizations": [
            { "organizationId": "o1", "organizationName": "团队", "projects": [{ "projectId": "t", "projectType": 2 }] },
            { "organizationId": "o2", "organizationName": "默认机构", "projects": [
                { "projectId": "p1", "projectName": "其它", "projectType": 1 },
                { "projectId": "p2", "projectName": "默认项目", "projectType": 1 }
            ]}
        ]});
        assert_eq!(
            personal_project(&customer),
            Some(("o2".into(), "p2".into()))
        );
        let config = json!({ "config": {
            "providerConfigRules": { "providerRules": [
                { "providerId": "account:zai-individual-coding-plan", "builtinModelIds": ["GLM-5.3", "GLM-5.2"] }
            ]},
            "modelConfigRules": { "builtinProviderModelRules": [
                { "providerId": "account:zai-individual-coding-plan", "modelId": "GLM-5-Turbo" },
                { "providerId": "account:zai-individual-coding-plan", "modelId": "GLM-Off", "config": { "enabled": false } },
                { "providerId": "account:zai-start-plan", "modelId": "glm-x" }
            ]}
        }});
        assert_eq!(
            models_from_config(&config, Site::Zai),
            vec!["glm-5.3", "glm-5.2", "glm-5-turbo"]
        );
        assert!(models_from_config(&config, Site::BigModel).is_empty());
        assert_eq!(list_of(&json!({ "list": [1] })), vec![json!(1)]);
    }
}
