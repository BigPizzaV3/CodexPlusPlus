use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use hmac::{Hmac, Mac};
use reqwest::header::{HeaderMap, HeaderValue};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

const AUTH_DB_FILE: &str = "jiyi-codex-local.sqlite";
const SMS_SERVICE: &str = "sms";
const SMS_HOST: &str = "sms.tencentcloudapi.com";
const SMS_ENDPOINT: &str = "https://sms.tencentcloudapi.com";
const SMS_ACTION: &str = "SendSms";
const SMS_VERSION: &str = "2021-01-11";
const DEFAULT_SESSION_TTL_HOURS: i64 = 24 * 30;
const DEFAULT_PLAN_ID: &str = "local_trial";
const DEFAULT_PLAN_NAME: &str = "本地试用";
const DEFAULT_SMS_REGION: &str = "ap-guangzhou";
const DEFAULT_SMS_TEMPLATE_PARAM_MODE: &str = "code_ttl";
const DEFAULT_SMS_TTL_MINUTES: i64 = 10;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsProviderSettings {
    #[serde(default = "default_sms_region")]
    pub region: String,
    #[serde(default)]
    pub app_id: String,
    #[serde(default)]
    pub sign_name: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default = "default_sms_ttl_minutes")]
    pub ttl_minutes: i64,
    #[serde(default = "default_sms_template_param_mode")]
    pub template_param_mode: String,
    #[serde(default = "default_sms_dry_run")]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsProviderSettingsState {
    pub settings_path: String,
    pub settings: SmsProviderSettings,
    pub sms_config: SmsConfigState,
    pub secret_id_ref: String,
    pub secret_key_ref: String,
}

impl Default for SmsProviderSettings {
    fn default() -> Self {
        Self {
            region: DEFAULT_SMS_REGION.to_string(),
            app_id: String::new(),
            sign_name: String::new(),
            template_id: String::new(),
            ttl_minutes: DEFAULT_SMS_TTL_MINUTES,
            template_param_mode: DEFAULT_SMS_TEMPLATE_PARAM_MODE.to_string(),
            dry_run: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsConfigState {
    pub configured: bool,
    pub dry_run: bool,
    pub region: String,
    pub secret_id_set: bool,
    pub secret_key_set: bool,
    pub secret_id_source: String,
    pub secret_key_source: String,
    pub app_id_set: bool,
    pub sign_name_set: bool,
    pub template_id_set: bool,
    pub ttl_minutes: i64,
    pub template_param_mode: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAuthState {
    pub authenticated: bool,
    pub user_id: Option<String>,
    pub phone: Option<String>,
    pub phone_masked: Option<String>,
    pub login_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub device_id: Option<String>,
    pub session_ttl_hours: i64,
    pub session_expired: bool,
    pub db_path: String,
    pub sms_config: SmsConfigState,
    pub entitlement: LocalEntitlementState,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntitlementState {
    pub user_id: Option<String>,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub source: String,
    pub updated_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmsCodeIssue {
    pub phone: String,
    pub phone_masked: String,
    pub expires_at_ms: i64,
    pub dry_run: bool,
    pub dev_code: Option<String>,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginSession {
    pub user_id: String,
    pub phone: String,
    pub phone_masked: String,
    pub login_at_ms: i64,
    pub expires_at_ms: i64,
    pub device_id: String,
    pub session_ttl_hours: i64,
    pub entitlement: LocalEntitlementState,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAccountExport {
    pub generated_at_ms: i64,
    pub db_path: String,
    pub active_session: Option<LocalAuthSessionExport>,
    pub users: Vec<LocalUserExport>,
    pub devices: Vec<LocalDeviceExport>,
    pub entitlements: Vec<LocalEntitlementExport>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalUserExport {
    pub user_id: String,
    pub phone_masked: String,
    pub phone_hash: String,
    pub created_at_ms: i64,
    pub last_login_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDeviceExport {
    pub user_id: String,
    pub device_id: String,
    pub first_seen_at_ms: i64,
    pub last_seen_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalEntitlementExport {
    pub user_id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub daily_token_limit: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalAuthSessionExport {
    pub user_id: String,
    pub phone_masked: String,
    pub login_at_ms: i64,
    pub expires_at_ms: i64,
    pub device_id: String,
    pub session_expired: bool,
}

#[derive(Debug, Clone)]
pub struct LocalAccountStore {
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
struct NormalizedPhone {
    e164: String,
}

#[derive(Debug, Clone)]
struct SmsProviderConfig {
    region: String,
    secret_id: String,
    secret_key: String,
    secret_id_source: String,
    secret_key_source: String,
    app_id: String,
    sign_name: String,
    template_id: String,
    configured: bool,
    dry_run: bool,
    ttl_minutes: i64,
    template_param_mode: String,
}

#[derive(Debug, Deserialize)]
struct TencentSmsResponse {
    #[serde(rename = "Response")]
    response: TencentSmsResponseBody,
}

#[derive(Debug, Deserialize)]
struct TencentSmsResponseBody {
    #[serde(rename = "RequestId")]
    request_id: Option<String>,
    #[serde(rename = "Error")]
    error: Option<TencentSmsError>,
    #[serde(rename = "SendStatusSet")]
    send_status_set: Option<Vec<TencentSmsStatus>>,
}

#[derive(Debug, Deserialize)]
struct TencentSmsError {
    #[serde(rename = "Code")]
    code: String,
    #[serde(rename = "Message")]
    message: String,
}

#[derive(Debug, Deserialize)]
struct TencentSmsStatus {
    #[serde(rename = "Code")]
    code: Option<String>,
    #[serde(rename = "Message")]
    message: Option<String>,
    #[serde(rename = "PhoneNumber")]
    phone_number: Option<String>,
}

impl Default for LocalAccountStore {
    fn default() -> Self {
        Self::new(default_auth_db_path())
    }
}

impl LocalAccountStore {
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    pub fn load_auth_state(&self) -> anyhow::Result<LocalAuthState> {
        self.ensure_schema()?;
        let config = SmsProviderConfig::from_env();
        let ttl_hours = session_ttl_hours();
        let now = now_ms();
        let db = Connection::open(&self.db_path)?;
        let row = db
            .query_row(
                "SELECT user_id, phone, login_at_ms, session_expires_at_ms, device_id FROM auth_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(match row {
            Some((user_id, phone, login_at_ms, expires_at_ms, device_id))
                if expires_at_ms >= now =>
            {
                let entitlement = load_or_create_entitlement(&db, &user_id, now)?;
                LocalAuthState {
                    authenticated: true,
                    user_id: Some(user_id.clone()),
                    phone_masked: Some(mask_phone(&phone)),
                    phone: Some(phone),
                    login_at_ms: Some(login_at_ms),
                    expires_at_ms: Some(expires_at_ms),
                    device_id: Some(device_id),
                    session_ttl_hours: ttl_hours,
                    session_expired: false,
                    db_path: self.db_path.to_string_lossy().to_string(),
                    sms_config: config.state(),
                    entitlement,
                }
            }
            Some(_) => {
                db.execute("DELETE FROM auth_state WHERE id = 1", [])?;
                LocalAuthState::signed_out(&self.db_path, config.state(), ttl_hours, true)
            }
            None => LocalAuthState::signed_out(&self.db_path, config.state(), ttl_hours, false),
        })
    }

    pub async fn request_sms_code(&self, phone_input: &str) -> anyhow::Result<SmsCodeIssue> {
        self.ensure_schema()?;
        let phone = normalize_phone(phone_input)?;
        let config = SmsProviderConfig::from_env();
        let now = now_ms();
        {
            let db = Connection::open(&self.db_path)?;
            ensure_sms_rate_limit(&db, &phone.e164, now)?;
        }

        let code = generate_sms_code();
        let expires_at_ms = now + config.ttl_minutes.max(1) * 60 * 1000;
        let send_result = send_sms_code(&config, &phone.e164, &code).await?;
        let code_hash = hash_sms_code(&phone.e164, &code);

        let db = Connection::open(&self.db_path)?;
        db.execute(
            "INSERT INTO sms_codes (id, phone, code_hash, created_at_ms, expires_at_ms, consumed_at_ms, dry_run, request_id)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7)",
            params![
                Uuid::new_v4().to_string(),
                phone.e164,
                code_hash,
                now,
                expires_at_ms,
                if config.dry_run { 1 } else { 0 },
                send_result.request_id,
            ],
        )?;

        let _ = crate::diagnostic_log::append_diagnostic_log(
            "local_account.sms_code_requested",
            json!({
                "phone": mask_phone(&phone.e164),
                "dry_run": config.dry_run,
                "expires_at_ms": expires_at_ms,
                "request_id": send_result.request_id,
            }),
        );

        Ok(SmsCodeIssue {
            phone: phone.e164.clone(),
            phone_masked: mask_phone(&phone.e164),
            expires_at_ms,
            dry_run: config.dry_run,
            dev_code: config.dry_run.then_some(code),
            request_id: send_result.request_id,
        })
    }

    pub fn login_with_sms_code(
        &self,
        phone_input: &str,
        code_input: &str,
    ) -> anyhow::Result<LoginSession> {
        self.ensure_schema()?;
        let phone = normalize_phone(phone_input)?;
        let code = normalize_sms_code(code_input)?;
        let now = now_ms();
        let code_hash = hash_sms_code(&phone.e164, &code);
        let mut db = Connection::open(&self.db_path)?;
        let tx = db.transaction()?;
        let ttl_hours = session_ttl_hours();
        let expires_at_ms = now + ttl_hours.max(1) * 60 * 60 * 1000;
        let device_id = load_or_create_device_id(&tx)?;
        let code_id = tx
            .query_row(
                "SELECT id FROM sms_codes
                 WHERE phone = ?1 AND code_hash = ?2 AND consumed_at_ms IS NULL AND expires_at_ms >= ?3
                 ORDER BY created_at_ms DESC
                 LIMIT 1",
                params![phone.e164, code_hash, now],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("验证码无效或已过期。"))?;

        tx.execute(
            "UPDATE sms_codes SET consumed_at_ms = ?1 WHERE id = ?2",
            params![now, code_id],
        )?;

        let user_id = tx
            .query_row(
                "SELECT user_id FROM local_users WHERE phone = ?1",
                params![phone.e164],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        tx.execute(
            "INSERT INTO local_users (user_id, phone, created_at_ms, last_login_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(phone) DO UPDATE SET last_login_at_ms = excluded.last_login_at_ms",
            params![user_id, phone.e164, now, now],
        )?;
        bind_local_user_device(&tx, &user_id, &device_id, now)?;
        let entitlement = load_or_create_entitlement(&tx, &user_id, now)?;
        tx.execute(
            "INSERT INTO auth_state (id, user_id, phone, session_token, login_at_ms, session_expires_at_ms, device_id)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET user_id = excluded.user_id, phone = excluded.phone, session_token = excluded.session_token, login_at_ms = excluded.login_at_ms, session_expires_at_ms = excluded.session_expires_at_ms, device_id = excluded.device_id",
            params![user_id, phone.e164, Uuid::new_v4().to_string(), now, expires_at_ms, device_id],
        )?;
        tx.commit()?;

        let _ = crate::diagnostic_log::append_diagnostic_log(
            "local_account.login",
            json!({
                "user_id": user_id,
                "phone": mask_phone(&phone.e164),
                "device_id": device_id,
                "plan_id": entitlement.plan_id,
                "daily_token_limit": entitlement.daily_token_limit,
            }),
        );

        Ok(LoginSession {
            user_id,
            phone: phone.e164.clone(),
            phone_masked: mask_phone(&phone.e164),
            login_at_ms: now,
            expires_at_ms,
            device_id,
            session_ttl_hours: ttl_hours,
            entitlement,
        })
    }

    pub fn load_active_entitlement(&self) -> anyhow::Result<Option<LocalEntitlementState>> {
        let state = self.load_auth_state()?;
        Ok(state.authenticated.then_some(state.entitlement))
    }

    pub fn export_state(&self) -> anyhow::Result<LocalAccountExport> {
        self.ensure_schema()?;
        let generated_at_ms = now_ms();
        let db = Connection::open(&self.db_path)?;
        let active_session = db
            .query_row(
                "SELECT user_id, phone, login_at_ms, session_expires_at_ms, device_id
                 FROM auth_state
                 WHERE id = 1",
                [],
                |row| {
                    let phone = row.get::<_, String>(1)?;
                    let expires_at_ms = row.get::<_, i64>(3)?;
                    Ok(LocalAuthSessionExport {
                        user_id: row.get(0)?,
                        phone_masked: mask_phone(&phone),
                        login_at_ms: row.get(2)?,
                        expires_at_ms,
                        device_id: row.get(4)?,
                        session_expired: expires_at_ms < generated_at_ms,
                    })
                },
            )
            .optional()?;

        let users = {
            let mut statement = db.prepare(
                "SELECT user_id, phone, created_at_ms, last_login_at_ms
                 FROM local_users
                 ORDER BY last_login_at_ms DESC, created_at_ms DESC",
            )?;
            statement
                .query_map([], |row| {
                    let phone = row.get::<_, String>(1)?;
                    Ok(LocalUserExport {
                        user_id: row.get(0)?,
                        phone_masked: mask_phone(&phone),
                        phone_hash: phone_export_hash(&phone),
                        created_at_ms: row.get(2)?,
                        last_login_at_ms: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };

        let devices = {
            let mut statement = db.prepare(
                "SELECT user_id, device_id, first_seen_at_ms, last_seen_at_ms
                 FROM local_user_devices
                 ORDER BY last_seen_at_ms DESC, first_seen_at_ms DESC",
            )?;
            statement
                .query_map([], |row| {
                    Ok(LocalDeviceExport {
                        user_id: row.get(0)?,
                        device_id: row.get(1)?,
                        first_seen_at_ms: row.get(2)?,
                        last_seen_at_ms: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };

        let entitlements = {
            let mut statement = db.prepare(
                "SELECT user_id, plan_id, plan_name, daily_token_limit, updated_at_ms
                 FROM local_entitlements
                 ORDER BY updated_at_ms DESC",
            )?;
            statement
                .query_map([], |row| {
                    Ok(LocalEntitlementExport {
                        user_id: row.get(0)?,
                        plan_id: row.get(1)?,
                        plan_name: row.get(2)?,
                        daily_token_limit: row.get::<_, i64>(3)?.max(0),
                        updated_at_ms: row.get(4)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?
        };

        Ok(LocalAccountExport {
            generated_at_ms,
            db_path: self.db_path.to_string_lossy().to_string(),
            active_session,
            users,
            devices,
            entitlements,
        })
    }

    pub fn update_active_entitlement(
        &self,
        plan_id: &str,
        plan_name: &str,
        daily_token_limit: i64,
    ) -> anyhow::Result<LocalEntitlementState> {
        let auth = self.load_auth_state()?;
        let user_id = auth
            .user_id
            .filter(|_| auth.authenticated)
            .ok_or_else(|| anyhow::anyhow!("请先完成手机号验证码登录。"))?;
        let plan_id = normalize_plan_id(plan_id)?;
        let plan_name = normalize_plan_name(plan_name)?;
        let daily_token_limit = normalize_daily_token_limit(daily_token_limit)?;
        let now = now_ms();
        let db = Connection::open(&self.db_path)?;
        db.execute(
            "INSERT INTO local_entitlements (user_id, plan_id, plan_name, daily_token_limit, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(user_id) DO UPDATE SET
               plan_id = excluded.plan_id,
               plan_name = excluded.plan_name,
               daily_token_limit = excluded.daily_token_limit,
               updated_at_ms = excluded.updated_at_ms",
            params![user_id, plan_id, plan_name, daily_token_limit, now],
        )?;

        let _ = crate::diagnostic_log::append_diagnostic_log(
            "local_account.entitlement_updated",
            json!({
                "user_id": user_id,
                "plan_id": plan_id,
                "daily_token_limit": daily_token_limit,
            }),
        );

        Ok(LocalEntitlementState {
            user_id: Some(user_id),
            plan_id,
            plan_name,
            daily_token_limit,
            source: "local_entitlement_admin".to_string(),
            updated_at_ms: Some(now),
        })
    }

    pub fn logout(&self) -> anyhow::Result<()> {
        self.ensure_schema()?;
        let db = Connection::open(&self.db_path)?;
        db.execute("DELETE FROM auth_state WHERE id = 1", [])?;
        let _ = crate::diagnostic_log::append_diagnostic_log("local_account.logout", json!({}));
        Ok(())
    }

    pub fn hard_reset(&self) -> anyhow::Result<()> {
        let base_db_path = self.db_path.to_string_lossy();
        let paths = [
            PathBuf::from(base_db_path.as_ref()),
            PathBuf::from(format!("{base_db_path}-shm")),
            PathBuf::from(format!("{base_db_path}-wal")),
        ];
        for path in paths {
            match fs::remove_file(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("清理本地账号数据库失败"),
            }
        }
        let _ = crate::diagnostic_log::append_diagnostic_log("local_account.hard_reset", json!({}));
        Ok(())
    }

    fn ensure_schema(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(&self.db_path)?;
        db.execute_batch(
            r#"
	CREATE TABLE IF NOT EXISTS local_users (
	  user_id TEXT PRIMARY KEY,
	  phone TEXT NOT NULL UNIQUE,
	  created_at_ms INTEGER NOT NULL,
	  last_login_at_ms INTEGER NOT NULL
	);

	CREATE TABLE IF NOT EXISTS local_user_devices (
	  user_id TEXT NOT NULL,
	  device_id TEXT NOT NULL,
	  first_seen_at_ms INTEGER NOT NULL,
	  last_seen_at_ms INTEGER NOT NULL,
	  PRIMARY KEY(user_id, device_id)
	);

	CREATE TABLE IF NOT EXISTS local_entitlements (
	  user_id TEXT PRIMARY KEY,
	  plan_id TEXT NOT NULL,
	  plan_name TEXT NOT NULL,
	  daily_token_limit INTEGER NOT NULL DEFAULT 0,
	  updated_at_ms INTEGER NOT NULL
	);
	
	CREATE TABLE IF NOT EXISTS local_device (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sms_codes (
  id TEXT PRIMARY KEY,
  phone TEXT NOT NULL,
  code_hash TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  expires_at_ms INTEGER NOT NULL,
  consumed_at_ms INTEGER,
  dry_run INTEGER NOT NULL DEFAULT 0,
  request_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_sms_codes_phone_created
  ON sms_codes(phone, created_at_ms DESC);

CREATE TABLE IF NOT EXISTS auth_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  user_id TEXT NOT NULL,
  phone TEXT NOT NULL,
  session_token TEXT NOT NULL,
  login_at_ms INTEGER NOT NULL,
  session_expires_at_ms INTEGER NOT NULL DEFAULT 0,
  device_id TEXT NOT NULL DEFAULT ''
);
	"#,
        )?;
        ensure_column(
            &db,
            "auth_state",
            "session_expires_at_ms",
            "ALTER TABLE auth_state ADD COLUMN session_expires_at_ms INTEGER NOT NULL DEFAULT 0",
        )?;
        ensure_column(
            &db,
            "auth_state",
            "device_id",
            "ALTER TABLE auth_state ADD COLUMN device_id TEXT NOT NULL DEFAULT ''",
        )?;
        let now = now_ms();
        let fallback_expires = now + session_ttl_hours().max(1) * 60 * 60 * 1000;
        db.execute(
            "UPDATE auth_state SET session_expires_at_ms = ?1 WHERE session_expires_at_ms <= 0",
            params![fallback_expires],
        )?;
        let device_id = load_or_create_device_id(&db)?;
        db.execute(
            "UPDATE auth_state SET device_id = ?1 WHERE device_id = ''",
            params![device_id],
        )?;
        Ok(())
    }
}

impl LocalAuthState {
    fn signed_out(
        db_path: &std::path::Path,
        sms_config: SmsConfigState,
        session_ttl_hours: i64,
        session_expired: bool,
    ) -> Self {
        Self {
            authenticated: false,
            user_id: None,
            phone: None,
            phone_masked: None,
            login_at_ms: None,
            expires_at_ms: None,
            device_id: None,
            session_ttl_hours,
            session_expired,
            db_path: db_path.to_string_lossy().to_string(),
            sms_config,
            entitlement: LocalEntitlementState::signed_out(),
        }
    }
}

impl LocalEntitlementState {
    fn signed_out() -> Self {
        Self {
            user_id: None,
            plan_id: DEFAULT_PLAN_ID.to_string(),
            plan_name: DEFAULT_PLAN_NAME.to_string(),
            daily_token_limit: default_entitlement_daily_token_limit(),
            source: "signed_out_default".to_string(),
            updated_at_ms: None,
        }
    }
}

pub fn default_auth_db_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(AUTH_DB_FILE)
}

pub fn load_sms_provider_settings_state() -> anyhow::Result<SmsProviderSettingsState> {
    let settings = read_sms_provider_settings()?;
    let config = SmsProviderConfig::from_settings(&settings);
    Ok(SmsProviderSettingsState {
        settings_path: default_sms_provider_settings_path()
            .to_string_lossy()
            .to_string(),
        settings,
        sms_config: config.state(),
        secret_id_ref: crate::secret_store::keychain_ref(
            crate::secret_store::tencent_sms_secret_id_account(),
        ),
        secret_key_ref: crate::secret_store::keychain_ref(
            crate::secret_store::tencent_sms_secret_key_account(),
        ),
    })
}

pub fn save_sms_provider_settings(settings: &SmsProviderSettings) -> anyhow::Result<()> {
    let settings = normalize_sms_provider_settings(settings.clone());
    let path = default_sms_provider_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = format!("{}\n", serde_json::to_string_pretty(&settings)?);
    fs::write(path, contents)?;
    Ok(())
}

pub fn default_sms_provider_settings_path() -> PathBuf {
    crate::paths::default_sms_provider_settings_path()
}

fn read_sms_provider_settings() -> anyhow::Result<SmsProviderSettings> {
    let path = default_sms_provider_settings_path();
    if !path.exists() {
        return Ok(SmsProviderSettings::default());
    }
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("读取短信配置失败：{}", path.display()))?;
    let settings: SmsProviderSettings = serde_json::from_str(&contents)
        .with_context(|| format!("解析短信配置失败：{}", path.display()))?;
    Ok(normalize_sms_provider_settings(settings))
}

fn normalize_sms_provider_settings(mut settings: SmsProviderSettings) -> SmsProviderSettings {
    settings.region = non_empty_or(settings.region.trim(), DEFAULT_SMS_REGION);
    settings.app_id = settings.app_id.trim().to_string();
    settings.sign_name = settings.sign_name.trim().to_string();
    settings.template_id = settings.template_id.trim().to_string();
    settings.ttl_minutes = settings.ttl_minutes.clamp(1, 60);
    settings.template_param_mode = normalize_sms_template_param_mode(&settings.template_param_mode);
    settings
}

fn normalize_sms_template_param_mode(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "code" => "code".to_string(),
        "ttl_code" => "ttl_code".to_string(),
        _ => DEFAULT_SMS_TEMPLATE_PARAM_MODE.to_string(),
    }
}

fn ensure_sms_rate_limit(db: &Connection, phone: &str, now: i64) -> anyhow::Result<()> {
    let latest = db
        .query_row(
            "SELECT created_at_ms FROM sms_codes WHERE phone = ?1 ORDER BY created_at_ms DESC LIMIT 1",
            params![phone],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    if let Some(created_at_ms) = latest {
        let remain_ms = 60_000 - (now - created_at_ms);
        if remain_ms > 0 {
            anyhow::bail!("验证码刚发送过，请 {} 秒后再试。", (remain_ms + 999) / 1000);
        }
    }
    Ok(())
}

fn normalize_phone(input: &str) -> anyhow::Result<NormalizedPhone> {
    let trimmed = input
        .chars()
        .filter(|ch| ch.is_ascii_digit() || *ch == '+')
        .collect::<String>();
    let local = if let Some(rest) = trimmed.strip_prefix("+86") {
        rest.to_string()
    } else if trimmed.len() == 13 && trimmed.starts_with("86") {
        trimmed[2..].to_string()
    } else {
        trimmed
    };
    if local.len() != 11 || !local.starts_with('1') || !local.chars().all(|ch| ch.is_ascii_digit())
    {
        anyhow::bail!("请输入有效的中国大陆 11 位手机号。");
    }
    Ok(NormalizedPhone {
        e164: format!("+86{local}"),
    })
}

fn normalize_sms_code(input: &str) -> anyhow::Result<String> {
    let code = input
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if code.len() != 6 {
        anyhow::bail!("请输入 6 位短信验证码。");
    }
    Ok(code)
}

fn normalize_plan_id(input: &str) -> anyhow::Result<String> {
    let value = input.trim();
    if value.is_empty() {
        anyhow::bail!("套餐 ID 不能为空。");
    }
    if value.len() > 64 {
        anyhow::bail!("套餐 ID 不能超过 64 个字符。");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        anyhow::bail!("套餐 ID 只能包含英文字母、数字、下划线、短横线或点。");
    }
    Ok(value.to_string())
}

fn normalize_plan_name(input: &str) -> anyhow::Result<String> {
    let value = input.trim();
    if value.is_empty() {
        anyhow::bail!("套餐名称不能为空。");
    }
    if value.chars().count() > 40 {
        anyhow::bail!("套餐名称不能超过 40 个字符。");
    }
    Ok(value.to_string())
}

fn normalize_daily_token_limit(value: i64) -> anyhow::Result<i64> {
    if value < 0 {
        anyhow::bail!("每日额度不能小于 0。");
    }
    Ok(value.min(1_000_000_000))
}

fn mask_phone(phone: &str) -> String {
    let local = phone
        .strip_prefix("+86")
        .or_else(|| phone.strip_prefix("86"))
        .unwrap_or(phone);
    if local.len() == 11 {
        format!("+86 {}****{}", &local[..3], &local[7..])
    } else {
        phone.to_string()
    }
}

fn generate_sms_code() -> String {
    let value = Uuid::new_v4().as_u128() % 1_000_000;
    format!("{value:06}")
}

fn hash_sms_code(phone: &str, code: &str) -> String {
    sha256_hex(format!("{phone}:{code}:jiyi-codex-local-auth").as_bytes())
}

fn phone_export_hash(phone: &str) -> String {
    sha256_hex(format!("{phone}:jiyi-codex-phone-export-v1").as_bytes())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn session_ttl_hours() -> i64 {
    std::env::var("JIYI_CODEX_SESSION_TTL_HOURS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(DEFAULT_SESSION_TTL_HOURS)
        .clamp(1, 24 * 365)
}

fn ensure_column(
    db: &Connection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> anyhow::Result<()> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>(1))?;
    for existing in columns {
        if existing? == column {
            return Ok(());
        }
    }
    db.execute(alter_sql, [])?;
    Ok(())
}

fn load_or_create_device_id(db: &Connection) -> anyhow::Result<String> {
    let current = db
        .query_row(
            "SELECT value FROM local_device WHERE key = 'device_id'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if let Some(device_id) = current.filter(|value| !value.trim().is_empty()) {
        return Ok(device_id);
    }
    let device_id = format!("jiyi-device-{}", Uuid::new_v4());
    db.execute(
        "INSERT OR REPLACE INTO local_device (key, value) VALUES ('device_id', ?1)",
        params![device_id],
    )?;
    Ok(device_id)
}

fn bind_local_user_device(
    db: &Connection,
    user_id: &str,
    device_id: &str,
    now: i64,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO local_user_devices (user_id, device_id, first_seen_at_ms, last_seen_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(user_id, device_id) DO UPDATE SET last_seen_at_ms = excluded.last_seen_at_ms",
        params![user_id, device_id, now, now],
    )?;
    Ok(())
}

fn load_or_create_entitlement(
    db: &Connection,
    user_id: &str,
    now: i64,
) -> anyhow::Result<LocalEntitlementState> {
    let existing = db
        .query_row(
            "SELECT plan_id, plan_name, daily_token_limit, updated_at_ms
             FROM local_entitlements
             WHERE user_id = ?1",
            params![user_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((plan_id, plan_name, daily_token_limit, updated_at_ms)) = existing {
        return Ok(LocalEntitlementState {
            user_id: Some(user_id.to_string()),
            plan_id,
            plan_name,
            daily_token_limit: daily_token_limit.max(0),
            source: "local_entitlement".to_string(),
            updated_at_ms: Some(updated_at_ms),
        });
    }

    let plan_id = env_or("JIYI_CODEX_LOCAL_PLAN_ID", DEFAULT_PLAN_ID);
    let plan_name = env_or("JIYI_CODEX_LOCAL_PLAN_NAME", DEFAULT_PLAN_NAME);
    let daily_token_limit = default_entitlement_daily_token_limit();
    db.execute(
        "INSERT INTO local_entitlements (user_id, plan_id, plan_name, daily_token_limit, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![user_id, plan_id, plan_name, daily_token_limit, now],
    )?;
    Ok(LocalEntitlementState {
        user_id: Some(user_id.to_string()),
        plan_id,
        plan_name,
        daily_token_limit,
        source: "local_entitlement_default".to_string(),
        updated_at_ms: Some(now),
    })
}

fn default_entitlement_daily_token_limit() -> i64 {
    std::env::var("JIYI_CODEX_LOCAL_DAILY_TOKEN_LIMIT")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0)
}

impl SmsProviderConfig {
    fn from_env() -> Self {
        let settings = read_sms_provider_settings().unwrap_or_default();
        Self::from_settings(&settings)
    }

    fn from_settings(settings: &SmsProviderSettings) -> Self {
        let settings = normalize_sms_provider_settings(settings.clone());
        let region = env_or_non_empty("TENCENT_SMS_REGION", &settings.region);
        let (secret_id, secret_id_source) = sms_secret_from_env_or_keychain(
            "TENCENT_SMS_SECRET_ID",
            crate::secret_store::tencent_sms_secret_id_account(),
        );
        let (secret_key, secret_key_source) = sms_secret_from_env_or_keychain(
            "TENCENT_SMS_SECRET_KEY",
            crate::secret_store::tencent_sms_secret_key_account(),
        );
        let app_id = env_or_non_empty("TENCENT_SMS_APP_ID", &settings.app_id);
        let sign_name = env_or_non_empty("TENCENT_SMS_SIGN_NAME", &settings.sign_name);
        let template_id = env_or_non_empty("TENCENT_SMS_TEMPLATE_ID", &settings.template_id);
        let ttl_minutes = std::env::var("SMS_CODE_TTL_MINUTES")
            .ok()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(settings.ttl_minutes)
            .clamp(1, 60);
        let template_param_mode = normalize_sms_template_param_mode(&env_or_non_empty(
            "TENCENT_SMS_TEMPLATE_PARAM_MODE",
            &settings.template_param_mode,
        ));
        let configured = !secret_id.trim().is_empty()
            && !secret_key.trim().is_empty()
            && !app_id.trim().is_empty()
            && !sign_name.trim().is_empty()
            && !template_id.trim().is_empty();
        let dry_run = env_truthy("JIYI_CODEX_SMS_DRY_RUN") || settings.dry_run || !configured;
        Self {
            region,
            secret_id,
            secret_key,
            secret_id_source,
            secret_key_source,
            app_id,
            sign_name,
            template_id,
            configured,
            dry_run,
            ttl_minutes,
            template_param_mode,
        }
    }

    fn state(&self) -> SmsConfigState {
        SmsConfigState {
            configured: self.configured,
            dry_run: self.dry_run,
            region: self.region.clone(),
            secret_id_set: !self.secret_id.trim().is_empty(),
            secret_key_set: !self.secret_key.trim().is_empty(),
            secret_id_source: self.secret_id_source.clone(),
            secret_key_source: self.secret_key_source.clone(),
            app_id_set: !self.app_id.trim().is_empty(),
            sign_name_set: !self.sign_name.trim().is_empty(),
            template_id_set: !self.template_id.trim().is_empty(),
            ttl_minutes: self.ttl_minutes,
            template_param_mode: self.template_param_mode.clone(),
        }
    }
}

fn sms_secret_from_env_or_keychain(env_name: &str, keychain_account: &str) -> (String, String) {
    if let Ok(raw) = std::env::var(env_name) {
        let raw = raw.trim().to_string();
        if !raw.is_empty() {
            let resolved = crate::secret_store::resolve_secret_value(&raw);
            let source = if crate::secret_store::is_keychain_ref(&raw) {
                "env_keychain_ref"
            } else {
                "env_plaintext"
            };
            return (resolved, source.to_string());
        }
    }
    let reference = crate::secret_store::keychain_ref(keychain_account);
    let resolved = crate::secret_store::resolve_secret_value(&reference);
    if resolved.trim().is_empty() {
        (String::new(), "missing".to_string())
    } else {
        (resolved, "default_keychain".to_string())
    }
}

#[derive(Debug, Clone)]
struct SmsSendResult {
    request_id: Option<String>,
}

async fn send_sms_code(
    config: &SmsProviderConfig,
    phone: &str,
    code: &str,
) -> anyhow::Result<SmsSendResult> {
    if config.dry_run {
        return Ok(SmsSendResult {
            request_id: Some(format!("dry-run-{}", Uuid::new_v4())),
        });
    }

    let timestamp = now_seconds();
    let date = utc_date(timestamp)?;
    let params = sms_template_params(config, code);
    let payload = json!({
        "PhoneNumberSet": [phone],
        "SmsSdkAppId": config.app_id,
        "SignName": config.sign_name,
        "TemplateId": config.template_id,
        "TemplateParamSet": params,
    });
    let body = serde_json::to_string(&payload)?;
    let authorization = tencent_cloud_authorization(
        &config.secret_id,
        &config.secret_key,
        timestamp,
        &date,
        &body,
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    headers.insert("Host", HeaderValue::from_static(SMS_HOST));
    headers.insert("X-TC-Action", HeaderValue::from_static(SMS_ACTION));
    headers.insert(
        "X-TC-Timestamp",
        HeaderValue::from_str(&timestamp.to_string())?,
    );
    headers.insert("X-TC-Version", HeaderValue::from_static(SMS_VERSION));
    headers.insert("X-TC-Region", HeaderValue::from_str(&config.region)?);
    headers.insert("Authorization", HeaderValue::from_str(&authorization)?);

    let response = reqwest::Client::new()
        .post(SMS_ENDPOINT)
        .headers(headers)
        .body(body)
        .send()
        .await
        .context("调用腾讯云短信 API 失败")?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("腾讯云短信 API 返回 HTTP {status}：{text}");
    }
    let parsed: TencentSmsResponse =
        serde_json::from_str(&text).context("解析腾讯云短信响应失败")?;
    validate_tencent_sms_response(&parsed)?;
    Ok(SmsSendResult {
        request_id: parsed.response.request_id,
    })
}

fn sms_template_params(config: &SmsProviderConfig, code: &str) -> Vec<String> {
    match config.template_param_mode.as_str() {
        "code" => vec![code.to_string()],
        "ttl_code" => vec![config.ttl_minutes.to_string(), code.to_string()],
        _ => vec![code.to_string(), config.ttl_minutes.to_string()],
    }
}

fn validate_tencent_sms_response(parsed: &TencentSmsResponse) -> anyhow::Result<()> {
    if let Some(error) = &parsed.response.error {
        anyhow::bail!("腾讯云短信发送失败：{} {}", error.code, error.message);
    }
    let statuses = parsed
        .response
        .send_status_set
        .as_ref()
        .filter(|statuses| !statuses.is_empty())
        .ok_or_else(|| anyhow::anyhow!("腾讯云短信响应缺少 SendStatusSet。"))?;
    if let Some(failed) = statuses
        .iter()
        .find(|item| item.code.as_deref().unwrap_or_default() != "Ok")
    {
        anyhow::bail!(
            "腾讯云短信发送失败：{} {} {}",
            failed.code.as_deref().unwrap_or("Unknown"),
            failed.message.as_deref().unwrap_or(""),
            failed.phone_number.as_deref().unwrap_or("")
        );
    }
    Ok(())
}

fn tencent_cloud_authorization(
    secret_id: &str,
    secret_key: &str,
    timestamp: i64,
    date: &str,
    payload: &str,
) -> anyhow::Result<String> {
    let canonical_headers =
        format!("content-type:application/json; charset=utf-8\nhost:{SMS_HOST}\n");
    let signed_headers = "content-type;host";
    let hashed_payload = sha256_hex(payload.as_bytes());
    let canonical_request =
        format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{hashed_payload}");
    let credential_scope = format!("{date}/{SMS_SERVICE}/tc3_request");
    let string_to_sign = format!(
        "TC3-HMAC-SHA256\n{timestamp}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let secret_date = hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes())?;
    let secret_service = hmac_sha256(&secret_date, SMS_SERVICE.as_bytes())?;
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request")?;
    let signature = hex_lower(&hmac_sha256(&secret_signing, string_to_sign.as_bytes())?);
    Ok(format!(
        "TC3-HMAC-SHA256 Credential={secret_id}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}"
    ))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(message);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_lower(&hasher.finalize())
}

fn hex_lower(bytes: &[u8]) -> String {
    const CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(CHARS[(byte >> 4) as usize] as char);
        out.push(CHARS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn utc_date(timestamp: i64) -> anyhow::Result<String> {
    let datetime = OffsetDateTime::from_unix_timestamp(timestamp)?;
    Ok(format!(
        "{:04}-{:02}-{:02}",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day()
    ))
}

fn default_sms_region() -> String {
    DEFAULT_SMS_REGION.to_string()
}

fn default_sms_ttl_minutes() -> i64 {
    DEFAULT_SMS_TTL_MINUTES
}

fn default_sms_template_param_mode() -> String {
    DEFAULT_SMS_TEMPLATE_PARAM_MODE.to_string()
}

fn default_sms_dry_run() -> bool {
    true
}

fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

fn env_or_non_empty(name: &str, fallback: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.trim().to_string()
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_mainland_phone() {
        let phone = normalize_phone("138 1234 5678").unwrap();
        assert_eq!(phone.e164, "+8613812345678");
        assert_eq!(mask_phone(&phone.e164), "+86 138****5678");
    }

    #[test]
    fn rejects_invalid_phone() {
        assert!(normalize_phone("12345").is_err());
    }

    #[test]
    fn code_hash_is_stable() {
        assert_eq!(
            hash_sms_code("+8613812345678", "123456"),
            hash_sms_code("+8613812345678", "123456")
        );
        assert_ne!(
            hash_sms_code("+8613812345678", "123456"),
            hash_sms_code("+8613812345678", "654321")
        );
    }

    #[test]
    fn sms_template_params_follow_configured_mode() {
        let mut config = SmsProviderConfig {
            region: "ap-guangzhou".to_string(),
            secret_id: "secret-id".to_string(),
            secret_key: "secret-key".to_string(),
            secret_id_source: "env_plaintext".to_string(),
            secret_key_source: "env_plaintext".to_string(),
            app_id: "1400000000".to_string(),
            sign_name: "极义".to_string(),
            template_id: "2603280".to_string(),
            configured: true,
            dry_run: false,
            ttl_minutes: 10,
            template_param_mode: "code_ttl".to_string(),
        };

        assert_eq!(sms_template_params(&config, "123456"), vec!["123456", "10"]);
        config.template_param_mode = "code".to_string();
        assert_eq!(sms_template_params(&config, "123456"), vec!["123456"]);
        config.template_param_mode = "ttl_code".to_string();
        assert_eq!(sms_template_params(&config, "123456"), vec!["10", "123456"]);
    }

    #[test]
    fn sms_provider_settings_default_to_dry_run() {
        let config = SmsProviderConfig::from_settings(&SmsProviderSettings {
            region: "ap-guangzhou".to_string(),
            app_id: "1400000000".to_string(),
            sign_name: "极义".to_string(),
            template_id: "123456".to_string(),
            ttl_minutes: 10,
            template_param_mode: "code_ttl".to_string(),
            dry_run: true,
        });

        assert!(config.dry_run);
    }

    #[test]
    fn sms_provider_settings_normalize_template_mode_and_ttl() {
        let settings = normalize_sms_provider_settings(SmsProviderSettings {
            region: "".to_string(),
            app_id: " 1400000000 ".to_string(),
            sign_name: " 极义 ".to_string(),
            template_id: " 123456 ".to_string(),
            ttl_minutes: 120,
            template_param_mode: "unknown".to_string(),
            dry_run: false,
        });

        assert_eq!(settings.region, DEFAULT_SMS_REGION);
        assert_eq!(settings.app_id, "1400000000");
        assert_eq!(settings.sign_name, "极义");
        assert_eq!(settings.template_id, "123456");
        assert_eq!(settings.ttl_minutes, 60);
        assert_eq!(
            settings.template_param_mode,
            DEFAULT_SMS_TEMPLATE_PARAM_MODE
        );
    }

    #[test]
    fn tencent_sms_response_requires_ok_status() {
        let ok = TencentSmsResponse {
            response: TencentSmsResponseBody {
                request_id: Some("request-1".to_string()),
                error: None,
                send_status_set: Some(vec![TencentSmsStatus {
                    code: Some("Ok".to_string()),
                    message: Some("send success".to_string()),
                    phone_number: Some("+8613812345678".to_string()),
                }]),
            },
        };
        validate_tencent_sms_response(&ok).unwrap();

        let missing_status = TencentSmsResponse {
            response: TencentSmsResponseBody {
                request_id: Some("request-2".to_string()),
                error: None,
                send_status_set: None,
            },
        };
        assert!(
            validate_tencent_sms_response(&missing_status)
                .unwrap_err()
                .to_string()
                .contains("SendStatusSet")
        );

        let failed_status = TencentSmsResponse {
            response: TencentSmsResponseBody {
                request_id: Some("request-3".to_string()),
                error: None,
                send_status_set: Some(vec![TencentSmsStatus {
                    code: Some("LimitExceeded.PhoneNumberDailyLimit".to_string()),
                    message: Some("daily limit exceeded".to_string()),
                    phone_number: Some("+8613812345678".to_string()),
                }]),
            },
        };
        assert!(
            validate_tencent_sms_response(&failed_status)
                .unwrap_err()
                .to_string()
                .contains("LimitExceeded.PhoneNumberDailyLimit")
        );
    }

    #[test]
    fn expired_local_session_is_cleared_on_load() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalAccountStore::new(temp.path().join("auth.sqlite"));
        store.ensure_schema().unwrap();
        let db = Connection::open(temp.path().join("auth.sqlite")).unwrap();
        db.execute(
            "INSERT OR REPLACE INTO auth_state (id, user_id, phone, session_token, login_at_ms, session_expires_at_ms, device_id)
             VALUES (1, 'user-1', '+8613812345678', 'token-1', ?1, ?2, 'device-1')",
            params![now_ms() - 10_000, now_ms() - 1_000],
        )
        .unwrap();

        let state = store.load_auth_state().unwrap();

        assert!(!state.authenticated);
        assert!(state.session_expired);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM auth_state", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn hard_reset_removes_auth_database_files() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("auth.sqlite");
        let store = LocalAccountStore::new(base.clone());
        store.ensure_schema().unwrap();

        let sidecar_shm = PathBuf::from(format!("{}-shm", base.to_string_lossy()));
        let sidecar_wal = PathBuf::from(format!("{}-wal", base.to_string_lossy()));
        std::fs::write(&sidecar_shm, b"tmp").unwrap();
        std::fs::write(&sidecar_wal, b"tmp").unwrap();

        assert!(base.exists());
        assert!(sidecar_shm.exists());
        assert!(sidecar_wal.exists());

        store.hard_reset().unwrap();

        assert!(!base.exists());
        assert!(!sidecar_shm.exists());
        assert!(!sidecar_wal.exists());
    }

    #[test]
    fn device_id_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        let db = Connection::open(temp.path().join("auth.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE local_device (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();

        let first = load_or_create_device_id(&db).unwrap();
        let second = load_or_create_device_id(&db).unwrap();

        assert_eq!(first, second);
        assert!(first.starts_with("jiyi-device-"));
    }

    #[test]
    fn login_creates_user_device_and_default_entitlement() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalAccountStore::new(temp.path().join("auth.sqlite"));
        store.ensure_schema().unwrap();
        let phone = "+8613812345678";
        let db = Connection::open(temp.path().join("auth.sqlite")).unwrap();
        db.execute(
            "INSERT INTO sms_codes (id, phone, code_hash, created_at_ms, expires_at_ms, consumed_at_ms, dry_run, request_id)
             VALUES ('code-1', ?1, ?2, ?3, ?4, NULL, 1, 'test')",
            params![
                phone,
                hash_sms_code(phone, "123456"),
                now_ms(),
                now_ms() + 60_000
            ],
        )
        .unwrap();

        let session = store.login_with_sms_code(phone, "123456").unwrap();
        let state = store.load_auth_state().unwrap();

        assert_eq!(session.entitlement.plan_id, DEFAULT_PLAN_ID);
        assert_eq!(state.entitlement.plan_name, DEFAULT_PLAN_NAME);
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM local_user_devices", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM local_entitlements", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn update_active_entitlement_updates_current_local_user() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalAccountStore::new(temp.path().join("auth.sqlite"));
        store.ensure_schema().unwrap();
        let phone = "+8613812345678";
        let db = Connection::open(temp.path().join("auth.sqlite")).unwrap();
        db.execute(
            "INSERT INTO sms_codes (id, phone, code_hash, created_at_ms, expires_at_ms, consumed_at_ms, dry_run, request_id)
             VALUES ('code-1', ?1, ?2, ?3, ?4, NULL, 1, 'test')",
            params![
                phone,
                hash_sms_code(phone, "123456"),
                now_ms(),
                now_ms() + 60_000
            ],
        )
        .unwrap();
        let session = store.login_with_sms_code(phone, "123456").unwrap();

        let entitlement = store
            .update_active_entitlement("team_basic", "团队基础版", 100_000)
            .unwrap();
        let state = store.load_auth_state().unwrap();

        assert_eq!(
            entitlement.user_id.as_deref(),
            Some(session.user_id.as_str())
        );
        assert_eq!(entitlement.plan_id, "team_basic");
        assert_eq!(state.entitlement.plan_name, "团队基础版");
        assert_eq!(state.entitlement.daily_token_limit, 100_000);
        assert_eq!(
            db.query_row(
                "SELECT daily_token_limit FROM local_entitlements WHERE user_id = ?1",
                params![session.user_id],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            100_000
        );
    }

    #[test]
    fn update_active_entitlement_requires_login() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalAccountStore::new(temp.path().join("auth.sqlite"));

        let error = store
            .update_active_entitlement("team_basic", "团队基础版", 100_000)
            .unwrap_err();

        assert!(error.to_string().contains("请先完成手机号验证码登录"));
    }

    #[test]
    fn export_state_redacts_phone_and_includes_account_records() {
        let temp = tempfile::tempdir().unwrap();
        let store = LocalAccountStore::new(temp.path().join("auth.sqlite"));
        store.ensure_schema().unwrap();
        let phone = "+8613812345678";
        let db = Connection::open(temp.path().join("auth.sqlite")).unwrap();
        db.execute(
            "INSERT INTO sms_codes (id, phone, code_hash, created_at_ms, expires_at_ms, consumed_at_ms, dry_run, request_id)
             VALUES ('code-1', ?1, ?2, ?3, ?4, NULL, 1, 'test')",
            params![
                phone,
                hash_sms_code(phone, "123456"),
                now_ms(),
                now_ms() + 60_000
            ],
        )
        .unwrap();
        let session = store.login_with_sms_code(phone, "123456").unwrap();
        store
            .update_active_entitlement("team_basic", "团队基础版", 100_000)
            .unwrap();

        let export = store.export_state().unwrap();
        let serialized = serde_json::to_string(&export).unwrap();

        assert_eq!(export.users.len(), 1);
        assert_eq!(export.devices.len(), 1);
        assert_eq!(export.entitlements.len(), 1);
        assert_eq!(
            export
                .active_session
                .as_ref()
                .map(|value| value.user_id.as_str()),
            Some(session.user_id.as_str())
        );
        assert_eq!(export.users[0].phone_masked, "+86 138****5678");
        assert_ne!(export.users[0].phone_hash, phone);
        assert!(!serialized.contains(phone));
        assert!(serialized.contains("团队基础版"));
    }
}
