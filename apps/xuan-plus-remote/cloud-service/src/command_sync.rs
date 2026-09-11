use aes_gcm::aead::Aead;
use aes_gcm::Nonce;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::pairing::GatewayEventValidation;
use super::task_sync::TaskSnapshot;
use super::{
    format_timestamp, random_opaque_id, validate_opaque_id, validate_request_base,
    validate_request_envelope, AppRequestAuthentication, CloudError, CloudService, GatewayIdentity,
    RequestBase, CONTRACT_VERSION,
};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const COMMAND_TTL_MINUTES: i64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_config_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppCommandRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub remote_task_id: String,
    pub client_request_id: String,
    pub expected_state_version: i64,
    pub expires_at: String,
    pub command_type: String,
    pub payload: CommandPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandRecord {
    pub command_id: String,
    pub client_request_id: String,
    pub remote_task_id: String,
    pub command_type: String,
    pub payload_digest: String,
    pub expected_state_version: i64,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_state_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandAcceptedResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub command: CommandRecord,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub command_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandQueryResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub command: CommandRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandDispatch {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub event_id: String,
    pub environment: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub state_version: i64,
    pub sent_at: String,
    pub command: CommandRecord,
    pub payload: CommandPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayCommandResult {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub event_id: String,
    pub causation_id: Option<String>,
    pub environment: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub state_version: i64,
    pub sent_at: String,
    pub command: CommandRecord,
}

struct CommandScope {
    binding_id: String,
    pc_device_id: String,
    installation_id: String,
    binding_epoch: i64,
    last_gateway_observed_at: Option<i64>,
    snapshot: TaskSnapshot,
}

impl CloudService {
    pub fn submit_command(
        &self,
        path_remote_task_id: &str,
        request: AppCommandRequest,
        authentication: AppRequestAuthentication,
        body: &[u8],
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<CommandAcceptedResponse, CloudError> {
        let app_device_id =
            self.authenticate_app_request(&authentication, "POST", canonical_path, body, now)?;
        validate_request_envelope(
            &request.schema_version,
            &request.message_type,
            "app/command",
            &request.message_id,
            &request.environment,
            self.environment(),
        )?;
        validate_opaque_id(path_remote_task_id)?;
        validate_opaque_id(&request.client_request_id)?;
        if request.remote_task_id != path_remote_task_id
            || !(1..=MAX_SAFE_INTEGER).contains(&request.expected_state_version)
        {
            return Err(CloudError::InvalidRequest);
        }
        let expires_at = DateTime::parse_from_rfc3339(&request.expires_at)
            .map_err(|_| CloudError::InvalidRequest)?
            .with_timezone(&Utc);
        if expires_at <= now {
            return Err(CloudError::CommandExpired);
        }
        if expires_at > now + Duration::minutes(COMMAND_TTL_MINUTES) {
            return Err(CloudError::InvalidRequest);
        }
        let expires_at_text = format_timestamp(expires_at);
        validate_command_payload(&request.command_type, &request.payload)?;
        let payload_json =
            serde_json::to_vec(&request.payload).map_err(|_| CloudError::InvalidRequest)?;
        let payload_digest = sha256_hex(&payload_json);

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let existing = transaction
            .query_row(
                "SELECT remote_commands.command_id, remote_commands.client_request_id,
                        remote_commands.remote_task_id, remote_commands.command_type,
                        remote_commands.payload_digest, remote_commands.expected_state_version,
                        remote_commands.status, remote_commands.created_at,
                        remote_commands.expires_at, remote_commands.applied_state_version,
                        remote_commands.error_code
                 FROM remote_commands
                 INNER JOIN bindings ON bindings.binding_id = remote_commands.binding_id
                   AND bindings.environment = remote_commands.environment
                 WHERE remote_commands.environment = ?1 AND remote_commands.app_device_id = ?2
                   AND remote_commands.remote_task_id = ?3
                   AND remote_commands.client_request_id = ?4 AND bindings.state = 'active'
                 ORDER BY bindings.activated_at DESC LIMIT 1",
                params![
                    self.environment().as_str(),
                    app_device_id,
                    path_remote_task_id,
                    request.client_request_id
                ],
                command_record_from_row,
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let command = if let Some(existing) = existing {
            if existing.payload_digest != payload_digest
                || existing.command_type != request.command_type
                || existing.expected_state_version != request.expected_state_version
                || existing.expires_at != expires_at_text
            {
                return Err(CloudError::PayloadDigestConflict);
            }
            existing
        } else {
            let scope = command_scope(
                &transaction,
                self.environment().as_str(),
                &app_device_id,
                path_remote_task_id,
            )?;
            if command_requires_fresh_source_state(&request.command_type)
                && scope.snapshot.state_version != request.expected_state_version
            {
                return Err(CloudError::StateConflict);
            }
            validate_snapshot_command_state(&request.command_type, &scope.snapshot.task_status)?;
            let is_online = scope
                .last_gateway_observed_at
                .is_some_and(|observed| now.timestamp().saturating_sub(observed) <= 45);
            if !is_online && request.command_type != "send_input" {
                return Err(CloudError::PcOffline);
            }
            let encrypted_payload = self.encrypt_push_token(&payload_json)?;
            let command_id = random_opaque_id();
            transaction
                .execute(
                    "INSERT INTO remote_commands (
                       environment, command_id, binding_id, app_device_id, pc_device_id,
                       installation_id, binding_epoch, remote_task_id, client_request_id,
                       command_type, payload_digest, payload_encrypted, expected_state_version,
                       status, created_at, expires_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                               'queued', ?14, ?15, ?14)",
                    params![
                        self.environment().as_str(),
                        command_id,
                        scope.binding_id,
                        app_device_id,
                        scope.pc_device_id,
                        scope.installation_id,
                        scope.binding_epoch,
                        path_remote_task_id,
                        request.client_request_id,
                        request.command_type,
                        payload_digest,
                        encrypted_payload,
                        request.expected_state_version,
                        now.timestamp(),
                        expires_at.timestamp()
                    ],
                )
                .map_err(|_| CloudError::StorageUnavailable)?;
            CommandRecord {
                command_id,
                client_request_id: request.client_request_id,
                remote_task_id: path_remote_task_id.to_owned(),
                command_type: request.command_type,
                payload_digest,
                expected_state_version: request.expected_state_version,
                status: "queued".into(),
                created_at: format_timestamp(now),
                expires_at: expires_at_text,
                applied_state_version: None,
                error_code: None,
            }
        };
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(CommandAcceptedResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/command-accepted",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment().as_str().to_owned(),
            server_received_at: format_timestamp(now),
            command,
        })
    }

    pub fn query_command(
        &self,
        path_command_id: &str,
        request: CommandQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<CommandQueryResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/command-query",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment(),
            now,
        )?;
        validate_opaque_id(path_command_id)?;
        if request.command_id != path_command_id
            || request.app_device_id != authenticated_app_device_id
        {
            return Err(CloudError::InvalidRequest);
        }
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE remote_commands
                 SET status = 'expired', payload_encrypted = NULL, error_code = 'command_expired',
                     updated_at = ?1
                 WHERE environment = ?2 AND command_id = ?3 AND expires_at <= ?1
                   AND status IN ('queued', 'dispatched')",
                params![
                    now.timestamp(),
                    self.environment().as_str(),
                    path_command_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let command = connection
            .query_row(
                "SELECT command_id, client_request_id, remote_task_id, command_type,
                        payload_digest, expected_state_version, status, created_at, expires_at,
                        applied_state_version, error_code
                 FROM remote_commands
                 WHERE environment = ?1 AND command_id = ?2 AND app_device_id = ?3",
                params![
                    self.environment().as_str(),
                    path_command_id,
                    authenticated_app_device_id
                ],
                command_record_from_row,
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::NotFound)?;
        Ok(CommandQueryResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/command-status",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment().as_str().to_owned(),
            server_received_at: format_timestamp(now),
            command,
        })
    }

    pub fn pending_gateway_commands(
        &self,
        identity: &GatewayIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<CommandDispatch>, CloudError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "UPDATE remote_commands
                 SET status = 'expired', payload_encrypted = NULL, error_code = 'command_expired',
                     updated_at = ?1
                 WHERE environment = ?2 AND pc_device_id = ?3 AND installation_id = ?4
                   AND expires_at <= ?1 AND status IN ('queued', 'dispatched')",
                params![
                    now.timestamp(),
                    self.environment().as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut statement = transaction
            .prepare(
                "SELECT command_id, client_request_id, remote_task_id, command_type,
                        payload_digest, expected_state_version, status, created_at, expires_at,
                        applied_state_version, error_code, payload_encrypted, binding_epoch
                 FROM remote_commands
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND status IN ('queued', 'dispatched') AND expires_at > ?4
                 ORDER BY created_at LIMIT 16",
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let rows = statement
            .query_map(
                params![
                    self.environment().as_str(),
                    identity.pc_device_id,
                    identity.installation_id,
                    now.timestamp()
                ],
                |row| {
                    Ok((
                        command_record_from_row(row)?,
                        row.get::<_, Vec<u8>>(11)?,
                        row.get::<_, i64>(12)?,
                    ))
                },
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut commands = Vec::new();
        for row in rows {
            let (mut command, encrypted_payload, binding_epoch) =
                row.map_err(|_| CloudError::StorageUnavailable)?;
            let plaintext = self.decrypt_command_payload(&encrypted_payload)?;
            let payload: CommandPayload =
                serde_json::from_slice(&plaintext).map_err(|_| CloudError::StorageUnavailable)?;
            command.status = "dispatched".into();
            commands.push(CommandDispatch {
                schema_version: CONTRACT_VERSION.into(),
                message_type: "command/dispatch".into(),
                message_id: random_opaque_id(),
                event_id: random_opaque_id(),
                environment: self.environment().as_str().to_owned(),
                pc_device_id: identity.pc_device_id.clone(),
                installation_id: identity.installation_id.clone(),
                binding_epoch,
                state_version: command.expected_state_version,
                sent_at: format_timestamp(now),
                command,
                payload,
            });
        }
        drop(statement);
        for dispatch in &commands {
            transaction
                .execute(
                    "UPDATE remote_commands SET status = 'dispatched', updated_at = ?1
                     WHERE environment = ?2 AND command_id = ?3
                       AND status IN ('queued', 'dispatched')",
                    params![
                        now.timestamp(),
                        self.environment().as_str(),
                        dispatch.command.command_id
                    ],
                )
                .map_err(|_| CloudError::StorageUnavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(commands)
    }

    pub fn accept_gateway_command_result(
        &self,
        identity: &GatewayIdentity,
        request: GatewayCommandResult,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        self.validate_gateway_event(
            identity,
            GatewayEventValidation {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "command/result",
                message_id: &request.message_id,
                event_id: &request.event_id,
                causation_id: request.causation_id.as_deref(),
                environment: &request.environment,
                pc_device_id: &request.pc_device_id,
                installation_id: &request.installation_id,
                binding_epoch: request.binding_epoch,
                state_version: request.state_version,
                sent_at: &request.sent_at,
                allow_previous_binding_epoch: false,
                now,
            },
        )?;
        if !matches!(
            request.command.status.as_str(),
            "completed" | "rejected" | "failed" | "reconciling"
        ) || request.command.applied_state_version != Some(request.state_version)
        {
            return Err(CloudError::InvalidRequest);
        }
        let connection = self.connection()?;
        let stored = connection
            .query_row(
                "SELECT command_id, client_request_id, remote_task_id, command_type,
                        payload_digest, expected_state_version, status, created_at, expires_at,
                        applied_state_version, error_code
                 FROM remote_commands
                 WHERE environment = ?1 AND command_id = ?2 AND pc_device_id = ?3
                   AND installation_id = ?4 AND binding_epoch = ?5",
                params![
                    self.environment().as_str(),
                    request.command.command_id,
                    identity.pc_device_id,
                    identity.installation_id,
                    request.binding_epoch
                ],
                command_record_from_row,
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::NotFound)?;
        if stored.client_request_id != request.command.client_request_id
            || stored.remote_task_id != request.command.remote_task_id
            || stored.command_type != request.command.command_type
            || stored.payload_digest != request.command.payload_digest
            || stored.expected_state_version != request.command.expected_state_version
            || stored.created_at != request.command.created_at
            || stored.expires_at != request.command.expires_at
        {
            return Err(CloudError::PayloadDigestConflict);
        }
        connection
            .execute(
                "UPDATE remote_commands
                 SET status = ?1, applied_state_version = ?2, error_code = ?3,
                     payload_encrypted = NULL, updated_at = ?4
                 WHERE environment = ?5 AND command_id = ?6
                   AND status IN ('queued', 'dispatched')",
                params![
                    request.command.status,
                    request.command.applied_state_version,
                    request.command.error_code,
                    now.timestamp(),
                    self.environment().as_str(),
                    request.command.command_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(())
    }

    fn decrypt_command_payload(&self, encrypted: &[u8]) -> Result<Vec<u8>, CloudError> {
        if encrypted.len() <= 12 {
            return Err(CloudError::StorageUnavailable);
        }
        self.token_cipher
            .decrypt(Nonce::from_slice(&encrypted[..12]), &encrypted[12..])
            .map_err(|_| CloudError::StorageUnavailable)
    }
}

fn command_scope(
    connection: &rusqlite::Connection,
    environment: &str,
    app_device_id: &str,
    remote_task_id: &str,
) -> Result<CommandScope, CloudError> {
    connection
        .query_row(
            "SELECT bindings.binding_id, bindings.pc_device_id, bindings.installation_id,
                    bindings.binding_epoch, pc_devices.last_gateway_observed_at,
                    remote_task_snapshots.snapshot_json
             FROM bindings
             INNER JOIN pc_devices
               ON pc_devices.environment = bindings.environment
              AND pc_devices.pc_device_id = bindings.pc_device_id
              AND pc_devices.installation_id = bindings.installation_id
             INNER JOIN remote_task_snapshots
               ON remote_task_snapshots.environment = bindings.environment
              AND remote_task_snapshots.pc_device_id = bindings.pc_device_id
              AND remote_task_snapshots.installation_id = bindings.installation_id
              AND remote_task_snapshots.binding_epoch = bindings.binding_epoch
             WHERE bindings.environment = ?1 AND bindings.app_device_id = ?2
               AND bindings.state = 'active' AND remote_task_snapshots.remote_task_id = ?3
               AND remote_task_snapshots.tombstoned = 0
             ORDER BY bindings.activated_at DESC LIMIT 1",
            params![environment, app_device_id, remote_task_id],
            |row| {
                let snapshot_json: String = row.get(5)?;
                let snapshot = serde_json::from_str(&snapshot_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        5,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok(CommandScope {
                    binding_id: row.get(0)?,
                    pc_device_id: row.get(1)?,
                    installation_id: row.get(2)?,
                    binding_epoch: row.get(3)?,
                    last_gateway_observed_at: row.get(4)?,
                    snapshot,
                })
            },
        )
        .optional()
        .map_err(|_| CloudError::StorageUnavailable)?
        .ok_or(CloudError::DeviceNotBound)
}

fn validate_command_payload(
    command_type: &str,
    payload: &CommandPayload,
) -> Result<(), CloudError> {
    match command_type {
        "create_task" => {
            let name = payload.text.as_deref().ok_or(CloudError::InvalidRequest)?;
            let initial_text = payload
                .initial_text
                .as_deref()
                .ok_or(CloudError::InvalidRequest)?;
            let model_config_id = payload
                .model_config_id
                .as_deref()
                .ok_or(CloudError::InvalidRequest)?;
            if name.trim().is_empty()
                || name.chars().count() > 60
                || initial_text.trim().is_empty()
                || initial_text.len() > 8_192
                || model_config_id.len() != 64
                || !model_config_id
                    .bytes()
                    .all(|value| value.is_ascii_hexdigit() && !value.is_ascii_uppercase())
            {
                return Err(CloudError::InvalidRequest);
            }
        }
        "start_task" | "send_input" => {
            let text = payload.text.as_deref().ok_or(CloudError::InvalidRequest)?;
            if text.trim().is_empty() || text.len() > 8_192 {
                return Err(CloudError::InvalidRequest);
            }
        }
        "stop_task" if payload.text.is_none() => {}
        "pause_task" | "resume_task" => return Err(CloudError::UnsupportedOperation),
        _ => return Err(CloudError::InvalidRequest),
    }
    Ok(())
}

fn validate_snapshot_command_state(command_type: &str, status: &str) -> Result<(), CloudError> {
    let allowed = match command_type {
        "create_task" => true,
        "start_task" => matches!(status, "created" | "stopped" | "failed"),
        "send_input" => matches!(status, "running" | "stopped" | "failed"),
        "stop_task" => matches!(
            status,
            "queued" | "starting" | "running" | "pausing" | "paused"
        ),
        _ => false,
    };
    allowed.then_some(()).ok_or(CloudError::InvalidCommandState)
}

fn command_requires_fresh_source_state(command_type: &str) -> bool {
    // A create command uses its source task solely as a binding-scoped workspace route.
    // Its current status/version cannot change the new task's operation, unlike actions
    // that operate on that source task itself.
    command_type != "create_task"
}

fn command_record_from_row(row: &Row<'_>) -> rusqlite::Result<CommandRecord> {
    let created_at: i64 = row.get(7)?;
    let expires_at: i64 = row.get(8)?;
    let created_at = DateTime::from_timestamp(created_at, 0)
        .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(7, created_at))?;
    let expires_at = DateTime::from_timestamp(expires_at, 0)
        .ok_or_else(|| rusqlite::Error::IntegralValueOutOfRange(8, expires_at))?;
    Ok(CommandRecord {
        command_id: row.get(0)?,
        client_request_id: row.get(1)?,
        remote_task_id: row.get(2)?,
        command_type: row.get(3)?,
        payload_digest: row.get(4)?,
        expected_state_version: row.get(5)?,
        status: row.get(6)?,
        created_at: format_timestamp(created_at),
        expires_at: format_timestamp(expires_at),
        applied_state_version: row.get(9)?,
        error_code: row.get(10)?,
    })
}

fn sha256_hex(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::TempDir;

    const APP_DEVICE_ID: &str = "app_device_command_0001";
    const DEVICE_KEY_ID: &str = "device_key_command_0001";
    const PC_DEVICE_ID: &str = "pc_device_command_0001";
    const INSTALLATION_ID: &str = "installation_command_0001";
    const BINDING_ID: &str = "binding_command_0001";
    const REMOTE_TASK_ID: &str = "remote_task_command_0001";
    const CLIENT_REQUEST_ID: &str = "client_request_command_0001";

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-29T08:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn service(directory: &TempDir) -> CloudService {
        CloudService::open(
            &directory.path().join("remote.sqlite3"),
            super::super::Environment::Dev,
            [11_u8; 32],
        )
        .expect("service")
    }

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[12_u8; 32])
    }

    fn task_snapshot(task_status: &str, state_version: i64) -> TaskSnapshot {
        TaskSnapshot {
            remote_task_id: REMOTE_TASK_ID.into(),
            pc_device_id: PC_DEVICE_ID.into(),
            installation_id: INSTALLATION_ID.into(),
            binding_epoch: 1,
            name: "远程命令测试任务".into(),
            workspace_name: "测试工作区".into(),
            model_label: "gpt-5.6-sol".into(),
            task_status: task_status.into(),
            turn_status: "idle".into(),
            last_turn_outcome: "none".into(),
            last_reply: None,
            last_reply_state: "absent".into(),
            last_reply_version: None,
            last_error: None,
            pc_observed_at: format_timestamp(now()),
            server_received_at: None,
            state_version,
            pc_connection_state: "online".into(),
        }
    }

    fn install_command_scope(
        service: &CloudService,
        signing_key: &SigningKey,
        task_status: &str,
        state_version: i64,
        gateway_observed_at: i64,
    ) {
        let public_key_der = signing_key
            .verifying_key()
            .to_public_key_der()
            .expect("SPKI")
            .as_bytes()
            .to_vec();
        let snapshot_json =
            serde_json::to_string(&task_snapshot(task_status, state_version)).expect("snapshot");
        let connection = service.connection().expect("connection");
        connection
            .execute(
                "INSERT INTO app_devices (
                   environment, app_device_id, device_key_id, public_key_der, public_key_digest,
                   push_token_encrypted, app_display_name, app_version, status, updated_at
                 ) VALUES ('dev', ?1, ?2, ?3, ?4, ?5, '测试手机', '0.1.3-dev', 'active', ?6)",
                params![
                    APP_DEVICE_ID,
                    DEVICE_KEY_ID,
                    public_key_der,
                    sha256_hex(&public_key_der),
                    vec![1_u8; 16],
                    now().timestamp()
                ],
            )
            .expect("app device");
        connection
            .execute(
                "INSERT INTO pc_devices (
                   environment, pc_device_id, installation_id, device_key_id, public_key_der,
                   public_key_digest, display_name, current_binding_epoch,
                   last_gateway_observed_at, status, updated_at
                 ) VALUES ('dev', ?1, ?2, 'pc_device_key_command_0001', ?3, ?4,
                           '测试电脑', 1, ?5, 'active', ?6)",
                params![
                    PC_DEVICE_ID,
                    INSTALLATION_ID,
                    vec![2_u8; 44],
                    "pc_public_key_digest_command_0001",
                    gateway_observed_at,
                    now().timestamp()
                ],
            )
            .expect("pc device");
        connection
            .execute(
                "INSERT INTO bindings (
                   environment, binding_id, pc_pairing_message_id, app_device_id, pc_device_id,
                   installation_id, binding_epoch, confirmation_nonce_digest,
                   confirmation_nonce_encrypted, confirmation_expires_at, pc_display_name,
                   app_display_name, safety_phrase, summary_digest, state, created_at, activated_at
                 ) VALUES ('dev', ?1, 'pairing_message_command_0001', ?2, ?3, ?4, 1,
                           'confirmation_digest_command_0001', ?5, ?6, '测试电脑', '测试手机',
                           '青山-流水', 'summary_digest_command_0001', 'active', ?7, ?7)",
                params![
                    BINDING_ID,
                    APP_DEVICE_ID,
                    PC_DEVICE_ID,
                    INSTALLATION_ID,
                    vec![3_u8; 16],
                    (now() + Duration::minutes(10)).timestamp(),
                    now().timestamp()
                ],
            )
            .expect("binding");
        connection
            .execute(
                "INSERT INTO remote_task_snapshots (
                   environment, pc_device_id, installation_id, binding_epoch, remote_task_id,
                   state_version, last_event_id, snapshot_json, tombstoned, server_received_at
                 ) VALUES ('dev', ?1, ?2, 1, ?3, ?4, 'task_event_command_0001', ?5, 0, ?6)",
                params![
                    PC_DEVICE_ID,
                    INSTALLATION_ID,
                    REMOTE_TASK_ID,
                    state_version,
                    snapshot_json,
                    now().timestamp()
                ],
            )
            .expect("task snapshot");
    }

    fn app_auth(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
        timestamp: DateTime<Utc>,
    ) -> AppRequestAuthentication {
        let timestamp_text = format_timestamp(timestamp);
        let canonical = format!(
            "workagents-device-request-v1\n{method}\n{path}\ndev\n{timestamp_text}\n{nonce}\n{}",
            sha256_hex(body)
        );
        AppRequestAuthentication {
            device_key_id: DEVICE_KEY_ID.into(),
            timestamp: timestamp_text,
            nonce: nonce.into(),
            signature: URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes()),
        }
    }

    fn command_request(command_type: &str, text: Option<&str>) -> AppCommandRequest {
        AppCommandRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "app/command".into(),
            message_id: "command_message_0001".into(),
            environment: "dev".into(),
            remote_task_id: REMOTE_TASK_ID.into(),
            client_request_id: CLIENT_REQUEST_ID.into(),
            expected_state_version: 10,
            expires_at: format_timestamp(now() + Duration::minutes(5)),
            command_type: command_type.into(),
            payload: CommandPayload {
                text: text.map(str::to_owned),
                initial_text: (command_type == "create_task").then(|| "检查当前工作区".into()),
                model_config_id: (command_type == "create_task").then(|| "a".repeat(64)),
            },
        }
    }

    fn submit(
        service: &CloudService,
        signing_key: &SigningKey,
        request: AppCommandRequest,
        body: &[u8],
        nonce: &str,
    ) -> Result<CommandAcceptedResponse, CloudError> {
        let path = format!("/v1/tasks/{REMOTE_TASK_ID}/commands");
        service.submit_command(
            REMOTE_TASK_ID,
            request,
            app_auth(signing_key, "POST", &path, body, nonce, now()),
            body,
            &path,
            now(),
        )
    }

    #[test]
    fn only_requested_command_slice_is_enabled() {
        assert!(validate_command_payload(
            "create_task",
            &CommandPayload {
                text: Some("远程新任务".into()),
                initial_text: Some("检查当前工作区".into()),
                model_config_id: Some("a".repeat(64)),
            }
        )
        .is_ok());
        assert!(validate_command_payload(
            "create_task",
            &CommandPayload {
                text: Some("远程新任务".into()),
                initial_text: None,
                model_config_id: Some("a".repeat(64)),
            }
        )
        .is_err());
        assert!(validate_command_payload(
            "send_input",
            &CommandPayload {
                text: Some("继续".into()),
                initial_text: None,
                model_config_id: None,
            }
        )
        .is_ok());
        assert!(validate_command_payload(
            "stop_task",
            &CommandPayload {
                text: None,
                initial_text: None,
                model_config_id: None,
            }
        )
        .is_ok());
        assert!(validate_command_payload(
            "send_input",
            &CommandPayload {
                text: Some("   ".into()),
                initial_text: None,
                model_config_id: None,
            }
        )
        .is_err());
        assert!(matches!(
            validate_command_payload(
                "pause_task",
                &CommandPayload {
                    text: None,
                    initial_text: None,
                    model_config_id: None,
                }
            ),
            Err(CloudError::UnsupportedOperation)
        ));
    }

    #[test]
    fn command_state_matrix_rejects_stale_user_actions() {
        assert!(validate_snapshot_command_state("create_task", "archived").is_ok());
        assert!(validate_snapshot_command_state("start_task", "created").is_ok());
        assert!(validate_snapshot_command_state("send_input", "running").is_ok());
        assert!(validate_snapshot_command_state("stop_task", "paused").is_ok());
        assert!(validate_snapshot_command_state("stop_task", "stopped").is_err());
    }

    #[test]
    fn create_task_accepts_a_changed_workspace_source_but_other_commands_do_not() {
        let directory = tempfile::tempdir().expect("tempdir");
        let service = service(&directory);
        let signing_key = signing_key();
        install_command_scope(&service, &signing_key, "running", 10, now().timestamp());

        let changed_snapshot =
            serde_json::to_string(&task_snapshot("stopped", 11)).expect("snapshot");
        let connection = service.connection().expect("connection");
        connection
            .execute(
                "UPDATE remote_task_snapshots SET state_version = 11, snapshot_json = ?1
                 WHERE environment = 'dev' AND remote_task_id = ?2",
                params![changed_snapshot, REMOTE_TASK_ID],
            )
            .expect("change task state");
        drop(connection);

        let mut create_request = command_request("create_task", Some("手机新任务"));
        create_request.client_request_id = "create_request_0002".into();
        let accepted = submit(
            &service,
            &signing_key,
            create_request,
            br#"{"command":"create"}"#,
            "command_nonce_create_0002",
        )
        .expect("create command accepts its stale workspace route");
        assert_eq!(accepted.command.status, "queued");
        assert_eq!(accepted.command.expected_state_version, 10);

        assert!(matches!(
            submit(
                &service,
                &signing_key,
                command_request("send_input", Some("继续执行")),
                br#"{"command":"send"}"#,
                "command_nonce_send_0002",
            ),
            Err(CloudError::StateConflict)
        ));
    }

    #[test]
    fn accepted_command_retry_is_idempotent_after_task_and_connection_change() {
        let directory = tempfile::tempdir().expect("tempdir");
        let service = service(&directory);
        let signing_key = signing_key();
        install_command_scope(&service, &signing_key, "running", 10, now().timestamp());
        let body = br#"{"command":"same"}"#;
        let first = submit(
            &service,
            &signing_key,
            command_request("send_input", Some("继续执行")),
            body,
            "command_nonce_0001",
        )
        .expect("first submission");

        let changed_snapshot =
            serde_json::to_string(&task_snapshot("stopped", 11)).expect("snapshot");
        let connection = service.connection().expect("connection");
        connection
            .execute(
                "UPDATE remote_task_snapshots SET state_version = 11, snapshot_json = ?1
                 WHERE environment = 'dev' AND remote_task_id = ?2",
                params![changed_snapshot, REMOTE_TASK_ID],
            )
            .expect("change task state");
        connection
            .execute(
                "UPDATE pc_devices SET last_gateway_observed_at = ?1
                 WHERE environment = 'dev' AND pc_device_id = ?2",
                params![(now() - Duration::minutes(2)).timestamp(), PC_DEVICE_ID],
            )
            .expect("make pc stale");
        drop(connection);

        let retried = submit(
            &service,
            &signing_key,
            command_request("send_input", Some("继续执行")),
            body,
            "command_nonce_0002",
        )
        .expect("idempotent retry");
        assert_eq!(retried.command, first.command);
    }

    #[test]
    fn reused_client_request_id_with_different_payload_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        let service = service(&directory);
        let signing_key = signing_key();
        install_command_scope(&service, &signing_key, "running", 10, now().timestamp());
        submit(
            &service,
            &signing_key,
            command_request("send_input", Some("第一条")),
            br#"{"command":"first"}"#,
            "command_nonce_0003",
        )
        .expect("first submission");

        let result = submit(
            &service,
            &signing_key,
            command_request("send_input", Some("第二条")),
            br#"{"command":"second"}"#,
            "command_nonce_0004",
        );
        assert!(matches!(result, Err(CloudError::PayloadDigestConflict)));
    }

    #[test]
    fn offline_stop_is_rejected_without_persisting_command() {
        let directory = tempfile::tempdir().expect("tempdir");
        let service = service(&directory);
        let signing_key = signing_key();
        install_command_scope(
            &service,
            &signing_key,
            "running",
            10,
            (now() - Duration::minutes(2)).timestamp(),
        );
        let result = submit(
            &service,
            &signing_key,
            command_request("stop_task", None),
            br#"{"command":"stop"}"#,
            "command_nonce_0005",
        );
        assert!(matches!(result, Err(CloudError::PcOffline)));
        let count: i64 = service
            .connection()
            .expect("connection")
            .query_row("SELECT COUNT(*) FROM remote_commands", [], |row| row.get(0))
            .expect("command count");
        assert_eq!(count, 0);
    }

    #[test]
    fn gateway_dispatch_decrypts_payload_and_terminal_result_clears_it() {
        let directory = tempfile::tempdir().expect("tempdir");
        let service = service(&directory);
        let signing_key = signing_key();
        install_command_scope(&service, &signing_key, "running", 10, now().timestamp());
        let accepted = submit(
            &service,
            &signing_key,
            command_request("send_input", Some("只在投递时解密")),
            br#"{"command":"dispatch"}"#,
            "command_nonce_0006",
        )
        .expect("submission");
        let identity = GatewayIdentity {
            pc_device_id: PC_DEVICE_ID.into(),
            installation_id: INSTALLATION_ID.into(),
        };
        let mut pending = service
            .pending_gateway_commands(&identity, now() + Duration::seconds(1))
            .expect("pending commands");
        assert_eq!(pending.len(), 1);
        let dispatch = pending.remove(0);
        assert_eq!(dispatch.payload.text.as_deref(), Some("只在投递时解密"));
        assert_eq!(dispatch.command.status, "dispatched");

        let mut result_command = dispatch.command;
        result_command.status = "completed".into();
        result_command.applied_state_version = Some(11);
        service
            .accept_gateway_command_result(
                &identity,
                GatewayCommandResult {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "command/result".into(),
                    message_id: "command_result_message_0001".into(),
                    event_id: "command_result_event_0001".into(),
                    causation_id: Some(dispatch.event_id),
                    environment: "dev".into(),
                    pc_device_id: PC_DEVICE_ID.into(),
                    installation_id: INSTALLATION_ID.into(),
                    binding_epoch: 1,
                    state_version: 11,
                    sent_at: format_timestamp(now() + Duration::seconds(2)),
                    command: result_command,
                },
                now() + Duration::seconds(2),
            )
            .expect("terminal result");

        let stored: (String, Option<Vec<u8>>) = service
            .connection()
            .expect("connection")
            .query_row(
                "SELECT status, payload_encrypted FROM remote_commands
                 WHERE environment = 'dev' AND command_id = ?1",
                [accepted.command.command_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stored command");
        assert_eq!(stored.0, "completed");
        assert!(stored.1.is_none());
    }
}
