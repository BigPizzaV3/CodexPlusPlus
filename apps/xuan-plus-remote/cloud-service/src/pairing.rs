use aes_gcm::aead::Aead;
use aes_gcm::Nonce;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use ed25519_dalek::pkcs8::DecodePublicKey;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{
    decode_public_key, format_timestamp, random_opaque_id, sha256_hex, validate_opaque_id,
    validate_request_base, CloudError, CloudService, Environment, RequestBase, CONTRACT_VERSION,
    REQUEST_CLOCK_SKEW_SECONDS,
};

const PAIRING_TTL_MINUTES: i64 = 10;
const MAX_APP_BINDINGS: i64 = 5;
const MAX_PC_BINDINGS: i64 = 3;

#[derive(Debug, Clone)]
pub struct AppRequestAuthentication {
    pub device_key_id: String,
    pub timestamp: String,
    pub nonce: String,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct PcRequestAuthentication {
    pub pc_device_id: String,
    pub installation_id: String,
    pub timestamp: String,
    pub nonce: String,
    pub public_key: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingQrPayload {
    pub pairing_qr_version: String,
    pub environment: String,
    pub pairing_handle: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingRegistrationRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub pairing: PairingQrPayload,
    pub pc_display_name: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRegistrationResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub pairing_handle: String,
    pub expires_at: String,
    pub registration_state: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PairingConsumeRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub pairing: PairingQrPayload,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingRevocationRequest {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub binding_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingRevokedResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub binding_id: String,
    pub binding_state: &'static str,
    pub revoked_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingSummary {
    pub environment: String,
    pub pc_display_name: String,
    pub app_display_name: String,
    pub safety_phrase: String,
    pub summary_digest: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingPendingResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub binding_id: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub confirmation_nonce: String,
    pub confirmation_expires_at: String,
    pub binding_summary: BindingSummary,
    pub binding_state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PcDeviceListQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcDeviceSummary {
    pub binding_id: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub display_name: String,
    pub pc_connection_state: &'static str,
    pub pc_observed_at: String,
    pub model_configs: Vec<ModelConfigSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelConfigSummary {
    pub id: String,
    pub model: String,
    pub provider: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PcDeviceListResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub pc_devices: Vec<PcDeviceSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingConfirmationRequest {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub pc_pairing_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub binding_id: String,
    pub app_device_id: String,
    pub confirmation_nonce: String,
    pub confirmation_expires_at: String,
    pub binding_summary: BindingSummary,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingLocalConfirmation {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub binding_id: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub confirmation_nonce: String,
    pub summary_digest: String,
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BindingActiveResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub binding_id: String,
    pub app_device_id: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub binding_state: &'static str,
    pub activated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PcHello {
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
    pub supported_schema_versions: Vec<String>,
    pub last_ack_event_id: Option<String>,
    pub last_ack_state_version: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PcHeartbeat {
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
    pub pc_observed_at: String,
    #[serde(default)]
    pub model_configs: Vec<ModelConfigSummary>,
}

#[derive(Debug, Clone)]
pub struct GatewayIdentity {
    pub pc_device_id: String,
    pub installation_id: String,
}

impl CloudService {
    pub fn register_pairing(
        &self,
        request: PairingRegistrationRequest,
        authentication: PcRequestAuthentication,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<PairingRegistrationResponse, CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "pairing/register",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        let expires_at = validate_pairing_registration(&request, self.environment, now)?;
        if authentication.pc_device_id != request.pc_device_id
            || authentication.installation_id != request.installation_id
        {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        self.authenticate_pc_request(
            &authentication,
            PcAuthenticationContext {
                method: "POST",
                path: "/v1/gateway/pairings",
                body,
                now,
                allow_enrollment: true,
                display_name: Some(&request.pc_display_name),
            },
        )?;

        let handle_digest = sha256_hex(request.pairing.pairing_handle.as_bytes());
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "DELETE FROM pairings WHERE environment = ?1 AND expires_at < ?2",
                params![
                    self.environment.as_str(),
                    (now - Duration::days(30)).timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "DELETE FROM pairings
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND consumed_at IS NULL",
                params![
                    self.environment.as_str(),
                    request.pc_device_id,
                    request.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "UPDATE bindings SET state = 'superseded', revoked_at = ?1
                 WHERE environment = ?2 AND pc_device_id = ?3 AND installation_id = ?4
                   AND state = 'pending_local_confirmation'",
                params![
                    now.timestamp(),
                    self.environment.as_str(),
                    request.pc_device_id,
                    request.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "INSERT INTO pairings (
                   environment, pairing_handle_digest, pc_pairing_message_id, pc_device_id,
                   installation_id, pc_display_name, expires_at,
                   consumed_at, consumed_app_device_id, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)",
                params![
                    self.environment.as_str(),
                    handle_digest,
                    request.message_id,
                    request.pc_device_id,
                    request.installation_id,
                    request.pc_display_name,
                    expires_at.timestamp(),
                    now.timestamp()
                ],
            )
            .map_err(|_| CloudError::PairingQrInvalid)?;
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;

        Ok(PairingRegistrationResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "pairing/registered",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            pairing_handle: request.pairing.pairing_handle,
            expires_at: format_timestamp(expires_at),
            registration_state: "ready",
        })
    }

    pub fn consume_pairing(
        &self,
        request: PairingConsumeRequest,
        authentication: AppRequestAuthentication,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<PairingPendingResponse, CloudError> {
        let authenticated_app_device_id = self.authenticate_app_request(
            &authentication,
            "POST",
            "/v1/pairings/consume",
            body,
            now,
        )?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "pairing/consume",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        if authenticated_app_device_id != request.app_device_id {
            return Err(CloudError::DeviceAuthenticationFailed);
        }

        validate_pairing_payload(&request.pairing, self.environment)?;
        let handle_digest = sha256_hex(request.pairing.pairing_handle.as_bytes());
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let pairing = transaction
            .query_row(
                "SELECT pc_pairing_message_id, pc_device_id, installation_id, pc_display_name,
                        expires_at, consumed_at, consumed_app_device_id
                 FROM pairings WHERE environment = ?1 AND pairing_handle_digest = ?2",
                params![self.environment.as_str(), handle_digest],
                read_pairing_row,
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::PairingQrInvalid)?;

        if pairing.consumed_at.is_some() {
            if pairing.consumed_app_device_id.as_deref() != Some(request.app_device_id.as_str()) {
                return Err(CloudError::PairingReplayed);
            }
            let existing = transaction
                .query_row(
                    "SELECT binding_id, confirmation_nonce_encrypted, confirmation_expires_at,
                            pc_display_name, app_display_name, safety_phrase, summary_digest, state
                     FROM bindings
                     WHERE environment = ?1 AND pc_pairing_message_id = ?2 AND app_device_id = ?3
                     ORDER BY created_at DESC LIMIT 1",
                    params![
                        self.environment.as_str(),
                        pairing.pc_pairing_message_id,
                        request.app_device_id
                    ],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Vec<u8>>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| CloudError::StorageUnavailable)?
                .ok_or(CloudError::PairingReplayed)?;
            if existing.7 != "pending_local_confirmation" && existing.7 != "active" {
                return Err(CloudError::PairingReplayed);
            }
            return Ok(PairingPendingResponse {
                schema_version: CONTRACT_VERSION,
                message_type: "pairing/pending",
                message_id: random_opaque_id(),
                request_message_id: request.message_id,
                environment: self.environment.as_str().to_owned(),
                server_received_at: format_timestamp(now),
                binding_id: existing.0,
                pc_device_id: pairing.pc_device_id,
                installation_id: pairing.installation_id,
                confirmation_nonce: self.decrypt_sensitive(&existing.1)?,
                confirmation_expires_at: format_timestamp(
                    DateTime::from_timestamp(existing.2, 0)
                        .ok_or(CloudError::StorageUnavailable)?,
                ),
                binding_summary: BindingSummary {
                    environment: self.environment.as_str().to_owned(),
                    pc_display_name: existing.3,
                    app_display_name: existing.4,
                    safety_phrase: existing.5,
                    summary_digest: existing.6,
                },
                binding_state: existing.7,
            });
        }
        if pairing.expires_at <= now.timestamp() {
            return Err(CloudError::PairingExpired);
        }

        enforce_binding_limits(
            &transaction,
            self.environment,
            &request.app_device_id,
            &pairing.pc_device_id,
        )?;
        let app_display_name: String = transaction
            .query_row(
                "SELECT app_display_name FROM app_devices
                 WHERE environment = ?1 AND app_device_id = ?2 AND status = 'active'",
                params![self.environment.as_str(), request.app_device_id],
                |row| row.get(0),
            )
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        let binding_id = random_opaque_id();
        let confirmation_nonce = random_opaque_id();
        let confirmation_nonce_digest = sha256_hex(confirmation_nonce.as_bytes());
        let encrypted_confirmation_nonce =
            self.encrypt_push_token(confirmation_nonce.as_bytes())?;
        let confirmation_expires_at = now + Duration::minutes(PAIRING_TTL_MINUTES);
        let safety_phrase = random_safety_phrase();
        let summary_digest = binding_summary_digest(
            self.environment,
            &pairing.pc_display_name,
            &app_display_name,
            &safety_phrase,
        );
        transaction
            .execute(
                "INSERT INTO bindings (
                   environment, binding_id, pc_pairing_message_id, app_device_id, pc_device_id, installation_id,
                   binding_epoch, confirmation_nonce_digest, confirmation_nonce_encrypted,
                   confirmation_expires_at, pc_display_name, app_display_name, safety_phrase,
                   summary_digest, state, created_at, activated_at, revoked_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                           'pending_local_confirmation', ?14, NULL, NULL)",
                params![
                    self.environment.as_str(),
                    binding_id,
                    pairing.pc_pairing_message_id,
                    request.app_device_id,
                    pairing.pc_device_id,
                    pairing.installation_id,
                    confirmation_nonce_digest,
                    encrypted_confirmation_nonce,
                    confirmation_expires_at.timestamp(),
                    pairing.pc_display_name,
                    app_display_name,
                    safety_phrase,
                    summary_digest,
                    now.timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let consumed = transaction
            .execute(
                "UPDATE pairings SET consumed_at = ?1, consumed_app_device_id = ?2
                 WHERE environment = ?3 AND pairing_handle_digest = ?4 AND consumed_at IS NULL",
                params![
                    now.timestamp(),
                    request.app_device_id,
                    self.environment.as_str(),
                    handle_digest
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if consumed != 1 {
            return Err(CloudError::PairingReplayed);
        }
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;

        Ok(PairingPendingResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "pairing/pending",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            binding_id,
            pc_device_id: pairing.pc_device_id,
            installation_id: pairing.installation_id,
            confirmation_nonce,
            confirmation_expires_at: format_timestamp(confirmation_expires_at),
            binding_summary: BindingSummary {
                environment: self.environment.as_str().to_owned(),
                pc_display_name: pairing.pc_display_name,
                app_display_name,
                safety_phrase,
                summary_digest,
            },
            binding_state: "pending_local_confirmation".into(),
        })
    }

    pub fn authenticate_gateway(
        &self,
        authentication: PcRequestAuthentication,
        now: DateTime<Utc>,
    ) -> Result<GatewayIdentity, CloudError> {
        self.authenticate_pc_request(
            &authentication,
            PcAuthenticationContext {
                method: "GET",
                path: "/v1/gateway/connect",
                body: &[],
                now,
                allow_enrollment: false,
                display_name: None,
            },
        )
    }

    pub fn list_pc_devices(
        &self,
        request: PcDeviceListQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<PcDeviceListResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/pc-devices-query",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        if request.app_device_id != authenticated_app_device_id {
            return Err(CloudError::DeviceAuthenticationFailed);
        }

        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT bindings.binding_id, bindings.pc_device_id, bindings.installation_id,
                        bindings.binding_epoch, pc_devices.display_name,
                        pc_devices.last_gateway_observed_at, pc_devices.model_configs_json
                 FROM bindings
                 INNER JOIN pc_devices
                    ON pc_devices.environment = bindings.environment
                   AND pc_devices.pc_device_id = bindings.pc_device_id
                   AND pc_devices.installation_id = bindings.installation_id
                 WHERE bindings.environment = ?1 AND bindings.app_device_id = ?2
                   AND bindings.state = 'active'
                 ORDER BY bindings.activated_at DESC LIMIT 100",
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let rows = statement
            .query_map(
                params![self.environment.as_str(), request.app_device_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut pc_devices = Vec::new();
        for row in rows {
            let (
                binding_id,
                pc_device_id,
                installation_id,
                binding_epoch,
                display_name,
                observed_at,
                model_configs_json,
            ) = row.map_err(|_| CloudError::StorageUnavailable)?;
            let observed_at = observed_at.ok_or(CloudError::StorageUnavailable)?;
            let age_seconds = now.timestamp().saturating_sub(observed_at);
            let pc_connection_state = if age_seconds <= 45 {
                "online"
            } else if age_seconds <= 120 {
                "stale"
            } else {
                "offline"
            };
            let observed_at =
                DateTime::from_timestamp(observed_at, 0).ok_or(CloudError::StorageUnavailable)?;
            pc_devices.push(PcDeviceSummary {
                binding_id,
                pc_device_id,
                installation_id,
                binding_epoch,
                display_name,
                pc_connection_state,
                pc_observed_at: format_timestamp(observed_at),
                model_configs: serde_json::from_str(&model_configs_json)
                    .map_err(|_| CloudError::StorageUnavailable)?,
            });
        }
        Ok(PcDeviceListResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/pc-devices",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            pc_devices,
            next_cursor: None,
        })
    }

    pub fn revoke_binding(
        &self,
        request: BindingRevocationRequest,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<BindingRevokedResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "POST", canonical_path, body, now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "binding/revoke",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        validate_opaque_id(&request.binding_id)?;
        if request.app_device_id != authenticated_app_device_id {
            return Err(CloudError::DeviceAuthenticationFailed);
        }

        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE bindings SET state = 'revoked', revoked_at = ?1
                 WHERE environment = ?2 AND binding_id = ?3 AND app_device_id = ?4
                   AND state = 'active'",
                params![
                    now.timestamp(),
                    self.environment.as_str(),
                    request.binding_id,
                    request.app_device_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if updated != 1 {
            return Err(CloudError::DeviceNotBound);
        }
        Ok(BindingRevokedResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "binding/revoked",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            binding_id: request.binding_id,
            binding_state: "revoked",
            revoked_at: format_timestamp(now),
        })
    }

    pub fn accept_gateway_hello(
        &self,
        identity: &GatewayIdentity,
        hello: &PcHello,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        self.validate_gateway_event(
            identity,
            GatewayEventValidation {
                schema_version: &hello.schema_version,
                message_type: &hello.message_type,
                expected_message_type: "pc/hello",
                message_id: &hello.message_id,
                event_id: &hello.event_id,
                causation_id: hello.causation_id.as_deref(),
                environment: &hello.environment,
                pc_device_id: &hello.pc_device_id,
                installation_id: &hello.installation_id,
                binding_epoch: hello.binding_epoch,
                state_version: hello.state_version,
                sent_at: &hello.sent_at,
                allow_previous_binding_epoch: true,
                now,
            },
        )?;
        if hello.supported_schema_versions.is_empty()
            || hello.supported_schema_versions.len() > 4
            || !hello
                .supported_schema_versions
                .iter()
                .any(|version| version == CONTRACT_VERSION)
            || hello
                .supported_schema_versions
                .iter()
                .any(|version| version.is_empty() || version.len() > 8)
            || hello.last_ack_state_version < 0
        {
            return Err(CloudError::InvalidRequest);
        }
        let unique_versions = hello
            .supported_schema_versions
            .iter()
            .collect::<std::collections::HashSet<_>>();
        if unique_versions.len() != hello.supported_schema_versions.len() {
            return Err(CloudError::InvalidRequest);
        }
        if let Some(last_ack_event_id) = hello.last_ack_event_id.as_deref() {
            validate_opaque_id(last_ack_event_id)?;
        }
        self.record_gateway_observation(identity, now)
    }

    pub fn record_gateway_heartbeat(
        &self,
        identity: &GatewayIdentity,
        heartbeat: &PcHeartbeat,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        self.validate_gateway_event(
            identity,
            GatewayEventValidation {
                schema_version: &heartbeat.schema_version,
                message_type: &heartbeat.message_type,
                expected_message_type: "pc/heartbeat",
                message_id: &heartbeat.message_id,
                event_id: &heartbeat.event_id,
                causation_id: heartbeat.causation_id.as_deref(),
                environment: &heartbeat.environment,
                pc_device_id: &heartbeat.pc_device_id,
                installation_id: &heartbeat.installation_id,
                binding_epoch: heartbeat.binding_epoch,
                state_version: heartbeat.state_version,
                sent_at: &heartbeat.sent_at,
                allow_previous_binding_epoch: false,
                now,
            },
        )?;
        let observed_at = parse_timestamp(&heartbeat.pc_observed_at)?;
        if (now - observed_at).num_seconds().unsigned_abs() > REQUEST_CLOCK_SKEW_SECONDS as u64 {
            return Err(CloudError::InvalidRequest);
        }
        let model_configs = serde_json::to_string(&heartbeat.model_configs)
            .map_err(|_| CloudError::StorageUnavailable)?;
        let connection = self.connection()?;
        connection
            .execute(
                "UPDATE pc_devices SET last_gateway_observed_at = ?1, model_configs_json = ?2,
             updated_at = ?1 WHERE environment = ?3 AND pc_device_id = ?4 AND installation_id = ?5",
                params![
                    now.timestamp(),
                    model_configs,
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(())
    }

    pub(super) fn validate_gateway_event(
        &self,
        identity: &GatewayIdentity,
        event: GatewayEventValidation<'_>,
    ) -> Result<(), CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: event.schema_version,
                message_type: event.message_type,
                expected_message_type: event.expected_message_type,
                message_id: event.message_id,
                environment: event.environment,
                sent_at: event.sent_at,
            },
            self.environment,
            event.now,
        )?;
        validate_opaque_id(event.event_id)?;
        if let Some(causation_id) = event.causation_id {
            validate_opaque_id(causation_id)?;
        }
        if event.pc_device_id != identity.pc_device_id
            || event.installation_id != identity.installation_id
            || event.state_version < 1
        {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        let connection = self.connection()?;
        let current_binding_epoch = connection
            .query_row(
                "SELECT current_binding_epoch FROM pc_devices
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3",
                params![
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        let expected_binding_epoch = current_binding_epoch.max(1);
        let previous_epoch_recovery = event.allow_previous_binding_epoch
            && expected_binding_epoch > 1
            && event.binding_epoch == expected_binding_epoch - 1;
        if event.binding_epoch != expected_binding_epoch && !previous_epoch_recovery {
            return Err(CloudError::DeviceNotBound);
        }
        Ok(())
    }

    pub(super) fn record_gateway_observation(
        &self,
        identity: &GatewayIdentity,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        let connection = self.connection()?;
        let updated = connection
            .execute(
                "UPDATE pc_devices SET last_gateway_observed_at = ?1
                 WHERE environment = ?2 AND pc_device_id = ?3 AND installation_id = ?4",
                params![
                    now.timestamp(),
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if updated != 1 {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        Ok(())
    }

    pub fn pending_binding_confirmations(
        &self,
        identity: &GatewayIdentity,
        now: DateTime<Utc>,
    ) -> Result<Vec<BindingConfirmationRequest>, CloudError> {
        let connection = self.connection()?;
        let mut statement = connection
            .prepare(
                "SELECT binding_id, app_device_id, pc_pairing_message_id, confirmation_nonce_encrypted,
                        confirmation_expires_at, pc_display_name, app_display_name,
                        safety_phrase, summary_digest
                 FROM bindings
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND state = 'pending_local_confirmation' AND confirmation_expires_at > ?4
                 ORDER BY created_at LIMIT 8",
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let rows = statement
            .query_map(
                params![
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id,
                    now.timestamp()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                    ))
                },
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut confirmations = Vec::new();
        for row in rows {
            let (
                binding_id,
                app_device_id,
                pc_pairing_message_id,
                encrypted_nonce,
                expires_at,
                pc_display_name,
                app_display_name,
                safety_phrase,
                summary_digest,
            ) = row.map_err(|_| CloudError::StorageUnavailable)?;
            confirmations.push(BindingConfirmationRequest {
                schema_version: CONTRACT_VERSION,
                message_type: "binding/confirmation-request",
                message_id: random_opaque_id(),
                request_message_id: pc_pairing_message_id.clone(),
                pc_pairing_message_id,
                environment: self.environment.as_str().to_owned(),
                server_received_at: format_timestamp(now),
                binding_id,
                app_device_id,
                confirmation_nonce: self.decrypt_sensitive(&encrypted_nonce)?,
                confirmation_expires_at: format_timestamp(
                    DateTime::from_timestamp(expires_at, 0)
                        .ok_or(CloudError::StorageUnavailable)?,
                ),
                binding_summary: BindingSummary {
                    environment: self.environment.as_str().to_owned(),
                    pc_display_name,
                    app_display_name,
                    safety_phrase,
                    summary_digest,
                },
            });
        }
        Ok(confirmations)
    }

    pub fn active_binding_for_gateway(
        &self,
        identity: &GatewayIdentity,
        now: DateTime<Utc>,
    ) -> Result<Option<BindingActiveResponse>, CloudError> {
        let connection = self.connection()?;
        let binding = connection
            .query_row(
                "SELECT binding_id, app_device_id, binding_epoch, activated_at
                 FROM bindings
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND state = 'active'
                 ORDER BY activated_at DESC LIMIT 1",
                params![
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?;
        binding
            .map(|(binding_id, app_device_id, binding_epoch, activated_at)| {
                Ok(BindingActiveResponse {
                    schema_version: CONTRACT_VERSION,
                    message_type: "binding/active",
                    message_id: random_opaque_id(),
                    request_message_id: random_opaque_id(),
                    environment: self.environment.as_str().to_owned(),
                    server_received_at: format_timestamp(now),
                    binding_id,
                    app_device_id,
                    pc_device_id: identity.pc_device_id.clone(),
                    installation_id: identity.installation_id.clone(),
                    binding_epoch,
                    binding_state: "active",
                    activated_at: format_timestamp(
                        DateTime::from_timestamp(activated_at, 0)
                            .ok_or(CloudError::StorageUnavailable)?,
                    ),
                })
            })
            .transpose()
    }

    pub fn confirm_binding(
        &self,
        identity: &GatewayIdentity,
        request: BindingLocalConfirmation,
        now: DateTime<Utc>,
    ) -> Result<BindingActiveResponse, CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "binding/local-confirm",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment,
            now,
        )?;
        if !request.confirmed
            || request.pc_device_id != identity.pc_device_id
            || request.installation_id != identity.installation_id
        {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        validate_opaque_id(&request.binding_id)?;
        validate_opaque_id(&request.confirmation_nonce)?;
        if request.summary_digest.len() != 64 {
            return Err(CloudError::PairingSummaryMismatch);
        }

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let binding: (String, String, String, i64, String, String, String) = transaction
            .query_row(
                "SELECT app_device_id, confirmation_nonce_digest, summary_digest,
                        confirmation_expires_at, pc_device_id, installation_id, pc_display_name
                 FROM bindings WHERE environment = ?1 AND binding_id = ?2
                   AND state = 'pending_local_confirmation'",
                params![self.environment.as_str(), request.binding_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(|_| CloudError::DeviceNotBound)?;
        if binding.3 <= now.timestamp() {
            return Err(CloudError::PairingConfirmationExpired);
        }
        if binding.4 != identity.pc_device_id || binding.5 != identity.installation_id {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        if binding.1 != sha256_hex(request.confirmation_nonce.as_bytes())
            || binding.2 != request.summary_digest
        {
            return Err(CloudError::PairingSummaryMismatch);
        }
        let binding_epoch: i64 = transaction
            .query_row(
                "SELECT current_binding_epoch + 1 FROM pc_devices
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND status = 'active'",
                params![
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
                |row| row.get(0),
            )
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        transaction
            .execute(
                "UPDATE pc_devices SET current_binding_epoch = ?1, updated_at = ?2
                 WHERE environment = ?3 AND pc_device_id = ?4 AND installation_id = ?5",
                params![
                    binding_epoch,
                    now.timestamp(),
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "UPDATE bindings SET binding_epoch = ?1
                 WHERE environment = ?2 AND pc_device_id = ?3 AND installation_id = ?4
                   AND state = 'active'",
                params![
                    binding_epoch,
                    self.environment.as_str(),
                    identity.pc_device_id,
                    identity.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        transaction
            .execute(
                "UPDATE bindings SET state = 'superseded', revoked_at = ?1
                 WHERE environment = ?2 AND app_device_id = ?3 AND state = 'active'
                   AND (pc_device_id = ?4 OR pc_display_name = ?5)",
                params![
                    now.timestamp(),
                    self.environment.as_str(),
                    binding.0,
                    identity.pc_device_id,
                    binding.6
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let updated = transaction
            .execute(
                "UPDATE bindings SET state = 'active', binding_epoch = ?1, activated_at = ?2
                 WHERE environment = ?3 AND binding_id = ?4
                   AND state = 'pending_local_confirmation'",
                params![
                    binding_epoch,
                    now.timestamp(),
                    self.environment.as_str(),
                    request.binding_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        if updated != 1 {
            return Err(CloudError::DeviceNotBound);
        }
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;

        Ok(BindingActiveResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "binding/active",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment.as_str().to_owned(),
            server_received_at: format_timestamp(now),
            binding_id: request.binding_id,
            app_device_id: binding.0,
            pc_device_id: identity.pc_device_id.clone(),
            installation_id: identity.installation_id.clone(),
            binding_epoch,
            binding_state: "active",
            activated_at: format_timestamp(now),
        })
    }

    pub(super) fn authenticate_app_request(
        &self,
        authentication: &AppRequestAuthentication,
        method: &str,
        path: &str,
        body: &[u8],
        now: DateTime<Utc>,
    ) -> Result<String, CloudError> {
        validate_opaque_id(&authentication.device_key_id)
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        validate_request_authentication_values(
            &authentication.timestamp,
            &authentication.nonce,
            &authentication.signature,
            now,
        )?;
        let connection = self.connection()?;
        let device: (String, Vec<u8>) = connection
            .query_row(
                "SELECT app_device_id, public_key_der FROM app_devices
                 WHERE environment = ?1 AND device_key_id = ?2 AND status = 'active'",
                params![self.environment.as_str(), authentication.device_key_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        verify_request_signature(
            &device.1,
            "workagents-device-request-v1",
            RequestProof {
                method,
                path,
                environment: self.environment,
                timestamp: &authentication.timestamp,
                nonce: &authentication.nonce,
                body,
                pc_device_id: None,
                installation_id: None,
            },
            &authentication.signature,
        )?;
        persist_nonce(
            &connection,
            self.environment,
            &authentication.device_key_id,
            None,
            &authentication.nonce,
            &authentication.timestamp,
            now,
        )?;
        Ok(device.0)
    }

    fn authenticate_pc_request(
        &self,
        authentication: &PcRequestAuthentication,
        context: PcAuthenticationContext<'_>,
    ) -> Result<GatewayIdentity, CloudError> {
        validate_opaque_id(&authentication.pc_device_id)
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        validate_opaque_id(&authentication.installation_id)
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        validate_request_authentication_values(
            &authentication.timestamp,
            &authentication.nonce,
            &authentication.signature,
            context.now,
        )?;
        let public_key_der = decode_public_key(&authentication.public_key)
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        VerifyingKey::from_public_key_der(&public_key_der)
            .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
        verify_request_signature(
            &public_key_der,
            "workagents-pc-request-v1",
            RequestProof {
                method: context.method,
                path: context.path,
                environment: self.environment,
                timestamp: &authentication.timestamp,
                nonce: &authentication.nonce,
                body: context.body,
                pc_device_id: Some(&authentication.pc_device_id),
                installation_id: Some(&authentication.installation_id),
            },
            &authentication.signature,
        )?;

        let public_key_digest = sha256_hex(&public_key_der);
        let connection = self.connection()?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT public_key_digest FROM pc_devices
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3",
                params![
                    self.environment.as_str(),
                    authentication.pc_device_id,
                    authentication.installation_id
                ],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?;
        match existing {
            Some(existing_digest) if existing_digest == public_key_digest => {}
            Some(_) => return Err(CloudError::DeviceAuthenticationFailed),
            None if !context.allow_enrollment => {
                return Err(CloudError::DeviceAuthenticationFailed)
            }
            None => {
                let display_name = context
                    .display_name
                    .ok_or(CloudError::DeviceAuthenticationFailed)?;
                connection
                    .execute(
                        "INSERT INTO pc_devices (
                           environment, pc_device_id, installation_id, device_key_id,
                           public_key_der, public_key_digest, display_name,
                           current_binding_epoch, status, updated_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 'active', ?8)",
                        params![
                            self.environment.as_str(),
                            authentication.pc_device_id,
                            authentication.installation_id,
                            random_opaque_id(),
                            public_key_der,
                            public_key_digest,
                            display_name,
                            context.now.timestamp()
                        ],
                    )
                    .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
            }
        }
        persist_nonce(
            &connection,
            self.environment,
            &authentication.pc_device_id,
            Some(&authentication.installation_id),
            &authentication.nonce,
            &authentication.timestamp,
            context.now,
        )?;
        connection
            .execute(
                "UPDATE pc_devices SET updated_at = ?1
                 WHERE environment = ?2 AND pc_device_id = ?3 AND installation_id = ?4",
                params![
                    context.now.timestamp(),
                    self.environment.as_str(),
                    authentication.pc_device_id,
                    authentication.installation_id
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        Ok(GatewayIdentity {
            pc_device_id: authentication.pc_device_id.clone(),
            installation_id: authentication.installation_id.clone(),
        })
    }

    fn decrypt_sensitive(&self, encrypted: &[u8]) -> Result<String, CloudError> {
        if encrypted.len() <= 12 {
            return Err(CloudError::StorageUnavailable);
        }
        let plaintext = self
            .token_cipher
            .decrypt(Nonce::from_slice(&encrypted[..12]), &encrypted[12..])
            .map_err(|_| CloudError::StorageUnavailable)?;
        String::from_utf8(plaintext).map_err(|_| CloudError::StorageUnavailable)
    }
}

struct PairingRow {
    pc_pairing_message_id: String,
    pc_device_id: String,
    installation_id: String,
    pc_display_name: String,
    expires_at: i64,
    consumed_at: Option<i64>,
    consumed_app_device_id: Option<String>,
}

fn read_pairing_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PairingRow> {
    Ok(PairingRow {
        pc_pairing_message_id: row.get(0)?,
        pc_device_id: row.get(1)?,
        installation_id: row.get(2)?,
        pc_display_name: row.get(3)?,
        expires_at: row.get(4)?,
        consumed_at: row.get(5)?,
        consumed_app_device_id: row.get(6)?,
    })
}

fn validate_pairing_registration(
    request: &PairingRegistrationRequest,
    environment: Environment,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, CloudError> {
    validate_opaque_id(&request.pc_device_id)?;
    validate_opaque_id(&request.installation_id)?;
    validate_pairing_payload(&request.pairing, environment)?;
    if request.pc_display_name.trim().is_empty() || request.pc_display_name.len() > 128 {
        return Err(CloudError::InvalidRequest);
    }
    let sent_at = parse_timestamp(&request.sent_at)?;
    let requested_expires_at = parse_timestamp(&request.expires_at)?;
    if requested_expires_at <= now
        || requested_expires_at <= sent_at
        || requested_expires_at > sent_at + Duration::minutes(PAIRING_TTL_MINUTES)
    {
        return Err(CloudError::PairingExpired);
    }
    Ok(requested_expires_at.min(now + Duration::minutes(PAIRING_TTL_MINUTES)))
}

fn validate_pairing_payload(
    pairing: &PairingQrPayload,
    environment: Environment,
) -> Result<(), CloudError> {
    if pairing.pairing_qr_version != "2"
        || pairing.environment != environment.as_str()
        || validate_opaque_id(&pairing.pairing_handle).is_err()
    {
        return Err(CloudError::InvalidRequest);
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, CloudError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| CloudError::InvalidRequest)
}

fn enforce_binding_limits(
    transaction: &rusqlite::Transaction<'_>,
    environment: Environment,
    app_device_id: &str,
    pc_device_id: &str,
) -> Result<(), CloudError> {
    let app_count: i64 = transaction
        .query_row(
            "SELECT COUNT(DISTINCT pc_device_id) FROM bindings
             WHERE environment = ?1 AND app_device_id = ?2
               AND pc_device_id <> ?3
               AND state IN ('pending_local_confirmation', 'active')",
            params![environment.as_str(), app_device_id, pc_device_id],
            |row| row.get(0),
        )
        .map_err(|_| CloudError::StorageUnavailable)?;
    let pc_count: i64 = transaction
        .query_row(
            "SELECT COUNT(DISTINCT app_device_id) FROM bindings
             WHERE environment = ?1 AND pc_device_id = ?2
               AND app_device_id <> ?3
               AND state IN ('pending_local_confirmation', 'active')",
            params![environment.as_str(), pc_device_id, app_device_id],
            |row| row.get(0),
        )
        .map_err(|_| CloudError::StorageUnavailable)?;
    if app_count >= MAX_APP_BINDINGS || pc_count >= MAX_PC_BINDINGS {
        return Err(CloudError::DeviceLimitReached);
    }
    Ok(())
}

fn random_safety_phrase() -> String {
    const WORDS: &[&str] = &[
        "amber", "bamboo", "cedar", "cloud", "coral", "forest", "harbor", "lantern", "maple",
        "meadow", "orchid", "pebble", "river", "silver", "spruce", "willow",
    ];
    let mut selected = Vec::with_capacity(3);
    for _ in 0..3 {
        let mut random = [0_u8; 1];
        OsRng.fill_bytes(&mut random);
        selected.push(WORDS[random[0] as usize % WORDS.len()]);
    }
    selected.join("-")
}

fn binding_summary_digest(
    environment: Environment,
    pc_display_name: &str,
    app_display_name: &str,
    safety_phrase: &str,
) -> String {
    sha256_hex(
        format!(
            "workagents-binding-summary-v1\n{}\n{}\n{}\n{}",
            environment.as_str(),
            pc_display_name,
            app_display_name,
            safety_phrase
        )
        .as_bytes(),
    )
}

struct RequestProof<'a> {
    method: &'a str,
    path: &'a str,
    environment: Environment,
    timestamp: &'a str,
    nonce: &'a str,
    body: &'a [u8],
    pc_device_id: Option<&'a str>,
    installation_id: Option<&'a str>,
}

struct PcAuthenticationContext<'a> {
    method: &'a str,
    path: &'a str,
    body: &'a [u8],
    now: DateTime<Utc>,
    allow_enrollment: bool,
    display_name: Option<&'a str>,
}

pub(super) struct GatewayEventValidation<'a> {
    pub(super) schema_version: &'a str,
    pub(super) message_type: &'a str,
    pub(super) expected_message_type: &'a str,
    pub(super) message_id: &'a str,
    pub(super) event_id: &'a str,
    pub(super) causation_id: Option<&'a str>,
    pub(super) environment: &'a str,
    pub(super) pc_device_id: &'a str,
    pub(super) installation_id: &'a str,
    pub(super) binding_epoch: i64,
    pub(super) state_version: i64,
    pub(super) sent_at: &'a str,
    pub(super) allow_previous_binding_epoch: bool,
    pub(super) now: DateTime<Utc>,
}

fn verify_request_signature(
    public_key_der: &[u8],
    domain: &str,
    proof: RequestProof<'_>,
    encoded_signature: &str,
) -> Result<(), CloudError> {
    let verifying_key = VerifyingKey::from_public_key_der(public_key_der)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(encoded_signature)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    let body_digest = sha256_hex(proof.body);
    let canonical = match (proof.pc_device_id, proof.installation_id) {
        (Some(pc_device_id), Some(installation_id)) => format!(
            "{domain}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            proof.method,
            proof.path,
            proof.environment.as_str(),
            pc_device_id,
            installation_id,
            proof.timestamp,
            proof.nonce,
            body_digest
        ),
        _ => format!(
            "{domain}\n{}\n{}\n{}\n{}\n{}\n{}",
            proof.method,
            proof.path,
            proof.environment.as_str(),
            proof.timestamp,
            proof.nonce,
            body_digest
        ),
    };
    verifying_key
        .verify(canonical.as_bytes(), &signature)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)
}

fn validate_request_authentication_values(
    timestamp: &str,
    nonce: &str,
    signature: &str,
    now: DateTime<Utc>,
) -> Result<(), CloudError> {
    validate_opaque_id(nonce).map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    let request_time =
        parse_timestamp(timestamp).map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    if (now - request_time).num_seconds().unsigned_abs() > REQUEST_CLOCK_SKEW_SECONDS as u64 {
        return Err(CloudError::DeviceAuthenticationFailed);
    }
    let signature_bytes = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)?;
    if signature_bytes.len() != 64 {
        return Err(CloudError::DeviceAuthenticationFailed);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn persist_nonce(
    connection: &rusqlite::Connection,
    environment: Environment,
    identity: &str,
    installation_id: Option<&str>,
    nonce: &str,
    timestamp: &str,
    now: DateTime<Utc>,
) -> Result<(), CloudError> {
    let nonce_digest = sha256_hex(nonce.as_bytes());
    let request_timestamp = parse_timestamp(timestamp)
        .map_err(|_| CloudError::DeviceAuthenticationFailed)?
        .timestamp();
    let exists = match installation_id {
        Some(installation_id) => connection
            .query_row(
                "SELECT 1 FROM pc_signature_nonces
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND nonce_digest = ?4",
                params![
                    environment.as_str(),
                    identity,
                    installation_id,
                    nonce_digest
                ],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .is_some(),
        None => connection
            .query_row(
                "SELECT 1 FROM device_signature_nonces
                 WHERE environment = ?1 AND device_key_id = ?2 AND nonce_digest = ?3",
                params![environment.as_str(), identity, nonce_digest],
                |_| Ok(()),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .is_some(),
    };
    if exists {
        return Err(CloudError::DeviceRequestReplayed);
    }
    let expires_at = now + Duration::seconds(REQUEST_CLOCK_SKEW_SECONDS);
    match installation_id {
        Some(installation_id) => connection.execute(
            "INSERT INTO pc_signature_nonces (
               environment, pc_device_id, installation_id, nonce_digest,
               request_timestamp, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                environment.as_str(),
                identity,
                installation_id,
                nonce_digest,
                request_timestamp,
                expires_at.timestamp()
            ],
        ),
        None => connection.execute(
            "INSERT INTO device_signature_nonces (
               environment, device_key_id, nonce_digest, request_timestamp, expires_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                environment.as_str(),
                identity,
                nonce_digest,
                request_timestamp,
                expires_at.timestamp()
            ],
        ),
    }
    .map_err(|_| CloudError::StorageUnavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::pkcs8::EncodePublicKey;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::TempDir;

    use crate::{
        registration_proof, DeviceChallengeRequest, DeviceRegistrationRequest, CONTRACT_VERSION,
    };

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-28T08:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn service(directory: &TempDir) -> CloudService {
        CloudService::open(
            &directory.path().join("remote.sqlite3"),
            Environment::Dev,
            [9_u8; 32],
        )
        .expect("service")
    }

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn encoded_public_key(signing_key: &SigningKey) -> String {
        URL_SAFE_NO_PAD.encode(
            signing_key
                .verifying_key()
                .to_public_key_der()
                .expect("SPKI")
                .as_bytes(),
        )
    }

    fn register_app(service: &CloudService, signing_key: &SigningKey) -> (String, String) {
        register_app_as(service, signing_key, "app_device_000001", "0001")
    }

    fn register_app_as(
        service: &CloudService,
        signing_key: &SigningKey,
        app_device_id: &str,
        suffix: &str,
    ) -> (String, String) {
        let app_device_id = app_device_id.to_owned();
        let public_key = encoded_public_key(signing_key);
        let challenge = service
            .create_device_challenge(
                DeviceChallengeRequest {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "app/device-registration-challenge".into(),
                    message_id: format!("app_challenge_{suffix}"),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    app_device_id: app_device_id.clone(),
                    device_key_algorithm: "ed25519".into(),
                    device_public_key: public_key.clone(),
                },
                now(),
            )
            .expect("challenge");
        let public_key_der = URL_SAFE_NO_PAD.decode(&public_key).expect("public key");
        let proof = registration_proof(
            Environment::Dev,
            &app_device_id,
            &challenge.challenge_id,
            &challenge.challenge,
            &sha256_hex(&public_key_der),
        );
        let signature = signing_key.sign(proof.as_bytes());
        let registered = service
            .register_device(
                DeviceRegistrationRequest {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "app/device-register".into(),
                    message_id: format!("app_registration_{suffix}"),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    app_device_id: app_device_id.clone(),
                    device_key_algorithm: "ed25519".into(),
                    device_public_key: public_key,
                    challenge_id: challenge.challenge_id,
                    challenge: challenge.challenge,
                    registration_signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
                    push_provider: "huawei_push_kit".into(),
                    push_token: format!("isolated-push-token-{suffix}"),
                    app_display_name: format!("测试手机{suffix}"),
                    app_version: "0.1.0-dev".into(),
                },
                now(),
            )
            .expect("registration");
        (app_device_id, registered.device_key_id)
    }

    fn pc_auth(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
    ) -> PcRequestAuthentication {
        pc_auth_at(signing_key, method, path, body, nonce, now())
    }

    fn pc_auth_at(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
        request_time: DateTime<Utc>,
    ) -> PcRequestAuthentication {
        pc_auth_for_at(
            signing_key,
            method,
            path,
            body,
            nonce,
            request_time,
            PcDeviceIdentity {
                pc_device_id: "pc_device_000001",
                installation_id: "installation_0001",
            },
        )
    }

    fn pc_auth_for(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
        pc_device_id: &str,
        installation_id: &str,
    ) -> PcRequestAuthentication {
        pc_auth_for_at(
            signing_key,
            method,
            path,
            body,
            nonce,
            now(),
            PcDeviceIdentity {
                pc_device_id,
                installation_id,
            },
        )
    }

    struct PcDeviceIdentity<'a> {
        pc_device_id: &'a str,
        installation_id: &'a str,
    }

    fn pc_auth_for_at(
        signing_key: &SigningKey,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
        request_time: DateTime<Utc>,
        identity: PcDeviceIdentity<'_>,
    ) -> PcRequestAuthentication {
        let timestamp = format_timestamp(request_time);
        let canonical = format!(
            "workagents-pc-request-v1\n{method}\n{path}\ndev\n{}\n{}\n{timestamp}\n{nonce}\n{}",
            identity.pc_device_id,
            identity.installation_id,
            sha256_hex(body)
        );
        PcRequestAuthentication {
            pc_device_id: identity.pc_device_id.into(),
            installation_id: identity.installation_id.into(),
            timestamp,
            nonce: nonce.into(),
            public_key: encoded_public_key(signing_key),
            signature: URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes()),
        }
    }

    fn app_auth(
        signing_key: &SigningKey,
        device_key_id: &str,
        method: &str,
        path: &str,
        body: &[u8],
        nonce: &str,
    ) -> AppRequestAuthentication {
        let timestamp = format_timestamp(now());
        let canonical = format!(
            "workagents-device-request-v1\n{method}\n{path}\ndev\n{timestamp}\n{nonce}\n{}",
            sha256_hex(body)
        );
        AppRequestAuthentication {
            device_key_id: device_key_id.into(),
            timestamp,
            nonce: nonce.into(),
            signature: URL_SAFE_NO_PAD.encode(signing_key.sign(canonical.as_bytes()).to_bytes()),
        }
    }

    fn register_pairing(service: &CloudService, pc_key: &SigningKey) -> PairingRegistrationRequest {
        register_pairing_as(service, pc_key, "0001")
    }

    fn register_pairing_as(
        service: &CloudService,
        pc_key: &SigningKey,
        suffix: &str,
    ) -> PairingRegistrationRequest {
        register_pairing_for(
            service,
            pc_key,
            suffix,
            "pc_device_000001",
            "installation_0001",
            "开发电脑",
        )
    }

    fn register_pairing_for(
        service: &CloudService,
        pc_key: &SigningKey,
        suffix: &str,
        pc_device_id: &str,
        installation_id: &str,
        pc_display_name: &str,
    ) -> PairingRegistrationRequest {
        let request = PairingRegistrationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/register".into(),
            message_id: format!("pair_register_{suffix}"),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            pc_device_id: pc_device_id.into(),
            installation_id: installation_id.into(),
            pairing: PairingQrPayload {
                pairing_qr_version: "2".into(),
                environment: "dev".into(),
                pairing_handle: format!("pairing_handle_{suffix}"),
            },
            pc_display_name: pc_display_name.into(),
            expires_at: format_timestamp(now() + Duration::minutes(10)),
        };
        let body = serde_json::to_vec(&request).expect("request body");
        service
            .register_pairing(
                serde_json::from_slice(&body).expect("request"),
                pc_auth_for(
                    pc_key,
                    "POST",
                    "/v1/gateway/pairings",
                    &body,
                    &format!("pc_pairing_nonce_{suffix}"),
                    pc_device_id,
                    installation_id,
                ),
                &body,
                now(),
            )
            .expect("pairing registration");
        request
    }

    #[test]
    fn pairing_registration_accepts_allowed_positive_clock_skew_and_clamps_expiry() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(31);
        let client_now = now() + Duration::seconds(REQUEST_CLOCK_SKEW_SECONDS - 1);
        let request = PairingRegistrationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/register".into(),
            message_id: "pair_register_clock_skew_01".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(client_now),
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_0001".into(),
            pairing: PairingQrPayload {
                pairing_qr_version: "2".into(),
                environment: "dev".into(),
                pairing_handle: "pairing_handle_clock_skew_01".into(),
            },
            pc_display_name: "时钟偏差电脑".into(),
            expires_at: format_timestamp(client_now + Duration::minutes(PAIRING_TTL_MINUTES)),
        };
        let body = serde_json::to_vec(&request).expect("request body");

        let response = cloud
            .register_pairing(
                serde_json::from_slice(&body).expect("request"),
                pc_auth_at(
                    &pc_key,
                    "POST",
                    "/v1/gateway/pairings",
                    &body,
                    "pc_pairing_clock_skew_nonce_01",
                    client_now,
                ),
                &body,
                now(),
            )
            .expect("clock-skewed registration");

        assert_eq!(
            response.expires_at,
            format_timestamp(now() + Duration::minutes(PAIRING_TTL_MINUTES))
        );
    }

    #[test]
    fn pairing_registration_rejects_ttl_longer_than_signed_request_window() {
        let request = PairingRegistrationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/register".into(),
            message_id: "pair_register_ttl_too_long_01".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_0001".into(),
            pairing: PairingQrPayload {
                pairing_qr_version: "2".into(),
                environment: "dev".into(),
                pairing_handle: "pairing_handle_ttl_too_long_01".into(),
            },
            pc_display_name: "超长有效期电脑".into(),
            expires_at: format_timestamp(
                now() + Duration::minutes(PAIRING_TTL_MINUTES) + Duration::seconds(1),
            ),
        };

        assert!(matches!(
            validate_pairing_registration(&request, Environment::Dev, now()),
            Err(CloudError::PairingExpired)
        ));
    }

    fn consume_pairing_as(
        service: &CloudService,
        app_key: &SigningKey,
        device_key_id: &str,
        app_device_id: &str,
        pairing: PairingQrPayload,
        suffix: &str,
    ) -> PairingPendingResponse {
        let request = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: format!("pair_consume_{suffix}"),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: app_device_id.into(),
            pairing,
        };
        let body = serde_json::to_vec(&request).expect("consume body");
        service
            .consume_pairing(
                serde_json::from_slice(&body).expect("consume request"),
                app_auth(
                    app_key,
                    device_key_id,
                    "POST",
                    "/v1/pairings/consume",
                    &body,
                    &format!("app_pairing_nonce_{suffix}"),
                ),
                &body,
                now(),
            )
            .expect("pairing consumption")
    }

    fn confirm_pending_as(
        service: &CloudService,
        gateway: &GatewayIdentity,
        pending: PairingPendingResponse,
        suffix: &str,
    ) -> BindingActiveResponse {
        service
            .confirm_binding(
                gateway,
                BindingLocalConfirmation {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "binding/local-confirm".into(),
                    message_id: format!("local_confirm_{suffix}"),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    binding_id: pending.binding_id,
                    pc_device_id: gateway.pc_device_id.clone(),
                    installation_id: gateway.installation_id.clone(),
                    confirmation_nonce: pending.confirmation_nonce,
                    summary_digest: pending.binding_summary.summary_digest,
                    confirmed: true,
                },
                now(),
            )
            .expect("active binding")
    }

    #[test]
    fn signed_pairing_survives_restart_and_requires_local_confirmation() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(1);
        let app_key = signing_key(2);
        let (app_device_id, device_key_id) = register_app(&cloud, &app_key);
        let registered = register_pairing(&cloud, &pc_key);
        let pc_pairing_message_id = registered.message_id.clone();
        let pairing_payload = registered.pairing.clone();
        let consume = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: app_device_id.clone(),
            pairing: pairing_payload.clone(),
        };
        let body = serde_json::to_vec(&consume).expect("consume body");
        let pending = cloud
            .consume_pairing(
                serde_json::from_slice(&body).expect("consume"),
                app_auth(
                    &app_key,
                    &device_key_id,
                    "POST",
                    "/v1/pairings/consume",
                    &body,
                    "app_pairing_nonce_1",
                ),
                &body,
                now(),
            )
            .expect("pending binding");
        assert_eq!(pending.binding_state, "pending_local_confirmation");

        let retry = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_retry_001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: app_device_id.clone(),
            pairing: pairing_payload,
        };
        let retry_body = serde_json::to_vec(&retry).expect("retry body");
        let recovered_pending = cloud
            .consume_pairing(
                serde_json::from_slice(&retry_body).expect("retry consume"),
                app_auth(
                    &app_key,
                    &device_key_id,
                    "POST",
                    "/v1/pairings/consume",
                    &retry_body,
                    "app_pairing_nonce_retry_1",
                ),
                &retry_body,
                now(),
            )
            .expect("idempotent pending binding");
        assert_eq!(recovered_pending.binding_id, pending.binding_id);
        assert_eq!(
            recovered_pending.binding_state,
            "pending_local_confirmation"
        );

        drop(cloud);
        let reopened = service(&directory);
        let gateway = reopened
            .authenticate_gateway(
                pc_auth(
                    &pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_nonce_01",
                ),
                now(),
            )
            .expect("gateway authentication");
        reopened
            .accept_gateway_hello(
                &gateway,
                &PcHello {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "pc/hello".into(),
                    message_id: "gateway_hello_0001".into(),
                    event_id: "gateway_hello_event_0001".into(),
                    causation_id: None,
                    environment: "dev".into(),
                    pc_device_id: gateway.pc_device_id.clone(),
                    installation_id: gateway.installation_id.clone(),
                    binding_epoch: 1,
                    state_version: 1,
                    sent_at: format_timestamp(now()),
                    supported_schema_versions: vec![CONTRACT_VERSION.into()],
                    last_ack_event_id: None,
                    last_ack_state_version: 0,
                },
                now(),
            )
            .expect("gateway hello");
        let confirmations = reopened
            .pending_binding_confirmations(&gateway, now())
            .expect("confirmations");
        assert_eq!(confirmations.len(), 1);
        assert_eq!(confirmations[0].binding_summary, pending.binding_summary);
        assert_eq!(
            confirmations[0].pc_pairing_message_id,
            pc_pairing_message_id
        );
        assert_eq!(confirmations[0].request_message_id, pc_pairing_message_id);
        let active = reopened
            .confirm_binding(
                &gateway,
                BindingLocalConfirmation {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "binding/local-confirm".into(),
                    message_id: "local_confirm_001".into(),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    binding_id: pending.binding_id,
                    pc_device_id: gateway.pc_device_id.clone(),
                    installation_id: gateway.installation_id.clone(),
                    confirmation_nonce: pending.confirmation_nonce,
                    summary_digest: pending.binding_summary.summary_digest,
                    confirmed: true,
                },
                now(),
            )
            .expect("active binding");
        assert_eq!(active.binding_state, "active");
        assert_eq!(active.binding_epoch, 1);
        let recovered = reopened
            .active_binding_for_gateway(&gateway, now())
            .expect("active binding recovery")
            .expect("active binding");
        assert_eq!(recovered.binding_id, active.binding_id);
        assert_eq!(recovered.binding_epoch, active.binding_epoch);

        let query = PcDeviceListQuery {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "app/pc-devices-query".into(),
            message_id: "pc_devices_query_001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id,
        };
        let canonical_path = "/v1/pc-devices?request=pc_devices_query_001";
        let devices = reopened
            .list_pc_devices(
                query,
                app_auth(
                    &app_key,
                    &device_key_id,
                    "GET",
                    canonical_path,
                    &[],
                    "app_pc_devices_nonce_1",
                ),
                canonical_path,
                now(),
            )
            .expect("active pc device list");
        assert_eq!(devices.pc_devices.len(), 1);
        assert_eq!(devices.pc_devices[0].pc_connection_state, "online");
        assert_eq!(devices.pc_devices[0].binding_epoch, 1);
    }

    #[test]
    fn consumed_qr_is_rejected_for_a_different_registered_phone() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(4);
        let first_app_key = signing_key(5);
        let second_app_key = signing_key(6);
        let (first_app_id, first_key_id) = register_app(&cloud, &first_app_key);
        let (second_app_id, second_key_id) =
            register_app_as(&cloud, &second_app_key, "app_device_000002", "0002");
        let pairing = register_pairing(&cloud, &pc_key).pairing;

        let first = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_first_01".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: first_app_id,
            pairing: pairing.clone(),
        };
        let first_body = serde_json::to_vec(&first).expect("first body");
        cloud
            .consume_pairing(
                serde_json::from_slice(&first_body).expect("first consume"),
                app_auth(
                    &first_app_key,
                    &first_key_id,
                    "POST",
                    "/v1/pairings/consume",
                    &first_body,
                    "first_app_pair_nonce_1",
                ),
                &first_body,
                now(),
            )
            .expect("first consume accepted");

        let second = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_second_1".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: second_app_id,
            pairing,
        };
        let second_body = serde_json::to_vec(&second).expect("second body");
        let replay = cloud.consume_pairing(
            serde_json::from_slice(&second_body).expect("second consume"),
            app_auth(
                &second_app_key,
                &second_key_id,
                "POST",
                "/v1/pairings/consume",
                &second_body,
                "second_app_pair_nonce",
            ),
            &second_body,
            now(),
        );
        assert!(matches!(replay, Err(CloudError::PairingReplayed)));
    }

    #[test]
    fn expired_qr_is_distinguished_from_invalid_and_replayed_handles() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(7);
        let app_key = signing_key(8);
        let (app_device_id, device_key_id) = register_app(&cloud, &app_key);
        let pairing = register_pairing(&cloud, &pc_key).pairing;
        cloud
            .connection()
            .expect("connection")
            .execute(
                "UPDATE pairings SET expires_at = ?1 WHERE pairing_handle_digest = ?2",
                params![
                    now().timestamp() - 1,
                    sha256_hex(pairing.pairing_handle.as_bytes())
                ],
            )
            .expect("expire pairing");
        let consume = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_expired".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id,
            pairing,
        };
        let body = serde_json::to_vec(&consume).expect("body");
        let expired = cloud.consume_pairing(
            serde_json::from_slice(&body).expect("consume"),
            app_auth(
                &app_key,
                &device_key_id,
                "POST",
                "/v1/pairings/consume",
                &body,
                "expired_pair_nonce_01",
            ),
            &body,
            now(),
        );
        assert!(matches!(expired, Err(CloudError::PairingExpired)));
    }

    #[test]
    fn new_pairing_registration_supersedes_pending_and_unconsumed_predecessors() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(9);
        let app_key = signing_key(10);
        let (app_device_id, device_key_id) = register_app(&cloud, &app_key);
        let first = register_pairing_as(&cloud, &pc_key, "supersede_0001");
        let pending = consume_pairing_as(
            &cloud,
            &app_key,
            &device_key_id,
            &app_device_id,
            first.pairing.clone(),
            "supersede_0001",
        );
        let second = register_pairing_as(&cloud, &pc_key, "supersede_0002");
        let gateway = cloud
            .authenticate_gateway(
                pc_auth(
                    &pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_supersede_01",
                ),
                now(),
            )
            .expect("gateway authentication");

        assert!(cloud
            .pending_binding_confirmations(&gateway, now())
            .expect("pending confirmations")
            .is_empty());

        let replay_request = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_superseded_replay".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: app_device_id.clone(),
            pairing: first.pairing,
        };
        let replay_body = serde_json::to_vec(&replay_request).expect("replay body");
        let replay = cloud.consume_pairing(
            serde_json::from_slice(&replay_body).expect("replay request"),
            app_auth(
                &app_key,
                &device_key_id,
                "POST",
                "/v1/pairings/consume",
                &replay_body,
                "app_pairing_superseded_replay",
            ),
            &replay_body,
            now(),
        );
        assert!(matches!(replay, Err(CloudError::PairingReplayed)));

        let stale_confirmation = cloud.confirm_binding(
            &gateway,
            BindingLocalConfirmation {
                schema_version: CONTRACT_VERSION.into(),
                message_type: "binding/local-confirm".into(),
                message_id: "local_confirm_superseded".into(),
                environment: "dev".into(),
                sent_at: format_timestamp(now()),
                binding_id: pending.binding_id,
                pc_device_id: gateway.pc_device_id.clone(),
                installation_id: gateway.installation_id.clone(),
                confirmation_nonce: pending.confirmation_nonce,
                summary_digest: pending.binding_summary.summary_digest,
                confirmed: true,
            },
            now(),
        );
        assert!(matches!(
            stale_confirmation,
            Err(CloudError::DeviceNotBound)
        ));

        register_pairing_as(&cloud, &pc_key, "supersede_0003");
        let stale_qr_request = PairingConsumeRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/consume".into(),
            message_id: "pair_consume_stale_unconsumed".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id,
            pairing: second.pairing,
        };
        let stale_qr_body = serde_json::to_vec(&stale_qr_request).expect("stale QR body");
        let stale_qr = cloud.consume_pairing(
            serde_json::from_slice(&stale_qr_body).expect("stale QR request"),
            app_auth(
                &app_key,
                &device_key_id,
                "POST",
                "/v1/pairings/consume",
                &stale_qr_body,
                "app_pairing_stale_unconsumed",
            ),
            &stale_qr_body,
            now(),
        );
        assert!(matches!(stale_qr, Err(CloudError::PairingQrInvalid)));
    }

    #[test]
    fn new_pairing_registration_preserves_existing_active_binding() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(11);
        let app_key = signing_key(12);
        let (app_device_id, device_key_id) = register_app(&cloud, &app_key);
        let pairing = register_pairing_as(&cloud, &pc_key, "active_0001");
        let pending = consume_pairing_as(
            &cloud,
            &app_key,
            &device_key_id,
            &app_device_id,
            pairing.pairing,
            "active_0001",
        );
        let gateway = cloud
            .authenticate_gateway(
                pc_auth(
                    &pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_active_0001",
                ),
                now(),
            )
            .expect("gateway authentication");
        let active = cloud
            .confirm_binding(
                &gateway,
                BindingLocalConfirmation {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "binding/local-confirm".into(),
                    message_id: "local_confirm_active_0001".into(),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    binding_id: pending.binding_id,
                    pc_device_id: gateway.pc_device_id.clone(),
                    installation_id: gateway.installation_id.clone(),
                    confirmation_nonce: pending.confirmation_nonce,
                    summary_digest: pending.binding_summary.summary_digest,
                    confirmed: true,
                },
                now(),
            )
            .expect("active binding");

        let replacement_pairing = register_pairing_as(&cloud, &pc_key, "active_0002");

        let recovered = cloud
            .active_binding_for_gateway(&gateway, now())
            .expect("active binding recovery")
            .expect("active binding");
        assert_eq!(recovered.binding_id, active.binding_id);
        assert_eq!(recovered.binding_epoch, active.binding_epoch);

        let replacement_pending = consume_pairing_as(
            &cloud,
            &app_key,
            &device_key_id,
            &app_device_id,
            replacement_pairing.pairing,
            "active_0002",
        );
        let replacement = cloud
            .confirm_binding(
                &gateway,
                BindingLocalConfirmation {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "binding/local-confirm".into(),
                    message_id: "local_confirm_active_0002".into(),
                    environment: "dev".into(),
                    sent_at: format_timestamp(now()),
                    binding_id: replacement_pending.binding_id,
                    pc_device_id: gateway.pc_device_id.clone(),
                    installation_id: gateway.installation_id.clone(),
                    confirmation_nonce: replacement_pending.confirmation_nonce,
                    summary_digest: replacement_pending.binding_summary.summary_digest,
                    confirmed: true,
                },
                now(),
            )
            .expect("replacement active binding");
        let replaced = cloud
            .active_binding_for_gateway(&gateway, now())
            .expect("replacement binding recovery")
            .expect("replacement active binding");
        assert_eq!(replaced.binding_id, replacement.binding_id);
        assert_eq!(replaced.binding_epoch, active.binding_epoch + 1);
    }

    #[test]
    fn same_display_name_replaces_the_active_device_and_revocation_removes_it() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let app_key = signing_key(41);
        let first_pc_key = signing_key(42);
        let replacement_pc_key = signing_key(43);
        let (app_device_id, device_key_id) = register_app(&cloud, &app_key);

        let first_pairing = register_pairing_for(
            &cloud,
            &first_pc_key,
            "same_name_first",
            "pc_device_000001",
            "installation_0001",
            "同名电脑",
        );
        let first_pending = consume_pairing_as(
            &cloud,
            &app_key,
            &device_key_id,
            &app_device_id,
            first_pairing.pairing,
            "same_name_first",
        );
        let first_gateway = cloud
            .authenticate_gateway(
                pc_auth_for(
                    &first_pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_same_name_first",
                    "pc_device_000001",
                    "installation_0001",
                ),
                now(),
            )
            .expect("first gateway authentication");
        let first_active =
            confirm_pending_as(&cloud, &first_gateway, first_pending, "same_name_first");

        let replacement_pairing = register_pairing_for(
            &cloud,
            &replacement_pc_key,
            "same_name_replacement",
            "pc_device_000002",
            "installation_0002",
            "同名电脑",
        );
        let replacement_pending = consume_pairing_as(
            &cloud,
            &app_key,
            &device_key_id,
            &app_device_id,
            replacement_pairing.pairing,
            "same_name_replacement",
        );
        let replacement_gateway = cloud
            .authenticate_gateway(
                pc_auth_for(
                    &replacement_pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_same_name_replacement",
                    "pc_device_000002",
                    "installation_0002",
                ),
                now(),
            )
            .expect("replacement gateway authentication");
        let replacement_active = confirm_pending_as(
            &cloud,
            &replacement_gateway,
            replacement_pending,
            "same_name_replacement",
        );

        let connection = cloud.connection().expect("connection");
        let first_state: String = connection
            .query_row(
                "SELECT state FROM bindings WHERE binding_id = ?1",
                params![first_active.binding_id],
                |row| row.get(0),
            )
            .expect("first binding state");
        assert_eq!(first_state, "superseded");
        let active_bindings: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM bindings WHERE environment = 'dev' AND app_device_id = ?1 AND state = 'active'",
                params![app_device_id],
                |row| row.get(0),
            )
            .expect("active binding count");
        assert_eq!(active_bindings, 1);
        drop(connection);

        let revocation = BindingRevocationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "binding/revoke".into(),
            message_id: "binding_revoke_same_name_01".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            app_device_id: app_device_id.clone(),
            binding_id: replacement_active.binding_id.clone(),
        };
        let body = serde_json::to_vec(&revocation).expect("revocation body");
        let canonical_path = format!("/v1/bindings/{}/revoke", replacement_active.binding_id);
        let revoked = cloud
            .revoke_binding(
                revocation,
                app_auth(
                    &app_key,
                    &device_key_id,
                    "POST",
                    &canonical_path,
                    &body,
                    "app_binding_revoke_same_name",
                ),
                &canonical_path,
                &body,
                now(),
            )
            .expect("binding revoked");
        assert_eq!(revoked.binding_id, replacement_active.binding_id);

        let connection = cloud.connection().expect("connection");
        let replacement_state: String = connection
            .query_row(
                "SELECT state FROM bindings WHERE binding_id = ?1",
                params![replacement_active.binding_id],
                |row| row.get(0),
            )
            .expect("replacement binding state");
        assert_eq!(replacement_state, "revoked");
    }

    #[test]
    fn existing_device_can_rebind_when_pc_is_at_distinct_device_limit() {
        let directory = TempDir::new().expect("temp");
        let cloud = service(&directory);
        let pc_key = signing_key(21);
        let app_key_one = signing_key(22);
        let app_key_two = signing_key(23);
        let app_key_three = signing_key(24);
        let (app_id_one, device_key_one) =
            register_app_as(&cloud, &app_key_one, "app_device_limit_01", "limit_01");
        let (app_id_two, device_key_two) =
            register_app_as(&cloud, &app_key_two, "app_device_limit_02", "limit_02");
        let (app_id_three, device_key_three) =
            register_app_as(&cloud, &app_key_three, "app_device_limit_03", "limit_03");

        let pairing_one = register_pairing_as(&cloud, &pc_key, "limit_01");
        let gateway = cloud
            .authenticate_gateway(
                pc_auth(
                    &pc_key,
                    "GET",
                    "/v1/gateway/connect",
                    &[],
                    "pc_gateway_limit_01",
                ),
                now(),
            )
            .expect("gateway authentication");
        let active_one = confirm_pending_as(
            &cloud,
            &gateway,
            consume_pairing_as(
                &cloud,
                &app_key_one,
                &device_key_one,
                &app_id_one,
                pairing_one.pairing,
                "limit_01",
            ),
            "limit_01",
        );

        let pairing_two = register_pairing_as(&cloud, &pc_key, "limit_02");
        confirm_pending_as(
            &cloud,
            &gateway,
            consume_pairing_as(
                &cloud,
                &app_key_two,
                &device_key_two,
                &app_id_two,
                pairing_two.pairing,
                "limit_02",
            ),
            "limit_02",
        );
        let pairing_three = register_pairing_as(&cloud, &pc_key, "limit_03");
        confirm_pending_as(
            &cloud,
            &gateway,
            consume_pairing_as(
                &cloud,
                &app_key_three,
                &device_key_three,
                &app_id_three,
                pairing_three.pairing,
                "limit_03",
            ),
            "limit_03",
        );

        let replacement_pairing = register_pairing_as(&cloud, &pc_key, "limit_rebind_01");
        let replacement = confirm_pending_as(
            &cloud,
            &gateway,
            consume_pairing_as(
                &cloud,
                &app_key_one,
                &device_key_one,
                &app_id_one,
                replacement_pairing.pairing,
                "limit_rebind_01",
            ),
            "limit_rebind_01",
        );

        assert_ne!(replacement.binding_id, active_one.binding_id);
        assert_eq!(replacement.binding_epoch, active_one.binding_epoch + 3);
        let connection = cloud.connection().expect("connection");
        let active_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM bindings
                 WHERE environment = 'dev' AND pc_device_id = ?1 AND state = 'active'",
                params![gateway.pc_device_id],
                |row| row.get(0),
            )
            .expect("active binding count");
        assert_eq!(active_count, MAX_PC_BINDINGS);
    }

    #[test]
    fn pc_authentication_nonce_replay_is_rejected() {
        let directory = TempDir::new().expect("temp");
        let service = service(&directory);
        let pc_key = signing_key(3);
        let request = PairingRegistrationRequest {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "pairing/register".into(),
            message_id: "pair_register_0001".into(),
            environment: "dev".into(),
            sent_at: format_timestamp(now()),
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_0001".into(),
            pairing: PairingQrPayload {
                pairing_qr_version: "2".into(),
                environment: "dev".into(),
                pairing_handle: "pairing_handle_001".into(),
            },
            pc_display_name: "开发电脑".into(),
            expires_at: format_timestamp(now() + Duration::minutes(10)),
        };
        let body = serde_json::to_vec(&request).expect("body");
        let auth = pc_auth(
            &pc_key,
            "POST",
            "/v1/gateway/pairings",
            &body,
            "pc_replayed_nonce_1",
        );
        service
            .register_pairing(
                serde_json::from_slice(&body).expect("request"),
                auth.clone(),
                &body,
                now(),
            )
            .expect("first request");
        let replay = service.register_pairing(
            serde_json::from_slice(&body).expect("request"),
            auth,
            &body,
            now(),
        );
        assert!(matches!(replay, Err(CloudError::DeviceRequestReplayed)));
    }
}
