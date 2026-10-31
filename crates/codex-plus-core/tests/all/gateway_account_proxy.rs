//! #2775：网关账号类供应商（ChatGPT 官方登录、Copilot 等）单独填了代理时，
//! 额度 / 账号查询这些属于该供应商的请求也要经过它，不能只看系统代理。

use base64::Engine;
use codex_plus_core::gateway::quota;
use codex_plus_core::settings::{RelayMode, RelayProfile, RelayProtocol};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 本地「代理」：收下第一条请求的请求头，回 502 关掉（上游本来就到不了，只看请求有没有来）。
async fn record_first_request(listener: tokio::net::TcpListener) -> String {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let _ = stream
        .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await;
    let _ = stream.shutdown().await;
    String::from_utf8_lossy(&buffer).to_string()
}

/// 等代理收到请求；请求没走代理时不会有连接进来，超时按「没经过代理」报失败而不是卡住。
async fn proxied_request(proxy: tokio::task::JoinHandle<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), proxy)
        .await
        .expect("请求没有经过供应商代理")
        .unwrap()
}

fn has_proxy_auth(request: &str, user: &str, password: &str) -> bool {
    let expected = format!(
        "basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    );
    request.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("proxy-authorization")
                && value.trim().eq_ignore_ascii_case(&expected)
        })
    })
}

fn fake_jwt(payload: serde_json::Value) -> String {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = engine.encode(br#"{"alg":"none"}"#);
    let body = engine.encode(serde_json::to_vec(&payload).unwrap());
    format!("{header}.{body}.")
}

#[tokio::test]
async fn chatgpt_quota_lookup_uses_the_providers_own_proxy() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = tokio::spawn(record_first_request(listener));

    // 网关自己登录的 ChatGPT 账号，access token 还没过期：不刷新、不读本机状态，直接查额度。
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let relay = RelayProfile {
        id: "gateway-chatgpt-2775".to_string(),
        name: "ChatGPT Plus".to_string(),
        relay_mode: RelayMode::Official,
        protocol: RelayProtocol::Responses,
        auth_contents: json!({
            "auth_mode": "chatgpt",
            "codex_plus_gateway": true,
            "tokens": {
                "access_token": fake_jwt(json!({ "exp": exp })),
                "refresh_token": "refresh-2775",
                "account_id": "acct-quota-2775"
            }
        })
        .to_string(),
        gateway_proxy: Some(format!("http://quota-user:quota-pass@{address}")),
        ..RelayProfile::default()
    };
    assert!(codex_plus_core::official_login::member_uses_official_login_transport(&relay));

    // 代理回 502，查询本身失败；要看的是请求确实走了这个代理并带上认证。
    assert!(quota::refresh(&relay).await.is_err());
    let request = proxied_request(proxy).await;
    assert!(
        request.starts_with("CONNECT chatgpt.com:443 "),
        "额度查询没有经过供应商代理：{request}"
    );
    assert!(
        has_proxy_auth(&request, "quota-user", "quota-pass"),
        "{request}"
    );
}

#[tokio::test]
async fn copilot_account_lookup_uses_the_providers_own_proxy() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = tokio::spawn(record_first_request(listener));

    let relay = RelayProfile {
        id: "gateway-copilot-2775".to_string(),
        name: "Copilot · octocat".to_string(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        auth_contents: json!({
            "codex_plus_account": "copilot",
            "github_token": "gho_2775"
        })
        .to_string(),
        gateway_proxy: Some(format!("http://gh-user:gh-pass@{address}")),
        ..RelayProfile::default()
    };

    assert!(quota::refresh(&relay).await.is_err());
    let request = proxied_request(proxy).await;
    assert!(
        request.starts_with("CONNECT api.github.com:443 "),
        "Copilot 账号查询没有经过供应商代理：{request}"
    );
    assert!(has_proxy_auth(&request, "gh-user", "gh-pass"), "{request}");
}
