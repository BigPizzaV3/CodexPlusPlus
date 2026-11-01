//! 私有接口订阅：Cursor、Kiro、Zed、Command Code、Devin、Factory、Gemini Code Assist、Antigravity。
//!
//! 这些订阅的接口各不相同（protobuf、AWS 事件流、Code Assist、各家自己的 JSON），不适合塞进 `protocol_proxy`。
//! 做法是把每家包成一个「本地上游」：
//!
//! ```text
//! 网关管道 ──Chat Completions──► helper /sub/<供应商 id>/v1/chat/completions
//!                                  │ 读 gateway.json 里这个供应商的凭据，快过期时刷新并写回
//!                                  └─► Subscription::chat（各家协议）──► Chat SSE
//! ```
//!
//! 供应商池里它就是一个普通的 Chat Completions 供应商（`auth_contents` 里记着订阅类型和凭据），
//! 路由、多账号轮换、冷却、用量统计都照旧；请求前 [`prepare`] 把地址指到本地适配入口。
//!
//! 新增订阅：实现 [`Subscription`]，登记到 `catalog.rs` 的 `ITEMS`。
//! 这些都是冒用官方客户端身份调用未公开的接口，可能随时失效，也有账号被限制的风险（界面上会提示）。

pub mod antigravity;
pub mod b_shared;
pub mod catalog;
pub mod chat;
pub mod code_assist;
pub mod commandcode;
pub mod cursor;
pub mod devin;
pub mod editor_kit;
pub mod factory;
pub mod gemini;
pub mod google;
pub mod kiro;
pub mod oauth;
pub mod server;
pub mod zed;

use std::pin::Pin;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::gateway::accounts::ACCOUNT_KIND_FIELD;
use crate::settings::{RelayMode, RelayProfile, RelayProtocol};

/// `auth_contents` 里订阅类型的前缀：`sub:<订阅 id>`。
pub const KIND_PREFIX: &str = "sub:";

/// 订阅返回的 Chat Completions SSE 字节流（`data: {chunk}\n\n`，以 `data: [DONE]` 结束）。
pub type ChatStream =
    Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Vec<u8>>> + Send + 'static>>;

/// 登录方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginMethod {
    /// 浏览器登录（本机回调）。
    Browser,
    /// 设备码：在浏览器里输入验证码。
    DeviceCode,
    /// 导入本机官方客户端已经登录的凭据。
    ImportLocal,
    /// 粘贴 token / Cookie / API Key。
    Paste,
}

/// 开始登录的结果：要给用户看的（打开的网址、验证码），以及完成登录要用的状态。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    /// 要在浏览器里打开的网址（命令层负责打开）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_url: Option<String>,
    /// 设备码登录时给用户看的验证码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_code: Option<String>,
    /// 给用户的一句说明。
    #[serde(default)]
    pub message: String,
    /// 完成登录要用的内部状态（PKCE verifier、设备码等），原样传回 `finish_login`。
    #[serde(default)]
    pub state: Value,
}

/// 登录完成：凭据（存进 gateway.json）和账号名（邮箱 / 用户名，界面显示、同账号去重用）。
#[derive(Debug, Clone)]
pub struct LoginDone {
    pub credentials: Value,
    pub account: String,
}

/// 订阅请求失败：带上返回给网关管道的 HTTP 状态码，管道据此冷却 / 停用这个账号或换下一家。
/// 401 停用到重新登录，429 冷却，5xx 熔断，400 不换。
#[derive(Debug)]
pub struct SubscriptionError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for SubscriptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SubscriptionError {}

/// 带状态码的失败。
pub fn fail(status: u16, message: impl Into<String>) -> anyhow::Error {
    SubscriptionError {
        status,
        message: message.into(),
    }
    .into()
}

/// 一家订阅。每个方法都带一个 HTTP 客户端：登录时走系统代理，请求时按供应商自己的代理设置。
#[async_trait::async_trait]
pub trait Subscription: Send + Sync {
    /// 订阅 id：小写字母、数字和 `-`，存在凭据里，不能改。
    fn id(&self) -> &'static str;
    /// 界面上的名字。
    fn name(&self) -> &'static str;
    /// 支持的登录方式（第一种是默认）。
    fn login_methods(&self) -> &'static [LoginMethod];
    /// 一句话说明（需要什么套餐、风险等）；没有为空。
    fn note(&self) -> &'static str {
        ""
    }

    /// 「发现本机登录」：本机官方客户端已经登录时返回账号名（登录文件里没有账号名时为空串），没登录为 None。
    /// 只读登录文件，不读钥匙串（会弹系统授权框）、不联网；真正导入仍走 `ImportLocal`。
    fn detect_local(&self) -> Option<String> {
        None
    }

    /// 开始登录。`input` 是粘贴的内容（`Paste`）或空。`Paste` / `ImportLocal` 可以直接在
    /// `state` 里放好凭据，由 `finish_login` 校验后返回。
    async fn begin_login(
        &self,
        http: &reqwest::Client,
        method: LoginMethod,
        input: &str,
    ) -> anyhow::Result<LoginStart>;

    /// 完成登录：等浏览器回调 / 轮询设备码 / 校验粘贴的凭据。可以阻塞到用户完成（最多几分钟）。
    async fn finish_login(
        &self,
        http: &reqwest::Client,
        start: &LoginStart,
    ) -> anyhow::Result<LoginDone>;

    /// 请求前调用：token 快过期时刷新，返回新的凭据（写回 gateway.json）；不用刷新返回 None。
    async fn refresh(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
    ) -> anyhow::Result<Option<Value>>;

    /// 这个账号能用的模型（订阅自己的模型名）。
    async fn models(&self, http: &reqwest::Client, credentials: &Value)
    -> anyhow::Result<Vec<String>>;

    /// 发一个 Chat Completions 请求（`stream` 总是 true；`model` 是订阅自己的模型名），
    /// 返回 Chat SSE。出错时用 [`fail`] 带上状态码。
    async fn chat(
        &self,
        http: &reqwest::Client,
        credentials: &Value,
        request: Value,
    ) -> anyhow::Result<ChatStream>;
}

/// 全部订阅。
pub fn all() -> &'static [&'static dyn Subscription] {
    static ALL: OnceLock<Vec<&'static dyn Subscription>> = OnceLock::new();
    ALL.get_or_init(|| catalog::ITEMS.to_vec())
}

pub fn get(id: &str) -> Option<&'static dyn Subscription> {
    all().iter().copied().find(|item| item.id() == id)
}

/// 界面上「添加供应商 → 订阅」里列出的一项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionView {
    pub id: String,
    pub name: String,
    pub methods: Vec<LoginMethod>,
    pub note: String,
}

pub fn views() -> Vec<SubscriptionView> {
    all()
        .iter()
        .map(|item| SubscriptionView {
            id: item.id().to_string(),
            name: item.name().to_string(),
            methods: item.login_methods().to_vec(),
            note: item.note().to_string(),
        })
        .collect()
}

/// 本机已经登录、还没加进网关池的订阅账号（供应商页提示导入用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalLogin {
    /// 订阅 id。
    pub id: String,
    pub name: String,
    /// 账号名（邮箱 / 用户名）；登录文件里没有时为空。
    pub account: String,
}

/// 扫一遍本机各家官方客户端的登录（只读文件），去掉池里已经有的。
pub fn pending_local_logins(pool: &[RelayProfile]) -> Vec<LocalLogin> {
    let detected = all()
        .iter()
        .filter_map(|item| {
            item.detect_local().map(|account| LocalLogin {
                id: item.id().to_string(),
                name: item.name().to_string(),
                account,
            })
        })
        .collect();
    not_in_pool(detected, pool)
}

/// 池里已有的不再提示：有账号名的按账号名比（不分大小写）；登录文件里没有账号名的，池里有这家任意账号就算有。
fn not_in_pool(detected: Vec<LocalLogin>, pool: &[RelayProfile]) -> Vec<LocalLogin> {
    let pooled: Vec<(String, String)> = pool
        .iter()
        .filter_map(|relay| {
            let value: Value = serde_json::from_str(&relay.auth_contents).ok()?;
            let id = value
                .get(ACCOUNT_KIND_FIELD)?
                .as_str()?
                .strip_prefix(KIND_PREFIX)?
                .to_string();
            let account = value
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_lowercase();
            Some((id, account))
        })
        .collect();
    detected
        .into_iter()
        .filter(|login| {
            let account = login.account.to_lowercase();
            !pooled
                .iter()
                .any(|(id, pooled)| *id == login.id && (account.is_empty() || *pooled == account))
        })
        .collect()
}

/// 供应商是哪家订阅、凭据是什么；不是订阅供应商时为 None。
pub fn kind_of(relay: &RelayProfile) -> Option<(&'static dyn Subscription, Value)> {
    let value: Value = serde_json::from_str(&relay.auth_contents).ok()?;
    let kind = value.get(ACCOUNT_KIND_FIELD)?.as_str()?;
    let item = get(kind.strip_prefix(KIND_PREFIX)?)?;
    let credentials = value.get("credentials").cloned().unwrap_or(Value::Null);
    Some((item, credentials))
}

/// 登录得到的供应商（还没进池；进池由命令层按账号去重后添加）。
pub fn relay_for(item: &dyn Subscription, done: &LoginDone, models: &[String]) -> RelayProfile {
    let auth = json!({
        ACCOUNT_KIND_FIELD: format!("{KIND_PREFIX}{}", item.id()),
        "account": done.account,
        "credentials": done.credentials,
    });
    RelayProfile {
        name: if done.account.is_empty() {
            item.name().to_string()
        } else {
            format!("{} · {}", item.name(), done.account)
        },
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        // 真正的地址在请求前由 [`prepare`] 指到本地适配入口（端口可能变）。
        base_url: "http://127.0.0.1/sub".to_string(),
        api_key: "codex-plus-subscription".to_string(),
        auth_contents: auth.to_string(),
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        ..RelayProfile::default()
    }
}

/// 把新凭据写回 `auth_contents`（其余字段不动）。
pub fn with_credentials(auth_contents: &str, credentials: &Value) -> String {
    let mut value: Value = serde_json::from_str(auth_contents).unwrap_or_else(|_| json!({}));
    value["credentials"] = credentials.clone();
    value.to_string()
}

/// 本进程的适配入口口令：只有 helper 自己发出的请求才能用（每次启动换一个）。
pub fn local_secret() -> &'static str {
    static SECRET: OnceLock<String> = OnceLock::new();
    SECRET.get_or_init(|| format!("cpx-sub-{}", uuid::Uuid::new_v4().simple()))
}

/// 请求前：订阅供应商的地址指到本机 helper 的适配入口。
pub fn prepare(relay: &RelayProfile) -> RelayProfile {
    let port = crate::resident_helper::resident_helper_port()
        .unwrap_or_else(crate::protocol_proxy::protocol_proxy_port);
    let mut ready = relay.clone();
    ready.base_url = format!(
        "http://{}:{port}/sub/{}/v1",
        crate::wsl_network::LOOPBACK_HOST,
        relay.id
    );
    ready.api_key = local_secret().to_string();
    ready.protocol = RelayProtocol::ChatCompletions;
    // 本机这一跳不走代理；订阅自己访问上游时按供应商的代理设置走。
    ready.custom_headers.clear();
    ready
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_ids_are_unique_and_safe() {
        let mut seen = std::collections::HashSet::new();
        for item in all() {
            assert!(seen.insert(item.id()), "重复的订阅 id：{}", item.id());
            assert!(
                item.id()
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-'),
                "订阅 id 只能用小写字母、数字和 -：{}",
                item.id()
            );
            assert!(!item.login_methods().is_empty(), "{} 没有登录方式", item.id());
        }
    }

    fn pooled(id: &str, account: &str) -> RelayProfile {
        RelayProfile {
            auth_contents: json!({
                ACCOUNT_KIND_FIELD: format!("{KIND_PREFIX}{id}"),
                "account": account,
                "credentials": {},
            })
            .to_string(),
            ..RelayProfile::default()
        }
    }

    fn found(id: &str, account: &str) -> LocalLogin {
        LocalLogin {
            id: id.to_string(),
            name: id.to_string(),
            account: account.to_string(),
        }
    }

    #[test]
    fn local_logins_already_in_pool_are_not_offered() {
        let pool = vec![
            pooled("kiro", "a@example.com"),
            pooled("gemini", "Me@Example.com"),
        ];
        let left = not_in_pool(
            vec![
                // 登录文件里没有账号名：池里有这家就不提示。
                found("kiro", ""),
                // 同一账号，大小写不同。
                found("gemini", "me@example.com"),
                // 同一家的另一个账号照样提示。
                found("gemini", "other@example.com"),
                found("cursor", ""),
            ],
            &pool,
        );
        assert_eq!(
            left,
            vec![found("gemini", "other@example.com"), found("cursor", "")]
        );
    }

    #[test]
    fn credentials_round_trip_through_auth_contents() {
        let relay = RelayProfile {
            auth_contents: json!({
                ACCOUNT_KIND_FIELD: "sub:none",
                "credentials": { "token": "a" },
            })
            .to_string(),
            ..RelayProfile::default()
        };
        // 没登记的订阅不算订阅供应商。
        assert!(kind_of(&relay).is_none());
        let updated = with_credentials(&relay.auth_contents, &json!({ "token": "b" }));
        let value: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(value["credentials"]["token"], "b");
        assert_eq!(value[ACCOUNT_KIND_FIELD], "sub:none");
    }
}
