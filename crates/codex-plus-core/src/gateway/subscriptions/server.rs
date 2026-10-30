//! 订阅的本地适配入口：`/sub/<供应商 id>/v1/chat/completions`、`/sub/<供应商 id>/v1/models`。
//!
//! 只接受 helper 自己（网关管道）带着本进程口令发来的请求。读 gateway.json 里这个供应商的凭据，
//! 快过期时刷新并写回，然后交给对应订阅的实现；失败时按订阅给的状态码回给管道，
//! 管道据此冷却、停用这个账号或换下一家。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use crate::gateway::store::GatewayStore;
use crate::gateway::subscriptions::{
    self, SubscriptionError, chat, kind_of, local_secret, with_credentials,
};
use crate::launcher::{header_value_from_headers, write_http_response};

const JSON: &str = "application/json; charset=utf-8";

pub fn is_subscription_path(path: &str) -> bool {
    path.starts_with("/sub/")
}

/// `/sub/<id>/v1/chat/completions?x` → (`<id>`, `/chat/completions`)。
fn split(path: &str) -> Option<(String, String)> {
    let path = path.split('?').next().unwrap_or(path);
    let rest = path.strip_prefix("/sub/")?;
    let (id, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if id.is_empty() {
        return None;
    }
    let tail = format!("/{tail}");
    let tail = tail.strip_prefix("/v1").unwrap_or(&tail);
    Some((id.to_string(), tail.trim_end_matches('/').to_string()))
}

async fn write_error(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    message: &str,
) -> anyhow::Result<()> {
    let body = json!({ "error": { "message": message, "type": "subscription_error", "code": status } });
    write_http_response(
        stream,
        &crate::protocol_proxy::http_status_line(status),
        JSON,
        &serde_json::to_vec(&body)?,
    )
    .await?;
    stream.shutdown().await?;
    Ok(())
}

fn status_of(error: &anyhow::Error) -> u16 {
    error
        .downcast_ref::<SubscriptionError>()
        .map(|error| error.status)
        .unwrap_or(502)
}

/// 每个供应商一把刷新锁：并发请求只刷新一次。
fn refresh_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(id.to_string())
        .or_default()
        .clone()
}

/// 读供应商的最新凭据，必要时刷新并写回。返回 (订阅, 凭据, HTTP 客户端)。
async fn ready(
    id: &str,
) -> anyhow::Result<(
    &'static dyn subscriptions::Subscription,
    Value,
    reqwest::Client,
)> {
    let store = GatewayStore::default();
    let load = || -> anyhow::Result<_> {
        let gateway = store.load()?;
        let provider = gateway
            .provider(id)
            .ok_or_else(|| subscriptions::fail(404, format!("网关里没有供应商 {id}")))?;
        let (item, credentials) = kind_of(&provider.relay)
            .ok_or_else(|| subscriptions::fail(400, format!("{id} 不是订阅供应商")))?;
        let http = crate::http_client::proxied_client_for_relay("", &provider.relay)?;
        Ok((item, credentials, http))
    };
    let (item, credentials, http) = load()?;
    let lock = refresh_lock(id);
    let _guard = lock.lock().await;
    // 等锁期间别的请求可能已经刷新过：重新读一次。
    let (item, credentials, http) = match load() {
        Ok(latest) => latest,
        Err(_) => (item, credentials, http),
    };
    let Some(fresh) = item.refresh(&http, &credentials).await? else {
        return Ok((item, credentials, http));
    };
    let result = store.update(|gateway| {
        if let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        {
            provider.relay.auth_contents = with_credentials(&provider.relay.auth_contents, &fresh);
        }
        Ok(())
    });
    if let Err(error) = result {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "gateway.subscription_persist_failed",
            json!({ "relayId": id, "kind": item.id(), "error": format!("{error:#}") }),
        );
    }
    Ok((item, fresh, http))
}

/// helper 收到 `/sub/*` 时调用。
pub(crate) async fn handle(
    stream: &mut tokio::net::TcpStream,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let Some((id, tail)) = split(path) else {
        return write_error(stream, 404, "未知的订阅路径").await;
    };
    let token = header_value_from_headers(headers, "authorization")
        .and_then(|value| {
            value
                .trim()
                .strip_prefix("Bearer ")
                .map(|token| token.trim().to_string())
        })
        .or_else(|| header_value_from_headers(headers, "x-api-key"))
        .unwrap_or_default();
    if !crate::gateway::store::token_matches(local_secret(), token.trim()) {
        return write_error(stream, 401, "订阅入口只接受 Codex++ 网关自己的请求").await;
    }
    let (item, credentials, http) = match ready(&id).await {
        Ok(ready) => ready,
        Err(error) => {
            return write_error(stream, status_of(&error), &format!("{error:#}")).await;
        }
    };
    match (method, tail.as_str()) {
        ("GET", "/models") => match item.models(&http, &credentials).await {
            Ok(models) => {
                let body = json!({
                    "object": "list",
                    "data": models.iter().map(|model| json!({ "id": model, "object": "model", "owned_by": item.id() })).collect::<Vec<_>>(),
                });
                write_http_response(stream, "200 OK", JSON, &serde_json::to_vec(&body)?).await?;
                stream.shutdown().await?;
                Ok(())
            }
            Err(error) => write_error(stream, status_of(&error), &format!("{error:#}")).await,
        },
        ("POST", "/chat/completions") => {
            let mut request: Value = match serde_json::from_slice(body) {
                Ok(value) => value,
                Err(error) => {
                    return write_error(stream, 400, &format!("请求体不是 JSON：{error}")).await;
                }
            };
            let wants_stream = request
                .get("stream")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let model = request
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            request["stream"] = json!(true);
            let mut events = match item.chat(&http, &credentials, request).await {
                Ok(events) => events,
                Err(error) => {
                    return write_error(stream, status_of(&error), &format!("{error:#}")).await;
                }
            };
            if !wants_stream {
                let mut collected = Vec::new();
                while let Some(chunk) = events.next().await {
                    match chunk {
                        Ok(bytes) => collected.extend(bytes),
                        Err(error) => {
                            return write_error(stream, status_of(&error), &format!("{error:#}"))
                                .await;
                        }
                    }
                }
                return match chat::aggregate(&String::from_utf8_lossy(&collected), &model) {
                    Ok(completion) => {
                        write_http_response(
                            stream,
                            "200 OK",
                            JSON,
                            &serde_json::to_vec(&completion)?,
                        )
                        .await?;
                        stream.shutdown().await?;
                        Ok(())
                    }
                    Err(error) => write_error(stream, 502, &format!("{error:#}")).await,
                };
            }
            // 流式：第一块到了再回 200，第一块就失败时还能回真实的状态码。
            let first = events.next().await;
            let first = match first {
                Some(Ok(bytes)) => bytes,
                Some(Err(error)) => {
                    return write_error(stream, status_of(&error), &format!("{error:#}")).await;
                }
                None => return write_error(stream, 502, "上游没有返回任何内容").await,
            };
            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
            stream.write_all(head.as_bytes()).await?;
            stream.write_all(&first).await?;
            while let Some(chunk) = events.next().await {
                match chunk {
                    Ok(bytes) => stream.write_all(&bytes).await?,
                    Err(error) => {
                        let mut sse = chat::ChatSse::new(&model);
                        stream
                            .write_all(&sse.error(&format!("{error:#}")))
                            .await?;
                        break;
                    }
                }
            }
            stream.shutdown().await?;
            Ok(())
        }
        _ => write_error(stream, 404, &format!("订阅入口不支持 {method} {tail}")).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_paths() {
        assert_eq!(
            split("/sub/cursor-1/v1/chat/completions"),
            Some(("cursor-1".into(), "/chat/completions".into()))
        );
        assert_eq!(
            split("/sub/x/v1/models?a=1"),
            Some(("x".into(), "/models".into()))
        );
        assert_eq!(split("/sub/"), None);
    }
}
