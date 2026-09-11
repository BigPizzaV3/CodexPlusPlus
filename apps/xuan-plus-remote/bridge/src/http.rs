use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::runtime::{MobileRemote, official_tasks};

#[derive(Clone)]
pub struct RemoteBridgeState {
    remote: Arc<MobileRemote>,
    token: Option<Arc<str>>,
}

impl RemoteBridgeState {
    pub fn new(remote: Arc<MobileRemote>, token: Option<String>) -> Self {
        Self {
            remote,
            token: token.map(Arc::from),
        }
    }

    pub fn requires_authentication(&self) -> bool {
        self.token.is_some()
    }
}

pub fn router(state: RemoteBridgeState) -> Router {
    Router::new()
        .route("/v1/mobile/status", get(status))
        .route("/v1/mobile/pair", post(pair))
        .route("/v1/mobile/confirm", post(confirm))
        .route("/v1/mobile/tasks", post(tasks))
        .route("/v1/mobile/send-input", post(send_input))
        .route("/v1/mobile/stop", post(stop))
        .with_state(state)
}

async fn status(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    Ok(Json(
        serde_json::to_value(state.remote.status()).map_err(internal)?,
    ))
}

async fn pair(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let status = state.remote.pair().await.map_err(remote_error)?;
    Ok(Json(serde_json::to_value(status).map_err(internal)?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfirmRequest {
    request_id: String,
    confirmed: bool,
}

async fn confirm(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
    Json(request): Json<ConfirmRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let status = state
        .remote
        .confirm(request.request_id, request.confirmed)
        .await
        .map_err(remote_error)?;
    Ok(Json(serde_json::to_value(status).map_err(internal)?))
}

async fn tasks(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let home = state.remote.home_path().to_path_buf();
    let tasks = tokio::task::spawn_blocking(move || official_tasks::list_tasks(&home))
        .await
        .map_err(internal)?
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "tasks_unavailable",
                "暂时无法读取官方任务列表",
            )
        })?;
    Ok(Json(json!({"status": "ok", "tasks": tasks})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendInputRequest {
    thread_id: String,
    client_request_id: String,
    text: String,
}

async fn send_input(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
    Json(request): Json<SendInputRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let result = state
        .remote
        .send_input(request.thread_id, request.client_request_id, request.text)
        .await
        .map_err(command_error)?;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopRequest {
    thread_id: String,
    turn_id: String,
}

async fn stop(
    State(state): State<RemoteBridgeState>,
    headers: HeaderMap,
    Json(request): Json<StopRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let result = state
        .remote
        .stop(request.thread_id, request.turn_id)
        .await
        .map_err(command_error)?;
    Ok(Json(result))
}

fn authorize(state: &RemoteBridgeState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.token.as_deref() else {
        return Ok(());
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !constant_time_equal(supplied.as_bytes(), expected.as_bytes()) {
        return Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "手机桥接令牌无效",
        ));
    }
    Ok(())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "status": "error",
                "error": {"code": self.code, "message": self.message},
            })),
        )
            .into_response()
    }
}

fn internal(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal",
        error.to_string(),
    )
}

fn remote_error(message: String) -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "remote_unavailable",
        message,
    )
}

fn command_error(message: String) -> ApiError {
    let status = if message.contains("标识无效") || message.contains("64 KiB") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    ApiError::new(status, "command_unavailable", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_comparison_requires_same_bytes() {
        assert!(constant_time_equal(b"secret", b"secret"));
        assert!(!constant_time_equal(b"secret", b"other"));
        assert!(!constant_time_equal(b"secret", b"secret-long"));
    }
}
