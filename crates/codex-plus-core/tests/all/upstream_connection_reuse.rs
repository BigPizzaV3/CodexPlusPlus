//! 网关 / 协议代理转发上游时复用连接（#2761）：连续请求同一供应商只建一次 TCP 连接，
//! 供应商的代理设置变了就换新的 client，不会沿用旧代理。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use codex_plus_core::protocol_proxy::{
    ForcedRelay, ProxySessionHeaders, open_responses_proxy_request_for_relay,
};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn chat_completion() -> String {
    json!({
        "id": "c", "object": "chat.completion", "model": "m",
        "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }]
    })
    .to_string()
}

/// 读一个完整的 HTTP/1.1 请求（头 + content-length 的体），连接已关时返回 None。
async fn read_request(stream: &mut TcpStream, buffer: &mut Vec<u8>) -> Option<String> {
    let mut chunk = [0_u8; 8192];
    let (end, length) = loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            break (end + 4, length);
        }
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    while buffer.len() < end + length {
        let read = stream.read(&mut chunk).await.ok()?;
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let head = String::from_utf8_lossy(&buffer[..end]).into_owned();
    buffer.drain(..end + length);
    Some(head)
}

/// 支持 keep-alive 的假上游：每条连接上可以连续回多个请求。
/// `accepts` 记建立了几条 TCP 连接，`request_lines` 记收到的请求行。
/// `drop_request` 为 Some(n) 时，第 n 个请求（从 1 数）读完后不回应、直接关连接，
/// 模拟空闲连接刚被对端关掉。
/// `drop_delay` 是读完那个请求后等多久再关连接：等得久模拟上游处理到一半被中间设备断开。
struct MockUpstream {
    addr: std::net::SocketAddr,
    accepts: Arc<AtomicUsize>,
    request_lines: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn mock_upstream(drop_request: Option<usize>) -> MockUpstream {
    mock_upstream_with_drop_delay(drop_request, Duration::ZERO).await
}

async fn mock_upstream_with_drop_delay(
    drop_request: Option<usize>,
    drop_delay: Duration,
) -> MockUpstream {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let request_lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (counter, lines) = (accepts.clone(), request_lines.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let lines = lines.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                while let Some(head) = read_request(&mut stream, &mut buffer).await {
                    let seen = {
                        let mut lines = lines.lock().unwrap();
                        lines.push(head.lines().next().unwrap_or_default().to_string());
                        lines.len()
                    };
                    if drop_request == Some(seen) {
                        tokio::time::sleep(drop_delay).await;
                        return;
                    }
                    let body = chat_completion();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                        body.len()
                    );
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    MockUpstream {
        addr,
        accepts,
        request_lines,
    }
}

fn relay(id: &str, base_url: String, proxy: &str) -> RelayProfile {
    RelayProfile {
        id: id.into(),
        name: id.into(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        base_url,
        api_key: format!("sk-{id}"),
        model_list: "m".into(),
        // 网关供应商自己的代理设置：测试里固定，不受本机系统代理影响。
        gateway_proxy: Some(proxy.into()),
        ..RelayProfile::default()
    }
}

async fn send_once(relay: &RelayProfile, user_agent: Option<&str>) {
    let status = try_send_once(relay, user_agent).await.unwrap();
    assert_eq!(status, 200);
}

/// 发一次请求，成功时读完响应体并返回状态码，上游失败时返回错误。
async fn try_send_once(relay: &RelayProfile, user_agent: Option<&str>) -> anyhow::Result<u16> {
    let body = json!({ "model": "m", "input": "hi" }).to_string();
    let mut response = open_responses_proxy_request_for_relay(
        &body,
        BackendSettings::default(),
        ForcedRelay {
            relay: relay.clone(),
            upstream_model: None,
            auto_retry: false,
        },
        user_agent,
        "/responses",
        ProxySessionHeaders::default(),
        None,
    )
    .await?;
    // 读完响应体，连接才会回到连接池。
    response.read_body().await?;
    Ok(response.status_code)
}

#[tokio::test]
async fn consecutive_requests_to_same_relay_reuse_one_connection() {
    let upstream = mock_upstream(None).await;
    let relay = relay(
        "reuse-2761",
        format!("http://{}/v1", upstream.addr),
        "direct",
    );
    for _ in 0..5 {
        send_once(&relay, Some("Codex Desktop/0.165.0")).await;
    }
    assert_eq!(upstream.request_lines.lock().unwrap().len(), 5);
    assert_eq!(
        upstream.accepts.load(Ordering::SeqCst),
        1,
        "连续请求同一供应商应复用同一条连接，不该每次重新握手"
    );
}

#[tokio::test]
async fn changing_relay_proxy_switches_to_new_client() {
    // 两个假代理：HTTP 代理收到的是绝对地址的请求行。上游用不可解析的域名，
    // 请求只能经代理到达；回环地址默认不走代理，所以这里不能用 127.0.0.1。
    let first = mock_upstream(None).await;
    let second = mock_upstream(None).await;
    let base = "http://upstream-2761.invalid/v1".to_string();
    let mut relay = relay("proxy-2761", base, &format!("http://{}", first.addr));
    send_once(&relay, None).await;
    send_once(&relay, None).await;
    relay.gateway_proxy = Some(format!("http://{}", second.addr));
    send_once(&relay, None).await;

    let first_lines = first.request_lines.lock().unwrap().clone();
    let second_lines = second.request_lines.lock().unwrap().clone();
    assert_eq!(
        first_lines.len(),
        2,
        "改代理前的请求走第一个代理：{first_lines:?}"
    );
    assert_eq!(
        second_lines.len(),
        1,
        "改代理后必须换用新代理：{second_lines:?}"
    );
    assert!(
        second_lines[0].contains("http://upstream-2761.invalid/v1/chat/completions"),
        "{second_lines:?}"
    );
    assert_eq!(
        first.accepts.load(Ordering::SeqCst),
        1,
        "同一代理设置复用连接"
    );
}

#[tokio::test]
async fn request_on_connection_closed_by_upstream_is_resent_once() {
    // 第 2 个请求落在复用的连接上，对端读完不回应就关了：换新连接重发，调用方无感。
    let upstream = mock_upstream(Some(2)).await;
    let relay = relay(
        "stale-2761",
        format!("http://{}/v1", upstream.addr),
        "direct",
    );
    send_once(&relay, None).await;
    send_once(&relay, None).await;
    assert_eq!(
        upstream.request_lines.lock().unwrap().len(),
        3,
        "被丢掉的请求重发了一次"
    );
    assert_eq!(upstream.accepts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn request_closed_long_after_sending_is_not_resent() {
    // 第 2 个请求上游读完后过了重发窗口才断开：可能已收全请求、处理到一半被中转或 NAT 断开，
    // 重发会让请求在上游执行两次，所以原样报错，上游只收到这一次。
    let upstream = mock_upstream_with_drop_delay(Some(2), Duration::from_millis(1500)).await;
    let relay = relay(
        "stale-late-2761",
        format!("http://{}/v1", upstream.addr),
        "direct",
    );
    send_once(&relay, None).await;
    let second = try_send_once(&relay, None).await;
    assert!(
        !matches!(second, Ok(200)),
        "过了重发窗口才断开的请求不该被悄悄重发成功：{second:?}"
    );
    assert_eq!(
        upstream.request_lines.lock().unwrap().len(),
        2,
        "断开的请求不重发"
    );
    assert_eq!(upstream.accepts.load(Ordering::SeqCst), 1);
}

/// 复用的 client 按 tokio 运行时分开：连接由建立它的运行时驱动，一个运行时正在结束时
/// 另一个运行时恰好拿到它的连接会报 `dispatch task is gone`。两个运行时同时存活时，
/// 各自只复用自己建的连接，互不共用。
#[test]
fn reused_clients_are_scoped_to_their_runtime() {
    use std::io::{BufRead, BufReader, Read, Write};

    // 保持连接的阻塞式假上游，跑在普通线程里，不依赖任何 tokio 运行时；记建立了几条连接。
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let addr = listener.local_addr().unwrap();
    let accepts = Arc::new(AtomicUsize::new(0));
    let counter = accepts.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            counter.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut writer = stream;
                loop {
                    let mut length = 0_usize;
                    let mut line = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let lower = line.to_ascii_lowercase();
                        if let Some(value) = lower.strip_prefix("content-length:") {
                            length = value.trim().parse().unwrap_or(0);
                        }
                        if line == "\r\n" {
                            break;
                        }
                    }
                    let mut body = vec![0_u8; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let response =
                        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: 2\r\n\r\nok";
                    if writer.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });

    let relay = RelayProfile {
        id: "runtime-scoped".into(),
        base_url: format!("http://{addr}/v1"),
        gateway_proxy: Some("direct".into()),
        ..RelayProfile::default()
    };
    let user_agent = "codex-plus-test/runtime-scoped";
    let url = format!("http://{addr}/v1/ping");
    let runtime = || {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap()
    };
    let fetch = |runtime: &tokio::runtime::Runtime| {
        let (relay, url) = (relay.clone(), url.clone());
        runtime.block_on(async move {
            let client =
                codex_plus_core::http_client::proxied_client_for_relay(user_agent, &relay)
                    .unwrap();
            client.get(url).send().await.unwrap().text().await.unwrap()
        })
    };
    let (first, second) = (runtime(), runtime());
    assert_eq!(fetch(&first), "ok");
    assert_eq!(fetch(&first), "ok");
    assert_eq!(accepts.load(Ordering::SeqCst), 1, "同一运行时连续请求复用连接");
    assert_eq!(fetch(&second), "ok");
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        2,
        "另一个运行时不复用前一个运行时的连接"
    );
    assert_eq!(fetch(&second), "ok");
    assert_eq!(accepts.load(Ordering::SeqCst), 2);
}
