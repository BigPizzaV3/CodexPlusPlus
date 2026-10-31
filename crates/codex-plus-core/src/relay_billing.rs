//! 供应商卡片上的倍率 / 余额：识别中转站类型（sub2api、new-api），用供应商自己的 Key
//! 查倍率和余额。识别结果只用于展示，失败不影响任何转发。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::RelayProfile;

const BILLING_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// 同站重定向最多跟几次。
const BILLING_MAX_REDIRECTS: usize = 5;

/// 识别结论。前端据此决定缓存多久：
/// - `recognized`：识别出中转站
/// - `notRelay`：明确不是（404、返回结构不符、重定向到别处等），可以长时间不再识别
/// - `unavailable`：这次没拿到结论（断网、代理没起、超时、429、5xx、401/403），不能当成「不是」
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelayBillingOutcome {
    Recognized,
    #[default]
    NotRelay,
    Unavailable,
}

/// 识别结果。`platform` 为空表示没识别出中转站，原因看 `outcome`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayBillingProbe {
    /// `sub2api` / `newapi`
    pub platform: Option<String>,
    pub rate: Option<RelayRateInfo>,
    pub usage: Option<RelayUsageInfo>,
    /// 识别不出或请求失败的原因，仅供排查，前端不弹窗。
    pub detail: String,
    #[serde(default)]
    pub outcome: RelayBillingOutcome,
    /// new-api：访问令牌查账户余额的情况（#2793）。其它中转站、没识别出时为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newapi_access: Option<NewApiAccessStatus>,
}

impl RelayBillingProbe {
    /// 没识别出中转站的结果。
    pub fn unrecognized(outcome: RelayBillingOutcome, detail: String) -> Self {
        Self {
            platform: None,
            rate: None,
            usage: None,
            detail,
            outcome,
            newapi_access: None,
        }
    }
}

/// new-api 的访问令牌（new-api「个人设置 → 安全设置」里生成的系统访问令牌，区别于 API Key，#2793）。
/// API Key 只能查 Key 自己的额度；Key 不限额度时，要用访问令牌查账户余额（`GET /api/user/self`）。
/// 和 Key 同等对待：只存在本机配置里，界面只拿到打码后的样子，`Debug` 也不打印原文。
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewApiAccess {
    pub token: String,
    /// new-api 的用户 ID，放在请求头 `New-Api-User` 里：旧版 new-api 必须带，且要和令牌的主人一致。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub user_id: String,
}

impl std::fmt::Debug for NewApiAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewApiAccess")
            .field("token", &"<redacted>")
            .field("user_id", &self.user_id)
            .finish()
    }
}

/// 访问令牌查余额的情况：没配置、查到了（`error` 为空），或者查询失败的原因。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewApiAccessStatus {
    pub configured: bool,
    /// 查询失败的原因（只有地址和上游的提示，不含令牌）；没配置或查到了余额时为空。
    pub error: Option<String>,
}

/// 单个识别请求失败：区分「这个接口明确不是」和「这次请求没成功」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BillingFetchError {
    pub unavailable: bool,
    pub message: String,
}

impl BillingFetchError {
    pub(crate) fn not_relay(message: impl Into<String>) -> Self {
        Self {
            unavailable: false,
            message: message.into(),
        }
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            unavailable: true,
            message: message.into(),
        }
    }

    pub(crate) fn outcome(&self) -> RelayBillingOutcome {
        if self.unavailable {
            RelayBillingOutcome::Unavailable
        } else {
            RelayBillingOutcome::NotRelay
        }
    }
}

impl std::fmt::Display for BillingFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BillingFetchError {}

/// 倍率。sub2api 来自 `/v1/sub2api/billing`（已含高峰倍率）；
/// new-api 来自这个 Key 最近一条消费记录里的分组倍率。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayRateInfo {
    pub endpoint: String,
    pub multiplier: f64,
    pub applied_peak_multiplier: Option<f64>,
    /// new-api：消费记录里的分组名。
    pub group: Option<String>,
}

/// 余额摘要，按计费方式分：
/// - `keyQuota`：Key 自己设了总额度或限速
/// - `subscription`：订阅分组（sub2api），`remaining` 是日/周/月限额里最紧的那个
/// - `wallet`：账户余额
/// - `unlimited`：Key 不限额度，且站点不让查账户余额（new-api）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayUsageInfo {
    pub endpoint: String,
    pub kind: String,
    pub is_valid: bool,
    pub plan_name: Option<String>,
    pub remaining: Option<f64>,
    pub quota_limit: Option<f64>,
    pub quota_used: Option<f64>,
    pub unit: String,
    pub rate_limits: Vec<RelayRateLimitWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayRateLimitWindow {
    pub window: String,
    pub limit: f64,
    pub used: f64,
    pub remaining: f64,
}

/// 一次识别要用的上游地址、Key 和带代理的 client。
pub(crate) struct BillingTarget {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
}

/// 先试 sub2api，不是再试 new-api。官方登录、聚合、免鉴权供应商不发请求。
pub async fn probe_relay_billing(profile: &RelayProfile) -> RelayBillingProbe {
    probe_relay_billing_with_access(profile, None).await
}

/// 同 [`probe_relay_billing`]；识别成 new-api 且 Key 不限额度时，再用访问令牌查账户余额（#2793）。
pub async fn probe_relay_billing_with_access(
    profile: &RelayProfile,
    newapi_access: Option<&NewApiAccess>,
) -> RelayBillingProbe {
    if crate::official_login::is_official_login_profile(profile)
        || profile.relay_mode == crate::settings::RelayMode::Aggregate
        || profile.uses_no_auth()
    {
        return RelayBillingProbe::unrecognized(
            RelayBillingOutcome::NotRelay,
            "官方登录、聚合或免鉴权供应商不识别中转站类型".to_string(),
        );
    }
    let target = match billing_target(profile) {
        Ok(target) => target,
        Err(error) => return RelayBillingProbe::unrecognized(error.outcome(), error.message),
    };
    let sub2api = crate::sub2api::probe(&target).await;
    if sub2api.platform.is_some() {
        return sub2api;
    }
    let newapi = crate::newapi::probe(&target, newapi_access).await;
    if newapi.platform.is_some() {
        return newapi;
    }
    // 两边都没识别出：只要有一边是「这次请求没成功」，就不能断定不是中转站。
    let outcome = if sub2api.outcome == RelayBillingOutcome::Unavailable
        || newapi.outcome == RelayBillingOutcome::Unavailable
    {
        RelayBillingOutcome::Unavailable
    } else {
        RelayBillingOutcome::NotRelay
    };
    RelayBillingProbe::unrecognized(
        outcome,
        format!("sub2api：{}；new-api：{}", sub2api.detail, newapi.detail),
    )
}

fn billing_target(profile: &RelayProfile) -> Result<BillingTarget, BillingFetchError> {
    let base_url = if profile.upstream_base_url.trim().is_empty() {
        profile.base_url.trim()
    } else {
        profile.upstream_base_url.trim()
    };
    if base_url.is_empty() {
        return Err(BillingFetchError::not_relay("Base URL 不能为空"));
    }
    let api_key = crate::relay_config::relay_profile_api_key(profile);
    if api_key.trim().is_empty() {
        return Err(BillingFetchError::not_relay("API Key 不能为空"));
    }
    // 建 client 失败（代理地址无效、证书读不了）属于环境问题，修好后应能识别。
    let client = billing_client(profile)
        .map_err(|error| BillingFetchError::unavailable(format!("{error:#}")))?;
    Ok(BillingTarget {
        client,
        base_url: base_url.trim_end_matches('/').to_string(),
        api_key: api_key.trim().to_string(),
    })
}

/// 识别 / 查询倍率余额用的 client：按供应商代理建，只跟随同站重定向。
/// 请求同时带 Bearer 和 `x-api-key`，reqwest 跨主机重定向只去掉 Authorization，
/// 所以跨主机（含换端口）的重定向一律不跟，免得 Key 发到别的主机。
pub(crate) fn billing_client(profile: &RelayProfile) -> anyhow::Result<reqwest::Client> {
    crate::http_client::relay_client_with_redirect(
        &profile.user_agent,
        profile,
        reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > BILLING_MAX_REDIRECTS {
                return attempt.error("重定向次数过多");
            }
            let same_site = attempt
                .previous()
                .last()
                .is_some_and(|from| same_site_redirect(from, attempt.url()));
            if same_site {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }),
    )
}

/// 同主机同端口的重定向，或同主机默认端口的 http → https 升级，才算同站。
fn same_site_redirect(from: &reqwest::Url, to: &reqwest::Url) -> bool {
    if from.host_str() != to.host_str() {
        return false;
    }
    if from.scheme() == to.scheme() {
        return from.port_or_known_default() == to.port_or_known_default();
    }
    from.scheme() == "http" && to.scheme() == "https" && from.port().is_none() && to.port().is_none()
}

/// GET 一个 JSON 接口；`api_key` 为空时不带鉴权（如 new-api 的公开 `/api/status`）。
/// 断网、代理没起、超时、限流、上游故障、Key 暂时被拒都算「这次没拿到」，
/// 其余（404、405、没跟随的 3xx 等）算「这个接口明确不是」。
pub(crate) async fn get_json(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: Option<&str>,
) -> Result<String, BillingFetchError> {
    let mut request = client.get(endpoint);
    if let Some(api_key) = api_key {
        request = request.bearer_auth(api_key).header("x-api-key", api_key);
    }
    send_json(request, endpoint).await
}

/// 同 [`get_json`]，鉴权头由调用方给（如 new-api 访问令牌的 `Authorization` + `New-Api-User`）。
/// 出错时的信息只有地址和上游的提示，不含请求头。
pub(crate) async fn get_json_with_headers(
    client: &reqwest::Client,
    endpoint: &str,
    headers: &[(&str, &str)],
) -> Result<String, BillingFetchError> {
    let mut request = client.get(endpoint);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    send_json(request, endpoint).await
}

async fn send_json(
    request: reqwest::RequestBuilder,
    endpoint: &str,
) -> Result<String, BillingFetchError> {
    // 卡片后台识别用，上游卡住时不能让刷新一直转。
    let request = request.timeout(BILLING_REQUEST_TIMEOUT);
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            let message = format!("请求 {endpoint} 失败：{error}");
            // 重定向次数过多是站点自身的行为，不是网络问题。
            return Err(if error.is_redirect() {
                BillingFetchError::not_relay(message)
            } else {
                BillingFetchError::unavailable(message)
            });
        }
    };
    let status = response.status();
    if status.is_redirection() {
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        return Err(BillingFetchError::not_relay(format!(
            "HTTP {}：上游重定向到 {location}，为保护 Key 未跟随",
            status.as_u16()
        )));
    }
    let body = match response.text().await {
        Ok(body) => body,
        Err(error) if status.is_success() => {
            return Err(BillingFetchError::unavailable(format!(
                "读取 {endpoint} 响应失败：{error}"
            )));
        }
        Err(_) => String::new(),
    };
    if !status.is_success() {
        let message = format!(
            "HTTP {}：{}",
            status.as_u16(),
            error_message(&body).unwrap_or_else(|| "上游拒绝请求".to_string())
        );
        return Err(if status_is_transient(status) {
            BillingFetchError::unavailable(message)
        } else {
            BillingFetchError::not_relay(message)
        });
    }
    Ok(body)
}

/// 这些状态码说明接口可能存在、只是这次没拿到：鉴权暂时失败、超时、限流、上游故障。
fn status_is_transient(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 401 | 403 | 408 | 425 | 429) || status.is_server_error()
}

fn error_message(body: &str) -> Option<String> {
    let payload: Value = serde_json::from_str(body).ok()?;
    for path in [&["error", "message"][..], &["message"][..], &["error"][..]] {
        // 这一条路径走不通就试下一条（如 new-api 只有顶层 `message`）。
        let Some(current) = path
            .iter()
            .try_fold(&payload, |current, key| current.get(*key))
        else {
            continue;
        };
        if let Some(message) = current
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(message.to_string());
        }
    }
    None
}

/// 把 `.../v1` 这类带版本号的地址退回站点根，new-api 的管理接口挂在根下。
pub(crate) fn site_root(base_url: &str) -> String {
    let cleaned = base_url.trim().trim_end_matches('/');
    if crate::protocol_proxy::has_version_suffix(cleaned) {
        if let Some((root, _)) = cleaned.rsplit_once('/') {
            return root.to_string();
        }
    }
    cleaned.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn relay_profile(base_url: String) -> RelayProfile {
        RelayProfile {
            base_url,
            api_key: "sk-test".to_string(),
            relay_mode: crate::settings::RelayMode::PureApi,
            ..RelayProfile::default()
        }
    }

    #[test]
    fn site_root_strips_version_suffix() {
        assert_eq!(site_root("https://example.com/v1/"), "https://example.com");
        assert_eq!(site_root("https://example.com"), "https://example.com");
        assert_eq!(site_root("https://example.com/openai/v1"), "https://example.com/openai");
    }

    #[tokio::test]
    async fn probe_recognizes_old_sub2api_without_billing_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"mode":"unrestricted","isValid":true,"planName":"钱包余额","remaining":8,"unit":"USD","balance":8}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing(&relay_profile(server.uri())).await;

        assert_eq!(probe.platform.as_deref(), Some("sub2api"));
        assert!(probe.rate.is_none());
        assert_eq!(probe.usage.unwrap().remaining, Some(8.0));
    }

    #[tokio::test]
    async fn probe_falls_back_to_newapi() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/usage/token/"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"code":true,"message":"ok","data":{"object":"token_usage","name":"k","total_granted":1000000,
                    "total_used":250000,"total_available":750000,"unlimited_quota":false,"expires_at":0}}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing(&relay_profile(format!("{}/v1", server.uri()))).await;

        assert_eq!(probe.platform.as_deref(), Some("newapi"));
        assert_eq!(probe.outcome, RelayBillingOutcome::Recognized);
        let usage = probe.usage.unwrap();
        assert_eq!(usage.kind, "keyQuota");
        assert_eq!(usage.remaining, Some(1.5));
        assert_eq!(usage.quota_limit, Some(2.0));
    }

    /// new-api：Key 不限额度，站点不给账户余额。
    async fn mount_unlimited_newapi(server: &MockServer) {
        Mock::given(method("GET"))
            .and(path("/api/usage/token/"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"code":true,"message":"ok","data":{"object":"token_usage","name":"k",
                    "total_granted":0,"total_used":0,"total_available":0,"unlimited_quota":true,"expires_at":0}}"#,
            ))
            .mount(server)
            .await;
    }

    fn access(token: &str, user_id: &str) -> NewApiAccess {
        NewApiAccess {
            token: token.to_string(),
            user_id: user_id.to_string(),
        }
    }

    #[tokio::test]
    async fn newapi_without_access_token_reports_unlimited_and_not_configured() {
        let server = MockServer::start().await;
        mount_unlimited_newapi(&server).await;

        let probe = probe_relay_billing_with_access(&relay_profile(server.uri()), None).await;

        assert_eq!(probe.platform.as_deref(), Some("newapi"));
        assert_eq!(probe.usage.as_ref().unwrap().kind, "unlimited");
        assert_eq!(
            probe.newapi_access,
            Some(NewApiAccessStatus {
                configured: false,
                error: None
            })
        );
        // 没配置访问令牌时不去敲账户接口。
        let asked_self = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|request| request.url.path() == "/api/user/self");
        assert!(!asked_self);
    }

    #[tokio::test]
    async fn newapi_access_token_reads_account_balance() {
        let server = MockServer::start().await;
        mount_unlimited_newapi(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/user/self"))
            .and(header("authorization", "Bearer acc-token"))
            .and(header("new-api-user", "42"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"success":true,"message":"","data":{"id":42,"quota":4115000,"used_quota":10}}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing_with_access(
            &relay_profile(format!("{}/v1", server.uri())),
            Some(&access(" acc-token ", "42")),
        )
        .await;

        let usage = probe.usage.unwrap();
        assert_eq!(usage.kind, "wallet");
        assert_eq!(usage.remaining, Some(8.23));
        assert!(usage.endpoint.ends_with("/api/user/self"));
        assert_eq!(
            probe.newapi_access,
            Some(NewApiAccessStatus {
                configured: true,
                error: None
            })
        );
        // 账户接口只带访问令牌，不带 API Key。
        let self_requests: Vec<_> = server
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|request| request.url.path() == "/api/user/self")
            .collect();
        assert_eq!(self_requests.len(), 1);
        assert!(!self_requests[0].headers.contains_key("x-api-key"));
    }

    #[tokio::test]
    async fn newapi_access_token_failures_keep_card_and_report_reason() {
        // 旧版 new-api：鉴权失败也回 200，只是 success = false。
        let server = MockServer::start().await;
        mount_unlimited_newapi(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/user/self"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"success":false,"message":"无权进行此操作，access token 无效"}"#,
            ))
            .mount(&server)
            .await;

        let probe =
            probe_relay_billing_with_access(&relay_profile(server.uri()), Some(&access("bad", "")))
                .await;

        assert_eq!(probe.platform.as_deref(), Some("newapi"));
        assert_eq!(probe.outcome, RelayBillingOutcome::Recognized);
        assert_eq!(probe.usage.as_ref().unwrap().kind, "unlimited");
        let status = probe.newapi_access.unwrap();
        assert!(status.configured);
        let error = status.error.unwrap();
        assert!(error.contains("access token 无效"), "{error}");
        assert!(!error.contains("bad"), "出错信息不能带出令牌：{error}");

        // 新版：401 / 403（如缺 New-Api-User、令牌没勾「个人资料 · 查看」）。
        let server = MockServer::start().await;
        mount_unlimited_newapi(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/user/self"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"success":false,"message":"The access token lacks the required permission: profile:read"}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing_with_access(
            &relay_profile(server.uri()),
            Some(&access("nap_secret", "7")),
        )
        .await;

        assert_eq!(probe.usage.as_ref().unwrap().kind, "unlimited");
        let error = probe.newapi_access.unwrap().error.unwrap();
        assert!(error.contains("HTTP 403") && error.contains("profile:read"), "{error}");
        assert!(!error.contains("nap_secret"));
    }

    #[tokio::test]
    async fn newapi_access_token_is_not_used_when_key_has_its_own_quota() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/usage/token/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"code":true,"data":{"object":"token_usage","total_granted":1000000,
                    "total_used":0,"total_available":1000000,"unlimited_quota":false}}"#,
            ))
            .mount(&server)
            .await;

        let probe =
            probe_relay_billing_with_access(&relay_profile(server.uri()), Some(&access("t", "1")))
                .await;

        assert_eq!(probe.usage.unwrap().kind, "keyQuota");
        assert_eq!(probe.newapi_access.unwrap().error, None);
        let asked_self = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|request| request.url.path() == "/api/user/self");
        assert!(!asked_self);
    }

    #[test]
    fn newapi_access_debug_hides_token() {
        let text = format!("{:?}", access("nap_secret", "7"));
        assert!(!text.contains("nap_secret"));
        assert!(text.contains("7"));
    }

    #[tokio::test]
    async fn probe_reports_unknown_relays() {
        let server = MockServer::start().await;

        let probe = probe_relay_billing(&relay_profile(server.uri())).await;

        assert!(probe.platform.is_none());
        assert!(probe.rate.is_none() && probe.usage.is_none());
        // 所有接口都 404：明确不是中转站。
        assert_eq!(probe.outcome, RelayBillingOutcome::NotRelay);
    }

    #[tokio::test]
    async fn probe_marks_recognized_relays() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"mode":"unrestricted","isValid":true,"remaining":8,"unit":"USD","balance":8}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing(&relay_profile(server.uri())).await;

        assert_eq!(probe.outcome, RelayBillingOutcome::Recognized);
    }

    #[tokio::test]
    async fn probe_treats_transient_http_failures_as_unavailable() {
        for status in [401u16, 403, 429, 500, 502, 503] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            let probe = probe_relay_billing(&relay_profile(server.uri())).await;

            assert!(probe.platform.is_none(), "HTTP {status}");
            assert_eq!(probe.outcome, RelayBillingOutcome::Unavailable, "HTTP {status}");
        }
    }

    #[tokio::test]
    async fn probe_treats_network_errors_as_unavailable() {
        // 先占一个端口再放掉，确保连上去是「连接被拒」。
        let port = std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        let probe = probe_relay_billing(&relay_profile(format!("http://127.0.0.1:{port}"))).await;

        assert!(probe.platform.is_none());
        assert_eq!(probe.outcome, RelayBillingOutcome::Unavailable);
    }

    #[tokio::test]
    async fn probe_does_not_follow_cross_host_redirects_with_api_key() {
        // 第二个服务（另一个端口）返回合法的 sub2api 余额：只要 Key 被带过去就会被识别出来。
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"mode":"unrestricted","isValid":true,"remaining":8,"unit":"USD","balance":8}"#,
            ))
            .mount(&other)
            .await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/v1/usage", other.uri()).as_str()),
            )
            .mount(&server)
            .await;

        let probe = probe_relay_billing(&relay_profile(server.uri())).await;

        let leaked = other
            .received_requests()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|request| {
                request.headers.contains_key("x-api-key")
                    || request.headers.contains_key("authorization")
            })
            .count();
        assert_eq!(leaked, 0, "Key 不能被带到重定向后的另一个主机");
        assert!(probe.platform.is_none());
        assert_eq!(probe.outcome, RelayBillingOutcome::NotRelay);
    }

    #[tokio::test]
    async fn probe_follows_same_site_redirects() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/usage"))
            .respond_with(
                ResponseTemplate::new(301).insert_header("location", "/v2/usage"),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/usage"))
            .and(header("x-api-key", "sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"mode":"unrestricted","isValid":true,"remaining":8,"unit":"USD","balance":8}"#,
            ))
            .mount(&server)
            .await;

        let probe = probe_relay_billing(&relay_profile(server.uri())).await;

        assert_eq!(probe.platform.as_deref(), Some("sub2api"));
    }

    #[test]
    fn same_site_redirect_only_allows_same_origin_or_https_upgrade() {
        let url = |value: &str| reqwest::Url::parse(value).unwrap();
        assert!(same_site_redirect(&url("https://a.example/v1"), &url("https://a.example/v2")));
        assert!(same_site_redirect(&url("http://a.example/v1"), &url("https://a.example/v1")));
        assert!(!same_site_redirect(&url("https://a.example/v1"), &url("https://b.example/v1")));
        assert!(!same_site_redirect(&url("http://127.0.0.1:1000/"), &url("http://127.0.0.1:2000/")));
        assert!(!same_site_redirect(&url("https://a.example/"), &url("http://a.example/")));
    }

    #[test]
    fn probe_outcome_defaults_to_not_relay_when_missing() {
        let probe: RelayBillingProbe =
            serde_json::from_str(r#"{"platform":null,"rate":null,"usage":null,"detail":""}"#)
                .unwrap();
        assert_eq!(probe.outcome, RelayBillingOutcome::NotRelay);
        let json = serde_json::to_value(RelayBillingProbe::unrecognized(
            RelayBillingOutcome::Unavailable,
            String::new(),
        ))
        .unwrap();
        assert_eq!(json["outcome"], "unavailable");
    }

    #[tokio::test]
    async fn probe_skips_profiles_without_upstream_credentials() {
        let aggregate = RelayProfile {
            relay_mode: crate::settings::RelayMode::Aggregate,
            ..relay_profile("http://127.0.0.1:9".to_string())
        };
        let missing_key = RelayProfile {
            api_key: String::new(),
            ..relay_profile("http://127.0.0.1:9".to_string())
        };

        assert!(probe_relay_billing(&aggregate).await.platform.is_none());
        assert!(probe_relay_billing(&missing_key).await.platform.is_none());
    }
}
