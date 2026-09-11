use std::time::Duration as StdDuration;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode, Url};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::{
    format_timestamp, random_opaque_id, validate_opaque_id, validate_request_base,
    AppRequestAuthentication, CloudError, CloudService, RequestBase, TaskSnapshot,
    CONTRACT_VERSION,
};

const PUSH_LEASE_SECONDS: i64 = 30;
const MAX_PUSH_ATTEMPTS: i64 = 8;
const MAX_PROVIDER_RESPONSE_BYTES: usize = 64 * 1024;
const SUCCESS_CODE: &str = "80000000";
const NOTIFICATION_PUSH_TYPE: &str = "0";

pub struct HuaweiPushConfig {
    send_url: Url,
    key_id: String,
    sub_account: String,
    private_key: EncodingKey,
}

impl HuaweiPushConfig {
    pub fn from_base64url_service_account(
        send_url: &str,
        encoded_service_account: &str,
    ) -> Result<Self, String> {
        let send_url = validate_https_url(send_url, "Push send URL")?;
        if encoded_service_account.len() > 64 * 1024 {
            return Err("Push service account configuration is too large".into());
        }
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded_service_account)
            .map_err(|_| "Push service account must be unpadded base64url JSON".to_owned())?;
        let service_account: HuaweiServiceAccount = serde_json::from_slice(&decoded)
            .map_err(|_| "Push service account JSON is invalid".to_owned())?;
        validate_service_account(&service_account, &send_url)?;
        let private_key = EncodingKey::from_rsa_pem(service_account.private_key.as_bytes())
            .map_err(|_| "Push service account private key is invalid".to_owned())?;
        Ok(Self {
            send_url,
            key_id: service_account.key_id,
            sub_account: service_account.sub_account,
            private_key,
        })
    }
}

fn validate_service_account(
    service_account: &HuaweiServiceAccount,
    send_url: &Url,
) -> Result<(), String> {
    if service_account.project_id.len() < 6
        || service_account.project_id.len() > 30
        || !service_account
            .project_id
            .bytes()
            .all(|value| value.is_ascii_digit())
    {
        return Err("Push service account project ID is invalid".into());
    }
    if service_account.key_id.trim().is_empty()
        || service_account.key_id.len() > 512
        || service_account.sub_account.trim().is_empty()
        || service_account.sub_account.len() > 512
        || service_account.private_key.len() > 32 * 1024
    {
        return Err("Push service account identity is invalid".into());
    }
    let expected_path = format!("/v3/{}/messages:send", service_account.project_id);
    if send_url.host_str() != Some("push-api.cloud.huawei.com")
        || send_url.port().is_some()
        || send_url.path() != expected_path
        || send_url.query().is_some()
        || send_url.fragment().is_some()
    {
        return Err("Push send URL does not match the service account project".into());
    }
    Ok(())
}

fn validate_https_url(value: &str, label: &str) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|_| format!("{label} must be an absolute HTTPS URL"))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(format!("{label} must be an absolute HTTPS URL"));
    }
    Ok(parsed)
}

#[derive(Debug)]
pub struct PendingPushDelivery {
    push_id: String,
    refresh_ref: String,
    encrypted_push_token: Vec<u8>,
    attempt_count: i64,
}

#[derive(Debug)]
struct CachedJwtToken {
    value: String,
    expires_at: Instant,
}

#[derive(Deserialize)]
struct HuaweiServiceAccount {
    project_id: String,
    key_id: String,
    private_key: String,
    sub_account: String,
}

#[derive(Serialize)]
struct HuaweiJwtClaims<'a> {
    aud: &'static str,
    iss: &'a str,
    exp: i64,
    iat: i64,
}

fn build_huawei_jwt_header(key_id: &str) -> Header {
    let mut header = Header::new(Algorithm::PS256);
    header.typ = Some("JWT".to_owned());
    header.kid = Some(key_id.to_owned());
    header
}

fn build_huawei_jwt_claims(sub_account: &str, issued_at: i64) -> HuaweiJwtClaims<'_> {
    HuaweiJwtClaims {
        aud: "https://oauth-login.cloud.huawei.com/oauth2/v3/token",
        iss: sub_account,
        exp: issued_at + 3600,
        iat: issued_at,
    }
}

#[derive(Deserialize)]
struct ProviderResponse {
    #[serde(default)]
    code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RefreshHint<'a> {
    message_type: &'static str,
    refresh_ref: &'a str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PushRefreshQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRefreshResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub remote_task_id: String,
    pub terminal_state_version: i64,
}

#[derive(Debug, PartialEq, Eq)]
struct PushRefreshTarget {
    pc_device_id: String,
    installation_id: String,
    binding_epoch: i64,
    remote_task_id: String,
    terminal_state_version: i64,
}

#[derive(Debug, Clone, Copy)]
enum PushSendResult {
    Accepted,
    Retry(&'static str),
    Dead(&'static str),
}

pub struct HuaweiPushClient {
    config: HuaweiPushConfig,
    http: Client,
    jwt_token: Mutex<Option<CachedJwtToken>>,
}

impl HuaweiPushClient {
    pub fn new(config: HuaweiPushConfig) -> Result<Self, String> {
        let http = Client::builder()
            .https_only(true)
            .connect_timeout(StdDuration::from_secs(5))
            .timeout(StdDuration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "failed to create Huawei Push HTTPS client".to_owned())?;
        Ok(Self {
            config,
            http,
            jwt_token: Mutex::new(None),
        })
    }

    async fn send_refresh(&self, push_token: &str, refresh_ref: &str) -> PushSendResult {
        let access_token = match self.jwt_token().await {
            Ok(token) => token,
            Err(result) => return result,
        };
        let payload = match build_refresh_payload(push_token, refresh_ref) {
            Ok(payload) => payload,
            Err(_) => return PushSendResult::Dead("payload_invalid"),
        };
        let response = match self
            .http
            .post(self.config.send_url.clone())
            .bearer_auth(access_token)
            .header("push-type", NOTIFICATION_PUSH_TYPE)
            .json(&payload)
            .send()
            .await
        {
            Ok(response) => response,
            Err(_) => return PushSendResult::Retry("network_unavailable"),
        };
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            *self.jwt_token.lock().await = None;
            return PushSendResult::Retry("provider_authentication_failed");
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return PushSendResult::Retry("provider_rate_limited");
        }
        if status.is_server_error() {
            return PushSendResult::Retry("provider_unavailable");
        }
        if status.is_client_error() {
            return PushSendResult::Dead("provider_rejected_request");
        }
        let body = match response.bytes().await {
            Ok(body) if body.len() <= MAX_PROVIDER_RESPONSE_BYTES => body,
            _ => return PushSendResult::Retry("provider_response_invalid"),
        };
        let provider = match serde_json::from_slice::<ProviderResponse>(&body) {
            Ok(provider) => provider,
            Err(_) => return PushSendResult::Retry("provider_response_invalid"),
        };
        if provider.code == SUCCESS_CODE {
            PushSendResult::Accepted
        } else {
            if provider.code.starts_with("802") {
                *self.jwt_token.lock().await = None;
            }
            let provider_code = if provider.code.len() == 8
                && provider.code.bytes().all(|value| value.is_ascii_digit())
            {
                provider.code.as_str()
            } else {
                "invalid"
            };
            println!("workagents_push event=huawei_rejected provider_code={provider_code}");
            classify_provider_rejection(&provider.code)
        }
    }

    async fn jwt_token(&self) -> Result<String, PushSendResult> {
        let mut cached = self.jwt_token.lock().await;
        if let Some(token) = cached.as_ref() {
            if token.expires_at > Instant::now() + StdDuration::from_secs(60) {
                return Ok(token.value.clone());
            }
        }
        let issued_at = Utc::now().timestamp();
        let claims = build_huawei_jwt_claims(&self.config.sub_account, issued_at);
        let header = build_huawei_jwt_header(&self.config.key_id);
        let value = encode(&header, &claims, &self.config.private_key)
            .map_err(|_| PushSendResult::Dead("jwt_generation_failed"))?;
        *cached = Some(CachedJwtToken {
            value: value.clone(),
            expires_at: Instant::now() + StdDuration::from_secs(3600),
        });
        Ok(value)
    }
}

fn classify_provider_rejection(code: &str) -> PushSendResult {
    match code.get(..3) {
        Some("801") => PushSendResult::Dead("provider_rejected_request"),
        Some("802") => PushSendResult::Retry("provider_authentication_failed"),
        Some("803") => PushSendResult::Dead("provider_rejected_token"),
        Some("804") => PushSendResult::Retry("provider_rate_limited"),
        Some("805" | "806" | "807" | "808" | "809" | "810") => {
            PushSendResult::Retry("provider_unavailable")
        }
        _ => PushSendResult::Retry("provider_response_invalid"),
    }
}

fn build_refresh_payload(
    push_token: &str,
    refresh_ref: &str,
) -> Result<serde_json::Value, serde_json::Error> {
    let refresh_hint = serde_json::to_value(RefreshHint {
        message_type: "task-refresh",
        refresh_ref,
    })?;
    Ok(serde_json::json!({
        "payload": {
            "notification": {
                "category": "WORK",
                "title": "轩++远程",
                "body": "电脑上的任务已结束，点击查看最新状态",
                "clickAction": {
                    "actionType": 0,
                    "data": refresh_hint
                }
            }
        },
        "target": { "token": [push_token] }
    }))
}

pub struct PushDispatcher {
    service: CloudService,
    client: HuaweiPushClient,
}

impl PushDispatcher {
    pub fn new(service: CloudService, config: HuaweiPushConfig) -> Result<Self, String> {
        Ok(Self {
            service,
            client: HuaweiPushClient::new(config)?,
        })
    }

    pub async fn run(self) {
        loop {
            let now = Utc::now();
            let delivery = match self.service.claim_pending_push(now) {
                Ok(delivery) => delivery,
                Err(_) => {
                    eprintln!("workagents_push event=outbox_claim_failed");
                    tokio::time::sleep(StdDuration::from_secs(2)).await;
                    continue;
                }
            };
            let Some(delivery) = delivery else {
                tokio::time::sleep(StdDuration::from_secs(2)).await;
                continue;
            };
            println!(
                "workagents_push event=outbox_claimed attempt={}",
                delivery.attempt_count
            );
            let token = self
                .service
                .decrypt_push_token(&delivery.encrypted_push_token)
                .ok()
                .and_then(|token| String::from_utf8(token).ok());
            let result = match token {
                Some(token) if !token.is_empty() => {
                    self.client
                        .send_refresh(&token, &delivery.refresh_ref)
                        .await
                }
                _ => PushSendResult::Dead("stored_token_invalid"),
            };
            let now = Utc::now();
            let update = match result {
                PushSendResult::Accepted => self.service.mark_push_sent(&delivery.push_id, now),
                PushSendResult::Retry(error_code) => self.service.mark_push_retry(
                    &delivery.push_id,
                    delivery.attempt_count,
                    error_code,
                    now,
                ),
                PushSendResult::Dead(error_code) => {
                    self.service
                        .mark_push_dead(&delivery.push_id, error_code, now)
                }
            };
            match (result, update) {
                (PushSendResult::Accepted, Ok(())) => println!(
                    "workagents_push event=huawei_accepted attempt={}",
                    delivery.attempt_count
                ),
                (PushSendResult::Retry(error_code), Ok(())) => println!(
                    "workagents_push event=retry code={error_code} attempt={}",
                    delivery.attempt_count
                ),
                (PushSendResult::Dead(error_code), Ok(())) => println!(
                    "workagents_push event=dead code={error_code} attempt={}",
                    delivery.attempt_count
                ),
                (PushSendResult::Accepted, Err(_)) => {
                    eprintln!("workagents_push event=outbox_update_failed outcome=accepted")
                }
                (PushSendResult::Retry(_), Err(_)) => {
                    eprintln!("workagents_push event=outbox_update_failed outcome=retry")
                }
                (PushSendResult::Dead(_), Err(_)) => {
                    eprintln!("workagents_push event=outbox_update_failed outcome=dead")
                }
            }
        }
    }
}

impl CloudService {
    pub fn resolve_push_refresh(
        &self,
        refresh_ref: &str,
        request: PushRefreshQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<PushRefreshResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/push-refresh-query",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment(),
            now,
        )?;
        validate_opaque_id(refresh_ref)?;
        validate_opaque_id(&request.app_device_id)?;
        if authenticated_app_device_id != request.app_device_id {
            return Err(CloudError::InvalidRequest);
        }
        let target = self.resolve_push_refresh_for_app(refresh_ref, &request.app_device_id)?;
        Ok(PushRefreshResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/push-refresh",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment().as_str().to_owned(),
            server_received_at: format_timestamp(now),
            pc_device_id: target.pc_device_id,
            installation_id: target.installation_id,
            binding_epoch: target.binding_epoch,
            remote_task_id: target.remote_task_id,
            terminal_state_version: target.terminal_state_version,
        })
    }

    fn resolve_push_refresh_for_app(
        &self,
        refresh_ref: &str,
        app_device_id: &str,
    ) -> Result<PushRefreshTarget, CloudError> {
        self.connection()?
            .query_row(
                "SELECT outbox.pc_device_id, outbox.installation_id, outbox.binding_epoch,
                        outbox.remote_task_id, outbox.terminal_state_version
                 FROM push_outbox AS outbox
                 INNER JOIN bindings
                   ON bindings.environment = outbox.environment
                  AND bindings.app_device_id = outbox.app_device_id
                  AND bindings.pc_device_id = outbox.pc_device_id
                  AND bindings.installation_id = outbox.installation_id
                  AND bindings.binding_epoch = outbox.binding_epoch
                  AND bindings.state = 'active'
                 WHERE outbox.environment = ?1 AND outbox.refresh_ref = ?2
                   AND outbox.app_device_id = ?3",
                params![self.environment().as_str(), refresh_ref, app_device_id],
                |row| {
                    Ok(PushRefreshTarget {
                        pc_device_id: row.get(0)?,
                        installation_id: row.get(1)?,
                        binding_epoch: row.get(2)?,
                        remote_task_id: row.get(3)?,
                        terminal_state_version: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::DeviceNotBound)
    }

    fn claim_pending_push(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Option<PendingPushDelivery>, CloudError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "UPDATE push_outbox AS candidate
                 SET status = 'dead', last_error_code = 'binding_inactive',
                     lease_until = NULL, updated_at = ?2
                 WHERE candidate.environment = ?1
                   AND candidate.status IN ('pending', 'retry', 'delivering')
                   AND NOT EXISTS (
                     SELECT 1 FROM bindings
                     WHERE bindings.environment = candidate.environment
                       AND bindings.app_device_id = candidate.app_device_id
                       AND bindings.pc_device_id = candidate.pc_device_id
                       AND bindings.installation_id = candidate.installation_id
                       AND bindings.binding_epoch = candidate.binding_epoch
                       AND bindings.state = 'active'
                   )",
                params![self.environment().as_str(), now.timestamp()],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let pending = transaction
            .query_row(
                "SELECT outbox.push_id, outbox.refresh_ref, devices.push_token_encrypted,
                        outbox.attempt_count
                 FROM push_outbox AS outbox
                 INNER JOIN app_devices AS devices
                   ON devices.environment = outbox.environment
                  AND devices.app_device_id = outbox.app_device_id
                  AND devices.push_token_generation = outbox.push_token_generation
                  AND devices.status = 'active'
                 WHERE outbox.environment = ?1
                   AND ((outbox.status IN ('pending', 'retry') AND outbox.next_attempt_at <= ?2)
                     OR (outbox.status = 'delivering' AND outbox.lease_until <= ?2))
                 ORDER BY outbox.created_at, outbox.push_id LIMIT 1",
                params![self.environment().as_str(), now.timestamp()],
                |row| {
                    Ok(PendingPushDelivery {
                        push_id: row.get(0)?,
                        refresh_ref: row.get(1)?,
                        encrypted_push_token: row.get(2)?,
                        attempt_count: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?;
        if let Some(delivery) = pending.as_ref() {
            transaction
                .execute(
                    "UPDATE push_outbox
                     SET status = 'delivering', attempt_count = attempt_count + 1,
                         lease_until = ?1, updated_at = ?2
                     WHERE environment = ?3 AND push_id = ?4",
                    params![
                        (now + Duration::seconds(PUSH_LEASE_SECONDS)).timestamp(),
                        now.timestamp(),
                        self.environment().as_str(),
                        delivery.push_id
                    ],
                )
                .map_err(|_| CloudError::StorageUnavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(pending.map(|mut delivery| {
            delivery.attempt_count += 1;
            delivery
        }))
    }

    fn mark_push_sent(&self, push_id: &str, now: DateTime<Utc>) -> Result<(), CloudError> {
        self.finish_push(push_id, "sent", None, now, true)
    }

    fn mark_push_dead(
        &self,
        push_id: &str,
        error_code: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        self.finish_push(push_id, "dead", Some(error_code), now, false)
    }

    fn mark_push_retry(
        &self,
        push_id: &str,
        attempt_count: i64,
        error_code: &str,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        if attempt_count >= MAX_PUSH_ATTEMPTS {
            return self.mark_push_dead(push_id, error_code, now);
        }
        let exponent = u32::try_from(attempt_count.saturating_sub(1).min(6)).unwrap_or(6);
        let retry_seconds = 5_i64.saturating_mul(2_i64.pow(exponent)).min(300);
        let changed = self
            .connection()?
            .execute(
                "UPDATE push_outbox
                 SET status = 'retry', next_attempt_at = ?1, lease_until = NULL,
                     last_error_code = ?2, updated_at = ?3
                 WHERE environment = ?4 AND push_id = ?5 AND status = 'delivering'",
                params![
                    (now + Duration::seconds(retry_seconds)).timestamp(),
                    error_code,
                    now.timestamp(),
                    self.environment().as_str(),
                    push_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(CloudError::StorageUnavailable)
        }
    }

    fn finish_push(
        &self,
        push_id: &str,
        status: &str,
        error_code: Option<&str>,
        now: DateTime<Utc>,
        sent: bool,
    ) -> Result<(), CloudError> {
        let changed = self
            .connection()?
            .execute(
                "UPDATE push_outbox
                 SET status = ?1, lease_until = NULL, last_error_code = ?2,
                     updated_at = ?3, sent_at = CASE WHEN ?4 THEN ?3 ELSE sent_at END
                 WHERE environment = ?5 AND push_id = ?6 AND status = 'delivering'",
                params![
                    status,
                    error_code,
                    now.timestamp(),
                    sent,
                    self.environment().as_str(),
                    push_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(CloudError::StorageUnavailable)
        }
    }
}

pub(crate) fn enqueue_terminal_pushes(
    transaction: &Transaction<'_>,
    environment: &str,
    previous: Option<&TaskSnapshot>,
    snapshot: &TaskSnapshot,
    terminal_push_eligible: bool,
    now: DateTime<Utc>,
) -> Result<usize, CloudError> {
    if !should_enqueue_terminal_push(previous, snapshot, terminal_push_eligible) {
        return Ok(0);
    }
    let devices = {
        let mut statement = transaction
            .prepare(
                "SELECT bindings.app_device_id, app_devices.push_token_generation
                 FROM bindings
                 INNER JOIN app_devices
                   ON app_devices.environment = bindings.environment
                  AND app_devices.app_device_id = bindings.app_device_id
                  AND app_devices.status = 'active'
                 WHERE bindings.environment = ?1
                   AND bindings.pc_device_id = ?2
                   AND bindings.installation_id = ?3
                   AND bindings.binding_epoch = ?4
                   AND bindings.state = 'active'",
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let rows = statement
            .query_map(
                params![
                    environment,
                    snapshot.pc_device_id,
                    snapshot.installation_id,
                    snapshot.binding_epoch
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| CloudError::StorageUnavailable)?
    };
    let mut enqueued = 0;
    for (app_device_id, token_generation) in devices {
        enqueued += transaction
            .execute(
                "INSERT OR IGNORE INTO push_outbox(
                   environment, push_id, refresh_ref, app_device_id, pc_device_id,
                   installation_id, binding_epoch, remote_task_id, terminal_state_version,
                   terminal_outcome, push_token_generation, status, attempt_count,
                   next_attempt_at, lease_until, last_error_code, created_at, updated_at, sent_at
                 ) VALUES (
                   ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                   'pending', 0, ?12, NULL, NULL, ?12, ?12, NULL
                 )",
                params![
                    environment,
                    random_opaque_id(),
                    random_opaque_id(),
                    app_device_id,
                    snapshot.pc_device_id,
                    snapshot.installation_id,
                    snapshot.binding_epoch,
                    snapshot.remote_task_id,
                    snapshot.state_version,
                    snapshot.last_turn_outcome,
                    token_generation,
                    now.timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
    }
    Ok(enqueued)
}

fn should_enqueue_terminal_push(
    previous: Option<&TaskSnapshot>,
    snapshot: &TaskSnapshot,
    terminal_push_eligible: bool,
) -> bool {
    let terminal = matches!(snapshot.last_turn_outcome.as_str(), "completed" | "failed");
    terminal_push_eligible
        && terminal
        && previous.is_none_or(|previous| previous.state_version < snapshot.state_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const APP_DEVICE_ID: &str = "app_device_000001";
    const REFRESH_REF: &str = "push_refresh_ref_000001";

    fn service_with_refresh(binding_state: &str) -> (TempDir, CloudService) {
        let temp = TempDir::new().expect("temp directory");
        let service = CloudService::open(
            &temp.path().join("remote.sqlite3"),
            crate::Environment::Dev,
            [17_u8; 32],
        )
        .expect("open service");
        let connection = service.connection().expect("connection");
        connection
            .execute(
                "INSERT INTO app_devices (
                   environment, app_device_id, device_key_id, public_key_der,
                   public_key_digest, push_token_encrypted, push_token_generation,
                   app_display_name, app_version, status, updated_at
                 ) VALUES ('dev', ?1, 'device_key_push_000001', X'01',
                           'digest_push_000001', X'02', 1, 'APP', '0.1.6-dev', 'active', 1)",
                params![APP_DEVICE_ID],
            )
            .expect("app device");
        connection
            .execute(
                "INSERT INTO bindings (
                   environment, binding_id, pc_pairing_message_id, app_device_id, pc_device_id,
                   installation_id, binding_epoch, confirmation_nonce_digest,
                   confirmation_nonce_encrypted, confirmation_expires_at, pc_display_name,
                   app_display_name, safety_phrase, summary_digest, state, created_at, activated_at
                 ) VALUES ('dev', 'binding_push_000001', 'pairing_push_000001', ?1,
                           'pc_device_000001', 'installation_000001', 2,
                           'confirmation_push_000001', X'01', 1, 'PC', 'APP', '青山-流水',
                           'summary_push_000001', ?2, 1, 1)",
                params![APP_DEVICE_ID, binding_state],
            )
            .expect("binding");
        connection
            .execute(
                "INSERT INTO push_outbox (
                   environment, push_id, refresh_ref, app_device_id, pc_device_id,
                   installation_id, binding_epoch, remote_task_id, terminal_state_version,
                   terminal_outcome, push_token_generation, status, attempt_count,
                   next_attempt_at, created_at, updated_at
                 ) VALUES ('dev', 'push_message_000001', ?1, ?2, 'pc_device_000001',
                           'installation_000001', 2, 'remote_task_000001', 8,
                           'completed', 1, 'sent', 1, 1, 1, 1)",
                params![REFRESH_REF, APP_DEVICE_ID],
            )
            .expect("push outbox");
        drop(connection);
        (temp, service)
    }

    fn snapshot(outcome: &str, version: i64) -> TaskSnapshot {
        TaskSnapshot {
            remote_task_id: "remote_task_000001".into(),
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 2,
            name: "敏感任务名".into(),
            workspace_name: "敏感工作区".into(),
            model_label: "模型".into(),
            task_status: "running".into(),
            turn_status: outcome.into(),
            last_turn_outcome: outcome.into(),
            last_reply: None,
            last_reply_state: "absent".into(),
            last_reply_version: None,
            last_error: None,
            pc_observed_at: "2026-08-31T00:00:00Z".into(),
            server_received_at: None,
            state_version: version,
            pc_connection_state: "online".into(),
        }
    }

    #[test]
    fn every_eligible_newer_completed_or_failed_snapshot_enqueues_a_refresh_hint() {
        let running = snapshot("none", 1);
        let completed = snapshot("completed", 2);
        let next_short_turn = snapshot("completed", 3);
        let stale_terminal = snapshot("failed", 2);
        assert!(should_enqueue_terminal_push(
            Some(&running),
            &completed,
            true
        ));
        assert!(should_enqueue_terminal_push(
            Some(&completed),
            &next_short_turn,
            true
        ));
        assert!(!should_enqueue_terminal_push(
            Some(&completed),
            &stale_terminal,
            true
        ));
        assert!(should_enqueue_terminal_push(None, &completed, true));
        assert!(!should_enqueue_terminal_push(None, &running, true));
    }

    #[test]
    fn terminal_snapshot_without_a_live_connection_does_not_enqueue_a_refresh_hint() {
        let running = snapshot("none", 1);
        let completed = snapshot("completed", 2);
        assert!(!should_enqueue_terminal_push(
            Some(&running),
            &completed,
            false
        ));
    }

    #[test]
    fn outbox_is_written_only_for_a_task_end_observed_while_connected() {
        let (_temp, service) = service_with_refresh("active");
        let mut connection = service.connection().expect("connection");
        connection
            .execute("DELETE FROM push_outbox", [])
            .expect("clear seeded outbox");
        let running = snapshot("none", 1);
        let completed = snapshot("completed", 2);

        let disconnected = connection.transaction().expect("transaction");
        assert_eq!(
            enqueue_terminal_pushes(
                &disconnected,
                "dev",
                Some(&running),
                &completed,
                false,
                Utc::now(),
            )
            .expect("disconnected completion"),
            0
        );
        disconnected.commit().expect("commit disconnected case");
        let disconnected_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM push_outbox", [], |row| row.get(0))
            .expect("disconnected outbox count");
        assert_eq!(disconnected_count, 0);

        let connected = connection.transaction().expect("transaction");
        assert_eq!(
            enqueue_terminal_pushes(
                &connected,
                "dev",
                Some(&running),
                &completed,
                true,
                Utc::now(),
            )
            .expect("connected completion"),
            1
        );
        connected.commit().expect("commit connected case");
        let connected_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM push_outbox", [], |row| row.get(0))
            .expect("connected outbox count");
        assert_eq!(connected_count, 1);
    }

    #[test]
    fn refresh_hint_contains_no_task_content_or_internal_identifier() {
        let encoded = serde_json::to_string(&RefreshHint {
            message_type: "task-refresh",
            refresh_ref: "opaque_refresh_0001",
        })
        .expect("refresh hint");
        assert!(encoded.contains("opaque_refresh_0001"));
        assert!(!encoded.contains("remote_task"));
        assert!(!encoded.contains("敏感"));
    }

    #[test]
    fn huawei_payload_is_a_generic_notification_with_only_the_refresh_hint() {
        let payload = build_refresh_payload("push_token_test_0001", "opaque_refresh_0001")
            .expect("refresh payload");
        assert!(payload.get("pushOptions").is_none());
        assert_eq!(payload["target"]["token"][0], "push_token_test_0001");
        assert_eq!(
            payload["payload"]["notification"]["clickAction"]["data"],
            serde_json::json!({
                "messageType": "task-refresh",
                "refreshRef": "opaque_refresh_0001"
            })
        );
        let encoded = serde_json::to_string(&payload).expect("encoded payload");
        assert_eq!(NOTIFICATION_PUSH_TYPE, "0");
        assert!(payload["payload"].get("data").is_none());
        assert!(payload["payload"].get("extraData").is_none());
        assert_eq!(payload["payload"]["notification"]["category"], "WORK");
        assert_eq!(payload["payload"]["notification"]["title"], "轩++远程");
        assert_eq!(
            payload["payload"]["notification"]["body"],
            "电脑上的任务已结束，点击查看最新状态"
        );
        assert_eq!(
            payload["payload"]["notification"]["clickAction"]["actionType"],
            0
        );
        assert!(!encoded.contains("remote_task"));
        assert!(!encoded.contains("敏感"));
    }

    #[test]
    fn huawei_jwt_metadata_matches_the_service_account_contract() {
        let header = build_huawei_jwt_header("service_key_0001");
        assert_eq!(header.alg, Algorithm::PS256);
        assert_eq!(header.typ.as_deref(), Some("JWT"));
        assert_eq!(header.kid.as_deref(), Some("service_key_0001"));

        let claims = build_huawei_jwt_claims("service_account_0001", 1_788_235_200);
        assert_eq!(
            claims.aud,
            "https://oauth-login.cloud.huawei.com/oauth2/v3/token"
        );
        assert_eq!(claims.iss, "service_account_0001");
        assert_eq!(claims.iat, 1_788_235_200);
        assert_eq!(claims.exp, 1_788_238_800);
    }

    #[test]
    fn huawei_send_url_must_match_the_service_account_project() {
        let account = HuaweiServiceAccount {
            project_id: "101653523864848579".to_owned(),
            key_id: "service_key_0001".to_owned(),
            private_key: "test-only-key-placeholder".to_owned(),
            sub_account: "service_account_0001".to_owned(),
        };
        let matching =
            Url::parse("https://push-api.cloud.huawei.com/v3/101653523864848579/messages:send")
                .expect("matching URL");
        assert!(validate_service_account(&account, &matching).is_ok());

        let mismatched =
            Url::parse("https://push-api.cloud.huawei.com/v3/101653523864848570/messages:send")
                .expect("mismatched URL");
        assert!(validate_service_account(&account, &mismatched).is_err());
    }

    #[test]
    fn huawei_response_codes_map_only_to_allowlisted_delivery_outcomes() {
        assert!(matches!(
            classify_provider_rejection("80100003"),
            PushSendResult::Dead("provider_rejected_request")
        ));
        assert!(matches!(
            classify_provider_rejection("80200001"),
            PushSendResult::Retry("provider_authentication_failed")
        ));
        assert!(matches!(
            classify_provider_rejection("80300007"),
            PushSendResult::Dead("provider_rejected_token")
        ));
        assert!(matches!(
            classify_provider_rejection("unexpected"),
            PushSendResult::Retry("provider_response_invalid")
        ));
    }

    #[test]
    fn active_binding_resolves_refresh_reference() {
        let (_temp, service) = service_with_refresh("active");
        let target = service
            .resolve_push_refresh_for_app(REFRESH_REF, APP_DEVICE_ID)
            .expect("refresh target");
        assert_eq!(
            target,
            PushRefreshTarget {
                pc_device_id: "pc_device_000001".into(),
                installation_id: "installation_000001".into(),
                binding_epoch: 2,
                remote_task_id: "remote_task_000001".into(),
                terminal_state_version: 8,
            }
        );
    }

    #[test]
    fn refresh_reference_is_scoped_to_owning_app_device() {
        let (_temp, service) = service_with_refresh("active");
        let result = service.resolve_push_refresh_for_app(REFRESH_REF, "app_device_000002");
        assert!(matches!(result, Err(CloudError::DeviceNotBound)));
    }

    #[test]
    fn inactive_binding_cannot_resolve_refresh_reference() {
        let (_temp, service) = service_with_refresh("superseded");
        let result = service.resolve_push_refresh_for_app(REFRESH_REF, APP_DEVICE_ID);
        assert!(matches!(result, Err(CloudError::DeviceNotBound)));
    }

    #[test]
    fn unknown_refresh_reference_has_generic_binding_failure() {
        let (_temp, service) = service_with_refresh("active");
        let result = service.resolve_push_refresh_for_app("push_refresh_ref_999999", APP_DEVICE_ID);
        assert!(matches!(result, Err(CloudError::DeviceNotBound)));
    }
}
