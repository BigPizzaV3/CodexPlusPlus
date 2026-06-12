use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::local_account::LocalAccountExport;
use crate::local_usage::{LocalUsageEvent, LocalUsageExport};

const BACKEND_DB_FILE: &str = "jiyi-codex-local-backend.sqlite";
pub const JIYI_MANAGED_PROXY_DB_PATH_ENV: &str = "JIYI_MANAGED_PROXY_DB_PATH";
pub const JIYI_BACKEND_DB_PATH_ENV: &str = "JIYI_BACKEND_DB_PATH";
pub const DEFAULT_BACKEND_TEAM_ID: &str = "jiyi-default-team";
const DEFAULT_BACKEND_TEAM_NAME: &str = "极义默认团队";
const DEFAULT_BACKEND_TEAM_PLAN_ID: &str = "team_local_trial";
const DEFAULT_BACKEND_TEAM_PLAN_NAME: &str = "团队本地试用";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentitySyncBody {
    pub generated_at_ms: i64,
    pub schema_version: u32,
    pub pii_policy: String,
    pub account: LocalAccountExport,
    pub usage: LocalUsageExport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendState {
    pub db_path: String,
    pub initialized: bool,
    pub batch_count: i64,
    pub user_count: i64,
    pub blocked_user_count: i64,
    pub device_count: i64,
    pub team_count: i64,
    pub team_member_count: i64,
    pub entitlement_count: i64,
    pub billing_renewal_count: i64,
    pub billing_payment_event_count: i64,
    pub usage_summary_count: i64,
    pub audit_event_count: i64,
    pub session_count: i64,
    pub active_session_count: i64,
    pub revoked_session_count: i64,
    pub last_synced_at_ms: Option<i64>,
    pub last_audit_event_at_ms: Option<i64>,
    pub last_billing_renewal_at_ms: Option<i64>,
    pub last_billing_payment_event_at_ms: Option<i64>,
    pub last_user_access_updated_at_ms: Option<i64>,
    pub last_session_issued_at_ms: Option<i64>,
    pub last_session_revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSyncReceipt {
    pub backend_db_path: String,
    pub batch_id: String,
    pub received_at_ms: i64,
    pub users_upserted: usize,
    pub devices_upserted: usize,
    pub teams_upserted: usize,
    pub team_members_upserted: usize,
    pub entitlements_upserted: usize,
    pub usage_summaries_upserted: usize,
    pub sessions_issued: usize,
    pub active_session: Option<LocalBackendSessionReceipt>,
    pub total_user_count: i64,
    pub total_device_count: i64,
    pub total_team_count: i64,
    pub total_team_member_count: i64,
    pub total_entitlement_count: i64,
    pub total_usage_summary_count: i64,
    pub total_session_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSessionReceipt {
    pub user_id: String,
    pub device_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    #[serde(skip_serializing)]
    pub access_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSessionVerification {
    pub authenticated: bool,
    pub reason: Option<String>,
    pub subject: Option<LocalBackendSessionSubject>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSessionSubject {
    pub session_id: String,
    pub user_id: String,
    pub device_id: String,
    pub phone_masked: String,
    pub user_access_status: String,
    pub user_access_reason: Option<String>,
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub daily_token_limit: Option<i64>,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendQuotaSnapshot {
    pub authenticated: bool,
    pub reason: Option<String>,
    pub subject: Option<LocalBackendSessionSubject>,
    pub quota: Option<LocalBackendQuota>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendQuota {
    pub day: String,
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub daily_token_limit: i64,
    pub used_tokens: i64,
    pub request_count: i64,
    pub remaining_tokens: Option<i64>,
    pub limit_source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendSessionRevocation {
    pub authenticated: bool,
    pub reason: Option<String>,
    pub subject: Option<LocalBackendSessionSubject>,
    pub revoked_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendUsageRecordReceipt {
    pub authenticated: bool,
    pub reason: Option<String>,
    pub subject: Option<LocalBackendSessionSubject>,
    pub day: Option<String>,
    pub recorded_tokens: i64,
    pub total_used_tokens: i64,
    pub total_request_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendUserAccessChange {
    pub user_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub updated_at_ms: i64,
    pub sessions_revoked: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendEntitlementChange {
    pub user_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub previous_plan_id: Option<String>,
    pub previous_daily_token_limit: Option<i64>,
    pub reason: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendTeamEntitlementChange {
    pub team_id: String,
    pub team_name: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub previous_plan_id: Option<String>,
    pub previous_daily_token_limit: Option<i64>,
    pub reason: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendBillingRenewal {
    pub renewal_id: String,
    pub subject_type: String,
    pub subject_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub previous_plan_id: Option<String>,
    pub previous_daily_token_limit: Option<i64>,
    pub amount_cents: i64,
    pub currency: String,
    pub payment_channel: String,
    pub external_order_id: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendBillingRenewalList {
    pub renewals: Vec<LocalBackendBillingRenewal>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendBillingPaymentEvent {
    pub payment_event_id: String,
    pub provider: String,
    pub gateway_event_id: Option<String>,
    pub external_order_id: String,
    pub status: String,
    pub subject_type: String,
    pub subject_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub amount_cents: i64,
    pub currency: String,
    pub payment_channel: String,
    pub reason: Option<String>,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub renewal_id: Option<String>,
    pub processing_status: String,
    pub processing_error: Option<String>,
    pub received_at_ms: i64,
    pub processed_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendBillingPaymentWebhookReceipt {
    pub duplicate: bool,
    pub event: LocalBackendBillingPaymentEvent,
    pub renewal: Option<LocalBackendBillingRenewal>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendBillingReconciliation {
    pub attempted: usize,
    pub applied: usize,
    pub already_applied: usize,
    pub failed: usize,
    pub events: Vec<LocalBackendBillingPaymentEvent>,
    pub renewals: Vec<LocalBackendBillingRenewal>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendAuditEvent {
    pub event_id: String,
    pub event_type: String,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub subject_user_id: Option<String>,
    pub subject_device_id: Option<String>,
    pub reason: Option<String>,
    pub metadata: Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct LocalBackendAuditEventQuery {
    pub limit: usize,
    pub event_type: Option<String>,
    pub actor_type: Option<String>,
    pub subject_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendAdminUserList {
    pub day: String,
    pub users: Vec<LocalBackendAdminUserOverview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendAdminUserOverview {
    pub user_id: String,
    pub phone_masked: String,
    pub access_status: String,
    pub access_reason: Option<String>,
    pub plan_id: Option<String>,
    pub plan_name: Option<String>,
    pub daily_token_limit: Option<i64>,
    pub device_count: i64,
    pub session_count: i64,
    pub active_session_count: i64,
    pub revoked_session_count: i64,
    pub today_request_count: i64,
    pub today_used_tokens: i64,
    pub today_remaining_tokens: Option<i64>,
    pub last_login_at_ms: i64,
    pub last_synced_at_ms: i64,
    pub last_usage_at_ms: Option<i64>,
    pub last_session_issued_at_ms: Option<i64>,
    pub last_session_seen_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendAdminTeamList {
    pub day: String,
    pub teams: Vec<LocalBackendAdminTeamOverview>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalBackendAdminTeamOverview {
    pub team_id: String,
    pub team_name: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub member_count: i64,
    pub active_member_count: i64,
    pub blocked_member_count: i64,
    pub today_request_count: i64,
    pub today_used_tokens: i64,
    pub today_remaining_tokens: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_member_updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct LocalBackendStore {
    db_path: PathBuf,
}

impl Default for LocalBackendStore {
    fn default() -> Self {
        Self::new(default_backend_db_path())
    }
}

impl LocalBackendStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn from_env() -> Self {
        Self::new(backend_db_path_from_env())
    }

    pub fn db_path(&self) -> &std::path::Path {
        &self.db_path
    }

    pub fn state(&self) -> anyhow::Result<LocalBackendState> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        Ok(LocalBackendState {
            db_path: self.db_path.to_string_lossy().to_string(),
            initialized: true,
            batch_count: table_count(&db, "backend_sync_batches")?,
            user_count: table_count(&db, "backend_users")?,
            blocked_user_count: table_count_where(
                &db,
                "backend_user_access",
                "status <> 'active'",
                &[],
            )?,
            device_count: table_count(&db, "backend_devices")?,
            team_count: table_count(&db, "backend_teams")?,
            team_member_count: table_count(&db, "backend_team_members")?,
            entitlement_count: table_count(&db, "backend_entitlements")?,
            billing_renewal_count: table_count(&db, "backend_billing_renewals")?,
            billing_payment_event_count: table_count(&db, "backend_billing_payment_events")?,
            usage_summary_count: table_count(&db, "backend_usage_summaries")?,
            audit_event_count: table_count(&db, "backend_audit_events")?,
            session_count: table_count(&db, "backend_sessions")?,
            active_session_count: table_count_where(
                &db,
                "backend_sessions",
                "revoked_at_ms IS NULL AND expires_at_ms > ?1",
                &[&now],
            )?,
            revoked_session_count: table_count_where(
                &db,
                "backend_sessions",
                "revoked_at_ms IS NOT NULL",
                &[],
            )?,
            last_synced_at_ms: db
                .query_row(
                    "SELECT MAX(received_at_ms) FROM backend_sync_batches",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_audit_event_at_ms: db
                .query_row(
                    "SELECT MAX(created_at_ms) FROM backend_audit_events",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_billing_renewal_at_ms: db
                .query_row(
                    "SELECT MAX(created_at_ms) FROM backend_billing_renewals",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_billing_payment_event_at_ms: db
                .query_row(
                    "SELECT MAX(received_at_ms) FROM backend_billing_payment_events",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_user_access_updated_at_ms: db
                .query_row(
                    "SELECT MAX(updated_at_ms) FROM backend_user_access",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_session_issued_at_ms: db
                .query_row(
                    "SELECT MAX(issued_at_ms) FROM backend_sessions",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
            last_session_revoked_at_ms: db
                .query_row(
                    "SELECT MAX(revoked_at_ms) FROM backend_sessions",
                    [],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()?
                .flatten(),
        })
    }

    pub fn apply_identity_sync(
        &self,
        body: &IdentitySyncBody,
    ) -> anyhow::Result<LocalBackendSyncReceipt> {
        self.ensure_schema()?;
        let mut db = Connection::open(&self.db_path)?;
        let tx = db.transaction()?;
        let batch_id = Uuid::new_v4().to_string();
        let received_at_ms = now_ms();

        tx.execute(
            "INSERT INTO backend_sync_batches (
                batch_id, received_at_ms, source_generated_at_ms, schema_version, pii_policy,
                user_count, device_count, entitlement_count, usage_summary_count
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                &batch_id,
                received_at_ms,
                body.generated_at_ms,
                body.schema_version as i64,
                &body.pii_policy,
                body.account.users.len() as i64,
                body.account.devices.len() as i64,
                body.account.entitlements.len() as i64,
                body.usage.summaries.len() as i64,
            ],
        )?;

        for user in &body.account.users {
            tx.execute(
                "INSERT INTO backend_users (
                    user_id, phone_masked, phone_hash, created_at_ms, last_login_at_ms,
                    first_synced_at_ms, last_synced_at_ms
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(user_id) DO UPDATE SET
                   phone_masked = excluded.phone_masked,
                   phone_hash = excluded.phone_hash,
                   created_at_ms = excluded.created_at_ms,
                   last_login_at_ms = excluded.last_login_at_ms,
                   last_synced_at_ms = excluded.last_synced_at_ms",
                params![
                    &user.user_id,
                    &user.phone_masked,
                    &user.phone_hash,
                    user.created_at_ms,
                    user.last_login_at_ms,
                    received_at_ms,
                ],
            )?;
        }

        for device in &body.account.devices {
            tx.execute(
                "INSERT INTO backend_devices (
                    user_id, device_id, first_seen_at_ms, last_seen_at_ms, first_synced_at_ms,
                    last_synced_at_ms
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT(user_id, device_id) DO UPDATE SET
                   first_seen_at_ms = excluded.first_seen_at_ms,
                   last_seen_at_ms = excluded.last_seen_at_ms,
                   last_synced_at_ms = excluded.last_synced_at_ms",
                params![
                    &device.user_id,
                    &device.device_id,
                    device.first_seen_at_ms,
                    device.last_seen_at_ms,
                    received_at_ms,
                ],
            )?;
        }

        let teams_upserted = usize::from(!body.account.users.is_empty());
        let team_members_upserted = body.account.users.len();
        if !body.account.users.is_empty() {
            tx.execute(
                "INSERT OR IGNORE INTO backend_teams (
                    team_id, team_name, plan_id, plan_name, daily_token_limit,
                    created_at_ms, updated_at_ms
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                params![
                    DEFAULT_BACKEND_TEAM_ID,
                    DEFAULT_BACKEND_TEAM_NAME,
                    DEFAULT_BACKEND_TEAM_PLAN_ID,
                    DEFAULT_BACKEND_TEAM_PLAN_NAME,
                    0_i64,
                    received_at_ms,
                ],
            )?;
        }
        for user in &body.account.users {
            tx.execute(
                "INSERT INTO backend_team_members (
                    team_id, user_id, role, joined_at_ms, updated_at_ms
                 )
                 VALUES (?1, ?2, 'member', ?3, ?3)
                 ON CONFLICT(team_id, user_id) DO UPDATE SET
                   role = excluded.role,
                   updated_at_ms = excluded.updated_at_ms",
                params![DEFAULT_BACKEND_TEAM_ID, &user.user_id, received_at_ms],
            )?;
        }

        for entitlement in &body.account.entitlements {
            tx.execute(
                "INSERT INTO backend_entitlements (
                    user_id, plan_id, plan_name, daily_token_limit, updated_at_ms,
                    first_synced_at_ms, last_synced_at_ms
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
                 ON CONFLICT(user_id) DO UPDATE SET
                   plan_id = excluded.plan_id,
                   plan_name = excluded.plan_name,
                   daily_token_limit = excluded.daily_token_limit,
                   updated_at_ms = excluded.updated_at_ms,
                   last_synced_at_ms = excluded.last_synced_at_ms",
                params![
                    &entitlement.user_id,
                    &entitlement.plan_id,
                    &entitlement.plan_name,
                    entitlement.daily_token_limit,
                    entitlement.updated_at_ms,
                    received_at_ms,
                ],
            )?;
        }

        for summary in &body.usage.summaries {
            let plan_id = summary.plan_id.clone().unwrap_or_default();
            tx.execute(
                "INSERT INTO backend_usage_summaries (
                    day, subject_id, plan_id, request_count, request_bytes, response_bytes,
                    estimated_tokens, reported_total_tokens, effective_total_tokens,
                    first_seen_at_ms, last_seen_at_ms, first_synced_at_ms, last_synced_at_ms
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)
                 ON CONFLICT(day, subject_id, plan_id) DO UPDATE SET
                   request_count = excluded.request_count,
                   request_bytes = excluded.request_bytes,
                   response_bytes = excluded.response_bytes,
                   estimated_tokens = excluded.estimated_tokens,
                   reported_total_tokens = excluded.reported_total_tokens,
                   effective_total_tokens = excluded.effective_total_tokens,
                   first_seen_at_ms = excluded.first_seen_at_ms,
                   last_seen_at_ms = excluded.last_seen_at_ms,
                   last_synced_at_ms = excluded.last_synced_at_ms",
                params![
                    &summary.day,
                    &summary.subject_id,
                    &plan_id,
                    summary.request_count,
                    summary.request_bytes,
                    summary.response_bytes,
                    summary.estimated_tokens,
                    summary.reported_total_tokens,
                    summary.effective_total_tokens,
                    summary.first_seen_at_ms,
                    summary.last_seen_at_ms,
                    received_at_ms,
                ],
            )?;
        }

        let active_session = issue_backend_session_for_active_account(
            &tx,
            body.account.active_session.as_ref(),
            received_at_ms,
        )?;
        insert_audit_event(
            &tx,
            "identity_sync",
            "identity_sync_api",
            None,
            body.account
                .active_session
                .as_ref()
                .map(|session| session.user_id.as_str()),
            body.account
                .active_session
                .as_ref()
                .map(|session| session.device_id.as_str()),
            None,
            json!({
                "batchId": &batch_id,
                "schemaVersion": body.schema_version,
                "users": body.account.users.len(),
                "devices": body.account.devices.len(),
                "teams": teams_upserted,
                "teamMembers": team_members_upserted,
                "entitlements": body.account.entitlements.len(),
                "usageSummaries": body.usage.summaries.len(),
                "sessionIssued": active_session.is_some()
            }),
            received_at_ms,
        )?;
        let totals = backend_totals(&tx)?;
        tx.commit()?;

        let sessions_issued = usize::from(active_session.is_some());
        Ok(LocalBackendSyncReceipt {
            backend_db_path: self.db_path.to_string_lossy().to_string(),
            batch_id,
            received_at_ms,
            users_upserted: body.account.users.len(),
            devices_upserted: body.account.devices.len(),
            teams_upserted,
            team_members_upserted,
            entitlements_upserted: body.account.entitlements.len(),
            usage_summaries_upserted: body.usage.summaries.len(),
            sessions_issued,
            active_session,
            total_user_count: totals.user_count,
            total_device_count: totals.device_count,
            total_team_count: totals.team_count,
            total_team_member_count: totals.team_member_count,
            total_entitlement_count: totals.entitlement_count,
            total_usage_summary_count: totals.usage_summary_count,
            total_session_count: totals.session_count,
        })
    }

    pub fn verify_session_token(
        &self,
        access_token: &str,
    ) -> anyhow::Result<LocalBackendSessionVerification> {
        let access_token = access_token.trim();
        if access_token.is_empty() {
            return Ok(LocalBackendSessionVerification {
                authenticated: false,
                reason: Some("missing_token".to_string()),
                subject: None,
            });
        }

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        let token_hash = backend_session_token_hash(access_token);
        let row = db
            .query_row(
                "SELECT
                    s.session_id,
                    s.user_id,
                    s.device_id,
                    u.phone_masked,
                    COALESCE(a.status, 'active'),
                    NULLIF(a.reason, ''),
                    e.plan_id,
                    e.plan_name,
                    e.daily_token_limit,
                    s.issued_at_ms,
                    s.expires_at_ms,
                    s.last_seen_at_ms,
                    s.revoked_at_ms
                 FROM backend_sessions s
                 JOIN backend_users u ON u.user_id = s.user_id
                 LEFT JOIN backend_user_access a ON a.user_id = s.user_id
                 LEFT JOIN backend_entitlements e ON e.user_id = s.user_id
                 WHERE s.token_hash = ?1
                 LIMIT 1",
                params![token_hash],
                |row| {
                    Ok((
                        LocalBackendSessionSubject {
                            session_id: row.get(0)?,
                            user_id: row.get(1)?,
                            device_id: row.get(2)?,
                            phone_masked: row.get(3)?,
                            user_access_status: row.get(4)?,
                            user_access_reason: row.get(5)?,
                            plan_id: row.get(6)?,
                            plan_name: row.get(7)?,
                            daily_token_limit: row.get(8)?,
                            issued_at_ms: row.get(9)?,
                            expires_at_ms: row.get(10)?,
                            last_seen_at_ms: row.get(11)?,
                        },
                        row.get::<_, Option<i64>>(12)?,
                    ))
                },
            )
            .optional()?;

        let Some((mut subject, revoked_at_ms)) = row else {
            return Ok(LocalBackendSessionVerification {
                authenticated: false,
                reason: Some("invalid_or_expired_token".to_string()),
                subject: None,
            });
        };
        if subject.user_access_status != "active" {
            return Ok(LocalBackendSessionVerification {
                authenticated: false,
                reason: Some("user_blocked".to_string()),
                subject: None,
            });
        }
        if revoked_at_ms.is_some() || subject.expires_at_ms <= now {
            return Ok(LocalBackendSessionVerification {
                authenticated: false,
                reason: Some("invalid_or_expired_token".to_string()),
                subject: None,
            });
        }

        db.execute(
            "UPDATE backend_sessions SET last_seen_at_ms = ?1 WHERE session_id = ?2",
            params![now, &subject.session_id],
        )?;
        subject.last_seen_at_ms = now;

        Ok(LocalBackendSessionVerification {
            authenticated: true,
            reason: None,
            subject: Some(subject),
        })
    }

    pub fn quota_snapshot(&self, access_token: &str) -> anyhow::Result<LocalBackendQuotaSnapshot> {
        let verification = self.verify_session_token(access_token)?;
        let Some(subject) = verification.subject else {
            return Ok(LocalBackendQuotaSnapshot {
                authenticated: false,
                reason: verification.reason,
                subject: None,
                quota: None,
            });
        };

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let day = backend_day_key(now_ms());
        let (used_tokens, request_count) = db
            .query_row(
                "SELECT
                    COALESCE(SUM(effective_total_tokens), 0),
                    COALESCE(SUM(request_count), 0)
                 FROM backend_usage_summaries
                 WHERE day = ?1 AND subject_id = ?2",
                params![&day, &subject.user_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        let daily_token_limit = subject.daily_token_limit.unwrap_or(0).max(0);
        let remaining_tokens =
            (daily_token_limit > 0).then_some((daily_token_limit - used_tokens).max(0));
        let limit_source = if daily_token_limit > 0 {
            "backend_entitlement"
        } else if subject.daily_token_limit.is_some() {
            "backend_entitlement_unlimited"
        } else {
            "missing_backend_entitlement"
        }
        .to_string();

        Ok(LocalBackendQuotaSnapshot {
            authenticated: true,
            reason: None,
            quota: Some(LocalBackendQuota {
                day,
                plan_id: subject.plan_id.clone(),
                plan_name: subject.plan_name.clone(),
                daily_token_limit,
                used_tokens,
                request_count,
                remaining_tokens,
                limit_source,
            }),
            subject: Some(subject),
        })
    }

    pub fn revoke_session_token(
        &self,
        access_token: &str,
    ) -> anyhow::Result<LocalBackendSessionRevocation> {
        let access_token = access_token.trim();
        if access_token.is_empty() {
            return Ok(LocalBackendSessionRevocation {
                authenticated: false,
                reason: Some("missing_token".to_string()),
                subject: None,
                revoked_at_ms: None,
            });
        }

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        let token_hash = backend_session_token_hash(access_token);
        let row = db
            .query_row(
                "SELECT
                    s.session_id,
                    s.user_id,
                    s.device_id,
                    u.phone_masked,
                    COALESCE(a.status, 'active'),
                    NULLIF(a.reason, ''),
                    e.plan_id,
                    e.plan_name,
                    e.daily_token_limit,
                    s.issued_at_ms,
                    s.expires_at_ms,
                    s.last_seen_at_ms,
                    s.revoked_at_ms
                 FROM backend_sessions s
                 JOIN backend_users u ON u.user_id = s.user_id
                 LEFT JOIN backend_user_access a ON a.user_id = s.user_id
                 LEFT JOIN backend_entitlements e ON e.user_id = s.user_id
                 WHERE s.token_hash = ?1
                 LIMIT 1",
                params![token_hash],
                |row| {
                    Ok((
                        LocalBackendSessionSubject {
                            session_id: row.get(0)?,
                            user_id: row.get(1)?,
                            device_id: row.get(2)?,
                            phone_masked: row.get(3)?,
                            user_access_status: row.get(4)?,
                            user_access_reason: row.get(5)?,
                            plan_id: row.get(6)?,
                            plan_name: row.get(7)?,
                            daily_token_limit: row.get(8)?,
                            issued_at_ms: row.get(9)?,
                            expires_at_ms: row.get(10)?,
                            last_seen_at_ms: row.get(11)?,
                        },
                        row.get::<_, Option<i64>>(12)?,
                    ))
                },
            )
            .optional()?;

        let Some((mut subject, revoked_at_ms)) = row else {
            return Ok(LocalBackendSessionRevocation {
                authenticated: false,
                reason: Some("invalid_or_expired_token".to_string()),
                subject: None,
                revoked_at_ms: None,
            });
        };
        if subject.user_access_status != "active" {
            return Ok(LocalBackendSessionRevocation {
                authenticated: false,
                reason: Some("user_blocked".to_string()),
                subject: None,
                revoked_at_ms: None,
            });
        }
        if revoked_at_ms.is_some() || subject.expires_at_ms <= now {
            return Ok(LocalBackendSessionRevocation {
                authenticated: false,
                reason: Some("invalid_or_expired_token".to_string()),
                subject: None,
                revoked_at_ms: None,
            });
        }

        db.execute(
            "UPDATE backend_sessions
             SET revoked_at_ms = ?1, last_seen_at_ms = ?1
             WHERE session_id = ?2",
            params![now, &subject.session_id],
        )?;
        insert_audit_event(
            &db,
            "session_revoked",
            "session_api",
            None,
            Some(&subject.user_id),
            Some(&subject.device_id),
            None,
            json!({
                "sessionId": subject.session_id
            }),
            now,
        )?;
        subject.last_seen_at_ms = now;

        Ok(LocalBackendSessionRevocation {
            authenticated: true,
            reason: None,
            subject: Some(subject),
            revoked_at_ms: Some(now),
        })
    }

    pub fn record_usage_event(
        &self,
        access_token: &str,
        event: &LocalUsageEvent,
    ) -> anyhow::Result<LocalBackendUsageRecordReceipt> {
        let verification = self.verify_session_token(access_token)?;
        let Some(subject) = verification.subject else {
            return Ok(LocalBackendUsageRecordReceipt {
                authenticated: false,
                reason: verification.reason,
                subject: None,
                day: None,
                recorded_tokens: 0,
                total_used_tokens: 0,
                total_request_count: 0,
            });
        };

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        let day = backend_day_key(now);
        let plan_id = subject.plan_id.clone().unwrap_or_default();
        let estimated_tokens = crate::local_usage::estimate_tokens_from_bytes(
            event.request_bytes,
            event.response_bytes,
        );
        let reported_total_tokens = event
            .token_usage
            .and_then(|usage| {
                usage.total_tokens.or_else(|| {
                    usage
                        .input_tokens
                        .zip(usage.output_tokens)
                        .map(|(input, output)| input + output)
                })
            })
            .unwrap_or(0);
        let effective_total_tokens = if reported_total_tokens > 0 {
            reported_total_tokens
        } else {
            estimated_tokens
        };

        db.execute(
            "INSERT INTO backend_usage_summaries (
                day, subject_id, plan_id, request_count, request_bytes, response_bytes,
                estimated_tokens, reported_total_tokens, effective_total_tokens,
                first_seen_at_ms, last_seen_at_ms, first_synced_at_ms, last_synced_at_ms
             )
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?9, ?9)
             ON CONFLICT(day, subject_id, plan_id) DO UPDATE SET
               request_count = backend_usage_summaries.request_count + excluded.request_count,
               request_bytes = backend_usage_summaries.request_bytes + excluded.request_bytes,
               response_bytes = backend_usage_summaries.response_bytes + excluded.response_bytes,
               estimated_tokens = backend_usage_summaries.estimated_tokens + excluded.estimated_tokens,
               reported_total_tokens = backend_usage_summaries.reported_total_tokens + excluded.reported_total_tokens,
               effective_total_tokens = backend_usage_summaries.effective_total_tokens + excluded.effective_total_tokens,
               first_seen_at_ms = MIN(backend_usage_summaries.first_seen_at_ms, excluded.first_seen_at_ms),
               last_seen_at_ms = MAX(backend_usage_summaries.last_seen_at_ms, excluded.last_seen_at_ms),
               last_synced_at_ms = excluded.last_synced_at_ms",
            params![
                &day,
                &subject.user_id,
                &plan_id,
                event.request_bytes as i64,
                event.response_bytes as i64,
                estimated_tokens,
                reported_total_tokens,
                effective_total_tokens,
                now,
            ],
        )?;

        let (total_used_tokens, total_request_count) = db
            .query_row(
                "SELECT
                    COALESCE(SUM(effective_total_tokens), 0),
                    COALESCE(SUM(request_count), 0)
                 FROM backend_usage_summaries
                 WHERE day = ?1 AND subject_id = ?2",
                params![&day, &subject.user_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        insert_audit_event(
            &db,
            "usage_recorded",
            "managed_proxy",
            None,
            Some(&subject.user_id),
            Some(&subject.device_id),
            None,
            json!({
                "day": &day,
                "path": &event.path,
                "method": &event.method,
                "upstreamProtocol": &event.upstream_protocol,
                "statusCode": event.status_code,
                "recordedTokens": effective_total_tokens,
                "totalUsedTokens": total_used_tokens,
                "totalRequestCount": total_request_count
            }),
            now,
        )?;

        Ok(LocalBackendUsageRecordReceipt {
            authenticated: true,
            reason: None,
            subject: Some(subject),
            day: Some(day),
            recorded_tokens: effective_total_tokens,
            total_used_tokens,
            total_request_count,
        })
    }

    pub fn set_user_access_status(
        &self,
        user_id: &str,
        status: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<LocalBackendUserAccessChange> {
        self.set_user_access_status_with_actor(user_id, status, reason, "local_backend", None)
    }

    pub fn set_user_access_status_with_actor(
        &self,
        user_id: &str,
        status: &str,
        reason: Option<&str>,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendUserAccessChange> {
        let user_id = user_id.trim();
        if user_id.is_empty() {
            anyhow::bail!("user_id is required");
        }
        let status = normalize_user_access_status(status)?;
        let reason = reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>());
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        db.execute(
            "INSERT INTO backend_user_access (user_id, status, reason, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(user_id) DO UPDATE SET
               status = excluded.status,
               reason = excluded.reason,
               updated_at_ms = excluded.updated_at_ms",
            params![user_id, status, reason.as_deref().unwrap_or(""), now],
        )?;
        let sessions_revoked = if status == "active" {
            0
        } else {
            db.execute(
                "UPDATE backend_sessions
                 SET revoked_at_ms = ?1, last_seen_at_ms = ?1
                 WHERE user_id = ?2 AND revoked_at_ms IS NULL",
                params![now, user_id],
            )? as i64
        };
        insert_audit_event(
            &db,
            "user_access_updated",
            actor_type,
            actor_id,
            Some(user_id),
            None,
            reason.as_deref(),
            json!({
                "status": status,
                "sessionsRevoked": sessions_revoked
            }),
            now,
        )?;

        Ok(LocalBackendUserAccessChange {
            user_id: user_id.to_string(),
            status: status.to_string(),
            reason,
            updated_at_ms: now,
            sessions_revoked,
        })
    }

    pub fn block_user(
        &self,
        user_id: &str,
        reason: Option<&str>,
    ) -> anyhow::Result<LocalBackendUserAccessChange> {
        self.set_user_access_status(user_id, "blocked", reason)
    }

    pub fn unblock_user(&self, user_id: &str) -> anyhow::Result<LocalBackendUserAccessChange> {
        self.set_user_access_status(user_id, "active", None)
    }

    pub fn set_user_entitlement_with_actor(
        &self,
        user_id: &str,
        plan_id: &str,
        plan_name: &str,
        daily_token_limit: i64,
        reason: Option<&str>,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendEntitlementChange> {
        let user_id = user_id.trim();
        let plan_id = plan_id.trim();
        let plan_name = plan_name.trim();
        if user_id.is_empty() {
            anyhow::bail!("user_id is required");
        }
        if plan_id.is_empty() {
            anyhow::bail!("plan_id is required");
        }
        if plan_name.is_empty() {
            anyhow::bail!("plan_name is required");
        }
        if daily_token_limit < 0 {
            anyhow::bail!("daily_token_limit must be greater than or equal to 0");
        }
        let reason = reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>());

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let user_exists = table_count_where(&db, "backend_users", "user_id = ?1", &[&user_id])?;
        if user_exists == 0 {
            anyhow::bail!("backend user not found: {user_id}");
        }

        let previous = db
            .query_row(
                "SELECT plan_id, daily_token_limit
                 FROM backend_entitlements
                 WHERE user_id = ?1",
                params![user_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let now = now_ms();
        db.execute(
            "INSERT INTO backend_entitlements (
                user_id, plan_id, plan_name, daily_token_limit,
                updated_at_ms, first_synced_at_ms, last_synced_at_ms
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5)
             ON CONFLICT(user_id) DO UPDATE SET
                plan_id = excluded.plan_id,
                plan_name = excluded.plan_name,
                daily_token_limit = excluded.daily_token_limit,
                updated_at_ms = excluded.updated_at_ms,
                last_synced_at_ms = excluded.last_synced_at_ms",
            params![user_id, plan_id, plan_name, daily_token_limit, now],
        )?;
        insert_audit_event(
            &db,
            "user_entitlement_updated",
            actor_type,
            actor_id,
            Some(user_id),
            None,
            reason.as_deref(),
            json!({
                "planId": plan_id,
                "planName": plan_name,
                "dailyTokenLimit": daily_token_limit,
                "previousPlanId": previous.as_ref().map(|(plan_id, _)| plan_id.as_str()),
                "previousDailyTokenLimit": previous.as_ref().map(|(_, limit)| *limit)
            }),
            now,
        )?;

        Ok(LocalBackendEntitlementChange {
            user_id: user_id.to_string(),
            plan_id: plan_id.to_string(),
            plan_name: plan_name.to_string(),
            daily_token_limit,
            previous_plan_id: previous.as_ref().map(|(plan_id, _)| plan_id.clone()),
            previous_daily_token_limit: previous.as_ref().map(|(_, limit)| *limit),
            reason,
            updated_at_ms: now,
        })
    }

    pub fn set_team_entitlement_with_actor(
        &self,
        team_id: &str,
        plan_id: &str,
        plan_name: &str,
        daily_token_limit: i64,
        reason: Option<&str>,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendTeamEntitlementChange> {
        let team_id = team_id.trim();
        let plan_id = plan_id.trim();
        let plan_name = plan_name.trim();
        if team_id.is_empty() {
            anyhow::bail!("team_id is required");
        }
        if plan_id.is_empty() {
            anyhow::bail!("plan_id is required");
        }
        if plan_name.is_empty() {
            anyhow::bail!("plan_name is required");
        }
        if daily_token_limit < 0 {
            anyhow::bail!("daily_token_limit must be greater than or equal to 0");
        }
        let reason = reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>());

        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let previous = db
            .query_row(
                "SELECT team_name, plan_id, daily_token_limit
                 FROM backend_teams
                 WHERE team_id = ?1",
                params![team_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((team_name, previous_plan_id, previous_daily_token_limit)) = previous else {
            anyhow::bail!("backend team not found: {team_id}");
        };
        let now = now_ms();
        db.execute(
            "UPDATE backend_teams
             SET plan_id = ?1,
                 plan_name = ?2,
                 daily_token_limit = ?3,
                 updated_at_ms = ?4
             WHERE team_id = ?5",
            params![plan_id, plan_name, daily_token_limit, now, team_id],
        )?;
        insert_audit_event(
            &db,
            "team_entitlement_updated",
            actor_type,
            actor_id,
            None,
            None,
            reason.as_deref(),
            json!({
                "teamId": team_id,
                "teamName": team_name,
                "planId": plan_id,
                "planName": plan_name,
                "dailyTokenLimit": daily_token_limit,
                "previousPlanId": previous_plan_id,
                "previousDailyTokenLimit": previous_daily_token_limit
            }),
            now,
        )?;

        Ok(LocalBackendTeamEntitlementChange {
            team_id: team_id.to_string(),
            team_name,
            plan_id: plan_id.to_string(),
            plan_name: plan_name.to_string(),
            daily_token_limit,
            previous_plan_id: Some(previous_plan_id),
            previous_daily_token_limit: Some(previous_daily_token_limit),
            reason,
            updated_at_ms: now,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_billing_renewal_with_actor(
        &self,
        subject_type: &str,
        subject_id: &str,
        plan_id: &str,
        plan_name: &str,
        daily_token_limit: i64,
        amount_cents: i64,
        currency: &str,
        payment_channel: &str,
        external_order_id: Option<&str>,
        reason: Option<&str>,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendBillingRenewal> {
        let subject_type = subject_type.trim().to_ascii_lowercase();
        let subject_id = subject_id.trim();
        let plan_id = plan_id.trim();
        let plan_name = plan_name.trim();
        let currency = currency.trim().to_ascii_uppercase();
        let payment_channel = payment_channel.trim();
        if !matches!(subject_type.as_str(), "user" | "team") {
            anyhow::bail!("subject_type must be user or team");
        }
        if subject_id.is_empty() {
            anyhow::bail!("subject_id is required");
        }
        if plan_id.is_empty() {
            anyhow::bail!("plan_id is required");
        }
        if plan_name.is_empty() {
            anyhow::bail!("plan_name is required");
        }
        if daily_token_limit < 0 {
            anyhow::bail!("daily_token_limit must be greater than or equal to 0");
        }
        if amount_cents < 0 {
            anyhow::bail!("amount_cents must be greater than or equal to 0");
        }
        if currency.is_empty() || currency.len() > 12 {
            anyhow::bail!("currency is required");
        }
        if payment_channel.is_empty() {
            anyhow::bail!("payment_channel is required");
        }
        let external_order_id = external_order_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(160).collect::<String>());
        let reason = reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>());

        self.ensure_schema()?;
        let mut db = Connection::open(&self.db_path)?;
        let tx = db.transaction()?;
        let now = now_ms();
        let renewal_id = Uuid::new_v4().to_string();

        let (previous_plan_id, previous_daily_token_limit, team_name) = if subject_type == "user" {
            let user_exists =
                table_count_where(&tx, "backend_users", "user_id = ?1", &[&subject_id])?;
            if user_exists == 0 {
                anyhow::bail!("backend user not found: {subject_id}");
            }
            let previous = tx
                .query_row(
                    "SELECT plan_id, daily_token_limit
                         FROM backend_entitlements
                         WHERE user_id = ?1",
                    params![subject_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            tx.execute(
                "INSERT INTO backend_entitlements (
                        user_id, plan_id, plan_name, daily_token_limit,
                        updated_at_ms, first_synced_at_ms, last_synced_at_ms
                     )
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5)
                     ON CONFLICT(user_id) DO UPDATE SET
                        plan_id = excluded.plan_id,
                        plan_name = excluded.plan_name,
                        daily_token_limit = excluded.daily_token_limit,
                        updated_at_ms = excluded.updated_at_ms,
                        last_synced_at_ms = excluded.last_synced_at_ms",
                params![subject_id, plan_id, plan_name, daily_token_limit, now],
            )?;
            (
                previous.as_ref().map(|(plan_id, _)| plan_id.clone()),
                previous.as_ref().map(|(_, limit)| *limit),
                None,
            )
        } else {
            let previous = tx
                .query_row(
                    "SELECT team_name, plan_id, daily_token_limit
                         FROM backend_teams
                         WHERE team_id = ?1",
                    params![subject_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )
                .optional()?;
            let Some((team_name, previous_plan_id, previous_daily_token_limit)) = previous else {
                anyhow::bail!("backend team not found: {subject_id}");
            };
            tx.execute(
                "UPDATE backend_teams
                     SET plan_id = ?1,
                         plan_name = ?2,
                         daily_token_limit = ?3,
                         updated_at_ms = ?4
                     WHERE team_id = ?5",
                params![plan_id, plan_name, daily_token_limit, now, subject_id],
            )?;
            (
                Some(previous_plan_id),
                Some(previous_daily_token_limit),
                Some(team_name),
            )
        };

        tx.execute(
            "INSERT INTO backend_billing_renewals (
                renewal_id, subject_type, subject_id, plan_id, plan_name, daily_token_limit,
                previous_plan_id, previous_daily_token_limit, amount_cents, currency,
                payment_channel, external_order_id, status, reason, actor_type, actor_id,
                created_at_ms
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'paid', ?13, ?14, ?15, ?16)",
            params![
                &renewal_id,
                &subject_type,
                subject_id,
                plan_id,
                plan_name,
                daily_token_limit,
                previous_plan_id.as_deref().unwrap_or_default(),
                previous_daily_token_limit,
                amount_cents,
                &currency,
                payment_channel,
                external_order_id.as_deref().unwrap_or_default(),
                reason.as_deref().unwrap_or_default(),
                actor_type,
                actor_id.unwrap_or_default(),
                now,
            ],
        )?;
        insert_audit_event(
            &tx,
            "billing_renewal_recorded",
            actor_type,
            actor_id,
            (subject_type == "user").then_some(subject_id),
            None,
            reason.as_deref(),
            json!({
                "renewalId": renewal_id,
                "subjectType": subject_type,
                "subjectId": subject_id,
                "teamName": team_name,
                "planId": plan_id,
                "planName": plan_name,
                "dailyTokenLimit": daily_token_limit,
                "previousPlanId": previous_plan_id,
                "previousDailyTokenLimit": previous_daily_token_limit,
                "amountCents": amount_cents,
                "currency": currency,
                "paymentChannel": payment_channel,
                "externalOrderId": external_order_id,
                "status": "paid"
            }),
            now,
        )?;
        tx.commit()?;

        Ok(LocalBackendBillingRenewal {
            renewal_id,
            subject_type,
            subject_id: subject_id.to_string(),
            plan_id: plan_id.to_string(),
            plan_name: plan_name.to_string(),
            daily_token_limit,
            previous_plan_id,
            previous_daily_token_limit,
            amount_cents,
            currency,
            payment_channel: payment_channel.to_string(),
            external_order_id,
            status: "paid".to_string(),
            reason,
            actor_type: actor_type.to_string(),
            actor_id: actor_id.map(str::to_string),
            created_at_ms: now,
        })
    }

    pub fn billing_renewals(&self, limit: usize) -> anyhow::Result<LocalBackendBillingRenewalList> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let limit = limit.clamp(1, 500) as i64;
        let mut statement = db.prepare(
            "SELECT
                renewal_id, subject_type, subject_id, plan_id, plan_name, daily_token_limit,
                NULLIF(previous_plan_id, ''), previous_daily_token_limit, amount_cents, currency,
                payment_channel, NULLIF(external_order_id, ''), status, NULLIF(reason, ''),
                actor_type, NULLIF(actor_id, ''), created_at_ms
             FROM backend_billing_renewals
             ORDER BY created_at_ms DESC, rowid DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], |row| {
            Ok(LocalBackendBillingRenewal {
                renewal_id: row.get(0)?,
                subject_type: row.get(1)?,
                subject_id: row.get(2)?,
                plan_id: row.get(3)?,
                plan_name: row.get(4)?,
                daily_token_limit: row.get(5)?,
                previous_plan_id: row.get(6)?,
                previous_daily_token_limit: row.get(7)?,
                amount_cents: row.get(8)?,
                currency: row.get(9)?,
                payment_channel: row.get(10)?,
                external_order_id: row.get(11)?,
                status: row.get(12)?,
                reason: row.get(13)?,
                actor_type: row.get(14)?,
                actor_id: row.get(15)?,
                created_at_ms: row.get(16)?,
            })
        })?;
        let renewals = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(LocalBackendBillingRenewalList { renewals })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_billing_payment_webhook_with_actor(
        &self,
        provider: &str,
        gateway_event_id: Option<&str>,
        external_order_id: &str,
        status: &str,
        subject_type: &str,
        subject_id: &str,
        plan_id: &str,
        plan_name: &str,
        daily_token_limit: i64,
        amount_cents: i64,
        currency: &str,
        payment_channel: Option<&str>,
        reason: Option<&str>,
        raw_payload: &Value,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendBillingPaymentWebhookReceipt> {
        let provider = provider.trim().to_ascii_lowercase();
        let gateway_event_id = gateway_event_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(160).collect::<String>());
        let external_order_id = external_order_id.trim();
        let status = normalize_payment_status(status)?;
        let subject_type = subject_type.trim().to_ascii_lowercase();
        let subject_id = subject_id.trim();
        let plan_id = plan_id.trim();
        let plan_name = plan_name.trim();
        let currency = currency.trim().to_ascii_uppercase();
        let payment_channel = payment_channel
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(provider.as_str())
            .chars()
            .take(80)
            .collect::<String>();
        if provider.is_empty() {
            anyhow::bail!("provider is required");
        }
        if external_order_id.is_empty() {
            anyhow::bail!("external_order_id is required");
        }
        if !matches!(subject_type.as_str(), "user" | "team") {
            anyhow::bail!("subject_type must be user or team");
        }
        if subject_id.is_empty() {
            anyhow::bail!("subject_id is required");
        }
        if plan_id.is_empty() {
            anyhow::bail!("plan_id is required");
        }
        if plan_name.is_empty() {
            anyhow::bail!("plan_name is required");
        }
        if daily_token_limit < 0 {
            anyhow::bail!("daily_token_limit must be greater than or equal to 0");
        }
        if amount_cents < 0 {
            anyhow::bail!("amount_cents must be greater than or equal to 0");
        }
        if currency.is_empty() || currency.len() > 12 {
            anyhow::bail!("currency is required");
        }
        let reason = reason
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.chars().take(240).collect::<String>());
        let raw_payload_json = truncate_json_payload(raw_payload)?;
        let raw_payload_sha256 = sha256_hex(raw_payload_json.as_bytes());

        self.ensure_schema()?;
        if let Some(existing) = self.billing_payment_event_by_unique_key(
            &provider,
            gateway_event_id.as_deref(),
            external_order_id,
            status,
        )? {
            let renewal = existing
                .renewal_id
                .as_deref()
                .and_then(|renewal_id| self.billing_renewal_by_id(renewal_id).ok().flatten());
            return Ok(LocalBackendBillingPaymentWebhookReceipt {
                duplicate: true,
                event: existing,
                renewal,
            });
        }

        let payment_event_id = Uuid::new_v4().to_string();
        let now = now_ms();
        let processing_status = if status == "paid" {
            "pending"
        } else {
            "ignored"
        };
        let db = Connection::open(&self.db_path)?;
        db.execute(
            "INSERT INTO backend_billing_payment_events (
                payment_event_id, provider, gateway_event_id, external_order_id, status,
                subject_type, subject_id, plan_id, plan_name, daily_token_limit,
                amount_cents, currency, payment_channel, reason, raw_payload_json,
                raw_payload_sha256, actor_type, actor_id, renewal_id, processing_status,
                processing_error, received_at_ms, processed_at_ms
             )
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, '', ?19, '', ?20, NULL)",
            params![
                &payment_event_id,
                &provider,
                gateway_event_id.as_deref().unwrap_or_default(),
                external_order_id,
                status,
                &subject_type,
                subject_id,
                plan_id,
                plan_name,
                daily_token_limit,
                amount_cents,
                &currency,
                &payment_channel,
                reason.as_deref().unwrap_or_default(),
                &raw_payload_json,
                &raw_payload_sha256,
                actor_type,
                actor_id.unwrap_or_default(),
                processing_status,
                now,
            ],
        )?;
        insert_audit_event(
            &db,
            "billing_payment_webhook_received",
            actor_type,
            actor_id,
            (subject_type == "user").then_some(subject_id),
            None,
            reason.as_deref(),
            json!({
                "paymentEventId": payment_event_id,
                "provider": provider,
                "gatewayEventId": gateway_event_id,
                "externalOrderId": external_order_id,
                "status": status,
                "subjectType": subject_type,
                "subjectId": subject_id,
                "planId": plan_id,
                "planName": plan_name,
                "dailyTokenLimit": daily_token_limit,
                "amountCents": amount_cents,
                "currency": currency,
                "paymentChannel": payment_channel,
                "rawPayloadSha256": raw_payload_sha256
            }),
            now,
        )?;
        drop(db);

        let mut event = self
            .billing_payment_event_by_id(&payment_event_id)?
            .ok_or_else(|| anyhow::anyhow!("payment event not found after insert"))?;
        let mut renewal = None;
        if status == "paid" {
            let reconciliation = self.reconcile_billing_payment_event_with_actor(
                &payment_event_id,
                actor_type,
                actor_id,
            )?;
            event = reconciliation.event;
            renewal = reconciliation.renewal;
        }

        Ok(LocalBackendBillingPaymentWebhookReceipt {
            duplicate: false,
            event,
            renewal,
        })
    }

    pub fn reconcile_billing_payment_events_with_actor(
        &self,
        limit: usize,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendBillingReconciliation> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let limit = limit.clamp(1, 500) as i64;
        let mut statement = db.prepare(
            "SELECT payment_event_id
             FROM backend_billing_payment_events
             WHERE status = 'paid'
               AND (renewal_id = '' OR processing_status = 'failed')
             ORDER BY received_at_ms ASC, rowid ASC
             LIMIT ?1",
        )?;
        let ids = statement
            .query_map(params![limit], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(db);

        let mut result = LocalBackendBillingReconciliation {
            attempted: 0,
            applied: 0,
            already_applied: 0,
            failed: 0,
            events: Vec::new(),
            renewals: Vec::new(),
        };
        for payment_event_id in ids {
            result.attempted += 1;
            let receipt = self.reconcile_billing_payment_event_with_actor(
                &payment_event_id,
                actor_type,
                actor_id,
            )?;
            match receipt.event.processing_status.as_str() {
                "applied" => result.applied += 1,
                "already_applied" => result.already_applied += 1,
                "failed" => result.failed += 1,
                _ => {}
            }
            if let Some(renewal) = receipt.renewal.clone() {
                result.renewals.push(renewal);
            }
            result.events.push(receipt.event);
        }
        Ok(result)
    }

    fn reconcile_billing_payment_event_with_actor(
        &self,
        payment_event_id: &str,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendBillingPaymentWebhookReceipt> {
        let event = self
            .billing_payment_event_by_id(payment_event_id)?
            .ok_or_else(|| anyhow::anyhow!("payment event not found: {payment_event_id}"))?;
        if event.status != "paid" {
            return Ok(LocalBackendBillingPaymentWebhookReceipt {
                duplicate: false,
                event,
                renewal: None,
            });
        }

        if let Some(renewal) =
            self.billing_renewal_by_order(&event.payment_channel, &event.external_order_id)?
        {
            let event = self.mark_payment_event_processed(
                &event.payment_event_id,
                "already_applied",
                Some(&renewal.renewal_id),
                None,
                actor_type,
                actor_id,
            )?;
            return Ok(LocalBackendBillingPaymentWebhookReceipt {
                duplicate: false,
                event,
                renewal: Some(renewal),
            });
        }

        match self.record_billing_renewal_with_actor(
            &event.subject_type,
            &event.subject_id,
            &event.plan_id,
            &event.plan_name,
            event.daily_token_limit,
            event.amount_cents,
            &event.currency,
            &event.payment_channel,
            Some(&event.external_order_id),
            event.reason.as_deref(),
            actor_type,
            actor_id,
        ) {
            Ok(renewal) => {
                let event = self.mark_payment_event_processed(
                    &event.payment_event_id,
                    "applied",
                    Some(&renewal.renewal_id),
                    None,
                    actor_type,
                    actor_id,
                )?;
                Ok(LocalBackendBillingPaymentWebhookReceipt {
                    duplicate: false,
                    event,
                    renewal: Some(renewal),
                })
            }
            Err(error) => {
                let message = error.to_string();
                let event = self.mark_payment_event_processed(
                    &event.payment_event_id,
                    "failed",
                    None,
                    Some(&message),
                    actor_type,
                    actor_id,
                )?;
                Ok(LocalBackendBillingPaymentWebhookReceipt {
                    duplicate: false,
                    event,
                    renewal: None,
                })
            }
        }
    }

    fn mark_payment_event_processed(
        &self,
        payment_event_id: &str,
        processing_status: &str,
        renewal_id: Option<&str>,
        processing_error: Option<&str>,
        actor_type: &str,
        actor_id: Option<&str>,
    ) -> anyhow::Result<LocalBackendBillingPaymentEvent> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        db.execute(
            "UPDATE backend_billing_payment_events
             SET renewal_id = COALESCE(NULLIF(?1, ''), renewal_id),
                 processing_status = ?2,
                 processing_error = ?3,
                 processed_at_ms = ?4
             WHERE payment_event_id = ?5",
            params![
                renewal_id.unwrap_or_default(),
                processing_status,
                processing_error.unwrap_or_default(),
                now,
                payment_event_id
            ],
        )?;
        let event = billing_payment_event_by_id(&db, payment_event_id)?
            .ok_or_else(|| anyhow::anyhow!("payment event not found after update"))?;
        insert_audit_event(
            &db,
            "billing_payment_event_reconciled",
            actor_type,
            actor_id,
            (event.subject_type == "user").then_some(event.subject_id.as_str()),
            None,
            event.reason.as_deref(),
            json!({
                "paymentEventId": event.payment_event_id,
                "provider": event.provider,
                "gatewayEventId": event.gateway_event_id,
                "externalOrderId": event.external_order_id,
                "status": event.status,
                "processingStatus": event.processing_status,
                "processingError": event.processing_error,
                "renewalId": event.renewal_id
            }),
            now,
        )?;
        Ok(event)
    }

    fn billing_payment_event_by_unique_key(
        &self,
        provider: &str,
        gateway_event_id: Option<&str>,
        external_order_id: &str,
        status: &str,
    ) -> anyhow::Result<Option<LocalBackendBillingPaymentEvent>> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        if let Some(gateway_event_id) = gateway_event_id {
            if let Some(event) =
                billing_payment_event_by_gateway_event(&db, provider, gateway_event_id)?
            {
                return Ok(Some(event));
            }
        }
        billing_payment_event_by_order_status(&db, provider, external_order_id, status)
    }

    fn billing_payment_event_by_id(
        &self,
        payment_event_id: &str,
    ) -> anyhow::Result<Option<LocalBackendBillingPaymentEvent>> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        billing_payment_event_by_id(&db, payment_event_id)
    }

    fn billing_renewal_by_id(
        &self,
        renewal_id: &str,
    ) -> anyhow::Result<Option<LocalBackendBillingRenewal>> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        billing_renewal_by_id(&db, renewal_id)
    }

    fn billing_renewal_by_order(
        &self,
        payment_channel: &str,
        external_order_id: &str,
    ) -> anyhow::Result<Option<LocalBackendBillingRenewal>> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        billing_renewal_by_order(&db, payment_channel, external_order_id)
    }

    pub fn recent_audit_events(&self, limit: usize) -> anyhow::Result<Vec<LocalBackendAuditEvent>> {
        self.audit_events(LocalBackendAuditEventQuery {
            limit,
            ..LocalBackendAuditEventQuery::default()
        })
    }

    pub fn audit_events(
        &self,
        query: LocalBackendAuditEventQuery,
    ) -> anyhow::Result<Vec<LocalBackendAuditEvent>> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let limit = query.limit.clamp(1, 500) as i64;
        let event_type = query
            .event_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let actor_type = query
            .actor_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let subject_user_id = query
            .subject_user_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let mut statement = db.prepare(
            "SELECT
                event_id, event_type, actor_type, NULLIF(actor_id, ''),
                NULLIF(subject_user_id, ''), NULLIF(subject_device_id, ''),
                NULLIF(reason, ''), metadata_json, created_at_ms
             FROM backend_audit_events
             WHERE (?1 IS NULL OR event_type = ?1)
               AND (?2 IS NULL OR actor_type = ?2)
               AND (?3 IS NULL OR subject_user_id = ?3)
             ORDER BY created_at_ms DESC, rowid DESC
             LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![event_type, actor_type, subject_user_id, limit],
            |row| {
                let metadata_json: String = row.get(7)?;
                let metadata = serde_json::from_str(&metadata_json).unwrap_or(Value::Null);
                Ok(LocalBackendAuditEvent {
                    event_id: row.get(0)?,
                    event_type: row.get(1)?,
                    actor_type: row.get(2)?,
                    actor_id: row.get(3)?,
                    subject_user_id: row.get(4)?,
                    subject_device_id: row.get(5)?,
                    reason: row.get(6)?,
                    metadata,
                    created_at_ms: row.get(8)?,
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn admin_user_overviews(&self, limit: usize) -> anyhow::Result<LocalBackendAdminUserList> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        let day = backend_day_key(now);
        let limit = limit.clamp(1, 500) as i64;
        let mut statement = db.prepare(
            "SELECT
                u.user_id,
                u.phone_masked,
                COALESCE(a.status, 'active'),
                NULLIF(a.reason, ''),
                e.plan_id,
                e.plan_name,
                e.daily_token_limit,
                COALESCE(d.device_count, 0),
                COALESCE(s.session_count, 0),
                COALESCE(s.active_session_count, 0),
                COALESCE(s.revoked_session_count, 0),
                COALESCE(us.request_count, 0),
                COALESCE(us.used_tokens, 0),
                u.last_login_at_ms,
                u.last_synced_at_ms,
                us.last_usage_at_ms,
                s.last_session_issued_at_ms,
                s.last_session_seen_at_ms
             FROM backend_users u
             LEFT JOIN backend_user_access a ON a.user_id = u.user_id
             LEFT JOIN backend_entitlements e ON e.user_id = u.user_id
             LEFT JOIN (
                SELECT user_id, COUNT(*) AS device_count
                FROM backend_devices
                GROUP BY user_id
             ) d ON d.user_id = u.user_id
             LEFT JOIN (
                SELECT
                    user_id,
                    COUNT(*) AS session_count,
                    SUM(CASE WHEN revoked_at_ms IS NULL AND expires_at_ms > ?1 THEN 1 ELSE 0 END)
                        AS active_session_count,
                    SUM(CASE WHEN revoked_at_ms IS NOT NULL THEN 1 ELSE 0 END)
                        AS revoked_session_count,
                    MAX(issued_at_ms) AS last_session_issued_at_ms,
                    MAX(last_seen_at_ms) AS last_session_seen_at_ms
                FROM backend_sessions
                GROUP BY user_id
             ) s ON s.user_id = u.user_id
             LEFT JOIN (
                SELECT
                    subject_id,
                    SUM(request_count) AS request_count,
                    SUM(effective_total_tokens) AS used_tokens,
                    MAX(last_seen_at_ms) AS last_usage_at_ms
                FROM backend_usage_summaries
                WHERE day = ?2
                GROUP BY subject_id
             ) us ON us.subject_id = u.user_id
             ORDER BY u.last_synced_at_ms DESC, u.last_login_at_ms DESC, u.user_id ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![now, &day, limit], |row| {
            let daily_token_limit: Option<i64> = row.get(6)?;
            let today_used_tokens: i64 = row.get(12)?;
            let today_remaining_tokens = daily_token_limit
                .and_then(|limit| (limit > 0).then_some((limit - today_used_tokens).max(0)));
            Ok(LocalBackendAdminUserOverview {
                user_id: row.get(0)?,
                phone_masked: row.get(1)?,
                access_status: row.get(2)?,
                access_reason: row.get(3)?,
                plan_id: row.get(4)?,
                plan_name: row.get(5)?,
                daily_token_limit,
                device_count: row.get(7)?,
                session_count: row.get(8)?,
                active_session_count: row.get(9)?,
                revoked_session_count: row.get(10)?,
                today_request_count: row.get(11)?,
                today_used_tokens,
                today_remaining_tokens,
                last_login_at_ms: row.get(13)?,
                last_synced_at_ms: row.get(14)?,
                last_usage_at_ms: row.get(15)?,
                last_session_issued_at_ms: row.get(16)?,
                last_session_seen_at_ms: row.get(17)?,
            })
        })?;
        let users = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(LocalBackendAdminUserList { day, users })
    }

    pub fn admin_team_overviews(&self, limit: usize) -> anyhow::Result<LocalBackendAdminTeamList> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        let now = now_ms();
        let day = backend_day_key(now);
        let limit = limit.clamp(1, 500) as i64;
        let mut statement = db.prepare(
            "SELECT
                t.team_id,
                t.team_name,
                t.plan_id,
                t.plan_name,
                t.daily_token_limit,
                COUNT(tm.user_id) AS member_count,
                SUM(CASE WHEN tm.user_id IS NOT NULL AND COALESCE(a.status, 'active') = 'active' THEN 1 ELSE 0 END)
                    AS active_member_count,
                SUM(CASE WHEN tm.user_id IS NOT NULL AND COALESCE(a.status, 'active') <> 'active' THEN 1 ELSE 0 END)
                    AS blocked_member_count,
                COALESCE(us.request_count, 0),
                COALESCE(us.used_tokens, 0),
                t.created_at_ms,
                t.updated_at_ms,
                MAX(tm.updated_at_ms) AS last_member_updated_at_ms
             FROM backend_teams t
             LEFT JOIN backend_team_members tm ON tm.team_id = t.team_id
             LEFT JOIN backend_user_access a ON a.user_id = tm.user_id
             LEFT JOIN (
                SELECT
                    tm2.team_id,
                    SUM(s.request_count) AS request_count,
                    SUM(s.effective_total_tokens) AS used_tokens
                FROM backend_team_members tm2
                JOIN backend_usage_summaries s ON s.subject_id = tm2.user_id
                WHERE s.day = ?1
                GROUP BY tm2.team_id
             ) us ON us.team_id = t.team_id
             GROUP BY
                t.team_id, t.team_name, t.plan_id, t.plan_name, t.daily_token_limit,
                us.request_count, us.used_tokens, t.created_at_ms, t.updated_at_ms
             ORDER BY t.updated_at_ms DESC, t.team_id ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![&day, limit], |row| {
            let daily_token_limit: i64 = row.get(4)?;
            let today_used_tokens: i64 = row.get(9)?;
            let today_remaining_tokens =
                (daily_token_limit > 0).then_some((daily_token_limit - today_used_tokens).max(0));
            Ok(LocalBackendAdminTeamOverview {
                team_id: row.get(0)?,
                team_name: row.get(1)?,
                plan_id: row.get(2)?,
                plan_name: row.get(3)?,
                daily_token_limit,
                member_count: row.get(5)?,
                active_member_count: row.get(6)?,
                blocked_member_count: row.get(7)?,
                today_request_count: row.get(8)?,
                today_used_tokens,
                today_remaining_tokens,
                created_at_ms: row.get(10)?,
                updated_at_ms: row.get(11)?,
                last_member_updated_at_ms: row.get(12)?,
            })
        })?;
        let teams = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(LocalBackendAdminTeamList { day, teams })
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS backend_sync_batches (
  batch_id TEXT PRIMARY KEY,
  received_at_ms INTEGER NOT NULL,
  source_generated_at_ms INTEGER NOT NULL,
  schema_version INTEGER NOT NULL,
  pii_policy TEXT NOT NULL,
  user_count INTEGER NOT NULL,
  device_count INTEGER NOT NULL,
  entitlement_count INTEGER NOT NULL,
  usage_summary_count INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backend_users (
  user_id TEXT PRIMARY KEY,
  phone_masked TEXT NOT NULL,
  phone_hash TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  last_login_at_ms INTEGER NOT NULL,
  first_synced_at_ms INTEGER NOT NULL,
  last_synced_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backend_user_access (
  user_id TEXT PRIMARY KEY,
  status TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backend_devices (
  user_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  first_seen_at_ms INTEGER NOT NULL,
  last_seen_at_ms INTEGER NOT NULL,
  first_synced_at_ms INTEGER NOT NULL,
  last_synced_at_ms INTEGER NOT NULL,
  PRIMARY KEY(user_id, device_id)
);

CREATE TABLE IF NOT EXISTS backend_teams (
  team_id TEXT PRIMARY KEY,
  team_name TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  plan_name TEXT NOT NULL,
  daily_token_limit INTEGER NOT NULL,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backend_team_members (
  team_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  role TEXT NOT NULL,
  joined_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  PRIMARY KEY(team_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_backend_team_members_user
  ON backend_team_members(user_id);

CREATE TABLE IF NOT EXISTS backend_entitlements (
  user_id TEXT PRIMARY KEY,
  plan_id TEXT NOT NULL,
  plan_name TEXT NOT NULL,
  daily_token_limit INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  first_synced_at_ms INTEGER NOT NULL,
  last_synced_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS backend_billing_renewals (
  renewal_id TEXT PRIMARY KEY,
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  plan_name TEXT NOT NULL,
  daily_token_limit INTEGER NOT NULL,
  previous_plan_id TEXT NOT NULL DEFAULT '',
  previous_daily_token_limit INTEGER,
  amount_cents INTEGER NOT NULL,
  currency TEXT NOT NULL,
  payment_channel TEXT NOT NULL,
  external_order_id TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  actor_type TEXT NOT NULL,
  actor_id TEXT NOT NULL DEFAULT '',
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backend_billing_renewals_subject
  ON backend_billing_renewals(subject_type, subject_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS backend_billing_payment_events (
  payment_event_id TEXT PRIMARY KEY,
  provider TEXT NOT NULL,
  gateway_event_id TEXT NOT NULL DEFAULT '',
  external_order_id TEXT NOT NULL,
  status TEXT NOT NULL,
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  plan_name TEXT NOT NULL,
  daily_token_limit INTEGER NOT NULL,
  amount_cents INTEGER NOT NULL,
  currency TEXT NOT NULL,
  payment_channel TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  raw_payload_json TEXT NOT NULL,
  raw_payload_sha256 TEXT NOT NULL,
  actor_type TEXT NOT NULL,
  actor_id TEXT NOT NULL DEFAULT '',
  renewal_id TEXT NOT NULL DEFAULT '',
  processing_status TEXT NOT NULL,
  processing_error TEXT NOT NULL DEFAULT '',
  received_at_ms INTEGER NOT NULL,
  processed_at_ms INTEGER
);

CREATE INDEX IF NOT EXISTS idx_backend_billing_payment_events_gateway
  ON backend_billing_payment_events(provider, gateway_event_id);

CREATE INDEX IF NOT EXISTS idx_backend_billing_payment_events_order
  ON backend_billing_payment_events(provider, external_order_id, status);

CREATE INDEX IF NOT EXISTS idx_backend_billing_payment_events_processing
  ON backend_billing_payment_events(status, processing_status, received_at_ms);

CREATE TABLE IF NOT EXISTS backend_usage_summaries (
  day TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  plan_id TEXT NOT NULL,
  request_count INTEGER NOT NULL,
  request_bytes INTEGER NOT NULL,
  response_bytes INTEGER NOT NULL,
  estimated_tokens INTEGER NOT NULL,
  reported_total_tokens INTEGER NOT NULL,
  effective_total_tokens INTEGER NOT NULL,
  first_seen_at_ms INTEGER NOT NULL,
  last_seen_at_ms INTEGER NOT NULL,
  first_synced_at_ms INTEGER NOT NULL,
  last_synced_at_ms INTEGER NOT NULL,
  PRIMARY KEY(day, subject_id, plan_id)
);

CREATE TABLE IF NOT EXISTS backend_audit_events (
  event_id TEXT PRIMARY KEY,
  event_type TEXT NOT NULL,
  actor_type TEXT NOT NULL,
  actor_id TEXT NOT NULL DEFAULT '',
  subject_user_id TEXT NOT NULL DEFAULT '',
  subject_device_id TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL DEFAULT '',
  metadata_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backend_audit_events_created_at
  ON backend_audit_events(created_at_ms DESC);

CREATE INDEX IF NOT EXISTS idx_backend_audit_events_subject
  ON backend_audit_events(subject_user_id, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS backend_sessions (
  session_id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  device_id TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  issued_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL,
  revoked_at_ms INTEGER,
  last_seen_at_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_backend_sessions_user_device
  ON backend_sessions(user_id, device_id, issued_at_ms DESC);
"#,
        )?;
        Ok(())
    }
}

struct BackendTotals {
    user_count: i64,
    device_count: i64,
    team_count: i64,
    team_member_count: i64,
    entitlement_count: i64,
    usage_summary_count: i64,
    session_count: i64,
}

pub fn default_backend_db_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(BACKEND_DB_FILE)
}

pub fn backend_db_path_from_env() -> PathBuf {
    backend_db_path_from_env_values(
        std::env::var(JIYI_MANAGED_PROXY_DB_PATH_ENV).ok(),
        std::env::var(JIYI_BACKEND_DB_PATH_ENV).ok(),
    )
}

fn backend_db_path_from_env_values(
    managed_proxy_db_path: Option<String>,
    backend_db_path: Option<String>,
) -> PathBuf {
    for value in [managed_proxy_db_path, backend_db_path]
        .into_iter()
        .flatten()
    {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_backend_db_path()
}

pub fn build_identity_sync_body() -> anyhow::Result<IdentitySyncBody> {
    let account = crate::local_account::LocalAccountStore::default().export_state()?;
    let usage = crate::local_usage::LocalUsageStore::default().export_summary()?;
    Ok(IdentitySyncBody {
        generated_at_ms: now_ms(),
        schema_version: 1,
        pii_policy: "手机号只导出脱敏展示值和稳定哈希，不导出明文手机号。".to_string(),
        account,
        usage,
    })
}

fn backend_totals(db: &Connection) -> anyhow::Result<BackendTotals> {
    Ok(BackendTotals {
        user_count: table_count(db, "backend_users")?,
        device_count: table_count(db, "backend_devices")?,
        team_count: table_count(db, "backend_teams")?,
        team_member_count: table_count(db, "backend_team_members")?,
        entitlement_count: table_count(db, "backend_entitlements")?,
        usage_summary_count: table_count(db, "backend_usage_summaries")?,
        session_count: table_count(db, "backend_sessions")?,
    })
}

fn issue_backend_session_for_active_account(
    db: &Connection,
    session: Option<&crate::local_account::LocalAuthSessionExport>,
    now: i64,
) -> anyhow::Result<Option<LocalBackendSessionReceipt>> {
    let Some(session) = session else {
        return Ok(None);
    };
    if session.session_expired || session.expires_at_ms <= now {
        return Ok(None);
    }
    let access_status = db
        .query_row(
            "SELECT status FROM backend_user_access WHERE user_id = ?1",
            params![&session.user_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(|| "active".to_string());
    if access_status != "active" {
        return Ok(None);
    }

    db.execute(
        "UPDATE backend_sessions
         SET revoked_at_ms = ?1
         WHERE user_id = ?2 AND device_id = ?3 AND revoked_at_ms IS NULL",
        params![now, &session.user_id, &session.device_id],
    )?;

    let access_token = format!("jiyi-local-{}", Uuid::new_v4());
    let token_hash = backend_session_token_hash(&access_token);
    let session_id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO backend_sessions (
            session_id, user_id, device_id, token_hash, issued_at_ms, expires_at_ms,
            revoked_at_ms, last_seen_at_ms
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?5)",
        params![
            session_id,
            &session.user_id,
            &session.device_id,
            token_hash,
            now,
            session.expires_at_ms,
        ],
    )?;

    Ok(Some(LocalBackendSessionReceipt {
        user_id: session.user_id.clone(),
        device_id: session.device_id.clone(),
        issued_at_ms: now,
        expires_at_ms: session.expires_at_ms,
        access_token,
    }))
}

fn backend_session_token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"jiyi-codex-local-backend-session-v1:");
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn insert_audit_event(
    db: &Connection,
    event_type: &str,
    actor_type: &str,
    actor_id: Option<&str>,
    subject_user_id: Option<&str>,
    subject_device_id: Option<&str>,
    reason: Option<&str>,
    metadata: Value,
    created_at_ms: i64,
) -> anyhow::Result<()> {
    let event_id = Uuid::new_v4().to_string();
    let metadata_json = serde_json::to_string(&metadata)?;
    db.execute(
        "INSERT INTO backend_audit_events (
            event_id, event_type, actor_type, actor_id, subject_user_id, subject_device_id,
            reason, metadata_json, created_at_ms
         )
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            event_id,
            event_type.trim(),
            actor_type.trim(),
            actor_id.map(str::trim).unwrap_or_default(),
            subject_user_id.map(str::trim).unwrap_or_default(),
            subject_device_id.map(str::trim).unwrap_or_default(),
            reason.map(str::trim).unwrap_or_default(),
            metadata_json,
            created_at_ms,
        ],
    )?;
    Ok(())
}

fn normalize_payment_status(status: &str) -> anyhow::Result<&'static str> {
    match status.trim().to_ascii_lowercase().as_str() {
        "paid" | "succeeded" | "success" | "trade_success" | "pay_success" | "payment_success" => {
            Ok("paid")
        }
        "" | "pending" | "created" | "processing" | "waiting" | "unpaid" => Ok("pending"),
        "failed" | "fail" | "closed" | "canceled" | "cancelled" | "timeout" => Ok("failed"),
        "refunded" | "refund" | "partial_refund" => Ok("refunded"),
        value => anyhow::bail!("unsupported payment status: {value}"),
    }
}

fn truncate_json_payload(value: &Value) -> anyhow::Result<String> {
    let mut raw = serde_json::to_string(value)?;
    const MAX_RAW_PAYLOAD_BYTES: usize = 64 * 1024;
    if raw.len() > MAX_RAW_PAYLOAD_BYTES {
        raw.truncate(MAX_RAW_PAYLOAD_BYTES);
    }
    Ok(raw)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn billing_renewal_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LocalBackendBillingRenewal> {
    Ok(LocalBackendBillingRenewal {
        renewal_id: row.get(0)?,
        subject_type: row.get(1)?,
        subject_id: row.get(2)?,
        plan_id: row.get(3)?,
        plan_name: row.get(4)?,
        daily_token_limit: row.get(5)?,
        previous_plan_id: row.get(6)?,
        previous_daily_token_limit: row.get(7)?,
        amount_cents: row.get(8)?,
        currency: row.get(9)?,
        payment_channel: row.get(10)?,
        external_order_id: row.get(11)?,
        status: row.get(12)?,
        reason: row.get(13)?,
        actor_type: row.get(14)?,
        actor_id: row.get(15)?,
        created_at_ms: row.get(16)?,
    })
}

fn billing_renewal_by_id(
    db: &Connection,
    renewal_id: &str,
) -> anyhow::Result<Option<LocalBackendBillingRenewal>> {
    db.query_row(
        "SELECT
            renewal_id, subject_type, subject_id, plan_id, plan_name, daily_token_limit,
            NULLIF(previous_plan_id, ''), previous_daily_token_limit, amount_cents, currency,
            payment_channel, NULLIF(external_order_id, ''), status, NULLIF(reason, ''),
            actor_type, NULLIF(actor_id, ''), created_at_ms
         FROM backend_billing_renewals
         WHERE renewal_id = ?1",
        params![renewal_id.trim()],
        billing_renewal_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn billing_renewal_by_order(
    db: &Connection,
    payment_channel: &str,
    external_order_id: &str,
) -> anyhow::Result<Option<LocalBackendBillingRenewal>> {
    if external_order_id.trim().is_empty() {
        return Ok(None);
    }
    db.query_row(
        "SELECT
            renewal_id, subject_type, subject_id, plan_id, plan_name, daily_token_limit,
            NULLIF(previous_plan_id, ''), previous_daily_token_limit, amount_cents, currency,
            payment_channel, NULLIF(external_order_id, ''), status, NULLIF(reason, ''),
            actor_type, NULLIF(actor_id, ''), created_at_ms
         FROM backend_billing_renewals
         WHERE payment_channel = ?1 AND external_order_id = ?2
         ORDER BY created_at_ms DESC, rowid DESC
         LIMIT 1",
        params![payment_channel.trim(), external_order_id.trim()],
        billing_renewal_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn billing_payment_event_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<LocalBackendBillingPaymentEvent> {
    Ok(LocalBackendBillingPaymentEvent {
        payment_event_id: row.get(0)?,
        provider: row.get(1)?,
        gateway_event_id: row.get(2)?,
        external_order_id: row.get(3)?,
        status: row.get(4)?,
        subject_type: row.get(5)?,
        subject_id: row.get(6)?,
        plan_id: row.get(7)?,
        plan_name: row.get(8)?,
        daily_token_limit: row.get(9)?,
        amount_cents: row.get(10)?,
        currency: row.get(11)?,
        payment_channel: row.get(12)?,
        reason: row.get(13)?,
        actor_type: row.get(14)?,
        actor_id: row.get(15)?,
        renewal_id: row.get(16)?,
        processing_status: row.get(17)?,
        processing_error: row.get(18)?,
        received_at_ms: row.get(19)?,
        processed_at_ms: row.get(20)?,
    })
}

fn billing_payment_event_select_sql() -> &'static str {
    "SELECT
        payment_event_id, provider, NULLIF(gateway_event_id, ''), external_order_id, status,
        subject_type, subject_id, plan_id, plan_name, daily_token_limit,
        amount_cents, currency, payment_channel, NULLIF(reason, ''),
        actor_type, NULLIF(actor_id, ''), NULLIF(renewal_id, ''), processing_status,
        NULLIF(processing_error, ''), received_at_ms, processed_at_ms
     FROM backend_billing_payment_events"
}

fn billing_payment_event_by_id(
    db: &Connection,
    payment_event_id: &str,
) -> anyhow::Result<Option<LocalBackendBillingPaymentEvent>> {
    let sql = format!(
        "{} WHERE payment_event_id = ?1",
        billing_payment_event_select_sql()
    );
    db.query_row(
        &sql,
        params![payment_event_id.trim()],
        billing_payment_event_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn billing_payment_event_by_gateway_event(
    db: &Connection,
    provider: &str,
    gateway_event_id: &str,
) -> anyhow::Result<Option<LocalBackendBillingPaymentEvent>> {
    let sql = format!(
        "{} WHERE provider = ?1 AND gateway_event_id = ?2
           ORDER BY received_at_ms DESC, rowid DESC
           LIMIT 1",
        billing_payment_event_select_sql()
    );
    db.query_row(
        &sql,
        params![provider.trim(), gateway_event_id.trim()],
        billing_payment_event_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn billing_payment_event_by_order_status(
    db: &Connection,
    provider: &str,
    external_order_id: &str,
    status: &str,
) -> anyhow::Result<Option<LocalBackendBillingPaymentEvent>> {
    let sql = format!(
        "{} WHERE provider = ?1 AND external_order_id = ?2 AND status = ?3
           ORDER BY received_at_ms DESC, rowid DESC
           LIMIT 1",
        billing_payment_event_select_sql()
    );
    db.query_row(
        &sql,
        params![provider.trim(), external_order_id.trim(), status],
        billing_payment_event_from_row,
    )
    .optional()
    .map_err(Into::into)
}

fn normalize_user_access_status(status: &str) -> anyhow::Result<&'static str> {
    match status.trim().to_ascii_lowercase().as_str() {
        "" | "active" | "enabled" | "unblocked" => Ok("active"),
        "blocked" | "disabled" | "banned" | "suspended" => Ok("blocked"),
        value => anyhow::bail!("unsupported user access status: {value}"),
    }
}

fn table_count(db: &Connection, table: &str) -> rusqlite::Result<i64> {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
}

fn table_count_where(
    db: &Connection,
    table: &str,
    predicate: &str,
    params: &[&dyn rusqlite::ToSql],
) -> rusqlite::Result<i64> {
    db.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
        params,
        |row| row.get(0),
    )
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn backend_day_key(timestamp_ms: i64) -> String {
    let days = timestamp_ms.div_euclid(86_400_000);
    let date = time::Date::from_julian_day((days + 2_440_588) as i32).unwrap_or(time::Date::MIN);
    format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month() as u8,
        date.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_account::{
        LocalAccountExport, LocalAuthSessionExport, LocalDeviceExport, LocalEntitlementExport,
        LocalUserExport,
    };
    use crate::local_usage::{LocalUsageEvent, LocalUsageExport, LocalUsageSummary, TokenUsage};

    #[test]
    fn backend_db_path_from_env_values_prefers_managed_proxy_specific_path() {
        let path = backend_db_path_from_env_values(
            Some(" /tmp/jiyi-managed-proxy.sqlite ".to_string()),
            Some("/tmp/jiyi-backend.sqlite".to_string()),
        );

        assert_eq!(path, PathBuf::from("/tmp/jiyi-managed-proxy.sqlite"));
    }

    #[test]
    fn backend_db_path_from_env_values_falls_back_to_backend_path() {
        let path = backend_db_path_from_env_values(
            Some("   ".to_string()),
            Some("/tmp/jiyi-backend.sqlite".to_string()),
        );

        assert_eq!(path, PathBuf::from("/tmp/jiyi-backend.sqlite"));
    }

    #[test]
    fn backend_db_path_from_env_values_uses_default_without_overrides() {
        let path = backend_db_path_from_env_values(None, None);

        assert_eq!(path, default_backend_db_path());
    }

    #[test]
    fn local_backend_applies_identity_sync_body_idempotently() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let body = sample_body();

        let first = store.apply_identity_sync(&body).expect("first sync");
        let second = store.apply_identity_sync(&body).expect("second sync");
        let state = store.state().expect("state");

        assert_eq!(first.users_upserted, 1);
        assert_eq!(second.users_upserted, 1);
        assert_eq!(state.batch_count, 2);
        assert_eq!(state.user_count, 1);
        assert_eq!(state.device_count, 1);
        assert_eq!(state.team_count, 1);
        assert_eq!(state.team_member_count, 1);
        assert_eq!(state.entitlement_count, 1);
        assert_eq!(state.usage_summary_count, 1);
        assert_eq!(state.session_count, 2);
        assert!(state.last_session_issued_at_ms.is_some());
        assert_eq!(first.sessions_issued, 1);
        assert_eq!(second.sessions_issued, 1);
        assert_eq!(first.teams_upserted, 1);
        assert_eq!(first.team_members_upserted, 1);
        assert_eq!(first.total_team_count, 1);
        assert_eq!(first.total_team_member_count, 1);
        assert!(first.active_session.is_some());
    }

    #[test]
    fn local_backend_persists_only_masked_phone_and_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("backend.sqlite");
        let store = LocalBackendStore::new(db_path.clone());
        store.apply_identity_sync(&sample_body()).expect("sync");

        let db = Connection::open(db_path).expect("open db");
        let (phone_masked, phone_hash): (String, String) = db
            .query_row(
                "SELECT phone_masked, phone_hash FROM backend_users WHERE user_id = 'user-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("user");

        assert_eq!(phone_masked, "+86 138****5678");
        assert_eq!(phone_hash, "hash-only");
        assert!(!phone_masked.contains("1234"));
    }

    #[test]
    fn local_backend_stores_session_hash_not_plain_token() {
        let temp = tempfile::tempdir().expect("tempdir");
        let db_path = temp.path().join("backend.sqlite");
        let store = LocalBackendStore::new(db_path.clone());
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let db = Connection::open(db_path).expect("open db");
        let (token_hash, plain_matches): (String, i64) = db
            .query_row(
                "SELECT token_hash, COUNT(*) FILTER (WHERE token_hash = ?1) FROM backend_sessions",
                params![token],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("session");

        assert!(token.starts_with("jiyi-local-"));
        assert_eq!(plain_matches, 0);
        assert_eq!(token_hash, backend_session_token_hash(&token));
    }

    #[test]
    fn local_backend_verifies_session_token_without_exposing_plain_phone() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let verified = store.verify_session_token(&token).expect("verify token");
        let rejected = store
            .verify_session_token("jiyi-local-invalid")
            .expect("reject token");
        let subject = verified.subject.expect("subject");

        assert!(verified.authenticated);
        assert_eq!(subject.user_id, "user-1");
        assert_eq!(subject.device_id, "device-1");
        assert_eq!(subject.phone_masked, "+86 138****5678");
        assert_eq!(subject.plan_id.as_deref(), Some("local_trial"));
        assert!(!subject.phone_masked.contains("12345678"));
        assert!(!rejected.authenticated);
        assert_eq!(rejected.reason.as_deref(), Some("invalid_or_expired_token"));
    }

    #[test]
    fn local_backend_quota_snapshot_uses_backend_session_and_usage_summary() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let snapshot = store.quota_snapshot(&token).expect("quota");
        let rejected = store
            .quota_snapshot("jiyi-local-invalid")
            .expect("rejected quota");
        let quota = snapshot.quota.expect("quota payload");

        assert!(snapshot.authenticated);
        assert_eq!(quota.plan_id.as_deref(), Some("local_trial"));
        assert_eq!(quota.daily_token_limit, 1000);
        assert_eq!(quota.used_tokens, 70);
        assert_eq!(quota.request_count, 2);
        assert_eq!(quota.remaining_tokens, Some(930));
        assert_eq!(quota.limit_source, "backend_entitlement");
        assert!(!rejected.authenticated);
        assert!(rejected.quota.is_none());
    }

    #[test]
    fn local_backend_revokes_session_token_and_rejects_future_verify() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let before = store.state().expect("state before");
        let revoked = store.revoke_session_token(&token).expect("revoke token");
        let verified = store.verify_session_token(&token).expect("verify revoked");
        let quota = store.quota_snapshot(&token).expect("quota revoked");
        let after = store.state().expect("state after");

        assert_eq!(before.active_session_count, 1);
        assert!(revoked.authenticated);
        assert!(revoked.revoked_at_ms.is_some());
        assert_eq!(
            revoked
                .subject
                .as_ref()
                .map(|subject| subject.user_id.as_str()),
            Some("user-1")
        );
        assert!(!verified.authenticated);
        assert!(!quota.authenticated);
        assert_eq!(after.active_session_count, 0);
        assert_eq!(after.revoked_session_count, 1);
        assert!(after.last_session_revoked_at_ms.is_some());
    }

    #[test]
    fn local_backend_blocks_user_and_prevents_new_session_issue() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let blocked = store
            .block_user("user-1", Some("abuse review"))
            .expect("block user");
        let blocked_verify = store
            .verify_session_token(&token)
            .expect("verify blocked token");
        let resync = store.apply_identity_sync(&sample_body()).expect("resync");
        let state = store.state().expect("state after block");

        assert_eq!(blocked.status, "blocked");
        assert_eq!(blocked.reason.as_deref(), Some("abuse review"));
        assert_eq!(blocked.sessions_revoked, 1);
        assert!(!blocked_verify.authenticated);
        assert_eq!(blocked_verify.reason.as_deref(), Some("user_blocked"));
        assert!(resync.active_session.is_none());
        assert_eq!(state.blocked_user_count, 1);
        assert_eq!(state.active_session_count, 0);
        assert!(state.last_user_access_updated_at_ms.is_some());
    }

    #[test]
    fn local_backend_records_audit_events_without_plain_tokens() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        store
            .record_usage_event(
                &token,
                &LocalUsageEvent {
                    method: "POST".to_string(),
                    path: "/v1/responses".to_string(),
                    upstream_protocol: "managed_responses".to_string(),
                    status_code: 200,
                    request_bytes: 100,
                    response_bytes: 200,
                    token_usage: Some(TokenUsage {
                        input_tokens: Some(10),
                        output_tokens: Some(20),
                        total_tokens: Some(30),
                    }),
                },
            )
            .expect("record usage");
        store
            .block_user("user-1", Some("abuse review"))
            .expect("block user");

        let state = store.state().expect("state");
        let events = store.recent_audit_events(10).expect("audit events");
        let filtered_events = store
            .audit_events(LocalBackendAuditEventQuery {
                limit: 10,
                event_type: Some("user_access_updated".to_string()),
                actor_type: Some("local_backend".to_string()),
                subject_user_id: Some("user-1".to_string()),
            })
            .expect("filtered audit events");
        let event_types = events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>();
        let serialized = serde_json::to_string(&events).expect("serialize events");

        assert!(state.audit_event_count >= 3);
        assert!(state.last_audit_event_at_ms.is_some());
        assert!(event_types.contains(&"identity_sync"));
        assert!(event_types.contains(&"usage_recorded"));
        assert!(event_types.contains(&"user_access_updated"));
        assert_eq!(filtered_events.len(), 1);
        assert_eq!(filtered_events[0].event_type, "user_access_updated");
        assert_eq!(filtered_events[0].actor_type, "local_backend");
        assert_eq!(
            filtered_events[0].subject_user_id.as_deref(),
            Some("user-1")
        );
        assert!(serialized.contains("abuse review"));
        assert!(!serialized.contains(&token));
    }

    #[test]
    fn local_backend_admin_updates_entitlement_and_records_audit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let change = store
            .set_user_entitlement_with_actor(
                "user-1",
                "jiyi_pro",
                "极义 Pro",
                5000,
                Some("renewal paid"),
                "managed_proxy_admin_api",
                Some("admin_api_key"),
            )
            .expect("update entitlement");
        let quota = store
            .quota_snapshot(&token)
            .expect("quota")
            .quota
            .expect("quota payload");
        let users = store.admin_user_overviews(10).expect("admin users");
        let events = store.recent_audit_events(10).expect("audit events");
        let serialized_events = serde_json::to_string(&events).expect("serialize events");

        assert_eq!(change.user_id, "user-1");
        assert_eq!(change.plan_id, "jiyi_pro");
        assert_eq!(change.plan_name, "极义 Pro");
        assert_eq!(change.daily_token_limit, 5000);
        assert_eq!(change.previous_plan_id.as_deref(), Some("local_trial"));
        assert_eq!(change.previous_daily_token_limit, Some(1000));
        assert_eq!(quota.plan_id.as_deref(), Some("jiyi_pro"));
        assert_eq!(quota.plan_name.as_deref(), Some("极义 Pro"));
        assert_eq!(quota.daily_token_limit, 5000);
        assert_eq!(quota.remaining_tokens, Some(4930));
        assert_eq!(users.users[0].plan_id.as_deref(), Some("jiyi_pro"));
        assert_eq!(users.users[0].daily_token_limit, Some(5000));
        assert_eq!(users.users[0].today_remaining_tokens, Some(4930));
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "user_entitlement_updated"
                    && event.actor_type == "managed_proxy_admin_api"
                    && event.subject_user_id.as_deref() == Some("user-1"))
        );
        assert!(serialized_events.contains("renewal paid"));
        assert!(!serialized_events.contains(&token));
    }

    #[test]
    fn local_backend_records_usage_event_and_updates_quota_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let recorded = store
            .record_usage_event(
                &token,
                &LocalUsageEvent {
                    method: "POST".to_string(),
                    path: "/v1/responses".to_string(),
                    upstream_protocol: "responses".to_string(),
                    status_code: 200,
                    request_bytes: 120,
                    response_bytes: 240,
                    token_usage: Some(TokenUsage {
                        input_tokens: Some(15),
                        output_tokens: Some(25),
                        total_tokens: Some(40),
                    }),
                },
            )
            .expect("record usage");
        let snapshot = store.quota_snapshot(&token).expect("quota");
        let quota = snapshot.quota.expect("quota payload");

        assert!(recorded.authenticated);
        assert_eq!(recorded.recorded_tokens, 40);
        assert_eq!(recorded.total_used_tokens, 110);
        assert_eq!(recorded.total_request_count, 3);
        assert_eq!(quota.used_tokens, 110);
        assert_eq!(quota.request_count, 3);
        assert_eq!(quota.remaining_tokens, Some(890));
    }

    #[test]
    fn local_backend_admin_user_overviews_include_quota_and_access_status() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        store
            .record_usage_event(
                &token,
                &LocalUsageEvent {
                    method: "POST".to_string(),
                    path: "/v1/responses".to_string(),
                    upstream_protocol: "managed_responses".to_string(),
                    status_code: 200,
                    request_bytes: 100,
                    response_bytes: 200,
                    token_usage: Some(TokenUsage {
                        input_tokens: Some(10),
                        output_tokens: Some(20),
                        total_tokens: Some(30),
                    }),
                },
            )
            .expect("record usage");
        store
            .block_user("user-1", Some("abuse review"))
            .expect("block user");

        let list = store.admin_user_overviews(10).expect("admin users");
        let user = list.users.first().expect("first user");
        let serialized = serde_json::to_string(&list).expect("serialize user list");

        assert_eq!(list.day, backend_day_key(now_ms()));
        assert_eq!(user.user_id, "user-1");
        assert_eq!(user.phone_masked, "+86 138****5678");
        assert_eq!(user.access_status, "blocked");
        assert_eq!(user.access_reason.as_deref(), Some("abuse review"));
        assert_eq!(user.plan_id.as_deref(), Some("local_trial"));
        assert_eq!(user.plan_name.as_deref(), Some("本地试用"));
        assert_eq!(user.daily_token_limit, Some(1000));
        assert_eq!(user.device_count, 1);
        assert_eq!(user.session_count, 1);
        assert_eq!(user.active_session_count, 0);
        assert_eq!(user.revoked_session_count, 1);
        assert_eq!(user.today_request_count, 3);
        assert_eq!(user.today_used_tokens, 100);
        assert_eq!(user.today_remaining_tokens, Some(900));
        assert!(!serialized.contains(&token));
        assert!(!serialized.contains("hash-only"));
    }

    #[test]
    fn local_backend_admin_team_overviews_include_members_and_usage() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        store
            .record_usage_event(
                &token,
                &LocalUsageEvent {
                    method: "POST".to_string(),
                    path: "/v1/responses".to_string(),
                    upstream_protocol: "managed_responses".to_string(),
                    status_code: 200,
                    request_bytes: 100,
                    response_bytes: 200,
                    token_usage: Some(TokenUsage {
                        input_tokens: Some(10),
                        output_tokens: Some(20),
                        total_tokens: Some(30),
                    }),
                },
            )
            .expect("record usage");
        store
            .block_user("user-1", Some("abuse review"))
            .expect("block user");
        let change = store
            .set_team_entitlement_with_actor(
                DEFAULT_BACKEND_TEAM_ID,
                "team_pro",
                "团队 Pro",
                5000,
                Some("team renewal"),
                "managed_proxy_admin_api",
                Some("billing_api_key"),
            )
            .expect("update team entitlement");

        let list = store.admin_team_overviews(10).expect("admin teams");
        let team = list.teams.first().expect("first team");
        let events = store.recent_audit_events(10).expect("audit events");
        let serialized = serde_json::to_string(&list).expect("serialize team list");

        assert_eq!(change.team_id, DEFAULT_BACKEND_TEAM_ID);
        assert_eq!(change.plan_id, "team_pro");
        assert_eq!(
            change.previous_plan_id.as_deref(),
            Some(DEFAULT_BACKEND_TEAM_PLAN_ID)
        );
        assert_eq!(list.day, backend_day_key(now_ms()));
        assert_eq!(team.team_id, DEFAULT_BACKEND_TEAM_ID);
        assert_eq!(team.team_name, DEFAULT_BACKEND_TEAM_NAME);
        assert_eq!(team.plan_id, "team_pro");
        assert_eq!(team.plan_name, "团队 Pro");
        assert_eq!(team.daily_token_limit, 5000);
        assert_eq!(team.member_count, 1);
        assert_eq!(team.active_member_count, 0);
        assert_eq!(team.blocked_member_count, 1);
        assert_eq!(team.today_request_count, 3);
        assert_eq!(team.today_used_tokens, 100);
        assert_eq!(team.today_remaining_tokens, Some(4900));
        assert!(team.last_member_updated_at_ms.is_some());
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "team_entitlement_updated"
                    && event.actor_type == "managed_proxy_admin_api"
                    && event.metadata["teamId"] == DEFAULT_BACKEND_TEAM_ID)
        );
        assert!(serialized.contains("团队 Pro"));
        assert!(!serialized.contains(&token));
        assert!(!serialized.contains("hash-only"));
    }

    #[test]
    fn local_backend_records_billing_renewal_and_updates_entitlement() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();

        let renewal = store
            .record_billing_renewal_with_actor(
                "user",
                "user-1",
                "jiyi_pro",
                "极义 Pro",
                5000,
                19900,
                "cny",
                "manual",
                Some("order-001"),
                Some("manual renewal"),
                "managed_proxy_admin_api",
                Some("billing_api_key"),
            )
            .expect("record user renewal");
        let team_renewal = store
            .record_billing_renewal_with_actor(
                "team",
                DEFAULT_BACKEND_TEAM_ID,
                "team_pro",
                "团队 Pro",
                50_000,
                99_000,
                "CNY",
                "bank_transfer",
                Some("team-order-001"),
                Some("team contract renewal"),
                "managed_proxy_admin_api",
                Some("billing_api_key"),
            )
            .expect("record team renewal");

        let quota = store
            .quota_snapshot(&token)
            .expect("quota")
            .quota
            .expect("quota payload");
        let teams = store.admin_team_overviews(10).expect("teams");
        let renewals = store.billing_renewals(10).expect("renewals");
        let events = store
            .audit_events(LocalBackendAuditEventQuery {
                limit: 10,
                event_type: Some("billing_renewal_recorded".to_string()),
                ..LocalBackendAuditEventQuery::default()
            })
            .expect("audit events");
        let state = store.state().expect("state");
        let serialized = serde_json::to_string(&renewals).expect("serialize renewals");

        assert_eq!(renewal.subject_type, "user");
        assert_eq!(renewal.subject_id, "user-1");
        assert_eq!(renewal.plan_id, "jiyi_pro");
        assert_eq!(renewal.amount_cents, 19900);
        assert_eq!(renewal.currency, "CNY");
        assert_eq!(renewal.previous_plan_id.as_deref(), Some("local_trial"));
        assert_eq!(team_renewal.subject_type, "team");
        assert_eq!(team_renewal.subject_id, DEFAULT_BACKEND_TEAM_ID);
        assert_eq!(
            team_renewal.previous_plan_id.as_deref(),
            Some(DEFAULT_BACKEND_TEAM_PLAN_ID)
        );
        assert_eq!(quota.plan_id.as_deref(), Some("jiyi_pro"));
        assert_eq!(quota.remaining_tokens, Some(4930));
        assert_eq!(teams.teams[0].plan_id, "team_pro");
        assert_eq!(teams.teams[0].daily_token_limit, 50_000);
        assert_eq!(renewals.renewals.len(), 2);
        assert_eq!(state.billing_renewal_count, 2);
        assert!(state.last_billing_renewal_at_ms.is_some());
        assert_eq!(events.len(), 2);
        assert!(
            events
                .iter()
                .any(|event| event.metadata["renewalId"] == renewal.renewal_id
                    && event.subject_user_id.as_deref() == Some("user-1"))
        );
        assert!(serialized.contains("order-001"));
        assert!(!serialized.contains(&token));
        assert!(!serialized.contains("hash-only"));
    }

    #[test]
    fn local_backend_records_payment_webhook_idempotently_and_reconciles() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalBackendStore::new(temp.path().join("backend.sqlite"));
        let receipt = store.apply_identity_sync(&sample_body()).expect("sync");
        let token = receipt
            .active_session
            .as_ref()
            .expect("active session")
            .access_token
            .clone();
        let raw_payload = json!({
            "gatewaySecret": "payment-secret",
            "payerPhone": "13800000000",
            "event": "evt_001"
        });

        let first = store
            .record_billing_payment_webhook_with_actor(
                "mockpay",
                Some("evt_001"),
                "pay-order-001",
                "trade_success",
                "user",
                "user-1",
                "jiyi_pro",
                "极义 Pro",
                5000,
                19900,
                "cny",
                None,
                Some("gateway callback"),
                &raw_payload,
                "payment_webhook_api",
                Some("payment_webhook_api_key"),
            )
            .expect("record webhook");
        let duplicate = store
            .record_billing_payment_webhook_with_actor(
                "mockpay",
                Some("evt_001"),
                "pay-order-001",
                "paid",
                "user",
                "user-1",
                "jiyi_pro",
                "极义 Pro",
                5000,
                19900,
                "CNY",
                None,
                Some("duplicate gateway callback"),
                &raw_payload,
                "payment_webhook_api",
                Some("payment_webhook_api_key"),
            )
            .expect("duplicate webhook");
        let reconciliation = store
            .reconcile_billing_payment_events_with_actor(
                10,
                "managed_proxy_admin_api",
                Some("billing_api_key"),
            )
            .expect("reconcile");

        let quota = store
            .quota_snapshot(&token)
            .expect("quota")
            .quota
            .expect("quota payload");
        let renewals = store.billing_renewals(10).expect("renewals");
        let events = store
            .audit_events(LocalBackendAuditEventQuery {
                limit: 20,
                event_type: None,
                ..LocalBackendAuditEventQuery::default()
            })
            .expect("audit events");
        let state = store.state().expect("state");
        let serialized_first = serde_json::to_string(&first).expect("serialize first");
        let serialized_events = serde_json::to_string(&events).expect("serialize audit");

        assert!(!first.duplicate);
        assert_eq!(first.event.provider, "mockpay");
        assert_eq!(first.event.status, "paid");
        assert_eq!(first.event.processing_status, "applied");
        assert_eq!(
            first
                .renewal
                .as_ref()
                .expect("renewal")
                .external_order_id
                .as_deref(),
            Some("pay-order-001")
        );
        assert!(duplicate.duplicate);
        assert_eq!(
            duplicate.event.payment_event_id,
            first.event.payment_event_id
        );
        assert_eq!(reconciliation.attempted, 0);
        assert_eq!(quota.plan_id.as_deref(), Some("jiyi_pro"));
        assert_eq!(quota.remaining_tokens, Some(4930));
        assert_eq!(renewals.renewals.len(), 1);
        assert_eq!(state.billing_payment_event_count, 1);
        assert_eq!(state.billing_renewal_count, 1);
        assert!(state.last_billing_payment_event_at_ms.is_some());
        assert!(events.iter().any(
            |event| event.event_type == "billing_payment_webhook_received"
                && event.metadata["rawPayloadSha256"].as_str().is_some()
        ));
        assert!(events.iter().any(
            |event| event.event_type == "billing_payment_event_reconciled"
                && event.metadata["processingStatus"] == "applied"
        ));
        assert!(!serialized_first.contains("payment-secret"));
        assert!(!serialized_first.contains("13800000000"));
        assert!(!serialized_events.contains("payment-secret"));
        assert!(!serialized_events.contains("13800000000"));
        assert!(!serialized_events.contains(&token));
    }

    fn sample_body() -> IdentitySyncBody {
        let expires_at_ms = now_ms() + 60 * 60 * 1000;
        IdentitySyncBody {
            generated_at_ms: 1_000,
            schema_version: 1,
            pii_policy: "masked".to_string(),
            account: LocalAccountExport {
                generated_at_ms: 1_000,
                db_path: "/tmp/local.sqlite".to_string(),
                active_session: Some(LocalAuthSessionExport {
                    user_id: "user-1".to_string(),
                    phone_masked: "+86 138****5678".to_string(),
                    login_at_ms: 900,
                    expires_at_ms,
                    device_id: "device-1".to_string(),
                    session_expired: false,
                }),
                users: vec![LocalUserExport {
                    user_id: "user-1".to_string(),
                    phone_masked: "+86 138****5678".to_string(),
                    phone_hash: "hash-only".to_string(),
                    created_at_ms: 800,
                    last_login_at_ms: 900,
                }],
                devices: vec![LocalDeviceExport {
                    user_id: "user-1".to_string(),
                    device_id: "device-1".to_string(),
                    first_seen_at_ms: 800,
                    last_seen_at_ms: 900,
                }],
                entitlements: vec![LocalEntitlementExport {
                    user_id: "user-1".to_string(),
                    plan_id: "local_trial".to_string(),
                    plan_name: "本地试用".to_string(),
                    daily_token_limit: 1000,
                    updated_at_ms: 900,
                }],
            },
            usage: LocalUsageExport {
                generated_at_ms: 1_000,
                db_path: "/tmp/usage.sqlite".to_string(),
                summaries: vec![LocalUsageSummary {
                    day: backend_day_key(now_ms()),
                    subject_id: "user-1".to_string(),
                    plan_id: Some("local_trial".to_string()),
                    request_count: 2,
                    request_bytes: 100,
                    response_bytes: 200,
                    estimated_tokens: 75,
                    reported_total_tokens: 70,
                    effective_total_tokens: 70,
                    first_seen_at_ms: 800,
                    last_seen_at_ms: 900,
                }],
            },
        }
    }
}
