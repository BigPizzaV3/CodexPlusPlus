use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::Context;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::relay_config::LOCAL_RELAY_PROXY_PORT;
use crate::settings::RelayProfile;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayProxyConfig {
    pub text_base_url: String,
    pub text_api_key: String,
    pub image_base_url: Option<String>,
    pub image_api_key: Option<String>,
}

impl RelayProxyConfig {
    pub fn from_profile(profile: &RelayProfile) -> Option<Self> {
        if !profile.needs_local_relay_proxy() {
            return None;
        }
        let (image_base_url, image_api_key) = if profile.uses_separate_image_generation_api() {
            (
                Some(normalize_base_url(&profile.image_generation_base_url)),
                Some(profile.image_generation_api_key.trim().to_string()),
            )
        } else {
            (None, None)
        };
        Some(Self {
            text_base_url: normalize_base_url(&profile.base_url),
            text_api_key: profile.api_key.trim().to_string(),
            image_base_url,
            image_api_key,
        })
    }
}

pub async fn start_local_relay_proxy(config: RelayProxyConfig) -> anyhow::Result<LocalRelayProxy> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", LOCAL_RELAY_PROXY_PORT))
        .await
        .with_context(|| {
            format!("failed to bind relay proxy on 127.0.0.1:{LOCAL_RELAY_PROXY_PORT}")
        })?;
    let client = crate::http_client::proxied_client("CodexPlusPlus-RelayProxy/1.0")?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                accepted = listener.accept() => {
                    if let Ok((stream, addr)) = accepted {
                        let config = config.clone();
                        let client = client.clone();
                        tokio::spawn(async move {
                            let _ = handle_proxy_connection(stream, addr, config, client).await;
                        });
                    }
                }
            }
        }
    });
    Ok(LocalRelayProxy {
        shutdown: shutdown_tx,
        task,
    })
}

pub struct LocalRelayProxy {
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl LocalRelayProxy {
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.task.await;
    }
}

async fn handle_proxy_connection(
    mut stream: tokio::net::TcpStream,
    remote_addr: SocketAddr,
    config: RelayProxyConfig,
    client: reqwest::Client,
) -> anyhow::Result<()> {
    let request = read_http_request(&mut stream).await?;
    let routed = route_request(&request.body, config.image_base_url.is_some());
    let (target_base, target_key) = match routed.route {
        RelayRoute::Image => match (&config.image_base_url, &config.image_api_key) {
            (Some(base_url), Some(api_key)) => (base_url, api_key),
            _ => (&config.text_base_url, &config.text_api_key),
        },
        RelayRoute::Text => (&config.text_base_url, &config.text_api_key),
    };
    let target_url = target_url(target_base, &request.path);

    let _ = crate::diagnostic_log::append_diagnostic_log(
        "relay_proxy.request",
        serde_json::json!({
            "method": request.method,
            "path": request.path,
            "route": routed.route.as_str(),
            "remote_addr": remote_addr.to_string(),
            "body_bytes": request.body.len(),
        }),
    );

    if request.method == "OPTIONS" {
        write_raw_response(&mut stream, 204, "No Content", &[], &[]).await?;
        return Ok(());
    }
    if request.method != "POST" && request.method != "GET" {
        let body = br#"{"error":{"message":"Method not allowed"}}"#;
        write_json_response(&mut stream, 405, "Method Not Allowed", body).await?;
        return Ok(());
    }

    let mut builder = match request.method.as_str() {
        "GET" => client.get(&target_url),
        _ => client.post(&target_url).body(routed.body),
    };
    builder = builder.bearer_auth(target_key);
    for (name, value) in request.forward_headers() {
        builder = builder.header(name, value);
    }

    match builder.send().await {
        Ok(response) => {
            let status = response.status();
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    let name = name.as_str().to_ascii_lowercase();
                    if matches!(
                        name.as_str(),
                        "content-type" | "cache-control" | "openai-request-id" | "x-request-id"
                    ) {
                        value.to_str().ok().map(|value| (name, value.to_string()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            let bytes = response.bytes().await.unwrap_or_default();
            write_raw_response(
                &mut stream,
                status.as_u16(),
                status.canonical_reason().unwrap_or("OK"),
                &headers,
                &bytes,
            )
            .await?;
        }
        Err(error) => {
            let body = serde_json::to_vec(&serde_json::json!({
                "error": {
                    "message": format!("Codex++ relay proxy request failed: {error}")
                }
            }))?;
            write_json_response(&mut stream, 502, "Bad Gateway", &body).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ParsedHttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl ParsedHttpRequest {
    fn forward_headers(&self) -> Vec<(&str, &str)> {
        self.headers
            .iter()
            .filter_map(|(name, value)| {
                let lowered = name.to_ascii_lowercase();
                if matches!(
                    lowered.as_str(),
                    "authorization" | "host" | "connection" | "content-length"
                ) {
                    None
                } else {
                    Some((name.as_str(), value.as_str()))
                }
            })
            .collect()
    }
}

async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
) -> anyhow::Result<ParsedHttpRequest> {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 8192];
    let header_end;
    loop {
        let read = stream.read(&mut temp).await?;
        if read == 0 {
            anyhow::bail!("empty relay proxy request");
        }
        buffer.extend_from_slice(&temp[..read]);
        if let Some(end) = find_header_end(&buffer) {
            header_end = end;
            break;
        }
        if buffer.len() > 1024 * 1024 {
            anyhow::bail!("relay proxy request headers are too large");
        }
    }

    let head = String::from_utf8_lossy(&buffer[..header_end]);
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let headers = parse_headers(lines);
    let content_length = headers
        .get("content-length")
        .or_else(|| headers.get("Content-Length"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buffer[header_end + 4..].to_vec();
    while body.len() < content_length {
        let read = stream.read(&mut temp).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&temp[..read]);
    }
    body.truncate(content_length);

    Ok(ParsedHttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn parse_headers<'a>(lines: impl Iterator<Item = &'a str>) -> HashMap<String, String> {
    let mut headers = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    headers
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayRoute {
    Text,
    Image,
}

impl RelayRoute {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedRequest {
    pub route: RelayRoute,
    pub body: Vec<u8>,
}

pub fn route_request(body: &[u8], allow_image_route: bool) -> RoutedRequest {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return RoutedRequest {
            route: RelayRoute::Text,
            body: body.to_vec(),
        };
    };
    if allow_image_route && request_requires_image_generation(&value) {
        return RoutedRequest {
            route: RelayRoute::Image,
            body: body.to_vec(),
        };
    }
    let sanitized = remove_image_generation_tools(value);
    RoutedRequest {
        route: RelayRoute::Text,
        body: serde_json::to_vec(&sanitized).unwrap_or_else(|_| body.to_vec()),
    }
}

fn request_requires_image_generation(value: &Value) -> bool {
    tool_choice_is_image_generation(value) || prompt_asks_for_image_generation(value)
}

fn tool_choice_is_image_generation(value: &Value) -> bool {
    value
        .get("tool_choice")
        .is_some_and(value_mentions_image_generation)
}

fn prompt_asks_for_image_generation(value: &Value) -> bool {
    let mut text = String::new();
    collect_text_content(value.get("input").unwrap_or(value), &mut text);
    let lower = text.to_ascii_lowercase();
    lower.contains("generate image")
        || lower.contains("create image")
        || lower.contains("draw ")
        || lower.contains("生成图片")
        || lower.contains("创建图片")
        || lower.contains("画一张")
        || lower.contains("绘制")
}

fn collect_text_content(value: &Value, output: &mut String) {
    match value {
        Value::String(value) => {
            output.push_str(value);
            output.push('\n');
        }
        Value::Array(items) => {
            for item in items {
                collect_text_content(item, output);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(key.as_str(), "text" | "content" | "input_text") {
                    collect_text_content(value, output);
                } else if matches!(key.as_str(), "input" | "messages") {
                    collect_text_content(value, output);
                }
            }
        }
        _ => {}
    }
}

fn remove_image_generation_tools(mut value: Value) -> Value {
    if let Some(tools) = value.get_mut("tools").and_then(Value::as_array_mut) {
        tools.retain(|tool| !tool_is_image_generation(tool));
    }
    if value
        .get("tool_choice")
        .is_some_and(value_mentions_image_generation)
    {
        if let Some(object) = value.as_object_mut() {
            object.remove("tool_choice");
        }
    }
    value
}

fn tool_is_image_generation(value: &Value) -> bool {
    value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "image_generation")
}

fn value_mentions_image_generation(value: &Value) -> bool {
    match value {
        Value::String(value) => value == "image_generation",
        Value::Array(items) => items.iter().any(value_mentions_image_generation),
        Value::Object(map) => map.iter().any(|(key, value)| {
            key == "image_generation"
                || (key == "type" && value.as_str() == Some("image_generation"))
                || value_mentions_image_generation(value)
        }),
        _ => false,
    }
}

pub fn target_url(base_url: &str, path: &str) -> String {
    let base = normalize_base_url(base_url);
    let path = path.split('?').next().unwrap_or(path);
    if path.ends_with("/responses") {
        format!("{base}/responses")
    } else if path.ends_with("/models") {
        format!("{base}/models")
    } else {
        format!(
            "{base}{}",
            ensure_leading_slash(path.trim_start_matches("/v1"))
        )
    }
}

fn ensure_leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn normalize_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

async fn write_json_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    write_raw_response(
        stream,
        status,
        reason,
        &[("content-type".to_string(), "application/json".to_string())],
        body,
    )
    .await
}

async fn write_raw_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> anyhow::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn route_request_removes_advertised_image_generation_tool_for_regular_text() {
        let body = serde_json::to_vec(&json!({
            "input": "hello",
            "tools": [{"type": "image_generation"}, {"type": "web_search"}]
        }))
        .unwrap();
        let routed = route_request(&body, true);
        let value: Value = serde_json::from_slice(&routed.body).unwrap();

        assert_eq!(routed.route, RelayRoute::Text);
        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "web_search");
    }

    #[test]
    fn route_request_uses_image_api_for_forced_image_tool_choice() {
        let body = serde_json::to_vec(&json!({
            "input": "make one",
            "tools": [{"type": "image_generation"}],
            "tool_choice": {"type": "image_generation"}
        }))
        .unwrap();

        assert_eq!(route_request(&body, true).route, RelayRoute::Image);
    }

    #[test]
    fn route_request_uses_image_api_for_image_prompt() {
        let body = serde_json::to_vec(&json!({
            "input": "请生成图片：一辆红色跑车",
            "tools": [{"type": "image_generation"}]
        }))
        .unwrap();

        assert_eq!(route_request(&body, true).route, RelayRoute::Image);
    }

    #[test]
    fn route_request_removes_image_tool_when_image_route_is_disabled() {
        let body = serde_json::to_vec(&json!({
            "input": "请生成图片：一辆红色跑车",
            "tools": [{"type": "image_generation"}, {"type": "web_search"}],
            "tool_choice": {"type": "image_generation"}
        }))
        .unwrap();

        let routed = route_request(&body, false);
        let value: Value = serde_json::from_slice(&routed.body).unwrap();

        assert_eq!(routed.route, RelayRoute::Text);
        assert_eq!(value["tools"].as_array().unwrap().len(), 1);
        assert_eq!(value["tools"][0]["type"], "web_search");
        assert!(value.get("tool_choice").is_none());
    }

    #[test]
    fn target_url_normalizes_supported_paths() {
        assert_eq!(
            target_url("https://relay.example", "/v1/responses"),
            "https://relay.example/v1/responses"
        );
        assert_eq!(
            target_url("https://relay.example/v1", "/models"),
            "https://relay.example/v1/models"
        );
    }
}
