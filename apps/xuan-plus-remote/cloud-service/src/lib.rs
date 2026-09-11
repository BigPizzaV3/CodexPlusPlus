use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod pairing;
pub use pairing::*;
mod command_sync;
pub use command_sync::*;
mod live_reply;
pub use live_reply::*;
mod task_sync;
pub use task_sync::*;
mod push;
pub use push::*;

pub const CONTRACT_VERSION: &str = "1.5";
pub const STORAGE_SCHEMA_VERSION: i64 = 7;
const CHALLENGE_TTL_MINUTES: i64 = 5;
pub(crate) const REQUEST_CLOCK_SKEW_SECONDS: i64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Dev,
    Prod,
}

impl Environment {
    pub fn parse(value: &str) -> Result<Self, CloudError> {
        match value {
            "dev" => Ok(Self::Dev),
            "prod" => Ok(Self::Prod),
            _ => Err(CloudError::EnvironmentMismatch),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Prod => "prod",
        }
    }
}

#[derive(Debug, Error)]
pub enum CloudError {
    #[error("invalid request")]
    InvalidRequest,
    #[error("environment mismatch")]
    EnvironmentMismatch,
    #[error("device key is invalid")]
    DeviceKeyInvalid,
    #[error("device proof is invalid")]
    DeviceProofInvalid,
    #[error("device challenge expired")]
    DeviceChallengeExpired,
    #[error("device authentication failed")]
    DeviceAuthenticationFailed,
    #[error("device request replayed")]
    DeviceRequestReplayed,
    #[error("pairing expired")]
    PairingExpired,
    #[error("pairing replayed")]
    PairingReplayed,
    #[error("pairing QR invalid")]
    PairingQrInvalid,
    #[error("pairing confirmation expired")]
    PairingConfirmationExpired,
    #[error("pairing summary mismatch")]
    PairingSummaryMismatch,
    #[error("device limit reached")]
    DeviceLimitReached,
    #[error("device is not bound")]
    DeviceNotBound,
    #[error("PC is offline")]
    PcOffline,
    #[error("task state conflicts with the command")]
    StateConflict,
    #[error("command payload digest conflicts with an existing request")]
    PayloadDigestConflict,
    #[error("command expired")]
    CommandExpired,
    #[error("command operation is unsupported")]
    UnsupportedOperation,
    #[error("command is not allowed in the current task state")]
    InvalidCommandState,
    #[error("record was not found")]
    NotFound,
    #[error("storage unavailable")]
    StorageUnavailable,
}

impl CloudError {
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::EnvironmentMismatch => "environment_mismatch",
            Self::DeviceKeyInvalid => "device_key_invalid",
            Self::DeviceProofInvalid => "device_proof_invalid",
            Self::DeviceChallengeExpired => "device_challenge_expired",
            Self::DeviceAuthenticationFailed => "device_authentication_failed",
            Self::DeviceRequestReplayed => "device_request_replayed",
            Self::PairingExpired => "pairing_expired",
            Self::PairingReplayed => "pairing_replayed",
            Self::PairingQrInvalid => "invalid_pairing_qr",
            Self::PairingConfirmationExpired => "pairing_confirmation_expired",
            Self::PairingSummaryMismatch => "pairing_summary_mismatch",
            Self::DeviceLimitReached => "device_limit_reached",
            Self::DeviceNotBound => "device_not_bound",
            Self::PcOffline => "pc_offline",
            Self::StateConflict => "state_conflict",
            Self::PayloadDigestConflict => "payload_digest_conflict",
            Self::CommandExpired => "command_expired",
            Self::UnsupportedOperation => "unsupported_operation",
            Self::InvalidCommandState => "invalid_command_state",
            Self::NotFound => "not_found",
            Self::StorageUnavailable => "storage_unavailable",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(self, Self::StorageUnavailable | Self::PcOffline)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceChallengeRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub device_key_algorithm: String,
    pub device_public_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceChallengeResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub app_device_id: String,
    pub challenge_id: String,
    pub challenge: String,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceRegistrationRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub device_key_algorithm: String,
    pub device_public_key: String,
    pub challenge_id: String,
    pub challenge: String,
    pub registration_signature: String,
    pub push_provider: String,
    pub push_token: String,
    pub app_display_name: String,
    pub app_version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistrationResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub app_device_id: String,
    pub device_key_id: String,
    pub registration_state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_message_id: Option<String>,
    pub error_code: &'static str,
    pub retryable: bool,
    pub message: &'static str,
    pub server_received_at: String,
}

#[derive(Clone)]
pub struct CloudService {
    environment: Environment,
    connection: Arc<Mutex<Connection>>,
    token_cipher: Arc<Aes256Gcm>,
    live_reply_broker: LiveReplyBroker,
}

impl CloudService {
    pub fn open(
        database_path: &Path,
        environment: Environment,
        token_encryption_key: [u8; 32],
    ) -> Result<Self, CloudError> {
        let connection =
            Connection::open(database_path).map_err(|_| CloudError::StorageUnavailable)?;
        initialize_schema(&connection)?;
        let token_cipher = Aes256Gcm::new_from_slice(&token_encryption_key)
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(Self {
            environment,
            connection: Arc::new(Mutex::new(connection)),
            token_cipher: Arc::new(token_cipher),
            live_reply_broker: LiveReplyBroker::new(environment),
        })
    }

    pub fn environment(&self) -> Environment {
        self.environment
    }

    pub fn live_reply_broker(&self) -> LiveReplyBroker {
        self.live_reply_broker.clone()
    }

    pub fn create_device_challenge(
        &self,
        request: DeviceChallengeRequest,
        now: DateTime<Utc>,
    ) -> Result<DeviceChallengeResponse, CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/device-registration-challenge",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        if request.device_key_algorithm != "ed25519" {
            return Err(CloudError::DeviceKeyInvalid);
        }
        let public_key_der = decode_public_key(&request.device_public_key)?;
        VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| CloudError::DeviceKeyInvalid)?;

        let challenge_id = random_opaque_id();
        let challenge = random_opaque_id();
        let expires_at = now + Duration::minutes(CHALLENGE_TTL_MINUTES);
        let public_key_digest = sha256_hex(&public_key_der);
        let challenge_digest = sha256_hex(challenge.as_bytes());
        self.connection()?
            .execute(
                "INSERT INTO app_device_challenges (
                   challenge_id, environment, app_device_id, public_key_digest,
                   challenge_digest, expires_at, consumed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)",
                params![
                    challenge_id,
                    self.environment.as_str(),
                    request.app_device_id,
                    public_key_digest,
                    challenge_digest,
                    expires_at.timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;

        Ok(DeviceChallengeResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/device-registration-challenged",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            app_device_id: request.app_device_id,
            challenge_id,
            challenge,
            expires_at: format_timestamp(expires_at),
        })
    }

    pub fn register_device(
        &self,
        request: DeviceRegistrationRequest,
        now: DateTime<Utc>,
    ) -> Result<DeviceRegistrationResponse, CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/device-register",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        validate_opaque_id(&request.challenge_id)?;
        validate_opaque_id(&request.challenge)?;
        if request.device_key_algorithm != "ed25519"
            || request.push_provider != "huawei_push_kit"
            || request.push_token.is_empty()
            || request.push_token.len() > 4096
            || request.app_display_name.is_empty()
            || request.app_display_name.len() > 128
            || request.app_version.is_empty()
            || request.app_version.len() > 64
        {
            return Err(CloudError::InvalidRequest);
        }

        let public_key_der = decode_public_key(&request.device_public_key)?;
        let verifying_key = VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| CloudError::DeviceKeyInvalid)?;
        let public_key_digest = sha256_hex(&public_key_der);
        let challenge_digest = sha256_hex(request.challenge.as_bytes());
        let signature_bytes = URL_SAFE_NO_PAD
            .decode(&request.registration_signature)
            .map_err(|_| CloudError::DeviceProofInvalid)?;
        let signature =
            Signature::from_slice(&signature_bytes).map_err(|_| CloudError::DeviceProofInvalid)?;
        let proof = registration_proof(
            self.environment,
            &request.app_device_id,
            &request.challenge_id,
            &request.challenge,
            &public_key_digest,
        );
        verifying_key
            .verify(proof.as_bytes(), &signature)
            .map_err(|_| CloudError::DeviceProofInvalid)?;

        let encrypted_push_token = self.encrypt_push_token(request.push_token.as_bytes())?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let challenge_row = transaction
            .query_row(
                "SELECT app_device_id, public_key_digest, challenge_digest, expires_at, consumed_at
                 FROM app_device_challenges
                 WHERE challenge_id = ?1 AND environment = ?2",
                params![request.challenge_id, self.environment.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::DeviceChallengeExpired)?;
        if challenge_row.0 != request.app_device_id
            || challenge_row.1 != public_key_digest
            || challenge_row.2 != challenge_digest
        {
            return Err(CloudError::DeviceProofInvalid);
        }
        if challenge_row.4.is_some() || challenge_row.3 < now.timestamp() {
            return Err(CloudError::DeviceChallengeExpired);
        }

        let existing_device = transaction
            .query_row(
                "SELECT device_key_id, public_key_digest FROM app_devices
                 WHERE environment = ?1 AND app_device_id = ?2",
                params![self.environment.as_str(), request.app_device_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let device_key_id = match existing_device {
            Some((device_key_id, existing_digest)) if existing_digest == public_key_digest => {
                device_key_id
            }
            Some(_) => return Err(CloudError::DeviceKeyInvalid),
            None => random_opaque_id(),
        };

        persist_registered_device(
            &transaction,
            self.environment,
            DevicePersistence {
                request: &request,
                device_key_id: &device_key_id,
                public_key_der: &public_key_der,
                public_key_digest: &public_key_digest,
                encrypted_push_token: &encrypted_push_token,
                now,
            },
        )?;
        let consumed = transaction
            .execute(
                "UPDATE app_device_challenges SET consumed_at = ?1
                 WHERE challenge_id = ?2 AND consumed_at IS NULL",
                params![now.timestamp(), request.challenge_id],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if consumed != 1 {
            return Err(CloudError::DeviceChallengeExpired);
        }
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;

        Ok(DeviceRegistrationResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/device-registered",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            app_device_id: request.app_device_id,
            device_key_id,
            registration_state: "registered",
        })
    }

    fn encrypt_push_token(&self, token: &[u8]) -> Result<Vec<u8>, CloudError> {
        let mut nonce_bytes = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let ciphertext = self
            .token_cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), token)
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut encrypted = nonce_bytes.to_vec();
        encrypted.extend_from_slice(&ciphertext);
        Ok(encrypted)
    }

    pub(crate) fn decrypt_push_token(&self, encrypted: &[u8]) -> Result<Vec<u8>, CloudError> {
        if encrypted.len() <= 12 {
            return Err(CloudError::StorageUnavailable);
        }
        self.token_cipher
            .decrypt(Nonce::from_slice(&encrypted[..12]), &encrypted[12..])
            .map_err(|_| CloudError::StorageUnavailable)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, CloudError> {
        self.connection
            .lock()
            .map_err(|_| CloudError::StorageUnavailable)
    }
}

pub fn registration_proof(
    environment: Environment,
    app_device_id: &str,
    challenge_id: &str,
    challenge: &str,
    public_key_digest: &str,
) -> String {
    format!(
        "workagents-device-registration-v1\n{}\n{}\n{}\n{}\n{}",
        environment.as_str(),
        app_device_id,
        challenge_id,
        challenge,
        public_key_digest
    )
}

pub fn decode_token_encryption_key(value: &str) -> Result<[u8; 32], CloudError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| CloudError::InvalidRequest)?;
    bytes.try_into().map_err(|_| CloudError::InvalidRequest)
}

struct DevicePersistence<'a> {
    request: &'a DeviceRegistrationRequest,
    device_key_id: &'a str,
    public_key_der: &'a [u8],
    public_key_digest: &'a str,
    encrypted_push_token: &'a [u8],
    now: DateTime<Utc>,
}

fn persist_registered_device(
    transaction: &Transaction<'_>,
    environment: Environment,
    persistence: DevicePersistence<'_>,
) -> Result<(), CloudError> {
    transaction
        .execute(
            "INSERT INTO app_devices (
               environment, app_device_id, device_key_id, public_key_der, public_key_digest,
               push_token_encrypted, push_token_generation, app_display_name, app_version,
               status, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, 'active', ?9)
             ON CONFLICT(environment, app_device_id) DO UPDATE SET
               push_token_encrypted = excluded.push_token_encrypted,
               push_token_generation = app_devices.push_token_generation + 1,
               app_display_name = excluded.app_display_name,
               app_version = excluded.app_version,
               status = 'active',
               updated_at = excluded.updated_at",
            params![
                environment.as_str(),
                persistence.request.app_device_id,
                persistence.device_key_id,
                persistence.public_key_der,
                persistence.public_key_digest,
                persistence.encrypted_push_token,
                persistence.request.app_display_name,
                persistence.request.app_version,
                persistence.now.timestamp()
            ],
        )
        .map_err(|_| CloudError::StorageUnavailable)?;
    Ok(())
}

fn initialize_schema(connection: &Connection) -> Result<(), CloudError> {
    let schema_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| CloudError::StorageUnavailable)?;
    if schema_version != 0 && schema_version != STORAGE_SCHEMA_VERSION {
        return Err(CloudError::StorageUnavailable);
    }
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS app_device_challenges (
               challenge_id TEXT PRIMARY KEY,
               environment TEXT NOT NULL,
               app_device_id TEXT NOT NULL,
               public_key_digest TEXT NOT NULL,
               challenge_digest TEXT NOT NULL,
               expires_at INTEGER NOT NULL,
               consumed_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS app_devices (
               environment TEXT NOT NULL,
               app_device_id TEXT NOT NULL,
               device_key_id TEXT NOT NULL,
               public_key_der BLOB NOT NULL,
               public_key_digest TEXT NOT NULL,
               push_token_encrypted BLOB NOT NULL,
               push_token_generation INTEGER NOT NULL DEFAULT 1,
               app_display_name TEXT NOT NULL,
               app_version TEXT NOT NULL,
               status TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (environment, app_device_id),
               UNIQUE (environment, device_key_id)
             );
             CREATE TABLE IF NOT EXISTS device_signature_nonces (
               environment TEXT NOT NULL,
               device_key_id TEXT NOT NULL,
               nonce_digest TEXT NOT NULL,
               request_timestamp INTEGER NOT NULL,
               expires_at INTEGER NOT NULL,
               PRIMARY KEY (environment, device_key_id, nonce_digest)
             );
             CREATE TABLE IF NOT EXISTS pc_devices (
               environment TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               device_key_id TEXT NOT NULL,
               public_key_der BLOB NOT NULL,
               public_key_digest TEXT NOT NULL,
               display_name TEXT NOT NULL,
               current_binding_epoch INTEGER NOT NULL DEFAULT 0,
               last_gateway_observed_at INTEGER,
               model_configs_json TEXT NOT NULL DEFAULT '[]',
               status TEXT NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY (environment, pc_device_id, installation_id),
               UNIQUE (environment, device_key_id)
             );
             CREATE TABLE IF NOT EXISTS pc_signature_nonces (
               environment TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               nonce_digest TEXT NOT NULL,
               request_timestamp INTEGER NOT NULL,
               expires_at INTEGER NOT NULL,
               PRIMARY KEY (environment, pc_device_id, installation_id, nonce_digest)
             );
             CREATE TABLE IF NOT EXISTS pairings (
               environment TEXT NOT NULL,
               pairing_handle_digest TEXT PRIMARY KEY,
               pc_pairing_message_id TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               pc_display_name TEXT NOT NULL,
               expires_at INTEGER NOT NULL,
               consumed_at INTEGER,
               consumed_app_device_id TEXT,
               created_at INTEGER NOT NULL,
               UNIQUE (environment, pc_pairing_message_id)
             );
             CREATE TABLE IF NOT EXISTS bindings (
               environment TEXT NOT NULL,
               binding_id TEXT PRIMARY KEY,
               pc_pairing_message_id TEXT NOT NULL,
               app_device_id TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               binding_epoch INTEGER,
               confirmation_nonce_digest TEXT NOT NULL,
               confirmation_nonce_encrypted BLOB NOT NULL,
               confirmation_expires_at INTEGER NOT NULL,
               pc_display_name TEXT NOT NULL,
               app_display_name TEXT NOT NULL,
               safety_phrase TEXT NOT NULL,
               summary_digest TEXT NOT NULL,
               state TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               activated_at INTEGER,
               revoked_at INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_bindings_pending_pc
               ON bindings(environment, pc_device_id, installation_id, state, created_at);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_bindings_active_pair
               ON bindings(environment, app_device_id, pc_device_id)
               WHERE state = 'active';
             CREATE TABLE IF NOT EXISTS remote_task_snapshots (
               environment TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               binding_epoch INTEGER NOT NULL,
               remote_task_id TEXT NOT NULL,
               state_version INTEGER NOT NULL,
               last_event_id TEXT NOT NULL,
               snapshot_json TEXT,
               tombstoned INTEGER NOT NULL DEFAULT 0,
               server_received_at INTEGER NOT NULL,
               PRIMARY KEY (
                 environment, pc_device_id, installation_id, binding_epoch, remote_task_id
               )
             );
             CREATE INDEX IF NOT EXISTS idx_remote_task_snapshots_list
               ON remote_task_snapshots(
                 environment, pc_device_id, installation_id, binding_epoch,
                 tombstoned, state_version DESC
               );
             CREATE TABLE IF NOT EXISTS remote_commands (
               environment TEXT NOT NULL,
               command_id TEXT NOT NULL,
               binding_id TEXT NOT NULL,
               app_device_id TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               binding_epoch INTEGER NOT NULL,
               remote_task_id TEXT NOT NULL,
               client_request_id TEXT NOT NULL,
               command_type TEXT NOT NULL,
               payload_digest TEXT NOT NULL,
               payload_encrypted BLOB,
               expected_state_version INTEGER NOT NULL,
               applied_state_version INTEGER,
               status TEXT NOT NULL,
               error_code TEXT,
               created_at INTEGER NOT NULL,
               expires_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY(environment, command_id),
               UNIQUE(environment, binding_id, app_device_id, remote_task_id, client_request_id)
             );
             CREATE INDEX IF NOT EXISTS idx_remote_commands_dispatch
               ON remote_commands(
                 environment, pc_device_id, installation_id, binding_epoch,
                 status, expires_at, created_at
               );
             CREATE TABLE IF NOT EXISTS push_outbox (
               environment TEXT NOT NULL,
               push_id TEXT NOT NULL,
               refresh_ref TEXT NOT NULL,
               app_device_id TEXT NOT NULL,
               pc_device_id TEXT NOT NULL,
               installation_id TEXT NOT NULL,
               binding_epoch INTEGER NOT NULL,
               remote_task_id TEXT NOT NULL,
               terminal_state_version INTEGER NOT NULL,
               terminal_outcome TEXT NOT NULL
                 CHECK(terminal_outcome IN ('completed', 'failed')),
               push_token_generation INTEGER NOT NULL,
               status TEXT NOT NULL
                 CHECK(status IN ('pending', 'delivering', 'retry', 'sent', 'dead')),
               attempt_count INTEGER NOT NULL DEFAULT 0,
               next_attempt_at INTEGER NOT NULL,
               lease_until INTEGER,
               last_error_code TEXT,
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL,
               sent_at INTEGER,
               PRIMARY KEY(environment, push_id),
               UNIQUE(environment, refresh_ref),
               UNIQUE(
                 environment, app_device_id, pc_device_id, installation_id,
                 binding_epoch, remote_task_id, terminal_state_version
               )
             );
             CREATE INDEX IF NOT EXISTS idx_push_outbox_dispatch
               ON push_outbox(environment, status, next_attempt_at, lease_until, created_at);",
        )
        .map_err(|_| CloudError::StorageUnavailable)?;
    if schema_version == 0 {
        connection
            .execute_batch(&format!("PRAGMA user_version = {STORAGE_SCHEMA_VERSION};"))
            .map_err(|_| CloudError::StorageUnavailable)?;
    }
    Ok(())
}

struct RequestBase<'a> {
    schema_version: &'a str,
    message_type: &'a str,
    expected_message_type: &'a str,
    message_id: &'a str,
    environment: &'a str,
    sent_at: &'a str,
}

fn validate_request_base(
    request: RequestBase<'_>,
    expected_environment: Environment,
    now: DateTime<Utc>,
) -> Result<(), CloudError> {
    validate_request_envelope(
        request.schema_version,
        request.message_type,
        request.expected_message_type,
        request.message_id,
        request.environment,
        expected_environment,
    )?;
    let sent_at = DateTime::parse_from_rfc3339(request.sent_at)
        .map_err(|_| CloudError::InvalidRequest)?
        .with_timezone(&Utc);
    if (now - sent_at).num_seconds().unsigned_abs() > REQUEST_CLOCK_SKEW_SECONDS as u64 {
        return Err(CloudError::InvalidRequest);
    }
    Ok(())
}

fn validate_request_envelope(
    schema_version: &str,
    message_type: &str,
    expected_message_type: &str,
    message_id: &str,
    environment: &str,
    expected_environment: Environment,
) -> Result<(), CloudError> {
    if schema_version != CONTRACT_VERSION || message_type != expected_message_type {
        return Err(CloudError::InvalidRequest);
    }
    validate_opaque_id(message_id)?;
    if Environment::parse(environment)? != expected_environment {
        return Err(CloudError::EnvironmentMismatch);
    }
    Ok(())
}

fn validate_opaque_id(value: &str) -> Result<(), CloudError> {
    if (16..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(CloudError::InvalidRequest)
    }
}

fn decode_public_key(value: &str) -> Result<Vec<u8>, CloudError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| CloudError::DeviceKeyInvalid)?;
    if !(32..=384).contains(&bytes.len()) {
        return Err(CloudError::DeviceKeyInvalid);
    }
    Ok(bytes)
}

fn random_opaque_id() -> String {
    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn format_timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::TempDir;

    fn service(temp: &TempDir) -> CloudService {
        CloudService::open(
            &temp.path().join("remote.sqlite3"),
            Environment::Dev,
            [7_u8; 32],
        )
        .expect("open service")
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-26T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn signing_key() -> SigningKey {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        SigningKey::from_bytes(&bytes)
    }

    fn public_key(signing_key: &SigningKey) -> String {
        let der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("SPKI DER");
        URL_SAFE_NO_PAD.encode(der.as_bytes())
    }

    fn challenge_request(public_key: String) -> DeviceChallengeRequest {
        DeviceChallengeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "app/device-registration-challenge".into(),
            message_id: "challenge_request_0001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: "app_device_000001".into(),
            device_key_algorithm: "ed25519".into(),
            device_public_key: public_key,
        }
    }

    fn registration_request(
        signing_key: &SigningKey,
        public_key: String,
        challenge: &DeviceChallengeResponse,
    ) -> DeviceRegistrationRequest {
        let public_key_der = decode_public_key(&public_key).expect("public key");
        let proof = registration_proof(
            Environment::Dev,
            &challenge.app_device_id,
            &challenge.challenge_id,
            &challenge.challenge,
            &sha256_hex(&public_key_der),
        );
        let signature = signing_key.sign(proof.as_bytes());
        DeviceRegistrationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "app/device-register".into(),
            message_id: "registration_req_001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: challenge.app_device_id.clone(),
            device_key_algorithm: "ed25519".into(),
            device_public_key: public_key,
            challenge_id: challenge.challenge_id.clone(),
            challenge: challenge.challenge.clone(),
            registration_signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
            push_provider: "huawei_push_kit".into(),
            push_token: "isolated-test-push-token".into(),
            app_display_name: "测试手机".into(),
            app_version: "0.1.0-dev".into(),
        }
    }

    #[test]
    fn valid_device_proof_registers_once_and_encrypts_push_token() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let signing_key = signing_key();
        let public_key = public_key(&signing_key);
        let challenge = service
            .create_device_challenge(challenge_request(public_key.clone()), now())
            .expect("challenge");
        let registration = registration_request(&signing_key, public_key, &challenge);
        let response = service
            .register_device(registration, now())
            .expect("registration");
        assert_eq!(response.registration_state, "registered");

        let database_bytes = std::fs::read(temp.path().join("remote.sqlite3")).expect("database");
        assert!(!database_bytes
            .windows(b"isolated-test-push-token".len())
            .any(|window| window == b"isolated-test-push-token"));
    }

    #[test]
    fn challenge_replay_is_rejected_after_success() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let signing_key = signing_key();
        let public_key = public_key(&signing_key);
        let challenge = service
            .create_device_challenge(challenge_request(public_key.clone()), now())
            .expect("challenge");
        let first = registration_request(&signing_key, public_key.clone(), &challenge);
        service
            .register_device(first, now())
            .expect("first registration");
        let replay = registration_request(&signing_key, public_key, &challenge);
        assert!(matches!(
            service.register_device(replay, now()),
            Err(CloudError::DeviceChallengeExpired)
        ));
    }

    #[test]
    fn challenge_digest_survives_service_restart_without_plaintext_storage() {
        let temp = TempDir::new().expect("temp");
        let signing_key = signing_key();
        let public_key = public_key(&signing_key);
        let challenge = service(&temp)
            .create_device_challenge(challenge_request(public_key.clone()), now())
            .expect("challenge");
        let reopened = service(&temp);
        let registration = registration_request(&signing_key, public_key, &challenge);
        reopened
            .register_device(registration, now())
            .expect("registration after restart");
        let database_bytes = std::fs::read(temp.path().join("remote.sqlite3")).expect("database");
        assert!(!database_bytes
            .windows(challenge.challenge.len())
            .any(|window| window == challenge.challenge.as_bytes()));
    }

    #[test]
    fn wrong_key_proof_is_rejected_without_consuming_challenge() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let valid_signing_key = signing_key();
        let wrong_key = signing_key();
        let public_key = public_key(&valid_signing_key);
        let challenge = service
            .create_device_challenge(challenge_request(public_key.clone()), now())
            .expect("challenge");
        let invalid = registration_request(&wrong_key, public_key.clone(), &challenge);
        assert!(matches!(
            service.register_device(invalid, now()),
            Err(CloudError::DeviceProofInvalid)
        ));
        let valid = registration_request(&valid_signing_key, public_key, &challenge);
        service
            .register_device(valid, now())
            .expect("challenge remains usable after invalid proof");
    }
}
