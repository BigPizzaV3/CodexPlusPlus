use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::pairing::GatewayEventValidation;
use super::{
    format_timestamp, random_opaque_id, validate_opaque_id, validate_request_base,
    AppRequestAuthentication, CloudError, CloudService, GatewayIdentity, RequestBase,
    CONTRACT_VERSION,
};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

fn task_list_cursor(state_version: i64, remote_task_id: &str) -> String {
    format!("{state_version}:{remote_task_id}")
}

fn parse_task_list_cursor(value: &str) -> Result<(i64, String), CloudError> {
    if value.is_empty() || value.len() > 256 {
        return Err(CloudError::InvalidRequest);
    }
    let (state_version_text, remote_task_id) =
        value.split_once(':').ok_or(CloudError::InvalidRequest)?;
    if state_version_text.is_empty()
        || !state_version_text.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(CloudError::InvalidRequest);
    }
    let state_version = state_version_text
        .parse::<i64>()
        .map_err(|_| CloudError::InvalidRequest)?;
    if !(1..=MAX_SAFE_INTEGER).contains(&state_version)
        || state_version.to_string() != state_version_text
    {
        return Err(CloudError::InvalidRequest);
    }
    validate_opaque_id(remote_task_id)?;
    Ok((state_version, remote_task_id.to_owned()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteReply {
    pub state: String,
    pub text: Option<String>,
    pub byte_length: i64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskSnapshot {
    pub remote_task_id: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub name: String,
    pub workspace_name: String,
    pub model_label: String,
    pub task_status: String,
    pub turn_status: String,
    pub last_turn_outcome: String,
    pub last_reply: Option<RemoteReply>,
    pub last_reply_state: String,
    pub last_reply_version: Option<i64>,
    pub last_error: Option<String>,
    pub pc_observed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_received_at: Option<String>,
    pub state_version: i64,
    pub pc_connection_state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotUpsert {
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
    #[serde(default)]
    pub terminal_push_eligible: bool,
    pub snapshot: TaskSnapshot,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotTombstone {
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
    pub remote_task_id: String,
    pub reason: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAck {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub environment: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub ack_event_id: String,
    pub ack_state_version: i64,
    pub server_received_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskListQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub pc_device_id: String,
    pub cursor: Option<String>,
    pub limit: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub tasks: Vec<TaskSnapshot>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskDetailQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
    pub remote_task_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetailResponse {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub request_message_id: String,
    pub environment: String,
    pub server_received_at: String,
    pub snapshot: TaskSnapshot,
}

impl CloudService {
    pub fn accept_gateway_snapshot(
        &self,
        identity: &GatewayIdentity,
        mut request: SnapshotUpsert,
        now: DateTime<Utc>,
    ) -> Result<SyncAck, CloudError> {
        self.validate_gateway_event(
            identity,
            GatewayEventValidation {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "snapshot/upsert",
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
        validate_snapshot(&request.snapshot, &request, now)?;

        let mut connection = self.connection()?;
        let transaction = connection
            .transaction()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let previous_snapshot = transaction
            .query_row(
                "SELECT snapshot_json FROM remote_task_snapshots
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND binding_epoch = ?4 AND remote_task_id = ?5",
                params![
                    self.environment().as_str(),
                    request.pc_device_id,
                    request.installation_id,
                    request.binding_epoch,
                    request.snapshot.remote_task_id
                ],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .flatten()
            .map(|value| {
                serde_json::from_str::<TaskSnapshot>(&value)
                    .map_err(|_| CloudError::StorageUnavailable)
            })
            .transpose()?;
        request.snapshot.server_received_at = Some(format_timestamp(now));
        let snapshot_json =
            serde_json::to_string(&request.snapshot).map_err(|_| CloudError::StorageUnavailable)?;
        let applied = transaction
            .execute(
                "INSERT INTO remote_task_snapshots (
                   environment, pc_device_id, installation_id, binding_epoch, remote_task_id,
                   state_version, last_event_id, snapshot_json, tombstoned, server_received_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)
                 ON CONFLICT(environment, pc_device_id, installation_id, binding_epoch, remote_task_id)
                 DO UPDATE SET
                   state_version = excluded.state_version,
                   last_event_id = excluded.last_event_id,
                   snapshot_json = excluded.snapshot_json,
                   tombstoned = 0,
                   server_received_at = excluded.server_received_at
                 WHERE excluded.state_version > remote_task_snapshots.state_version",
                params![
                    self.environment().as_str(),
                    request.pc_device_id,
                    request.installation_id,
                    request.binding_epoch,
                    request.snapshot.remote_task_id,
                    request.state_version,
                    request.event_id,
                    snapshot_json,
                    now.timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let enqueued_pushes = if applied > 0 {
            crate::enqueue_terminal_pushes(
                &transaction,
                self.environment().as_str(),
                previous_snapshot.as_ref(),
                &request.snapshot,
                request.terminal_push_eligible,
                now,
            )?
        } else {
            0
        };
        transaction
            .commit()
            .map_err(|_| CloudError::StorageUnavailable)?;
        if enqueued_pushes > 0 {
            println!("workagents_push event=outbox_enqueued count={enqueued_pushes}");
        }
        drop(connection);
        self.record_gateway_observation(identity, now)?;
        Ok(sync_ack(
            self,
            identity,
            request.binding_epoch,
            request.event_id,
            request.state_version,
            now,
        ))
    }

    pub fn accept_gateway_tombstone(
        &self,
        identity: &GatewayIdentity,
        request: SnapshotTombstone,
        now: DateTime<Utc>,
    ) -> Result<SyncAck, CloudError> {
        self.validate_gateway_event(
            identity,
            GatewayEventValidation {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "snapshot/tombstone",
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
        validate_opaque_id(&request.remote_task_id)?;
        if !matches!(
            request.reason.as_str(),
            "deleted" | "archived" | "remote_disabled" | "unbound" | "installation_reset"
        ) {
            return Err(CloudError::InvalidRequest);
        }
        self.connection()?
            .execute(
                "INSERT INTO remote_task_snapshots (
                   environment, pc_device_id, installation_id, binding_epoch, remote_task_id,
                   state_version, last_event_id, snapshot_json, tombstoned, server_received_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 1, ?8)
                 ON CONFLICT(environment, pc_device_id, installation_id, binding_epoch, remote_task_id)
                 DO UPDATE SET
                   state_version = excluded.state_version,
                   last_event_id = excluded.last_event_id,
                   snapshot_json = NULL,
                   tombstoned = 1,
                   server_received_at = excluded.server_received_at
                 WHERE excluded.state_version > remote_task_snapshots.state_version",
                params![
                    self.environment().as_str(),
                    request.pc_device_id,
                    request.installation_id,
                    request.binding_epoch,
                    request.remote_task_id,
                    request.state_version,
                    request.event_id,
                    now.timestamp()
                ],
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        self.record_gateway_observation(identity, now)?;
        Ok(sync_ack(
            self,
            identity,
            request.binding_epoch,
            request.event_id,
            request.state_version,
            now,
        ))
    }

    pub fn list_task_snapshots(
        &self,
        path_pc_device_id: &str,
        request: TaskListQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<TaskListResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/task-list-query",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment(),
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        validate_opaque_id(path_pc_device_id)?;
        let cursor = request
            .cursor
            .as_deref()
            .map(parse_task_list_cursor)
            .transpose()?;
        if authenticated_app_device_id != request.app_device_id
            || request.pc_device_id != path_pc_device_id
            || !(1..=100).contains(&request.limit.unwrap_or(100))
        {
            return Err(CloudError::InvalidRequest);
        }

        let connection = self.connection()?;
        let binding = connection
            .query_row(
                "SELECT bindings.installation_id, bindings.binding_epoch,
                        pc_devices.last_gateway_observed_at
                 FROM bindings
                 INNER JOIN pc_devices
                   ON pc_devices.environment = bindings.environment
                  AND pc_devices.pc_device_id = bindings.pc_device_id
                  AND pc_devices.installation_id = bindings.installation_id
                 WHERE bindings.environment = ?1 AND bindings.app_device_id = ?2
                   AND bindings.pc_device_id = ?3 AND bindings.state = 'active'
                 ORDER BY bindings.activated_at DESC LIMIT 1",
                params![
                    self.environment().as_str(),
                    request.app_device_id,
                    path_pc_device_id
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::DeviceNotBound)?;
        let pc_connection_state = connection_state(binding.2, now)?;
        let limit = i64::from(request.limit.unwrap_or(100));
        let cursor_version = cursor.as_ref().map(|value| value.0);
        let cursor_task_id = cursor.as_ref().map(|value| value.1.as_str());
        let mut statement = connection
            .prepare(
                "SELECT snapshot_json FROM remote_task_snapshots
                 WHERE environment = ?1 AND pc_device_id = ?2 AND installation_id = ?3
                   AND binding_epoch = ?4 AND tombstoned = 0
                   AND (?5 IS NULL OR state_version < ?5
                     OR (state_version = ?5 AND remote_task_id > ?6))
                 ORDER BY state_version DESC, remote_task_id LIMIT ?7",
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let rows = statement
            .query_map(
                params![
                    self.environment().as_str(),
                    path_pc_device_id,
                    binding.0,
                    binding.1,
                    cursor_version,
                    cursor_task_id,
                    limit + 1
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| CloudError::StorageUnavailable)?;
        let mut tasks = Vec::new();
        for row in rows {
            let mut snapshot: TaskSnapshot =
                serde_json::from_str(&row.map_err(|_| CloudError::StorageUnavailable)?)
                    .map_err(|_| CloudError::StorageUnavailable)?;
            snapshot.pc_connection_state = pc_connection_state.to_owned();
            tasks.push(snapshot);
        }
        let has_more = tasks.len() > limit as usize;
        if has_more {
            tasks.pop();
        }
        let next_cursor = if has_more {
            tasks
                .last()
                .map(|snapshot| task_list_cursor(snapshot.state_version, &snapshot.remote_task_id))
        } else {
            None
        };
        drop(statement);
        drop(connection);

        Ok(TaskListResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/task-list",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment().as_str().to_owned(),
            server_received_at: format_timestamp(now),
            pc_device_id: path_pc_device_id.to_owned(),
            installation_id: binding.0,
            binding_epoch: binding.1,
            tasks,
            next_cursor,
        })
    }

    pub fn task_snapshot(
        &self,
        path_remote_task_id: &str,
        request: TaskDetailQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<TaskDetailResponse, CloudError> {
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/task-query",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment(),
            now,
        )?;
        validate_opaque_id(&request.app_device_id)?;
        validate_opaque_id(path_remote_task_id)?;
        if authenticated_app_device_id != request.app_device_id
            || request.remote_task_id != path_remote_task_id
        {
            return Err(CloudError::InvalidRequest);
        }
        let connection = self.connection()?;
        let (snapshot_json, last_gateway_observed_at): (String, Option<i64>) = connection
            .query_row(
                "SELECT s.snapshot_json, p.last_gateway_observed_at
                 FROM bindings b
                 INNER JOIN pc_devices p
                   ON p.environment = b.environment
                  AND p.pc_device_id = b.pc_device_id
                  AND p.installation_id = b.installation_id
                 INNER JOIN remote_task_snapshots s
                   ON s.environment = b.environment
                  AND s.pc_device_id = b.pc_device_id
                  AND s.installation_id = b.installation_id
                  AND s.binding_epoch = b.binding_epoch
                 WHERE b.environment = ?1 AND b.app_device_id = ?2 AND b.state = 'active'
                   AND s.remote_task_id = ?3 AND s.tombstoned = 0
                 ORDER BY b.activated_at DESC LIMIT 1",
                params![
                    self.environment().as_str(),
                    request.app_device_id,
                    path_remote_task_id
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::NotFound)?;
        let mut snapshot: TaskSnapshot =
            serde_json::from_str(&snapshot_json).map_err(|_| CloudError::StorageUnavailable)?;
        snapshot.pc_connection_state = connection_state(last_gateway_observed_at, now)?.to_owned();
        Ok(TaskDetailResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "app/task",
            message_id: random_opaque_id(),
            request_message_id: request.message_id,
            environment: self.environment().as_str().to_owned(),
            server_received_at: format_timestamp(now),
            snapshot,
        })
    }
}

fn validate_snapshot(
    snapshot: &TaskSnapshot,
    request: &SnapshotUpsert,
    now: DateTime<Utc>,
) -> Result<(), CloudError> {
    validate_opaque_id(&snapshot.remote_task_id)?;
    if snapshot.pc_device_id != request.pc_device_id
        || snapshot.installation_id != request.installation_id
        || snapshot.binding_epoch != request.binding_epoch
        || snapshot.state_version != request.state_version
        || snapshot.server_received_at.is_some()
        || snapshot.state_version < 1
        || snapshot.state_version > MAX_SAFE_INTEGER
        || snapshot.name.len() > 256
        || snapshot.workspace_name.len() > 256
        || snapshot.model_label.trim().is_empty()
        || snapshot.model_label.len() > 256
        || snapshot
            .last_error
            .as_ref()
            .is_some_and(|value| value.len() > 512)
        || !matches!(
            snapshot.task_status.as_str(),
            "created"
                | "queued"
                | "starting"
                | "running"
                | "pausing"
                | "paused"
                | "stopping"
                | "stopped"
                | "failed"
                | "reconciling"
                | "archived"
        )
        || !matches!(
            snapshot.turn_status.as_str(),
            "idle"
                | "starting"
                | "running"
                | "pausing"
                | "completed"
                | "failed"
                | "interrupted"
                | "stopped"
                | "reconciling"
        )
        || !matches!(
            snapshot.last_turn_outcome.as_str(),
            "none" | "completed" | "failed" | "interrupted" | "stopped" | "reconciling"
        )
        || !matches!(
            snapshot.last_reply_state.as_str(),
            "available" | "absent" | "withheld" | "truncated"
        )
        || snapshot.pc_connection_state != "online"
    {
        return Err(CloudError::InvalidRequest);
    }
    let observed_at = DateTime::parse_from_rfc3339(&snapshot.pc_observed_at)
        .map_err(|_| CloudError::InvalidRequest)?
        .with_timezone(&Utc);
    if observed_at > now + chrono::Duration::seconds(120) {
        return Err(CloudError::InvalidRequest);
    }
    match &snapshot.last_reply {
        Some(reply) => {
            let text_length = reply.text.as_ref().map_or(0, String::len);
            if reply.state != snapshot.last_reply_state
                || !matches!(reply.state.as_str(), "available" | "truncated")
                || reply.text.is_none()
                || reply.byte_length != text_length as i64
                || snapshot.last_reply_version.is_none()
            {
                return Err(CloudError::InvalidRequest);
            }
        }
        None => {
            if !matches!(snapshot.last_reply_state.as_str(), "absent" | "withheld") {
                return Err(CloudError::InvalidRequest);
            }
        }
    }
    if snapshot
        .last_reply_version
        .is_some_and(|version| !(1..=MAX_SAFE_INTEGER).contains(&version))
    {
        return Err(CloudError::InvalidRequest);
    }
    Ok(())
}

fn connection_state(
    observed_at: Option<i64>,
    now: DateTime<Utc>,
) -> Result<&'static str, CloudError> {
    let observed_at = observed_at.ok_or(CloudError::StorageUnavailable)?;
    let age_seconds = now.timestamp().saturating_sub(observed_at);
    Ok(if age_seconds <= 45 {
        "online"
    } else if age_seconds <= 120 {
        "stale"
    } else {
        "offline"
    })
}

fn sync_ack(
    service: &CloudService,
    identity: &GatewayIdentity,
    binding_epoch: i64,
    event_id: String,
    state_version: i64,
    now: DateTime<Utc>,
) -> SyncAck {
    SyncAck {
        schema_version: CONTRACT_VERSION,
        message_type: "sync/ack",
        message_id: random_opaque_id(),
        environment: service.environment().as_str().to_owned(),
        pc_device_id: identity.pc_device_id.clone(),
        installation_id: identity.installation_id.clone(),
        binding_epoch,
        ack_event_id: event_id,
        ack_state_version: state_version,
        server_received_at: format_timestamp(now),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn service(temp: &TempDir) -> CloudService {
        let service = CloudService::open(
            &temp.path().join("remote.sqlite3"),
            super::super::Environment::Dev,
            [11_u8; 32],
        )
        .expect("open service");
        service
            .connection()
            .expect("connection")
            .execute(
                "INSERT INTO pc_devices (
                   environment, pc_device_id, installation_id, device_key_id, public_key_der,
                   public_key_digest, display_name, current_binding_epoch,
                   last_gateway_observed_at, status, updated_at
                 ) VALUES ('dev', ?1, ?2, 'device_key_000001', X'01', 'digest',
                           'PC', 2, ?3, 'active', ?3)",
                params!["pc_device_000001", "installation_000001", now().timestamp()],
            )
            .expect("seed PC");
        service
    }

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-29T00:00:00Z")
            .expect("timestamp")
            .with_timezone(&Utc)
    }

    fn upsert(version: i64, name: &str) -> SnapshotUpsert {
        SnapshotUpsert {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "snapshot/upsert".into(),
            message_id: format!("snapshot_message_{version:04}"),
            event_id: format!("snapshot_event_{version:04}"),
            causation_id: None,
            environment: "dev".into(),
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 2,
            state_version: version,
            sent_at: format_timestamp(now()),
            terminal_push_eligible: false,
            snapshot: TaskSnapshot {
                remote_task_id: "remote_task_000001".into(),
                pc_device_id: "pc_device_000001".into(),
                installation_id: "installation_000001".into(),
                binding_epoch: 2,
                name: name.into(),
                workspace_name: "工作区".into(),
                model_label: "gpt-5.6-sol".into(),
                task_status: "running".into(),
                turn_status: "running".into(),
                last_turn_outcome: "none".into(),
                last_reply: None,
                last_reply_state: "absent".into(),
                last_reply_version: None,
                last_error: None,
                pc_observed_at: format_timestamp(now()),
                server_received_at: None,
                state_version: version,
                pc_connection_state: "online".into(),
            },
        }
    }

    #[test]
    fn older_snapshot_cannot_overwrite_newer_state() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let identity = GatewayIdentity {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
        };
        service
            .accept_gateway_snapshot(&identity, upsert(20, "新状态"), now())
            .expect("new snapshot");
        service
            .accept_gateway_snapshot(&identity, upsert(19, "旧状态"), now())
            .expect("old snapshot is acknowledged without applying");
        let stored: String = service
            .connection()
            .expect("connection")
            .query_row(
                "SELECT snapshot_json FROM remote_task_snapshots WHERE remote_task_id = ?1",
                params!["remote_task_000001"],
                |row| row.get(0),
            )
            .expect("stored snapshot");
        let snapshot: TaskSnapshot = serde_json::from_str(&stored).expect("snapshot JSON");
        assert_eq!(snapshot.state_version, 20);
        assert_eq!(snapshot.name, "新状态");
        assert!(snapshot.server_received_at.is_some());
    }

    #[test]
    fn task_list_cursor_rejects_noncanonical_or_unsafe_values() {
        assert_eq!(
            parse_task_list_cursor("20:remote_task_000001").expect("valid cursor"),
            (20, "remote_task_000001".to_owned())
        );
        for invalid_cursor in [
            "",
            "0:remote_task_000001",
            "-1:remote_task_000001",
            "01:remote_task_000001",
            "20:☃",
            "20:remote/task",
            "not-a-cursor",
        ] {
            assert!(matches!(
                parse_task_list_cursor(invalid_cursor),
                Err(CloudError::InvalidRequest)
            ));
        }
    }

    #[test]
    fn stale_snapshot_transport_envelope_is_rejected() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let identity = GatewayIdentity {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
        };
        let mut request = upsert(20, "过期传输信封");
        request.sent_at = format_timestamp(now() - chrono::Duration::seconds(121));

        assert!(matches!(
            service.accept_gateway_snapshot(&identity, request, now()),
            Err(CloudError::InvalidRequest)
        ));
    }

    #[test]
    fn tombstone_keeps_version_watermark() {
        let temp = TempDir::new().expect("temp");
        let service = service(&temp);
        let identity = GatewayIdentity {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
        };
        service
            .accept_gateway_snapshot(&identity, upsert(20, "任务"), now())
            .expect("snapshot");
        service
            .accept_gateway_tombstone(
                &identity,
                SnapshotTombstone {
                    schema_version: CONTRACT_VERSION.into(),
                    message_type: "snapshot/tombstone".into(),
                    message_id: "tombstone_message_0021".into(),
                    event_id: "tombstone_event_0021".into(),
                    causation_id: None,
                    environment: "dev".into(),
                    pc_device_id: identity.pc_device_id.clone(),
                    installation_id: identity.installation_id.clone(),
                    binding_epoch: 2,
                    state_version: 21,
                    sent_at: format_timestamp(now()),
                    remote_task_id: "remote_task_000001".into(),
                    reason: "deleted".into(),
                },
                now(),
            )
            .expect("tombstone");
        service
            .accept_gateway_snapshot(&identity, upsert(20, "旧任务"), now())
            .expect("old snapshot acknowledged");
        let (version, tombstoned): (i64, i64) = service
            .connection()
            .expect("connection")
            .query_row(
                "SELECT state_version, tombstoned FROM remote_task_snapshots WHERE remote_task_id = ?1",
                params!["remote_task_000001"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("stored tombstone");
        assert_eq!(version, 21);
        assert_eq!(tombstoned, 1);
    }
}
