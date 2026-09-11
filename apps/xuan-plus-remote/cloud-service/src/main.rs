use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use xuan_plus_remote_cloud::{
    decode_token_encryption_key, AppCommandRequest, AppRequestAuthentication,
    BindingLocalConfirmation, BindingRevocationRequest, BindingRevokedResponse, CloudError,
    CloudService, CommandAcceptedResponse, CommandQuery, CommandQueryResponse,
    DeviceChallengeRequest, DeviceChallengeResponse, DeviceRegistrationRequest,
    DeviceRegistrationResponse, Environment, ErrorResponse, GatewayCommandResult, GatewayIdentity,
    HuaweiPushConfig, LiveReplyAuthorization, LiveReplyFrame, LiveReplyStreamQuery,
    PairingConsumeRequest, PairingPendingResponse, PairingRegistrationRequest,
    PairingRegistrationResponse, PcDeviceListQuery, PcDeviceListResponse, PcHeartbeat, PcHello,
    PcRequestAuthentication, PushDispatcher, PushRefreshQuery, PushRefreshResponse,
    SnapshotTombstone, SnapshotUpsert, TaskDetailQuery, TaskDetailResponse, TaskListQuery,
    TaskListResponse, CONTRACT_VERSION, STORAGE_SCHEMA_VERSION,
};

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    environment: &'static str,
    contract_version: &'static str,
    storage_schema_version: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GatewayMessageEnvelope {
    message_type: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("workagents remote cloud failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let environment = required_env("XUANPLUS_REMOTE_ENVIRONMENT")?;
    let environment = Environment::parse(&environment).map_err(|error| error.to_string())?;
    let listen: SocketAddr = required_env("XUANPLUS_REMOTE_LISTEN")?
        .parse()
        .map_err(|_| "XUANPLUS_REMOTE_LISTEN must be an explicit socket address".to_owned())?;
    if !listen.ip().is_loopback() {
        return Err(
            "XUANPLUS_REMOTE_LISTEN must use a loopback address behind the configured TLS proxy"
                .into(),
        );
    }
    let database_path = PathBuf::from(required_env("XUANPLUS_REMOTE_DATABASE")?);
    let encryption_key = decode_token_encryption_key(&required_env(
        "XUANPLUS_REMOTE_PUSH_TOKEN_KEY",
    )?)
    .map_err(|_| "XUANPLUS_REMOTE_PUSH_TOKEN_KEY must be 32 random bytes encoded as base64url without padding".to_owned())?;
    let service = CloudService::open(&database_path, environment, encryption_key)
        .map_err(|error| error.to_string())?;
    let push_config = HuaweiPushConfig::from_base64url_service_account(
        &required_env("XUANPLUS_REMOTE_HUAWEI_PUSH_SEND_URL")?,
        &required_env("XUANPLUS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64")?,
    )?;
    let push_dispatcher = PushDispatcher::new(service.clone(), push_config)?;
    tokio::spawn(push_dispatcher.run());

    let app = gateway_router(service);
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|error| format!("failed to bind configured loopback listener: {error}"))?;
    println!(
        "workagents remote cloud ready environment={} listen={listen}",
        environment.as_str()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("server failed: {error}"))
}

fn gateway_router(service: CloudService) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/app-devices/challenges", post(create_device_challenge))
        .route("/v1/app-devices", post(register_device))
        .route("/v1/gateway/pairings", post(register_pairing))
        .route("/v1/pairings/consume", post(consume_pairing))
        .route("/v1/bindings/{binding_id}/revoke", post(revoke_binding))
        .route("/v1/pc-devices", get(list_pc_devices))
        .route("/v1/push-refresh/{refresh_ref}", get(resolve_push_refresh))
        .route(
            "/v1/pc-devices/{pc_device_id}/tasks",
            get(list_task_snapshots),
        )
        .route(
            "/v1/tasks/{remote_task_id}/commands",
            post(submit_task_command),
        )
        .route("/v1/tasks/{remote_task_id}", get(get_task_snapshot))
        .route(
            "/v1/tasks/{remote_task_id}/reply-stream",
            get(live_reply_connect),
        )
        .route("/v1/commands/{command_id}", get(query_command))
        .route("/v1/gateway/connect", get(gateway_connect))
        .with_state(service)
}

async fn health(State(service): State<CloudService>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        environment: service.environment().as_str(),
        contract_version: CONTRACT_VERSION,
        storage_schema_version: STORAGE_SCHEMA_VERSION,
    })
}

async fn create_device_challenge(
    State(service): State<CloudService>,
    Json(request): Json<DeviceChallengeRequest>,
) -> Result<Json<DeviceChallengeResponse>, (StatusCode, Json<ErrorResponse>)> {
    let request_message_id = Some(request.message_id.clone());
    service
        .create_device_challenge(request, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn register_device(
    State(service): State<CloudService>,
    Json(request): Json<DeviceRegistrationRequest>,
) -> Result<Json<DeviceRegistrationResponse>, (StatusCode, Json<ErrorResponse>)> {
    let request_message_id = Some(request.message_id.clone());
    service
        .register_device(request, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn register_pairing(
    State(service): State<CloudService>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<PairingRegistrationResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        pc_authentication(&headers).map_err(|error| error_response(error, None))?;
    let request: PairingRegistrationRequest = serde_json::from_slice(&body)
        .map_err(|_| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .register_pairing(request, authentication, &body, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn consume_pairing(
    State(service): State<CloudService>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<PairingPendingResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let request: PairingConsumeRequest = serde_json::from_slice(&body)
        .map_err(|_| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .consume_pairing(request, authentication, &body, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn revoke_binding(
    State(service): State<CloudService>,
    Path(binding_id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Json<BindingRevokedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let request: BindingRevocationRequest = serde_json::from_slice(&body)
        .map_err(|_| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, request_message_id.clone()))?;
    if request.binding_id != binding_id {
        return Err(error_response(
            CloudError::InvalidRequest,
            request_message_id,
        ));
    }
    service
        .revoke_binding(request, authentication, canonical_path, &body, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn list_pc_devices(
    State(service): State<CloudService>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<PcDeviceListQuery>,
) -> Result<Json<PcDeviceListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .list_pc_devices(request, authentication, canonical_path, Utc::now())
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn list_task_snapshots(
    State(service): State<CloudService>,
    Path(pc_device_id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<TaskListQuery>,
) -> Result<Json<TaskListResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .list_task_snapshots(
            &pc_device_id,
            request,
            authentication,
            canonical_path,
            Utc::now(),
        )
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn resolve_push_refresh(
    State(service): State<CloudService>,
    Path(refresh_ref): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<PushRefreshQuery>,
) -> Result<Json<PushRefreshResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .resolve_push_refresh(
            &refresh_ref,
            request,
            authentication,
            canonical_path,
            Utc::now(),
        )
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn get_task_snapshot(
    State(service): State<CloudService>,
    Path(remote_task_id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<TaskDetailQuery>,
) -> Result<Json<TaskDetailResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .task_snapshot(
            &remote_task_id,
            request,
            authentication,
            canonical_path,
            Utc::now(),
        )
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn submit_task_command(
    State(service): State<CloudService>,
    Path(remote_task_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<CommandAcceptedResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let request: AppCommandRequest = serde_json::from_slice(&body)
        .map_err(|_| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    let canonical_path = format!("/v1/tasks/{remote_task_id}/commands");
    service
        .submit_command(
            &remote_task_id,
            request,
            authentication,
            &body,
            &canonical_path,
            Utc::now(),
        )
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn query_command(
    State(service): State<CloudService>,
    Path(command_id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<CommandQuery>,
) -> Result<Json<CommandQueryResponse>, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    service
        .query_command(
            &command_id,
            request,
            authentication,
            canonical_path,
            Utc::now(),
        )
        .map(Json)
        .map_err(|error| error_response(error, request_message_id))
}

async fn gateway_connect(
    State(service): State<CloudService>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        pc_authentication(&headers).map_err(|error| error_response(error, None))?;
    let identity = service
        .authenticate_gateway(authentication, Utc::now())
        .map_err(|error| error_response(error, None))?;
    Ok(upgrade.on_upgrade(move |socket| gateway_session(socket, service, identity)))
}

async fn live_reply_connect(
    State(service): State<CloudService>,
    Path(remote_task_id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Query(request): Query<LiveReplyStreamQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let authentication =
        app_authentication(&headers).map_err(|error| error_response(error, None))?;
    let canonical_path = uri
        .path_and_query()
        .map(|value| value.as_str())
        .ok_or_else(|| error_response(CloudError::InvalidRequest, None))?;
    let request_message_id = Some(request.message_id.clone());
    let authorization = service
        .authorize_live_reply_stream(
            &remote_task_id,
            request,
            authentication,
            canonical_path,
            Utc::now(),
        )
        .map_err(|error| error_response(error, request_message_id))?;
    Ok(upgrade.on_upgrade(move |socket| live_reply_session(socket, service, authorization)))
}

async fn live_reply_session(
    mut socket: WebSocket,
    service: CloudService,
    authorization: LiveReplyAuthorization,
) {
    let broker = service.live_reply_broker();
    let mut subscription = match broker.subscribe(&authorization.key, Utc::now()) {
        Ok(subscription) => subscription,
        Err(_) => return,
    };
    if let Some(current) = subscription.current.take() {
        let Ok(payload) = serde_json::to_string(&current) else {
            broker.unsubscribe(&authorization.key, Utc::now());
            return;
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            broker.unsubscribe(&authorization.key, Utc::now());
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(20));
    let mut next_authorization_check =
        tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        tokio::select! {
            frame = subscription.receiver.recv() => {
                if tokio::time::Instant::now() >= next_authorization_check {
                    if !matches!(
                        service.live_reply_authorization_is_active(&authorization),
                        Ok(true)
                    ) {
                        break;
                    }
                    next_authorization_check =
                        tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                }
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let Some(reset) = broker.current_reset(&authorization.key, Utc::now()) else {
                            continue;
                        };
                        reset
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                let Ok(payload) = serde_json::to_string(&frame) else { break; };
                if socket.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            _ = heartbeat.tick() => {
                if !matches!(
                    service.live_reply_authorization_is_active(&authorization),
                    Ok(true)
                ) {
                    break;
                }
                next_authorization_check =
                    tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    broker.unsubscribe(&authorization.key, Utc::now());
}

async fn gateway_session(mut socket: WebSocket, service: CloudService, identity: GatewayIdentity) {
    let hello_message =
        match tokio::time::timeout(std::time::Duration::from_secs(5), socket.recv()).await {
            Ok(Some(Ok(Message::Text(message)))) => message,
            _ => return,
        };
    let hello: PcHello = match serde_json::from_str(hello_message.as_str()) {
        Ok(hello) => hello,
        Err(_) => return,
    };
    if service
        .accept_gateway_hello(&identity, &hello, Utc::now())
        .is_err()
    {
        return;
    }
    let active_binding = match service.active_binding_for_gateway(&identity, Utc::now()) {
        Ok(active_binding) => active_binding,
        Err(_) => return,
    };
    if let Some(active_binding) = active_binding {
        let payload = match serde_json::to_string(&active_binding) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }

    let broker = service.live_reply_broker();
    let mut live_reply_controls = broker.gateway_controls();
    for subscription in broker.current_subscriptions(&identity, Utc::now()) {
        let payload = match serde_json::to_string(&subscription) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        if socket.send(Message::Text(payload.into())).await.is_err() {
            return;
        }
    }
    let mut confirmations = tokio::time::interval(std::time::Duration::from_secs(2));
    let mut delivered = std::collections::HashSet::new();
    loop {
        tokio::select! {
            subscription = live_reply_controls.recv() => {
                match subscription {
                    Ok(subscription)
                        if subscription.pc_device_id == identity.pc_device_id
                            && subscription.installation_id == identity.installation_id => {
                        let payload = match serde_json::to_string(&subscription) {
                            Ok(payload) => payload,
                            Err(_) => return,
                        };
                        if socket.send(Message::Text(payload.into())).await.is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            _ = confirmations.tick() => {
                let pending = match service.pending_binding_confirmations(&identity, Utc::now()) {
                    Ok(pending) => pending,
                    Err(_) => break,
                };
                for confirmation in pending {
                    if delivered.contains(&confirmation.binding_id) {
                        continue;
                    }
                    let payload = match serde_json::to_string(&confirmation) {
                        Ok(payload) => payload,
                        Err(_) => return,
                    };
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                    delivered.insert(confirmation.binding_id);
                }
                let commands = match service.pending_gateway_commands(&identity, Utc::now()) {
                    Ok(commands) => commands,
                    Err(_) => break,
                };
                for command in commands {
                    let payload = match serde_json::to_string(&command) {
                        Ok(payload) => payload,
                        Err(_) => return,
                    };
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
            }
            message = socket.recv() => {
                let Some(Ok(message)) = message else { break; };
                match message {
                    Message::Text(text) => {
                        let envelope: GatewayMessageEnvelope = match serde_json::from_str(text.as_str()) {
                            Ok(envelope) => envelope,
                            Err(_) => return,
                        };
                        match envelope.message_type.as_str() {
                            "pc/heartbeat" => {
                                let heartbeat: PcHeartbeat = match serde_json::from_str(text.as_str()) {
                                    Ok(heartbeat) => heartbeat,
                                    Err(_) => return,
                                };
                                if service
                                    .record_gateway_heartbeat(&identity, &heartbeat, Utc::now())
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            "binding/local-confirm" => {
                                let request: BindingLocalConfirmation = match serde_json::from_str(text.as_str()) {
                                    Ok(request) => request,
                                    Err(_) => return,
                                };
                                let response = match service.confirm_binding(&identity, request, Utc::now()) {
                                    Ok(response) => response,
                                    Err(_) => return,
                                };
                                let payload = match serde_json::to_string(&response) {
                                    Ok(payload) => payload,
                                    Err(_) => return,
                                };
                                if socket.send(Message::Text(payload.into())).await.is_err() {
                                    return;
                                }
                            }
                            "snapshot/upsert" => {
                                let request: SnapshotUpsert = match serde_json::from_str(text.as_str()) {
                                    Ok(request) => request,
                                    Err(_) => return,
                                };
                                let response = match service.accept_gateway_snapshot(&identity, request, Utc::now()) {
                                    Ok(response) => response,
                                    Err(_) => return,
                                };
                                let payload = match serde_json::to_string(&response) {
                                    Ok(payload) => payload,
                                    Err(_) => return,
                                };
                                if socket.send(Message::Text(payload.into())).await.is_err() {
                                    return;
                                }
                            }
                            "snapshot/tombstone" => {
                                let request: SnapshotTombstone = match serde_json::from_str(text.as_str()) {
                                    Ok(request) => request,
                                    Err(_) => return,
                                };
                                let response = match service.accept_gateway_tombstone(&identity, request, Utc::now()) {
                                    Ok(response) => response,
                                    Err(_) => return,
                                };
                                let payload = match serde_json::to_string(&response) {
                                    Ok(payload) => payload,
                                    Err(_) => return,
                                };
                                if socket.send(Message::Text(payload.into())).await.is_err() {
                                    return;
                                }
                            }
                            "reply-stream/reset" | "reply-stream/append" | "reply-stream/end" => {
                                let frame: LiveReplyFrame = match serde_json::from_str(text.as_str()) {
                                    Ok(frame) => frame,
                                    Err(_) => return,
                                };
                                if broker.publish(&identity, frame, Utc::now()).is_err() {
                                    return;
                                }
                            }
                            "command/result" => {
                                let request: GatewayCommandResult = match serde_json::from_str(text.as_str()) {
                                    Ok(request) => request,
                                    Err(_) => return,
                                };
                                if service
                                    .accept_gateway_command_result(&identity, request, Utc::now())
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            _ => return,
                        }
                    }
                    Message::Ping(payload) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }
}

fn app_authentication(headers: &HeaderMap) -> Result<AppRequestAuthentication, CloudError> {
    Ok(AppRequestAuthentication {
        device_key_id: required_header(headers, "x-workagents-device-key-id")?,
        timestamp: required_header(headers, "x-workagents-device-timestamp")?,
        nonce: required_header(headers, "x-workagents-device-nonce")?,
        signature: required_header(headers, "x-workagents-device-signature")?,
    })
}

fn pc_authentication(headers: &HeaderMap) -> Result<PcRequestAuthentication, CloudError> {
    Ok(PcRequestAuthentication {
        pc_device_id: required_header(headers, "x-workagents-pc-device-id")?,
        installation_id: required_header(headers, "x-workagents-pc-installation-id")?,
        timestamp: required_header(headers, "x-workagents-pc-timestamp")?,
        nonce: required_header(headers, "x-workagents-pc-nonce")?,
        public_key: required_header(headers, "x-workagents-pc-public-key")?,
        signature: required_header(headers, "x-workagents-pc-signature")?,
    })
}

fn required_header(headers: &HeaderMap, name: &str) -> Result<String, CloudError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 1024)
        .map(str::to_owned)
        .ok_or(CloudError::DeviceAuthenticationFailed)
}

fn error_response(
    error: CloudError,
    request_message_id: Option<String>,
) -> (StatusCode, Json<ErrorResponse>) {
    let status = match error {
        CloudError::InvalidRequest
        | CloudError::DeviceKeyInvalid
        | CloudError::DeviceProofInvalid
        | CloudError::PairingQrInvalid
        | CloudError::PairingSummaryMismatch
        | CloudError::UnsupportedOperation => StatusCode::BAD_REQUEST,
        CloudError::EnvironmentMismatch => StatusCode::FORBIDDEN,
        CloudError::DeviceAuthenticationFailed | CloudError::DeviceNotBound => {
            StatusCode::UNAUTHORIZED
        }
        CloudError::DeviceChallengeExpired
        | CloudError::PairingExpired
        | CloudError::PairingReplayed
        | CloudError::PairingConfirmationExpired
        | CloudError::CommandExpired => StatusCode::GONE,
        CloudError::DeviceRequestReplayed
        | CloudError::DeviceLimitReached
        | CloudError::PcOffline
        | CloudError::StateConflict
        | CloudError::PayloadDigestConflict
        | CloudError::InvalidCommandState => StatusCode::CONFLICT,
        CloudError::NotFound => StatusCode::NOT_FOUND,
        CloudError::StorageUnavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    let error_code = error.error_code();
    let retryable = error.retryable();
    (
        status,
        Json(ErrorResponse {
            schema_version: CONTRACT_VERSION,
            message_type: "error",
            message_id: random_response_id(),
            request_message_id,
            error_code,
            retryable,
            message: "Request rejected",
            server_received_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        }),
    )
}

fn required_env(name: &str) -> Result<String, String> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("required configuration is missing: {name}"))
}

fn random_response_id() -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use rand_core::{OsRng, RngCore};

    let mut bytes = [0_u8; 24];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
#[tokio::test]
#[ignore = "仅由桌面集成测试在临时目录和回环端口启动"]
async fn desktop_gateway_test_server() {
    let listen: SocketAddr = required_env("XUANPLUS_REMOTE_TEST_LISTEN").unwrap().parse().unwrap();
    assert!(listen.ip().is_loopback());
    let path = PathBuf::from(required_env("XUANPLUS_REMOTE_TEST_DATABASE").unwrap());
    let service = CloudService::open(&path, Environment::Dev, [47; 32]).unwrap();
    let listener = tokio::net::TcpListener::bind(listen).await.unwrap();
    axum::serve(listener, gateway_router(service)).await.unwrap();
}
