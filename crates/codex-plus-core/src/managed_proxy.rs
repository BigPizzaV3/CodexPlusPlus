use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use base64::Engine;
use hmac::{Hmac, Mac};
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature as RsaPkcs1v15Signature, VerifyingKey as RsaPkcs1v15VerifyingKey};
use rsa::pkcs8::DecodePublicKey;
use rsa::signature::Verifier;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::local_backend::{LocalBackendAuditEventQuery, LocalBackendStore};
use crate::local_usage::{LocalUsageEvent, estimate_tokens_from_bytes, token_usage_from_value};
use crate::settings::{JIYI_DEFAULT_RELAY_BASE_URL, JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK};

pub const DEFAULT_MANAGED_PROXY_PORT: u16 = 57421;
pub const DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL: &str = JIYI_DEFAULT_RELAY_BASE_URL;
const DEFAULT_MANAGED_PROXY_USER_AGENT: &str = "JiyiCodexManagedProxy/1.2.4";
const MAX_MANAGED_PROXY_REQUEST_BYTES: usize = 32 * 1024 * 1024;
const PAYMENT_WEBHOOK_SIGNATURE_TOLERANCE_MS: i64 = 5 * 60 * 1000;
type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone)]
pub struct ManagedProxyConfig {
    pub listen_addr: SocketAddr,
    pub upstream_base_url: String,
    pub upstream_api_key: String,
    pub identity_sync_api_key: String,
    pub admin_api_key: String,
    pub user_read_api_key: String,
    pub billing_api_key: String,
    pub payment_webhook_api_key: String,
    pub payment_webhook_signature_secret: String,
    pub payment_webhook_alipay_public_key: String,
    pub payment_webhook_wechatpay_public_key: String,
    pub access_api_key: String,
    pub audit_api_key: String,
    pub upstream_user_agent: String,
}

impl ManagedProxyConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let listen_addr = managed_proxy_listen_addr_from_env()?;
        let upstream_base_url = std::env::var("JIYI_MANAGED_PROXY_UPSTREAM_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL.to_string());
        let upstream_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_UPSTREAM_API_KEY",
            "JIYI_CODEX_UPSTREAM_API_KEY",
            "APIMART_API_KEY",
        ]);
        let identity_sync_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_SYNC_API_KEY",
            "JIYI_IDENTITY_SYNC_API_KEY",
            "JIYI_BACKEND_SYNC_API_KEY",
        ]);
        let admin_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_ADMIN_API_KEY",
            "JIYI_BACKEND_ADMIN_API_KEY",
        ]);
        let user_read_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_USER_READ_API_KEY",
            "JIYI_BACKEND_USER_READ_API_KEY",
        ]);
        let billing_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_BILLING_API_KEY",
            "JIYI_BACKEND_BILLING_API_KEY",
        ]);
        let payment_webhook_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_API_KEY",
            "JIYI_BACKEND_PAYMENT_WEBHOOK_API_KEY",
        ]);
        let payment_webhook_signature_secret = first_env_value(&[
            "JIYI_MANAGED_PROXY_PAYMENT_WEBHOOK_SIGNATURE_SECRET",
            "JIYI_BACKEND_PAYMENT_WEBHOOK_SIGNATURE_SECRET",
        ]);
        let payment_webhook_alipay_public_key = first_env_or_file_value(
            &[
                "JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY",
                "JIYI_BACKEND_ALIPAY_PUBLIC_KEY",
            ],
            &[
                "JIYI_MANAGED_PROXY_ALIPAY_PUBLIC_KEY_PATH",
                "JIYI_BACKEND_ALIPAY_PUBLIC_KEY_PATH",
            ],
        );
        let payment_webhook_wechatpay_public_key = first_env_or_file_value(
            &[
                "JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY",
                "JIYI_MANAGED_PROXY_WECHATPAY_PLATFORM_PUBLIC_KEY",
                "JIYI_BACKEND_WECHATPAY_PUBLIC_KEY",
                "JIYI_BACKEND_WECHATPAY_PLATFORM_PUBLIC_KEY",
            ],
            &[
                "JIYI_MANAGED_PROXY_WECHATPAY_PUBLIC_KEY_PATH",
                "JIYI_MANAGED_PROXY_WECHATPAY_PLATFORM_PUBLIC_KEY_PATH",
                "JIYI_BACKEND_WECHATPAY_PUBLIC_KEY_PATH",
                "JIYI_BACKEND_WECHATPAY_PLATFORM_PUBLIC_KEY_PATH",
            ],
        );
        let access_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_ACCESS_API_KEY",
            "JIYI_BACKEND_ACCESS_API_KEY",
        ]);
        let audit_api_key = first_env_value(&[
            "JIYI_MANAGED_PROXY_AUDIT_API_KEY",
            "JIYI_BACKEND_AUDIT_API_KEY",
        ]);
        let upstream_user_agent = std::env::var("JIYI_MANAGED_PROXY_USER_AGENT")
            .unwrap_or_else(|_| DEFAULT_MANAGED_PROXY_USER_AGENT.to_string());
        Ok(Self {
            listen_addr,
            upstream_base_url,
            upstream_api_key,
            identity_sync_api_key,
            admin_api_key,
            user_read_api_key,
            billing_api_key,
            payment_webhook_api_key,
            payment_webhook_signature_secret,
            payment_webhook_alipay_public_key,
            payment_webhook_wechatpay_public_key,
            access_api_key,
            audit_api_key,
            upstream_user_agent,
        })
    }
}

impl Default for ManagedProxyConfig {
    fn default() -> Self {
        Self {
            listen_addr: SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                DEFAULT_MANAGED_PROXY_PORT,
            ),
            upstream_base_url: DEFAULT_MANAGED_PROXY_UPSTREAM_BASE_URL.to_string(),
            upstream_api_key: String::new(),
            identity_sync_api_key: String::new(),
            admin_api_key: String::new(),
            user_read_api_key: String::new(),
            billing_api_key: String::new(),
            payment_webhook_api_key: String::new(),
            payment_webhook_signature_secret: String::new(),
            payment_webhook_alipay_public_key: String::new(),
            payment_webhook_wechatpay_public_key: String::new(),
            access_api_key: String::new(),
            audit_api_key: String::new(),
            upstream_user_agent: DEFAULT_MANAGED_PROXY_USER_AGENT.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProxyHttpResponse {
    pub status_code: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl ManagedProxyHttpResponse {
    fn json(status_code: u16, body: Value) -> Self {
        let body = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
        Self {
            status_code,
            content_type: "application/json; charset=utf-8".to_string(),
            body,
            headers: Vec::new(),
        }
    }

    fn empty(status_code: u16) -> Self {
        Self {
            status_code,
            content_type: String::new(),
            body: Vec::new(),
            headers: Vec::new(),
        }
    }

    fn to_http_bytes(&self) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, OpenAI-Beta\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n",
            http_status_line(self.status_code),
            self.body.len()
        );
        if !self.content_type.trim().is_empty() {
            response.push_str(&format!("Content-Type: {}\r\n", self.content_type));
        }
        for (name, value) in &self.headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

#[derive(Debug, Clone)]
struct ManagedProxyHttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProxyRoute {
    Health,
    IdentitySync,
    AdminUsers,
    AdminTeams,
    AdminUpdateEntitlement,
    AdminUpdateTeamEntitlement,
    AdminBillingRenewals,
    AdminBillingReconcile,
    BillingPaymentWebhook,
    AdminBlockUser,
    AdminUnblockUser,
    AdminAuditEvents,
    Models,
    Responses,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyHealth {
    status: &'static str,
    version: &'static str,
    listen_addr: String,
    upstream_base_url: String,
    backend_db_path: String,
    upstream_key_configured: bool,
    identity_sync_key_configured: bool,
    admin_key_configured: bool,
    user_read_key_configured: bool,
    billing_key_configured: bool,
    payment_webhook_key_configured: bool,
    payment_webhook_signature_configured: bool,
    payment_webhook_alipay_signature_configured: bool,
    payment_webhook_wechatpay_signature_configured: bool,
    access_key_configured: bool,
    audit_key_configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyIdentitySyncEnvelope {
    body: crate::local_backend::IdentitySyncBody,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyIdentitySyncResponse {
    status: &'static str,
    receipt: crate::local_backend::LocalBackendSyncReceipt,
    active_session: Option<ManagedProxyActiveSession>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyActiveSession {
    user_id: String,
    device_id: String,
    issued_at_ms: i64,
    expires_at_ms: i64,
    access_token: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyUserAccessRequest {
    user_id: String,
    reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyEntitlementRequest {
    user_id: String,
    plan_id: String,
    plan_name: String,
    daily_token_limit: i64,
    reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyTeamEntitlementRequest {
    team_id: String,
    plan_id: String,
    plan_name: String,
    daily_token_limit: i64,
    reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyBillingRenewalRequest {
    subject_type: String,
    subject_id: String,
    plan_id: String,
    plan_name: String,
    daily_token_limit: i64,
    amount_cents: i64,
    currency: String,
    payment_channel: String,
    external_order_id: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedProxyPaymentWebhookRequest {
    provider: String,
    gateway_event_id: Option<String>,
    external_order_id: String,
    status: String,
    subject_type: String,
    subject_id: String,
    plan_id: String,
    plan_name: String,
    daily_token_limit: i64,
    amount_cents: i64,
    currency: String,
    payment_channel: Option<String>,
    reason: Option<String>,
    raw_payload: Option<Value>,
}

pub async fn run_managed_proxy(config: ManagedProxyConfig) -> anyhow::Result<()> {
    run_managed_proxy_with_store(config, LocalBackendStore::from_env()).await
}

pub async fn run_managed_proxy_with_store(
    config: ManagedProxyConfig,
    store: LocalBackendStore,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    let client = Arc::new(crate::http_client::proxied_client(
        &config.upstream_user_agent,
    )?);
    loop {
        let (mut stream, remote_addr) = listener.accept().await?;
        let config = config.clone();
        let store = store.clone();
        let client = client.clone();
        tokio::spawn(async move {
            if let Err(error) =
                handle_managed_proxy_connection(&mut stream, remote_addr, &config, &store, &client)
                    .await
            {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "managed_proxy.connection_failed",
                    json!({
                        "remote_addr": remote_addr.to_string(),
                        "message": error.to_string()
                    }),
                );
            }
        });
    }
}

pub async fn handle_managed_proxy_http_request(
    request_bytes: &[u8],
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
    client: &reqwest::Client,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    let request = parse_http_request(request_bytes)?;
    if request.method.eq_ignore_ascii_case("OPTIONS") {
        return Ok(ManagedProxyHttpResponse::empty(204));
    }

    let Some(route) = managed_proxy_route(&request.path) else {
        return Ok(error_response(
            404,
            "not_found",
            "极义托管代理不支持该路径。",
        ));
    };

    if route == ManagedProxyRoute::Health {
        if !request.method.eq_ignore_ascii_case("GET") {
            return Ok(error_response(
                405,
                "method_not_allowed",
                "健康检查只支持 GET。",
            ));
        }
        return Ok(ManagedProxyHttpResponse::json(
            200,
            json!(ManagedProxyHealth {
                status: "ok",
                version: env!("CARGO_PKG_VERSION"),
                listen_addr: config.listen_addr.to_string(),
                upstream_base_url: config.upstream_base_url.trim().to_string(),
                backend_db_path: store.db_path().to_string_lossy().to_string(),
                upstream_key_configured: !config.upstream_api_key.trim().is_empty(),
                identity_sync_key_configured: !config.identity_sync_api_key.trim().is_empty(),
                admin_key_configured: !config.admin_api_key.trim().is_empty(),
                user_read_key_configured: !config.user_read_api_key.trim().is_empty(),
                billing_key_configured: !config.billing_api_key.trim().is_empty(),
                payment_webhook_key_configured: !config.payment_webhook_api_key.trim().is_empty(),
                payment_webhook_signature_configured: !config
                    .payment_webhook_signature_secret
                    .trim()
                    .is_empty(),
                payment_webhook_alipay_signature_configured: !config
                    .payment_webhook_alipay_public_key
                    .trim()
                    .is_empty(),
                payment_webhook_wechatpay_signature_configured: !config
                    .payment_webhook_wechatpay_public_key
                    .trim()
                    .is_empty(),
                access_key_configured: !config.access_api_key.trim().is_empty(),
                audit_key_configured: !config.audit_api_key.trim().is_empty(),
            }),
        ));
    }

    if route == ManagedProxyRoute::IdentitySync {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Ok(error_response(
                405,
                "method_not_allowed",
                "账号同步只支持 POST。",
            ));
        }
        return handle_identity_sync_request(&request, config, store);
    }

    if route == ManagedProxyRoute::BillingPaymentWebhook {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Ok(error_response(
                405,
                "method_not_allowed",
                "支付回调只支持 POST。",
            ));
        }
        return handle_payment_webhook_request(&request, config, store);
    }

    if matches!(
        route,
        ManagedProxyRoute::AdminUsers
            | ManagedProxyRoute::AdminTeams
            | ManagedProxyRoute::AdminUpdateEntitlement
            | ManagedProxyRoute::AdminUpdateTeamEntitlement
            | ManagedProxyRoute::AdminBillingRenewals
            | ManagedProxyRoute::AdminBillingReconcile
            | ManagedProxyRoute::AdminBlockUser
            | ManagedProxyRoute::AdminUnblockUser
            | ManagedProxyRoute::AdminAuditEvents
    ) {
        let method_ok = match route {
            ManagedProxyRoute::AdminUsers
            | ManagedProxyRoute::AdminTeams
            | ManagedProxyRoute::AdminAuditEvents => request.method.eq_ignore_ascii_case("GET"),
            ManagedProxyRoute::AdminBillingRenewals => {
                request.method.eq_ignore_ascii_case("GET")
                    || request.method.eq_ignore_ascii_case("POST")
            }
            _ => request.method.eq_ignore_ascii_case("POST"),
        };
        if !method_ok {
            return Ok(error_response(
                405,
                "method_not_allowed",
                "托管代理管理接口方法不允许。",
            ));
        }
        return handle_admin_request(&request, route, config, store);
    }

    let access_token = bearer_token(&request);
    if access_token.trim().is_empty() {
        return Ok(error_response(
            401,
            "missing_token",
            "缺少极义后端 session token，请先完成手机号登录。",
        ));
    }

    if config.upstream_api_key.trim().is_empty() {
        return Ok(error_response(
            502,
            "missing_upstream_key",
            "托管代理未配置上游 API Key。",
        ));
    }

    match route {
        ManagedProxyRoute::Health => unreachable!(),
        ManagedProxyRoute::IdentitySync => unreachable!(),
        ManagedProxyRoute::AdminUsers
        | ManagedProxyRoute::AdminTeams
        | ManagedProxyRoute::AdminUpdateEntitlement
        | ManagedProxyRoute::AdminUpdateTeamEntitlement
        | ManagedProxyRoute::AdminBillingRenewals
        | ManagedProxyRoute::AdminBillingReconcile
        | ManagedProxyRoute::BillingPaymentWebhook
        | ManagedProxyRoute::AdminBlockUser
        | ManagedProxyRoute::AdminUnblockUser
        | ManagedProxyRoute::AdminAuditEvents => unreachable!(),
        ManagedProxyRoute::Models => {
            if !request.method.eq_ignore_ascii_case("GET") {
                return Ok(error_response(
                    405,
                    "method_not_allowed",
                    "模型列表只支持 GET。",
                ));
            }
            let verification = store.verify_session_token(&access_token)?;
            if !verification.authenticated {
                return Ok(error_response(
                    401,
                    verification.reason.as_deref().unwrap_or("unauthorized"),
                    "极义后端 session token 无效或已过期。",
                ));
            }
            forward_models_request(config, client).await
        }
        ManagedProxyRoute::Responses => {
            if !request.method.eq_ignore_ascii_case("POST") {
                return Ok(error_response(
                    405,
                    "method_not_allowed",
                    "Responses 请求只支持 POST。",
                ));
            }
            let quota = store.quota_snapshot(&access_token)?;
            if !quota.authenticated {
                return Ok(error_response(
                    401,
                    quota.reason.as_deref().unwrap_or("unauthorized"),
                    "极义后端 session token 无效或已过期。",
                ));
            }
            if let Some(quota) = quota.quota.as_ref() {
                if let Some(remaining_tokens) = quota.remaining_tokens {
                    let estimated_request_tokens =
                        estimate_tokens_from_bytes(request.body.len(), 0);
                    if estimated_request_tokens > remaining_tokens {
                        return Ok(error_response(
                            429,
                            "quota_exceeded",
                            "极义账号今日额度不足，请升级套餐或明天再试。",
                        ));
                    }
                }
            }
            let content_type = request_content_type(&request);
            let response =
                forward_responses_request(config, client, request.body.clone(), content_type).await;
            if let Ok(response) = response.as_ref() {
                record_backend_usage(store, &access_token, &request.path, &request.body, response);
            }
            response
        }
    }
}

fn handle_admin_request(
    request: &ManagedProxyHttpRequest,
    route: ManagedProxyRoute,
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    if !admin_route_has_configured_key(route, config) {
        return Ok(error_response(
            503,
            "missing_admin_key",
            "托管代理未配置该管理接口 API Key。",
        ));
    }

    let access_token = bearer_token(request);
    if access_token.trim().is_empty() {
        return Ok(error_response(
            401,
            "missing_token",
            "缺少极义管理 API Key。",
        ));
    }
    let token = access_token.trim();
    let Some(auth_role) = admin_auth_role_for_route(route, token, config) else {
        return Ok(error_response(
            401,
            "invalid_token",
            "极义管理 API Key 无效。",
        ));
    };

    match route {
        ManagedProxyRoute::AdminUsers => {
            let list = store.admin_user_overviews(audit_limit_from_path(&request.path))?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "day": list.day,
                    "users": list.users
                }),
            ))
        }
        ManagedProxyRoute::AdminTeams => {
            let list = store.admin_team_overviews(audit_limit_from_path(&request.path))?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "day": list.day,
                    "teams": list.teams
                }),
            ))
        }
        ManagedProxyRoute::AdminAuditEvents => {
            let events = store.audit_events(audit_query_from_path(&request.path))?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "auditEvents": events
                }),
            ))
        }
        ManagedProxyRoute::AdminBillingRenewals => {
            if request.method.eq_ignore_ascii_case("GET") {
                let list = store.billing_renewals(audit_limit_from_path(&request.path))?;
                return Ok(ManagedProxyHttpResponse::json(
                    200,
                    json!({
                        "status": "ok",
                        "renewals": list.renewals
                    }),
                ));
            }
            let body: ManagedProxyBillingRenewalRequest = serde_json::from_slice(&request.body)?;
            let renewal = store.record_billing_renewal_with_actor(
                &body.subject_type,
                &body.subject_id,
                &body.plan_id,
                &body.plan_name,
                body.daily_token_limit,
                body.amount_cents,
                &body.currency,
                &body.payment_channel,
                body.external_order_id.as_deref(),
                body.reason.as_deref(),
                "managed_proxy_admin_api",
                Some(auth_role.actor_id()),
            )?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "renewal": renewal
                }),
            ))
        }
        ManagedProxyRoute::AdminBillingReconcile => {
            let receipt = store.reconcile_billing_payment_events_with_actor(
                audit_limit_from_path(&request.path),
                "managed_proxy_admin_api",
                Some(auth_role.actor_id()),
            )?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "reconciliation": receipt
                }),
            ))
        }
        ManagedProxyRoute::AdminUpdateEntitlement => {
            let body: ManagedProxyEntitlementRequest = serde_json::from_slice(&request.body)?;
            let receipt = store.set_user_entitlement_with_actor(
                &body.user_id,
                &body.plan_id,
                &body.plan_name,
                body.daily_token_limit,
                body.reason.as_deref(),
                "managed_proxy_admin_api",
                Some(auth_role.actor_id()),
            )?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "entitlement": receipt
                }),
            ))
        }
        ManagedProxyRoute::AdminUpdateTeamEntitlement => {
            let body: ManagedProxyTeamEntitlementRequest = serde_json::from_slice(&request.body)?;
            let receipt = store.set_team_entitlement_with_actor(
                &body.team_id,
                &body.plan_id,
                &body.plan_name,
                body.daily_token_limit,
                body.reason.as_deref(),
                "managed_proxy_admin_api",
                Some(auth_role.actor_id()),
            )?;
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "teamEntitlement": receipt
                }),
            ))
        }
        ManagedProxyRoute::AdminBlockUser | ManagedProxyRoute::AdminUnblockUser => {
            let body: ManagedProxyUserAccessRequest = serde_json::from_slice(&request.body)?;
            let receipt = match route {
                ManagedProxyRoute::AdminBlockUser => store.set_user_access_status_with_actor(
                    &body.user_id,
                    "blocked",
                    body.reason.as_deref(),
                    "managed_proxy_admin_api",
                    Some(auth_role.actor_id()),
                )?,
                ManagedProxyRoute::AdminUnblockUser => store.set_user_access_status_with_actor(
                    &body.user_id,
                    "active",
                    None,
                    "managed_proxy_admin_api",
                    Some(auth_role.actor_id()),
                )?,
                _ => unreachable!(),
            };
            Ok(ManagedProxyHttpResponse::json(
                200,
                json!({
                    "status": "ok",
                    "userAccess": receipt
                }),
            ))
        }
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedProxyAdminAuthRole {
    FullAdmin,
    UserRead,
    Billing,
    Access,
    Audit,
}

impl ManagedProxyAdminAuthRole {
    fn actor_id(self) -> &'static str {
        match self {
            Self::FullAdmin => "admin_api_key",
            Self::UserRead => "user_read_api_key",
            Self::Billing => "billing_api_key",
            Self::Access => "access_api_key",
            Self::Audit => "audit_api_key",
        }
    }
}

fn admin_route_has_configured_key(route: ManagedProxyRoute, config: &ManagedProxyConfig) -> bool {
    !config.admin_api_key.trim().is_empty()
        || match route {
            ManagedProxyRoute::AdminUsers | ManagedProxyRoute::AdminTeams => {
                !config.user_read_api_key.trim().is_empty()
            }
            ManagedProxyRoute::AdminUpdateEntitlement
            | ManagedProxyRoute::AdminUpdateTeamEntitlement
            | ManagedProxyRoute::AdminBillingRenewals
            | ManagedProxyRoute::AdminBillingReconcile => !config.billing_api_key.trim().is_empty(),
            ManagedProxyRoute::AdminBlockUser | ManagedProxyRoute::AdminUnblockUser => {
                !config.access_api_key.trim().is_empty()
            }
            ManagedProxyRoute::AdminAuditEvents => !config.audit_api_key.trim().is_empty(),
            _ => false,
        }
}

fn admin_auth_role_for_route(
    route: ManagedProxyRoute,
    token: &str,
    config: &ManagedProxyConfig,
) -> Option<ManagedProxyAdminAuthRole> {
    if !config.admin_api_key.trim().is_empty() && token == config.admin_api_key.trim() {
        return Some(ManagedProxyAdminAuthRole::FullAdmin);
    }
    match route {
        ManagedProxyRoute::AdminUsers | ManagedProxyRoute::AdminTeams
            if !config.user_read_api_key.trim().is_empty()
                && token == config.user_read_api_key.trim() =>
        {
            Some(ManagedProxyAdminAuthRole::UserRead)
        }
        ManagedProxyRoute::AdminUpdateEntitlement
        | ManagedProxyRoute::AdminUpdateTeamEntitlement
        | ManagedProxyRoute::AdminBillingRenewals
        | ManagedProxyRoute::AdminBillingReconcile
            if !config.billing_api_key.trim().is_empty()
                && token == config.billing_api_key.trim() =>
        {
            Some(ManagedProxyAdminAuthRole::Billing)
        }
        ManagedProxyRoute::AdminBlockUser | ManagedProxyRoute::AdminUnblockUser
            if !config.access_api_key.trim().is_empty()
                && token == config.access_api_key.trim() =>
        {
            Some(ManagedProxyAdminAuthRole::Access)
        }
        ManagedProxyRoute::AdminAuditEvents
            if !config.audit_api_key.trim().is_empty() && token == config.audit_api_key.trim() =>
        {
            Some(ManagedProxyAdminAuthRole::Audit)
        }
        _ => None,
    }
}

fn handle_payment_webhook_request(
    request: &ManagedProxyHttpRequest,
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    if config.payment_webhook_api_key.trim().is_empty() {
        return Ok(error_response(
            503,
            "missing_payment_webhook_key",
            "托管代理未配置支付回调 API Key。",
        ));
    }

    let access_token = bearer_token(request);
    if access_token.trim().is_empty() {
        return Ok(error_response(
            401,
            "missing_token",
            "缺少极义支付回调 API Key。",
        ));
    }
    if access_token.trim() != config.payment_webhook_api_key.trim() {
        return Ok(error_response(
            401,
            "invalid_token",
            "极义支付回调 API Key 无效。",
        ));
    }
    if let Some(response) = validate_payment_webhook_signature(request, config) {
        return Ok(response);
    }

    let raw_value: Value = serde_json::from_slice(&request.body)?;
    let body: ManagedProxyPaymentWebhookRequest = serde_json::from_value(raw_value.clone())?;
    if let Some(response) =
        validate_payment_webhook_official_signature(request, config, &body, &raw_value)
    {
        return Ok(response);
    }
    let raw_payload = body.raw_payload.as_ref().unwrap_or(&raw_value);
    let receipt = store.record_billing_payment_webhook_with_actor(
        &body.provider,
        body.gateway_event_id.as_deref(),
        &body.external_order_id,
        &body.status,
        &body.subject_type,
        &body.subject_id,
        &body.plan_id,
        &body.plan_name,
        body.daily_token_limit,
        body.amount_cents,
        &body.currency,
        body.payment_channel.as_deref(),
        body.reason.as_deref(),
        raw_payload,
        "payment_webhook_api",
        Some("payment_webhook_api_key"),
    )?;
    Ok(ManagedProxyHttpResponse::json(
        200,
        json!({
            "status": "ok",
            "payment": receipt
        }),
    ))
}

fn validate_payment_webhook_signature(
    request: &ManagedProxyHttpRequest,
    config: &ManagedProxyConfig,
) -> Option<ManagedProxyHttpResponse> {
    let secret = config.payment_webhook_signature_secret.trim();
    if secret.is_empty() {
        return None;
    }
    let timestamp = first_header_value(
        request,
        &[
            "x-jiyi-payment-timestamp",
            "x-jiyi-signature-timestamp",
            "x-payment-timestamp",
        ],
    );
    let Some(timestamp) = timestamp else {
        return Some(error_response(
            401,
            "missing_payment_signature",
            "支付回调已启用验签，但缺少 X-Jiyi-Payment-Timestamp。",
        ));
    };
    let timestamp_ms = match payment_signature_timestamp_ms(timestamp) {
        Some(value) => value,
        None => {
            return Some(error_response(
                401,
                "invalid_payment_signature",
                "支付回调时间戳无效。",
            ));
        }
    };
    let now_ms = OffsetDateTime::now_utc()
        .unix_timestamp()
        .saturating_mul(1000);
    if timestamp_ms < now_ms.saturating_sub(PAYMENT_WEBHOOK_SIGNATURE_TOLERANCE_MS)
        || timestamp_ms > now_ms.saturating_add(PAYMENT_WEBHOOK_SIGNATURE_TOLERANCE_MS)
    {
        return Some(error_response(
            401,
            "stale_payment_signature",
            "支付回调签名时间戳已过期。",
        ));
    }
    let signature = first_header_value(
        request,
        &[
            "x-jiyi-payment-signature",
            "x-jiyi-signature",
            "x-payment-signature",
        ],
    );
    let Some(signature) = signature else {
        return Some(error_response(
            401,
            "missing_payment_signature",
            "支付回调已启用验签，但缺少 X-Jiyi-Payment-Signature。",
        ));
    };
    let Some(signature_bytes) = payment_signature_bytes(signature) else {
        return Some(error_response(
            401,
            "invalid_payment_signature",
            "支付回调签名格式无效。",
        ));
    };
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(timestamp.trim().as_bytes());
    mac.update(b".");
    mac.update(&request.body);
    if mac.verify_slice(&signature_bytes).is_err() {
        return Some(error_response(
            401,
            "invalid_payment_signature",
            "支付回调签名校验失败。",
        ));
    }
    None
}

fn validate_payment_webhook_official_signature(
    request: &ManagedProxyHttpRequest,
    config: &ManagedProxyConfig,
    body: &ManagedProxyPaymentWebhookRequest,
    raw_value: &Value,
) -> Option<ManagedProxyHttpResponse> {
    let alipay_key = config.payment_webhook_alipay_public_key.trim();
    if !alipay_key.is_empty()
        && payment_webhook_matches_provider(body, raw_value, &["alipay", "支付宝"])
    {
        return validate_alipay_payment_signature(alipay_key, body, raw_value);
    }

    let wechatpay_key = config.payment_webhook_wechatpay_public_key.trim();
    if !wechatpay_key.is_empty()
        && payment_webhook_matches_provider(
            body,
            raw_value,
            &["wechatpay", "wechat", "wxpay", "weixin", "微信"],
        )
    {
        return validate_wechatpay_payment_signature(wechatpay_key, request);
    }

    None
}

fn validate_alipay_payment_signature(
    public_key: &str,
    body: &ManagedProxyPaymentWebhookRequest,
    raw_value: &Value,
) -> Option<ManagedProxyHttpResponse> {
    let payload = body
        .raw_payload
        .as_ref()
        .and_then(Value::as_object)
        .or_else(|| raw_value.as_object());
    let Some(payload) = payload else {
        return Some(error_response(
            401,
            "missing_official_payment_signature",
            "支付宝回调已启用官方验签，但缺少原始回调参数。",
        ));
    };
    let sign = payload
        .get("sign")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(sign) = sign else {
        return Some(error_response(
            401,
            "missing_official_payment_signature",
            "支付宝回调已启用官方验签，但缺少 sign。",
        ));
    };
    if let Some(sign_type) = payload
        .get("sign_type")
        .or_else(|| payload.get("signType"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !sign_type.eq_ignore_ascii_case("RSA2") {
            return Some(error_response(
                401,
                "unsupported_official_payment_signature",
                "支付宝回调仅支持 RSA2 官方验签。",
            ));
        }
    }
    let Some(message) = alipay_signature_message(payload) else {
        return Some(error_response(
            401,
            "invalid_official_payment_signature",
            "支付宝回调原始参数无法生成验签串。",
        ));
    };
    if !verify_rsa_sha256_signature(public_key, message.as_bytes(), sign) {
        return Some(error_response(
            401,
            "invalid_official_payment_signature",
            "支付宝回调官方签名校验失败。",
        ));
    }
    None
}

fn validate_wechatpay_payment_signature(
    public_key: &str,
    request: &ManagedProxyHttpRequest,
) -> Option<ManagedProxyHttpResponse> {
    let timestamp = first_header_value(
        request,
        &[
            "wechatpay-timestamp",
            "x-wechatpay-timestamp",
            "x-wxpay-timestamp",
        ],
    );
    let nonce = first_header_value(
        request,
        &["wechatpay-nonce", "x-wechatpay-nonce", "x-wxpay-nonce"],
    );
    let signature_value = first_header_value(
        request,
        &[
            "wechatpay-signature",
            "x-wechatpay-signature",
            "x-wxpay-signature",
        ],
    );
    let (Some(timestamp), Some(nonce), Some(signature_value)) = (timestamp, nonce, signature_value)
    else {
        return Some(error_response(
            401,
            "missing_official_payment_signature",
            "微信支付回调已启用官方验签，但缺少 Wechatpay-Timestamp / Nonce / Signature。",
        ));
    };
    if timestamp
        .parse::<i64>()
        .ok()
        .filter(|value| *value > 0)
        .is_none()
    {
        return Some(error_response(
            401,
            "invalid_official_payment_signature",
            "微信支付回调时间戳无效。",
        ));
    }
    let mut message = Vec::new();
    message.extend_from_slice(timestamp.trim().as_bytes());
    message.push(b'\n');
    message.extend_from_slice(nonce.trim().as_bytes());
    message.push(b'\n');
    message.extend_from_slice(&request.body);
    message.push(b'\n');
    if !verify_rsa_sha256_signature(public_key, &message, signature_value) {
        return Some(error_response(
            401,
            "invalid_official_payment_signature",
            "微信支付回调官方签名校验失败。",
        ));
    }
    None
}

fn payment_webhook_matches_provider(
    body: &ManagedProxyPaymentWebhookRequest,
    raw_value: &Value,
    aliases: &[&str],
) -> bool {
    let mut values = vec![body.provider.as_str()];
    if let Some(channel) = body.payment_channel.as_deref() {
        values.push(channel);
    }
    if let Some(raw) = body.raw_payload.as_ref().or(Some(raw_value)) {
        collect_payment_provider_values(raw, &mut values);
    }
    values.into_iter().any(|value| {
        let value = value.trim().to_ascii_lowercase();
        aliases
            .iter()
            .any(|alias| value.contains(&alias.to_ascii_lowercase()))
    })
}

fn collect_payment_provider_values<'a>(value: &'a Value, out: &mut Vec<&'a str>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in [
        "provider",
        "paymentChannel",
        "payment_channel",
        "channel",
        "trade_type",
        "app_id",
    ] {
        if let Some(value) = object.get(key).and_then(Value::as_str) {
            out.push(value);
        }
    }
}

fn alipay_signature_message(payload: &Map<String, Value>) -> Option<String> {
    let mut pairs: Vec<(String, String)> = payload
        .iter()
        .filter_map(|(key, value)| {
            if key == "sign" || key == "sign_type" || key == "signType" {
                return None;
            }
            let value = payment_signature_scalar_value(value)?;
            (!value.is_empty()).then(|| (key.clone(), value))
        })
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    (!pairs.is_empty()).then(|| {
        pairs
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("&")
    })
}

fn payment_signature_scalar_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => None,
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn verify_rsa_sha256_signature(public_key: &str, message: &[u8], signature_value: &str) -> bool {
    let Some(public_key) = public_key_from_text(public_key) else {
        return false;
    };
    let Some(signature_bytes) = base64::engine::general_purpose::STANDARD
        .decode(signature_value.trim())
        .ok()
    else {
        return false;
    };
    let Ok(signature) = RsaPkcs1v15Signature::try_from(signature_bytes.as_slice()) else {
        return false;
    };
    RsaPkcs1v15VerifyingKey::<Sha256>::new(public_key)
        .verify(message, &signature)
        .is_ok()
}

fn public_key_from_text(value: &str) -> Option<RsaPublicKey> {
    let value = value.trim().replace("\\n", "\n");
    if value.is_empty() {
        return None;
    }
    if value.contains("-----BEGIN") {
        return RsaPublicKey::from_public_key_pem(&value).ok();
    }
    let der_text: String = value.chars().filter(|ch| !ch.is_whitespace()).collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(der_text)
        .ok()?;
    RsaPublicKey::from_public_key_der(&der).ok()
}

fn handle_identity_sync_request(
    request: &ManagedProxyHttpRequest,
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    if config.identity_sync_api_key.trim().is_empty() {
        return Ok(error_response(
            503,
            "missing_identity_sync_key",
            "托管代理未配置极义账号同步 API Key。",
        ));
    }

    let access_token = bearer_token(request);
    if access_token.trim().is_empty() {
        return Ok(error_response(
            401,
            "missing_token",
            "缺少极义账号同步 API Key。",
        ));
    }
    if access_token.trim() != config.identity_sync_api_key.trim() {
        return Ok(error_response(
            401,
            "invalid_token",
            "极义账号同步 API Key 无效。",
        ));
    }

    let body = identity_sync_body_from_request_body(&request.body)?;
    let receipt = store.apply_identity_sync(&body)?;
    let active_session = receipt
        .active_session
        .as_ref()
        .map(|session| ManagedProxyActiveSession {
            user_id: session.user_id.clone(),
            device_id: session.device_id.clone(),
            issued_at_ms: session.issued_at_ms,
            expires_at_ms: session.expires_at_ms,
            access_token: session.access_token.clone(),
        });
    Ok(ManagedProxyHttpResponse::json(
        200,
        json!(ManagedProxyIdentitySyncResponse {
            status: "ok",
            receipt,
            active_session,
        }),
    ))
}

fn identity_sync_body_from_request_body(
    request_body: &[u8],
) -> anyhow::Result<crate::local_backend::IdentitySyncBody> {
    let value: Value = serde_json::from_slice(request_body)?;
    if value.get("body").is_some() {
        let envelope: ManagedProxyIdentitySyncEnvelope = serde_json::from_value(value)?;
        return Ok(envelope.body);
    }
    Ok(serde_json::from_value(value)?)
}

fn managed_proxy_base_url_candidates(base_url: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    let add = |value: &str, urls: &mut Vec<String>, seen: &mut BTreeSet<String>| {
        let trimmed = value.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            urls.push(trimmed.to_string());
        }
    };

    let base_url = base_url.trim();
    let uses_default_relay_family = managed_proxy_base_url_is_default_family(base_url);

    add(base_url, &mut urls, &mut seen);
    if uses_default_relay_family || urls.is_empty() {
        add(JIYI_DEFAULT_RELAY_BASE_URL, &mut urls, &mut seen);
        add(JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK, &mut urls, &mut seen);
    }

    if urls.is_empty() {
        add(JIYI_DEFAULT_RELAY_BASE_URL, &mut urls, &mut seen);
        add(JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK, &mut urls, &mut seen);
    }

    urls
}

fn managed_proxy_base_url_is_default_family(value: &str) -> bool {
    let value = value.to_lowercase();
    value.contains("dashscope.aliyuncs.com")
        || value.contains("bailian")
        || value.contains("aliyuncs.com/compatible-mode")
        || value.contains("apimart.ai")
}

fn should_retry_upstream_status(status: u16) -> bool {
    status >= 500
}

async fn forward_models_with_retries(
    config: &ManagedProxyConfig,
    client: &reqwest::Client,
) -> anyhow::Result<reqwest::Response> {
    let urls = managed_proxy_base_url_candidates(&config.upstream_base_url);
    let key = config.upstream_api_key.trim();

    for (index, base_url) in urls.iter().enumerate() {
        let upstream = client
            .get(crate::protocol_proxy::models_url(base_url))
            .bearer_auth(key)
            .send()
            .await
            .map_err(upstream_error)?;
        if !should_retry_upstream_status(upstream.status().as_u16()) || index + 1 >= urls.len() {
            return Ok(upstream);
        }
    }
    unreachable!("forward_models_with_retries should return in loop");
}

async fn forward_responses_with_retries(
    config: &ManagedProxyConfig,
    client: &reqwest::Client,
    body: Vec<u8>,
    content_type: String,
) -> anyhow::Result<reqwest::Response> {
    let urls = managed_proxy_base_url_candidates(&config.upstream_base_url);
    let key = config.upstream_api_key.trim();
    let content_type = if content_type.trim().is_empty() {
        "application/json".to_string()
    } else {
        content_type
    };

    for (index, base_url) in urls.iter().enumerate() {
        let upstream = client
            .post(crate::protocol_proxy::responses_url(base_url))
            .bearer_auth(key)
            .header(reqwest::header::CONTENT_TYPE, content_type.clone())
            .body(body.clone())
            .send()
            .await
            .map_err(upstream_error)?;
        if !should_retry_upstream_status(upstream.status().as_u16()) || index + 1 >= urls.len() {
            return Ok(upstream);
        }
    }
    unreachable!("forward_responses_with_retries should return in loop");
}

async fn handle_managed_proxy_connection(
    stream: &mut tokio::net::TcpStream,
    remote_addr: SocketAddr,
    config: &ManagedProxyConfig,
    store: &LocalBackendStore,
    client: &reqwest::Client,
) -> anyhow::Result<()> {
    let request_bytes = read_http_request(stream).await?;
    let response = handle_managed_proxy_http_request(&request_bytes, config, store, client).await?;
    log_managed_proxy_response(&request_bytes, &response, remote_addr);
    stream.write_all(&response.to_http_bytes()).await?;
    Ok(())
}

async fn forward_models_request(
    config: &ManagedProxyConfig,
    client: &reqwest::Client,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    let upstream = forward_models_with_retries(config, client).await?;
    response_from_upstream(upstream).await
}

async fn forward_responses_request(
    config: &ManagedProxyConfig,
    client: &reqwest::Client,
    body: Vec<u8>,
    content_type: String,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    let upstream = forward_responses_with_retries(config, client, body, content_type).await?;
    response_from_upstream(upstream).await
}

async fn response_from_upstream(
    upstream: reqwest::Response,
) -> anyhow::Result<ManagedProxyHttpResponse> {
    let status_code = upstream.status().as_u16();
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json; charset=utf-8")
        .to_string();
    let body = upstream.bytes().await?.to_vec();
    Ok(ManagedProxyHttpResponse {
        status_code,
        content_type,
        body,
        headers: Vec::new(),
    })
}

fn record_backend_usage(
    store: &LocalBackendStore,
    access_token: &str,
    path: &str,
    request_body: &[u8],
    response: &ManagedProxyHttpResponse,
) {
    let token_usage = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|value| token_usage_from_value(&value));
    let event = LocalUsageEvent {
        method: "POST".to_string(),
        path: path_without_query(path).to_string(),
        upstream_protocol: "managed_responses".to_string(),
        status_code: response.status_code,
        request_bytes: request_body.len(),
        response_bytes: response.body.len(),
        token_usage,
    };
    match store.record_usage_event(access_token, &event) {
        Ok(receipt) if receipt.authenticated => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "managed_proxy.usage_record_ok",
                json!({
                    "status_code": response.status_code,
                    "day": receipt.day,
                    "recorded_tokens": receipt.recorded_tokens,
                    "total_used_tokens": receipt.total_used_tokens
                }),
            );
        }
        Ok(receipt) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "managed_proxy.usage_record_rejected",
                json!({
                    "status_code": response.status_code,
                    "reason": receipt.reason
                }),
            );
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "managed_proxy.usage_record_failed",
                json!({
                    "status_code": response.status_code,
                    "message": error.to_string()
                }),
            );
        }
    }
}

fn upstream_error(error: reqwest::Error) -> anyhow::Error {
    anyhow::anyhow!("托管代理请求上游失败：{}", error)
}

fn error_response(status_code: u16, code: &str, message: &str) -> ManagedProxyHttpResponse {
    ManagedProxyHttpResponse::json(
        status_code,
        json!({
            "error": {
                "code": code,
                "message": message,
                "type": "jiyi_managed_proxy_error"
            }
        }),
    )
}

fn managed_proxy_route(path: &str) -> Option<ManagedProxyRoute> {
    let path = path_without_query(path);
    if path == "/jiyi/v1/health" {
        return Some(ManagedProxyRoute::Health);
    }
    if matches!(
        path,
        "/jiyi/v1/identity/sync" | "/jiyi/v1/identity-sync" | "/jiyi/v1/sync/identity"
    ) {
        return Some(ManagedProxyRoute::IdentitySync);
    }
    if path == "/jiyi/v1/admin/users/block" {
        return Some(ManagedProxyRoute::AdminBlockUser);
    }
    if path == "/jiyi/v1/admin/users/unblock" {
        return Some(ManagedProxyRoute::AdminUnblockUser);
    }
    if path == "/jiyi/v1/admin/users/entitlement" {
        return Some(ManagedProxyRoute::AdminUpdateEntitlement);
    }
    if path == "/jiyi/v1/admin/users" {
        return Some(ManagedProxyRoute::AdminUsers);
    }
    if path == "/jiyi/v1/admin/teams/entitlement" {
        return Some(ManagedProxyRoute::AdminUpdateTeamEntitlement);
    }
    if path == "/jiyi/v1/admin/teams" {
        return Some(ManagedProxyRoute::AdminTeams);
    }
    if path == "/jiyi/v1/admin/billing/renewals" {
        return Some(ManagedProxyRoute::AdminBillingRenewals);
    }
    if path == "/jiyi/v1/admin/billing/reconcile" {
        return Some(ManagedProxyRoute::AdminBillingReconcile);
    }
    if matches!(
        path,
        "/jiyi/v1/billing/payment-webhook" | "/jiyi/v1/billing/payment_webhook"
    ) {
        return Some(ManagedProxyRoute::BillingPaymentWebhook);
    }
    if path == "/jiyi/v1/admin/audit/events" {
        return Some(ManagedProxyRoute::AdminAuditEvents);
    }
    if crate::protocol_proxy::is_models_proxy_path(path) {
        return Some(ManagedProxyRoute::Models);
    }
    if crate::protocol_proxy::is_responses_proxy_path(path) {
        return Some(ManagedProxyRoute::Responses);
    }
    None
}

fn path_without_query(path: &str) -> &str {
    path.split_once('?').map_or(path, |(path, _)| path)
}

fn audit_limit_from_path(path: &str) -> usize {
    let Some((_, query)) = path.split_once('?') else {
        return 50;
    };
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(name, value)| {
            if name == "limit" {
                value.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(50)
        .clamp(1, 500)
}

fn audit_query_from_path(path: &str) -> LocalBackendAuditEventQuery {
    LocalBackendAuditEventQuery {
        limit: audit_limit_from_path(path),
        event_type: query_value_from_path(path, "eventType"),
        actor_type: query_value_from_path(path, "actorType"),
        subject_user_id: query_value_from_path(path, "subjectUserId"),
    }
}

fn query_value_from_path(path: &str, key: &str) -> Option<String> {
    let (_, query) = path.split_once('?')?;
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(name, value)| {
            if name == key {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            } else {
                None
            }
        })
}

fn parse_http_request(request_bytes: &[u8]) -> anyhow::Result<ManagedProxyHttpRequest> {
    let Some(header_end) = find_header_end(request_bytes) else {
        anyhow::bail!("HTTP 请求缺少 header 结束符");
    };
    let header_text = String::from_utf8_lossy(&request_bytes[..header_end]);
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    if method.is_empty() || path.is_empty() {
        anyhow::bail!("HTTP 请求行无效");
    }
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let body_start = header_end + 4;
    let body = request_bytes
        .get(body_start..)
        .map_or_else(Vec::new, |body| body.to_vec());
    Ok(ManagedProxyHttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn bearer_token(request: &ManagedProxyHttpRequest) -> String {
    let Some(value) = request.headers.get("authorization") else {
        return String::new();
    };
    value
        .trim()
        .strip_prefix("Bearer ")
        .or_else(|| value.trim().strip_prefix("bearer "))
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn first_header_value<'a>(request: &'a ManagedProxyHttpRequest, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| request.headers.get(*name).map(String::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn payment_signature_timestamp_ms(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let parsed = value.parse::<i64>().ok()?;
    if parsed <= 0 {
        return None;
    }
    if value.len() >= 13 {
        Some(parsed)
    } else {
        parsed.checked_mul(1000)
    }
}

fn payment_signature_bytes(value: &str) -> Option<Vec<u8>> {
    let value = value.trim();
    let value = value
        .strip_prefix("sha256=")
        .or_else(|| value.strip_prefix("SHA256="))
        .unwrap_or(value)
        .trim();
    if value.len() == 64 && value.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return hex_to_bytes(value);
    }
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()
        .filter(|bytes| bytes.len() == 32)
}

fn hex_to_bytes(value: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for chunk in bytes.chunks_exact(2) {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn request_content_type(request: &ManagedProxyHttpRequest) -> String {
    request
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_default()
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut chunk = vec![0_u8; 4096];
    let mut header_end = None;
    let mut content_length = 0_usize;

    loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                content_length = content_length_from_headers(&buffer[..end]).unwrap_or(0);
            }
        }
        if let Some(end) = header_end {
            if buffer.len() >= end + 4 + content_length {
                break;
            }
        }
        if buffer.len() > MAX_MANAGED_PROXY_REQUEST_BYTES {
            anyhow::bail!("HTTP 请求过大");
        }
    }

    Ok(buffer)
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length_from_headers(headers: &[u8]) -> Option<usize> {
    let text = String::from_utf8_lossy(headers);
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

fn http_status_line(status_code: u16) -> String {
    format!("{status_code} {}", http_status_reason(status_code))
}

fn http_status_reason(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Upstream",
    }
}

fn first_env_value(keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn first_env_or_file_value(value_keys: &[&str], path_keys: &[&str]) -> String {
    let direct = first_env_value(value_keys);
    if !direct.is_empty() {
        return direct;
    }

    path_keys
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn managed_proxy_listen_addr_from_env() -> anyhow::Result<SocketAddr> {
    if let Ok(value) = std::env::var("JIYI_MANAGED_PROXY_LISTEN") {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.parse()?);
        }
    }

    let host = std::env::var("JIYI_MANAGED_PROXY_HOST")
        .unwrap_or_else(|_| Ipv4Addr::LOCALHOST.to_string());
    let port = std::env::var("JIYI_MANAGED_PROXY_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .unwrap_or(DEFAULT_MANAGED_PROXY_PORT);
    Ok(format!("{}:{}", host.trim(), port).parse()?)
}

fn log_managed_proxy_response(
    request_bytes: &[u8],
    response: &ManagedProxyHttpResponse,
    remote_addr: SocketAddr,
) {
    let request_line = String::from_utf8_lossy(request_bytes)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "managed_proxy.response",
        json!({
            "request_line": request_line,
            "status_code": response.status_code,
            "remote_addr": remote_addr.to_string()
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{JIYI_DEFAULT_RELAY_BASE_URL, JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK};

    #[test]
    fn managed_proxy_base_url_candidates_includes_apimart_fallback_when_bailian_used() {
        let candidates = managed_proxy_base_url_candidates(JIYI_DEFAULT_RELAY_BASE_URL);
        assert_eq!(
            candidates.first().map(String::as_str),
            Some(JIYI_DEFAULT_RELAY_BASE_URL)
        );
        assert!(
            candidates
                .iter()
                .any(|value| value == JIYI_DEFAULT_RELAY_BASE_URL_FALLBACK.trim_end_matches('/'))
        );
    }

    #[test]
    fn managed_proxy_base_url_candidates_keeps_custom_base_without_apimart_fallback() {
        let candidates = managed_proxy_base_url_candidates("https://custom.provider.example/v1");
        assert_eq!(candidates, vec!["https://custom.provider.example/v1"]);
    }
}
