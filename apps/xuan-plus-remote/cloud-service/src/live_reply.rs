use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::{
    format_timestamp, random_opaque_id, validate_opaque_id, validate_request_base,
    AppRequestAuthentication, CloudError, CloudService, Environment, GatewayIdentity, RequestBase,
    CONTRACT_VERSION,
};

const LIVE_REPLY_BROADCAST_CAPACITY: usize = 64;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveReplyStreamQuery {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub sent_at: String,
    pub app_device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiveReplyKey {
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub remote_task_id: String,
}

#[derive(Debug, Clone)]
pub struct LiveReplyAuthorization {
    pub request_message_id: String,
    pub app_device_id: String,
    pub key: LiveReplyKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LiveReplyFrame {
    pub schema_version: String,
    pub message_type: String,
    pub message_id: String,
    pub environment: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub remote_task_id: String,
    pub stream_id: String,
    pub stream_seq: u64,
    pub sent_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_received_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveReplySubscription {
    pub schema_version: &'static str,
    pub message_type: &'static str,
    pub message_id: String,
    pub environment: String,
    pub pc_device_id: String,
    pub installation_id: String,
    pub binding_epoch: i64,
    pub remote_task_id: String,
    pub active: bool,
    pub sent_at: String,
}

#[derive(Clone)]
pub struct LiveReplyBroker {
    environment: Environment,
    inner: Arc<Mutex<LiveReplyBrokerState>>,
    gateway_controls: broadcast::Sender<LiveReplySubscription>,
}

struct LiveReplyBrokerState {
    tasks: HashMap<LiveReplyKey, LiveReplyTaskState>,
}

struct LiveReplyTaskState {
    subscribers: usize,
    sender: broadcast::Sender<LiveReplyFrame>,
    current: Option<LiveReplyFrame>,
}

pub struct LiveReplyReceiver {
    pub receiver: broadcast::Receiver<LiveReplyFrame>,
    pub current: Option<LiveReplyFrame>,
}

impl LiveReplyBroker {
    pub fn new(environment: Environment) -> Self {
        let (gateway_controls, _) = broadcast::channel(LIVE_REPLY_BROADCAST_CAPACITY);
        Self {
            environment,
            inner: Arc::new(Mutex::new(LiveReplyBrokerState {
                tasks: HashMap::new(),
            })),
            gateway_controls,
        }
    }

    pub fn gateway_controls(&self) -> broadcast::Receiver<LiveReplySubscription> {
        self.gateway_controls.subscribe()
    }

    pub fn current_subscriptions(
        &self,
        identity: &GatewayIdentity,
        now: DateTime<Utc>,
    ) -> Vec<LiveReplySubscription> {
        let Ok(inner) = self.inner.lock() else {
            return Vec::new();
        };
        inner
            .tasks
            .iter()
            .filter(|(key, state)| {
                state.subscribers > 0
                    && key.pc_device_id == identity.pc_device_id
                    && key.installation_id == identity.installation_id
            })
            .map(|(key, _)| self.subscription(key, true, now))
            .collect()
    }

    pub fn subscribe(
        &self,
        key: &LiveReplyKey,
        now: DateTime<Utc>,
    ) -> Result<LiveReplyReceiver, CloudError> {
        let (receiver, current, became_active) = {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| CloudError::StorageUnavailable)?;
            let state = inner.tasks.entry(key.clone()).or_insert_with(|| {
                let (sender, _) = broadcast::channel(LIVE_REPLY_BROADCAST_CAPACITY);
                LiveReplyTaskState {
                    subscribers: 0,
                    sender,
                    current: None,
                }
            });
            let became_active = state.subscribers == 0;
            state.subscribers = state.subscribers.saturating_add(1);
            (
                state.sender.subscribe(),
                state.current.clone(),
                became_active,
            )
        };
        if became_active {
            let _ = self
                .gateway_controls
                .send(self.subscription(key, true, now));
        }
        Ok(LiveReplyReceiver { receiver, current })
    }

    pub fn unsubscribe(&self, key: &LiveReplyKey, now: DateTime<Utc>) {
        let became_inactive = {
            let Ok(mut inner) = self.inner.lock() else {
                return;
            };
            let Some(state) = inner.tasks.get_mut(key) else {
                return;
            };
            state.subscribers = state.subscribers.saturating_sub(1);
            if state.subscribers == 0 {
                inner.tasks.remove(key);
                true
            } else {
                false
            }
        };
        if became_inactive {
            let _ = self
                .gateway_controls
                .send(self.subscription(key, false, now));
        }
    }

    pub fn publish(
        &self,
        identity: &GatewayIdentity,
        mut frame: LiveReplyFrame,
        now: DateTime<Utc>,
    ) -> Result<(), CloudError> {
        validate_frame(self.environment, identity, &frame, now)?;
        let key = LiveReplyKey {
            pc_device_id: frame.pc_device_id.clone(),
            installation_id: frame.installation_id.clone(),
            binding_epoch: frame.binding_epoch,
            remote_task_id: frame.remote_task_id.clone(),
        };
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| CloudError::StorageUnavailable)?;
        let Some(state) = inner.tasks.get_mut(&key) else {
            return Ok(());
        };
        if state.subscribers == 0 {
            return Ok(());
        }
        frame.server_received_at = Some(format_timestamp(now));
        match frame.message_type.as_str() {
            "reply-stream/reset" => state.current = Some(frame.clone()),
            "reply-stream/append" => {
                let current = state.current.as_mut().ok_or(CloudError::InvalidRequest)?;
                if current.stream_id != frame.stream_id
                    || frame.stream_seq != current.stream_seq.saturating_add(1)
                {
                    return Err(CloudError::InvalidRequest);
                }
                let delta = frame.text.as_deref().ok_or(CloudError::InvalidRequest)?;
                let current_text = current.text.get_or_insert_with(String::new);
                current_text.push_str(delta);
                current.stream_seq = frame.stream_seq;
                current.sent_at = frame.sent_at.clone();
                current.server_received_at = frame.server_received_at.clone();
            }
            "reply-stream/end" => {
                let Some(current) = state.current.as_ref() else {
                    return Ok(());
                };
                if current.stream_id != frame.stream_id {
                    return Ok(());
                }
                if frame.stream_seq != current.stream_seq.saturating_add(1) {
                    return Err(CloudError::InvalidRequest);
                }
                state.current = None;
            }
            _ => return Err(CloudError::InvalidRequest),
        }
        let _ = state.sender.send(frame);
        Ok(())
    }

    pub fn current_reset(&self, key: &LiveReplyKey, now: DateTime<Utc>) -> Option<LiveReplyFrame> {
        let inner = self.inner.lock().ok()?;
        let current = inner.tasks.get(key)?.current.as_ref()?;
        let mut reset = current.clone();
        reset.message_type = "reply-stream/reset".into();
        reset.message_id = random_opaque_id();
        reset.server_received_at = Some(format_timestamp(now));
        Some(reset)
    }

    fn subscription(
        &self,
        key: &LiveReplyKey,
        active: bool,
        now: DateTime<Utc>,
    ) -> LiveReplySubscription {
        LiveReplySubscription {
            schema_version: CONTRACT_VERSION,
            message_type: "reply-stream/subscription",
            message_id: random_opaque_id(),
            environment: self.environment.as_str().to_owned(),
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
            binding_epoch: key.binding_epoch,
            remote_task_id: key.remote_task_id.clone(),
            active,
            sent_at: format_timestamp(now),
        }
    }
}

impl CloudService {
    pub fn authorize_live_reply_stream(
        &self,
        remote_task_id: &str,
        request: LiveReplyStreamQuery,
        authentication: AppRequestAuthentication,
        canonical_path: &str,
        now: DateTime<Utc>,
    ) -> Result<LiveReplyAuthorization, CloudError> {
        validate_request_base(
            RequestBase {
                schema_version: &request.schema_version,
                message_type: &request.message_type,
                expected_message_type: "app/reply-stream-connect",
                message_id: &request.message_id,
                environment: &request.environment,
                sent_at: &request.sent_at,
            },
            self.environment(),
            now,
        )?;
        validate_opaque_id(remote_task_id)?;
        validate_opaque_id(&request.app_device_id)?;
        let authenticated_app_device_id =
            self.authenticate_app_request(&authentication, "GET", canonical_path, &[], now)?;
        if authenticated_app_device_id != request.app_device_id {
            return Err(CloudError::DeviceAuthenticationFailed);
        }
        let key = self
            .connection()?
            .query_row(
                "SELECT b.pc_device_id, b.installation_id, b.binding_epoch
                 FROM bindings b
                 JOIN remote_task_snapshots s
                   ON s.environment = b.environment
                  AND s.pc_device_id = b.pc_device_id
                  AND s.installation_id = b.installation_id
                  AND s.binding_epoch = b.binding_epoch
                 WHERE b.environment = ?1 AND b.app_device_id = ?2 AND b.state = 'active'
                   AND s.remote_task_id = ?3 AND s.tombstoned = 0",
                params![
                    self.environment().as_str(),
                    request.app_device_id,
                    remote_task_id
                ],
                |row| {
                    Ok(LiveReplyKey {
                        pc_device_id: row.get(0)?,
                        installation_id: row.get(1)?,
                        binding_epoch: row.get(2)?,
                        remote_task_id: remote_task_id.to_owned(),
                    })
                },
            )
            .optional()
            .map_err(|_| CloudError::StorageUnavailable)?
            .ok_or(CloudError::DeviceNotBound)?;
        Ok(LiveReplyAuthorization {
            request_message_id: request.message_id,
            app_device_id: request.app_device_id,
            key,
        })
    }

    pub fn live_reply_authorization_is_active(
        &self,
        authorization: &LiveReplyAuthorization,
    ) -> Result<bool, CloudError> {
        self.connection()?
            .query_row(
                "SELECT 1
                 FROM bindings b
                 JOIN remote_task_snapshots s
                   ON s.environment = b.environment
                  AND s.pc_device_id = b.pc_device_id
                  AND s.installation_id = b.installation_id
                  AND s.binding_epoch = b.binding_epoch
                 WHERE b.environment = ?1 AND b.app_device_id = ?2 AND b.state = 'active'
                   AND b.pc_device_id = ?3 AND b.installation_id = ?4 AND b.binding_epoch = ?5
                   AND s.remote_task_id = ?6 AND s.tombstoned = 0",
                params![
                    self.environment().as_str(),
                    authorization.app_device_id,
                    authorization.key.pc_device_id,
                    authorization.key.installation_id,
                    authorization.key.binding_epoch,
                    authorization.key.remote_task_id
                ],
                |_| Ok(true),
            )
            .optional()
            .map(Option::unwrap_or_default)
            .map_err(|_| CloudError::StorageUnavailable)
    }
}

fn validate_frame(
    environment: Environment,
    identity: &GatewayIdentity,
    frame: &LiveReplyFrame,
    now: DateTime<Utc>,
) -> Result<(), CloudError> {
    if frame.schema_version != CONTRACT_VERSION
        || frame.environment != environment.as_str()
        || frame.pc_device_id != identity.pc_device_id
        || frame.installation_id != identity.installation_id
        || !matches!(
            frame.message_type.as_str(),
            "reply-stream/reset" | "reply-stream/append" | "reply-stream/end"
        )
        || frame.stream_seq == 0
    {
        return Err(CloudError::InvalidRequest);
    }
    validate_opaque_id(&frame.message_id)?;
    validate_opaque_id(&frame.remote_task_id)?;
    validate_opaque_id(&frame.stream_id)?;
    let sent_at = DateTime::parse_from_rfc3339(&frame.sent_at)
        .map_err(|_| CloudError::InvalidRequest)?
        .with_timezone(&Utc);
    if (now - sent_at).num_seconds().unsigned_abs() > 120 {
        return Err(CloudError::InvalidRequest);
    }
    match frame.message_type.as_str() {
        "reply-stream/reset" | "reply-stream/append" => {
            if frame.text.is_none() || frame.outcome.is_some() {
                return Err(CloudError::InvalidRequest);
            }
        }
        "reply-stream/end" => {
            if frame.text.is_some()
                || !matches!(
                    frame.outcome.as_deref(),
                    Some("completed" | "failed" | "interrupted" | "stopped")
                )
            {
                return Err(CloudError::InvalidRequest);
            }
        }
        _ => return Err(CloudError::InvalidRequest),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_only_retains_content_while_a_detail_subscriber_exists() {
        let broker = LiveReplyBroker::new(Environment::Dev);
        let key = LiveReplyKey {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 3,
            remote_task_id: "remote_task_000001".into(),
        };
        let identity = GatewayIdentity {
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
        };
        let now = Utc::now();
        let complete_reply = "完整段落".repeat(9_000);
        let frame = LiveReplyFrame {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "reply-stream/reset".into(),
            message_id: "message_stream_000001".into(),
            environment: "dev".into(),
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
            binding_epoch: key.binding_epoch,
            remote_task_id: key.remote_task_id.clone(),
            stream_id: "stream_reply_000001".into(),
            stream_seq: 1,
            sent_at: format_timestamp(now),
            server_received_at: None,
            text: Some(complete_reply.clone()),
            outcome: None,
        };

        broker
            .publish(&identity, frame.clone(), now)
            .expect("无订阅时忽略");
        assert!(broker.current_reset(&key, now).is_none());
        let _receiver = broker.subscribe(&key, now).expect("订阅");
        broker.publish(&identity, frame, now).expect("有订阅时接收");
        assert_eq!(
            broker.current_reset(&key, now).unwrap().text,
            Some(complete_reply)
        );
        broker.unsubscribe(&key, now);
        assert!(broker.current_reset(&key, now).is_none());
    }

    #[test]
    fn subscription_control_changes_only_on_first_enter_and_last_leave() {
        let broker = LiveReplyBroker::new(Environment::Dev);
        let mut controls = broker.gateway_controls();
        let key = LiveReplyKey {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 3,
            remote_task_id: "remote_task_000001".into(),
        };
        let now = Utc::now();

        let _first = broker.subscribe(&key, now).expect("first subscription");
        assert!(controls.try_recv().expect("active control").active);
        let _second = broker.subscribe(&key, now).expect("second subscription");
        assert!(matches!(
            controls.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        broker.unsubscribe(&key, now);
        assert!(matches!(
            controls.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        broker.unsubscribe(&key, now);
        assert!(!controls.try_recv().expect("inactive control").active);
    }

    #[test]
    fn append_sequence_gap_is_rejected_and_reset_recovers() {
        let broker = LiveReplyBroker::new(Environment::Dev);
        let key = LiveReplyKey {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 3,
            remote_task_id: "remote_task_000001".into(),
        };
        let identity = GatewayIdentity {
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
        };
        let now = Utc::now();
        let _receiver = broker.subscribe(&key, now).expect("subscription");
        let reset = LiveReplyFrame {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "reply-stream/reset".into(),
            message_id: "message_stream_000001".into(),
            environment: "dev".into(),
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
            binding_epoch: key.binding_epoch,
            remote_task_id: key.remote_task_id.clone(),
            stream_id: "stream_reply_000001".into(),
            stream_seq: 1,
            sent_at: format_timestamp(now),
            server_received_at: None,
            text: Some("前缀".into()),
            outcome: None,
        };
        broker
            .publish(&identity, reset.clone(), now)
            .expect("reset");
        let mut gap = reset.clone();
        gap.message_type = "reply-stream/append".into();
        gap.message_id = "message_stream_000002".into();
        gap.stream_seq = 3;
        gap.text = Some("缺口".into());
        assert!(matches!(
            broker.publish(&identity, gap, now),
            Err(CloudError::InvalidRequest)
        ));
        let mut recovery = reset;
        recovery.message_id = "message_stream_000003".into();
        recovery.stream_id = "stream_reply_000002".into();
        recovery.text = Some("完整恢复🙂".into());
        broker
            .publish(&identity, recovery, now)
            .expect("recovery reset");
        assert_eq!(
            broker.current_reset(&key, now).unwrap().text.as_deref(),
            Some("完整恢复🙂")
        );
    }

    #[test]
    fn delayed_end_from_an_old_stream_cannot_clear_the_current_stream() {
        let broker = LiveReplyBroker::new(Environment::Dev);
        let key = LiveReplyKey {
            pc_device_id: "pc_device_000001".into(),
            installation_id: "installation_000001".into(),
            binding_epoch: 3,
            remote_task_id: "remote_task_000001".into(),
        };
        let identity = GatewayIdentity {
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
        };
        let now = Utc::now();
        let _receiver = broker.subscribe(&key, now).expect("subscription");
        let reset = LiveReplyFrame {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "reply-stream/reset".into(),
            message_id: "message_stream_000001".into(),
            environment: "dev".into(),
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
            binding_epoch: key.binding_epoch,
            remote_task_id: key.remote_task_id.clone(),
            stream_id: "stream_reply_current_0001".into(),
            stream_seq: 1,
            sent_at: format_timestamp(now),
            server_received_at: None,
            text: Some("当前流".into()),
            outcome: None,
        };
        broker
            .publish(&identity, reset, now)
            .expect("current reset");
        let stale_end = LiveReplyFrame {
            schema_version: CONTRACT_VERSION.into(),
            message_type: "reply-stream/end".into(),
            message_id: "message_stream_000002".into(),
            environment: "dev".into(),
            pc_device_id: key.pc_device_id.clone(),
            installation_id: key.installation_id.clone(),
            binding_epoch: key.binding_epoch,
            remote_task_id: key.remote_task_id.clone(),
            stream_id: "stream_reply_previous_001".into(),
            stream_seq: 2,
            sent_at: format_timestamp(now),
            server_received_at: None,
            text: None,
            outcome: Some("completed".into()),
        };
        broker
            .publish(&identity, stale_end, now)
            .expect("stale end is safely ignored");
        assert_eq!(
            broker.current_reset(&key, now).unwrap().text.as_deref(),
            Some("当前流")
        );
    }
}
