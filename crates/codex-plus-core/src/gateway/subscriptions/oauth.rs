//! 订阅登录共用的工具：PKCE、本机回调、设备码轮询、JWT、时间。

use std::collections::BTreeMap;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::Value;

/// 浏览器登录 / 设备码最多等这么久。
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// PKCE：(verifier, S256 challenge)。
pub fn pkce_pair() -> (String, String) {
    crate::gateway::accounts::pkce_pair()
}

/// 随机 state / nonce。
pub fn random_state() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// 毫秒时间戳。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// JWT 的载荷（不验签，只读字段）；解不开时为 Null。
pub fn jwt_claims(token: &str) -> Value {
    crate::gateway::accounts::jwt_claims(token)
}

/// JWT 的过期时间（毫秒）；没有 exp 时为 None。
pub fn jwt_expires_ms(token: &str) -> Option<u64> {
    jwt_claims(token)
        .get("exp")
        .and_then(Value::as_u64)
        .map(|exp| exp * 1000)
}

/// 凭据里的毫秒时间戳字段离现在不到 `ahead_ms`（或没有）：该刷新了。
pub fn expiring(credentials: &Value, field: &str, ahead_ms: u64) -> bool {
    credentials
        .get(field)
        .and_then(Value::as_u64)
        .is_none_or(|expires| expires <= now_ms() + ahead_ms)
}

/// URL 查询串编码。
pub fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// `a=1&b=2`。
pub fn query(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{}={}", encode(key), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// 在本机 `port` 上等浏览器回调 `path`（校验 state），返回回调带的参数（code 等）。
/// `port` 为 0 时由系统分配；监听要在打开浏览器前建好，所以分两步：先 [`bind_callback`]。
pub async fn bind_callback(port: u16) -> anyhow::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .with_context(|| format!("本机端口 {port} 被占用，无法接收登录回调"))
}

pub async fn wait_callback(
    listener: &tokio::net::TcpListener,
    path: &str,
    state: &str,
) -> anyhow::Result<BTreeMap<String, String>> {
    tokio::time::timeout(
        LOGIN_TIMEOUT,
        crate::gateway::accounts::wait_for_callback_params(listener, path, state),
    )
    .await
    .map_err(|_| anyhow::anyhow!("登录超时：5 分钟内没有完成浏览器登录"))?
}

/// POST 表单，期待 JSON 答复；非 2xx 时带上答复里的错误信息。
pub async fn post_form(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let response = http
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "application/json")
        .body(query(form))
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    read_json(response).await
}

/// POST JSON，期待 JSON 答复。
pub async fn post_json(
    http: &reqwest::Client,
    url: &str,
    body: &Value,
    headers: &[(&str, &str)],
) -> anyhow::Result<Value> {
    let mut request = http.post(url).json(body);
    for (key, value) in headers {
        request = request.header(*key, *value);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("无法连接 {url}"))?;
    read_json(response).await
}

/// 读 JSON 答复；非 2xx 时报错（带状态码，401/403 让管道停用这个账号）。
pub async fn read_json(response: reqwest::Response) -> anyhow::Result<Value> {
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("error_description")
                    .or_else(|| value.pointer("/error/message"))
                    .or_else(|| value.get("message"))
                    .or_else(|| value.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| text.chars().take(300).collect());
        return Err(crate::gateway::subscriptions::fail(
            status,
            format!("返回 {status}：{detail}"),
        ));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).context("答复不是 JSON")
}

/// 设备码轮询：每 `interval` 调一次 `poll`，返回 Some 时结束；超时报错。
pub async fn poll_until<F, Fut, T>(interval: Duration, mut poll: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Option<T>>>,
{
    let deadline = tokio::time::Instant::now() + LOGIN_TIMEOUT;
    loop {
        if let Some(value) = poll().await? {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            bail!("登录超时：5 分钟内没有完成授权");
        }
        tokio::time::sleep(interval.max(Duration::from_secs(1))).await;
    }
}

/// 读本机文件里的 JSON（导入官方客户端的登录用）；不存在时报清楚的错误。
pub fn read_local_json(path: &std::path::Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("没找到 {}，请先在官方客户端里登录", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("{} 不是合法的 JSON", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_encodes() {
        assert_eq!(query(&[("a b", "x/y"), ("c", "1")]), "a%20b=x%2Fy&c=1");
    }

    #[test]
    fn expiring_checks_ahead() {
        let soon = serde_json::json!({ "exp": now_ms() + 1000 });
        assert!(expiring(&soon, "exp", 60_000));
        let later = serde_json::json!({ "exp": now_ms() + 3_600_000 });
        assert!(!expiring(&later, "exp", 60_000));
        assert!(expiring(&serde_json::json!({}), "exp", 0));
    }
}
