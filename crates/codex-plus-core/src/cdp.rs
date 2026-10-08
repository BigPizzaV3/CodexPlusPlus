use anyhow::{Context, bail};
use serde::Deserialize;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

const CDP_HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const CDP_PROBE_TIMEOUT: Duration = Duration::from_millis(300);
const CDP_PROBE_MAX_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default, rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CdpBrowserIdentity {
    #[serde(rename = "Browser")]
    pub browser: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub web_socket_debugger_url: String,
}

impl CdpBrowserIdentity {
    pub fn browser_id(&self) -> anyhow::Result<String> {
        let url = reqwest::Url::parse(&self.web_socket_debugger_url)
            .context("invalid browser WebSocket URL")?;
        let mut segments = url
            .path_segments()
            .ok_or_else(|| anyhow::anyhow!("browser WebSocket URL has no path"))?;
        match (segments.next(), segments.next(), segments.next()) {
            (Some("devtools"), Some("browser"), Some(id)) if !id.is_empty() => Ok(id.to_string()),
            _ => bail!("browser WebSocket URL has no Browser ID"),
        }
    }
}

/// Returns whether the requested loopback port exposes a CDP target list.
pub(crate) fn endpoint_available(debug_port: u16) -> bool {
    [
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), debug_port),
        SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), debug_port),
    ]
    .into_iter()
    .any(|address| probe_endpoint(address, debug_port))
}

fn probe_endpoint(address: SocketAddr, debug_port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, CDP_PROBE_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(CDP_PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CDP_PROBE_TIMEOUT));
    let request =
        format!("GET /json HTTP/1.1\r\nHost: 127.0.0.1:{debug_port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }

    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    while response.len() < CDP_PROBE_MAX_BYTES {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&chunk[..read]);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => return false,
        }
    }

    response_contains_codex_target(&response, debug_port)
}

fn response_contains_codex_target(response: &[u8], debug_port: u16) -> bool {
    if response.len() > CDP_PROBE_MAX_BYTES {
        return false;
    }
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&response[..header_end]);
    let status_ok = headers
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/") && line.contains(" 200 "));
    if !status_ok {
        return false;
    }
    let Some(body) = decode_probe_response_body(&headers, &response[header_end + 4..]) else {
        return false;
    };
    let Ok(targets) = serde_json::from_slice::<Vec<CdpTarget>>(&body) else {
        return false;
    };
    targets.iter().any(|target| {
        is_injectable_page_target(target)
            // 主窗口标题会随产品名称或页面状态变化；与注入目标选择保持一致。
            && (is_primary_codex_page_target(target) || is_exact_codex_app_main_target(target))
            && target
                .url
                .trim()
                .to_ascii_lowercase()
                .starts_with("app://-/")
            && target
                .web_socket_debugger_url
                .as_deref()
                .is_some_and(|url| validate_cdp_websocket_url(url, debug_port).is_ok())
    })
}

// CDP 的 HTTP/1.1 响应可能使用 chunked，不能把分块长度直接交给 JSON 解析器。
fn decode_probe_response_body(headers: &str, body: &[u8]) -> Option<Vec<u8>> {
    let mut chunked = false;
    let mut content_length = None;
    for line in headers.lines().skip(1) {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("transfer-encoding") {
            if chunked || !value.trim().eq_ignore_ascii_case("chunked") {
                return None;
            }
            chunked = true;
        } else if name.trim().eq_ignore_ascii_case("content-length") {
            let length = value.trim().parse::<usize>().ok()?;
            if content_length.is_some_and(|previous| previous != length) {
                return None;
            }
            content_length = Some(length);
        }
    }
    if chunked {
        if content_length.is_some() {
            return None;
        }
        decode_probe_chunked_body(body)
    } else if let Some(length) = content_length {
        body.get(..length).map(<[u8]>::to_vec)
    } else {
        Some(body.to_vec())
    }
}

fn decode_probe_chunked_body(body: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::new();
    let mut offset = 0;
    loop {
        let remaining = body.get(offset..)?;
        let line_end = remaining.windows(2).position(|window| window == b"\r\n")?;
        let size_line = std::str::from_utf8(&remaining[..line_end]).ok()?;
        let size = usize::from_str_radix(size_line.split(';').next()?.trim(), 16).ok()?;
        offset += line_end + 2;
        if size == 0 {
            // 终止块后允许 trailer，但必须读到完整的空行才算响应完整。
            loop {
                let remaining = body.get(offset..)?;
                let line_end = remaining.windows(2).position(|window| window == b"\r\n")?;
                if line_end == 0 {
                    return Some(decoded);
                }
                if !remaining[..line_end].contains(&b':') {
                    return None;
                }
                offset += line_end + 2;
            }
        }
        let chunk_end = offset.checked_add(size)?;
        let terminator_end = chunk_end.checked_add(2)?;
        if body.get(chunk_end..terminator_end)? != b"\r\n" {
            return None;
        }
        if decoded.len().checked_add(size)? > CDP_PROBE_MAX_BYTES {
            return None;
        }
        decoded.extend_from_slice(body.get(offset..chunk_end)?);
        offset = terminator_end;
    }
}

pub async fn list_targets(debug_port: u16) -> anyhow::Result<Vec<CdpTarget>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(CDP_HTTP_TIMEOUT)
        .build()
        .context("failed to build CDP HTTP client")?;

    let urls = [
        format!("http://127.0.0.1:{debug_port}/json"),
        format!("http://[::1]:{debug_port}/json"),
    ];
    let mut errors = Vec::new();
    for url in urls {
        match query_targets_url(&client, &url, debug_port).await {
            Ok(targets) => return Ok(targets),
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }

    bail!(
        "failed to query CDP targets on loopback addresses: {}",
        errors.join("; ")
    )
}

pub async fn browser_identity(debug_port: u16) -> anyhow::Result<CdpBrowserIdentity> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(CDP_HTTP_TIMEOUT)
        .build()
        .context("failed to build CDP HTTP client")?;
    let urls = [
        format!("http://127.0.0.1:{debug_port}/json/version"),
        format!("http://[::1]:{debug_port}/json/version"),
    ];
    let mut errors = Vec::new();
    for url in urls {
        let result = async {
            let identity = client
                .get(&url)
                .send()
                .await
                .context("failed to query CDP browser identity")?
                .error_for_status()
                .context("CDP browser identity query failed")?
                .json::<CdpBrowserIdentity>()
                .await
                .context("failed to deserialize CDP browser identity")?;
            validate_cdp_websocket_url(&identity.web_socket_debugger_url, debug_port)?;
            identity.browser_id()?;
            anyhow::Ok(identity)
        }
        .await;
        match result {
            Ok(identity) => return Ok(identity),
            Err(error) => errors.push(format!("{url}: {error:#}")),
        }
    }
    bail!(
        "failed to query CDP browser identity on loopback addresses: {}",
        errors.join("; ")
    )
}

async fn query_targets_url(
    client: &reqwest::Client,
    url: &str,
    debug_port: u16,
) -> anyhow::Result<Vec<CdpTarget>> {
    let response = client
        .get(url)
        .send()
        .await
        .context("failed to query CDP targets")?
        .error_for_status()
        .context("CDP target query failed")?;

    let targets = response
        .json::<Vec<CdpTarget>>()
        .await
        .context("failed to deserialize CDP targets")?;
    for target in &targets {
        if let Some(websocket_url) = target.web_socket_debugger_url.as_deref() {
            validate_cdp_websocket_url(websocket_url, debug_port).with_context(|| {
                format!("unsafe CDP target WebSocket URL for target {}", target.id)
            })?;
        }
    }
    Ok(targets)
}

pub fn validate_cdp_websocket_url(url: &str, expected_port: u16) -> anyhow::Result<()> {
    let parsed = reqwest::Url::parse(url).context("invalid CDP WebSocket URL")?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        bail!("CDP WebSocket URL must use ws or wss");
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("CDP WebSocket URL has no host"))?;
    let address = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<IpAddr>()
        .with_context(|| "CDP WebSocket host must be a loopback IP address")?;
    if !address.is_loopback() {
        bail!("CDP WebSocket host must be loopback");
    }
    let port = parsed
        .port()
        .ok_or_else(|| anyhow::anyhow!("CDP WebSocket URL must include an explicit port"))?;
    if port != expected_port {
        bail!("CDP WebSocket port {port} does not match debug port {expected_port}");
    }
    Ok(())
}

pub fn pick_page_target(targets: &[CdpTarget]) -> anyhow::Result<CdpTarget> {
    let mut first_page = None;
    for target in targets
        .iter()
        .filter(|target| is_injectable_page_target(target))
    {
        first_page.get_or_insert(target);
        if is_primary_codex_page_target(target) {
            return Ok(target.clone());
        }
    }

    if let Some(target) = first_page {
        return Ok(target.clone());
    }

    bail!("No injectable page target found")
}

pub fn pick_injectable_codex_page_target(targets: &[CdpTarget]) -> anyhow::Result<CdpTarget> {
    let priorities: [fn(&CdpTarget) -> bool; 4] = [
        is_exact_codex_app_main_target,
        is_primary_codex_app_target,
        is_chatgpt_desktop_page_target,
        is_supported_codex_page_target,
    ];
    for matches_priority in priorities {
        if let Some(target) = targets
            .iter()
            .find(|target| is_injectable_page_target(target) && matches_priority(target))
        {
            return Ok(target.clone());
        }
    }
    bail!("No injectable Codex page target found")
}

fn is_codex_app_page_target(target: &CdpTarget) -> bool {
    let Ok(url) = reqwest::Url::parse(target.url.trim()) else {
        return false;
    };
    url.scheme().eq_ignore_ascii_case("app")
        && url.host_str() == Some("-")
        && url.path().eq_ignore_ascii_case("/index.html")
}

pub fn is_injectable_page_target(target: &CdpTarget) -> bool {
    target.target_type == "page"
        && target
            .web_socket_debugger_url
            .as_deref()
            .is_some_and(|url| !url.is_empty())
}

pub fn is_codex_page_target(target: &CdpTarget) -> bool {
    if target.target_type != "page" {
        return false;
    }
    let haystack = format!("{} {}", target.title, target.url).to_lowercase();
    haystack.contains("codex") || is_chatgpt_desktop_page(&target.title, &target.url)
}

pub fn is_primary_codex_page_target(target: &CdpTarget) -> bool {
    is_codex_page_target(target)
        && !is_avatar_overlay_page_target(target)
        && !is_quick_chat_page_target(target)
}

fn is_exact_codex_app_main_target(target: &CdpTarget) -> bool {
    target.url.trim().eq_ignore_ascii_case("app://-/index.html")
}

fn is_primary_codex_app_target(target: &CdpTarget) -> bool {
    is_codex_app_page_target(target) && is_primary_codex_page_target(target)
}

fn is_chatgpt_desktop_page_target(target: &CdpTarget) -> bool {
    is_primary_codex_page_target(target) && is_chatgpt_desktop_page(&target.title, &target.url)
}

fn is_supported_codex_page_target(target: &CdpTarget) -> bool {
    is_primary_codex_page_target(target)
        && (is_codex_app_page_target(target) || is_chatgpt_desktop_page(&target.title, &target.url))
}

pub fn is_avatar_overlay_page_target(target: &CdpTarget) -> bool {
    initial_route(target).is_some_and(|route| route.eq_ignore_ascii_case("/avatar-overlay"))
}

pub fn is_quick_chat_page_target(target: &CdpTarget) -> bool {
    initial_route(target).is_some_and(|route| {
        let route = route.to_ascii_lowercase();
        route == "/chatgpt/quick-chat"
            || route == "/chatgpt/quick-chat-prewarm"
            || route.starts_with("/chatgpt/quick-chat/")
    })
}

fn initial_route(target: &CdpTarget) -> Option<String> {
    if !is_injectable_page_target(target) {
        return None;
    }
    let url = reqwest::Url::parse(target.url.trim()).ok()?;
    if !is_codex_app_page_target(target) {
        return None;
    }
    url.query_pairs()
        .find(|(key, _)| key.eq_ignore_ascii_case("initialRoute"))
        .map(|(_, value)| value.into_owned())
}

fn is_chatgpt_desktop_page(title: &str, url: &str) -> bool {
    let title = title.trim().to_ascii_lowercase();
    let url = url.trim().to_ascii_lowercase();
    title == "chatgpt"
        && (url == "https://chatgpt.com"
            || url.starts_with("https://chatgpt.com/")
            || url == "https://chat.openai.com"
            || url.starts_with("https://chat.openai.com/")
            || url.starts_with("data:text/html"))
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn target_body(title: &str, url: &str) -> String {
        serde_json::json!([{
            "id": "codex",
            "type": "page",
            "title": title,
            "url": url,
            "webSocketDebuggerUrl": "ws://127.0.0.1:9229/devtools/page/1"
        }])
        .to_string()
    }

    #[test]
    fn response_parser_accepts_chunked_targets_with_extensions_and_trailers() {
        let body = target_body("Codex", "app://-/index.html");
        let (first, second) = body.split_at(body.len() / 2);
        let response = format!(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: Chunked\r\nConnection: close\r\n\r\n{:x};fixture=yes\r\n{first}\r\n{:x}\r\n{second}\r\n0\r\nX-Fixture: done\r\n\r\n",
            first.len(),
            second.len()
        );

        assert!(response_contains_codex_target(response.as_bytes(), 9229));
    }

    #[test]
    fn response_parser_accepts_app_main_independently_of_window_title() {
        for title in ["ChatGPT", ""] {
            let body = target_body(title, "app://-/index.html");
            let targets = serde_json::from_str::<Vec<CdpTarget>>(&body).unwrap();
            assert!(pick_injectable_codex_page_target(&targets).is_ok());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                body.len()
            );

            assert!(response_contains_codex_target(response.as_bytes(), 9229));
        }
    }

    #[test]
    fn response_parser_rejects_incomplete_or_conflicting_body_framing() {
        let body = target_body("Codex", "app://-/index.html");
        let encoded = format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
        let headers = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
        for prefix in 0..encoded.len() {
            let response = format!("{headers}{}", &encoded[..prefix]);
            assert!(!response_contains_codex_target(response.as_bytes(), 9229));
        }
        for response in [
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len() + 1
            ),
            format!(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: {}\r\n\r\n{encoded}",
                body.len()
            ),
            format!("{headers}ffffffffffffffff\r\n{body}\r\n0\r\n\r\n"),
            format!("{headers}1\r\n{body}\r\n0\r\n\r\n"),
        ] {
            assert!(!response_contains_codex_target(response.as_bytes(), 9229));
        }
    }

    #[test]
    fn response_parser_keeps_target_and_websocket_restrictions() {
        for (url, websocket) in [
            (
                "app://-/index.html?initialRoute=%2Favatar-overlay",
                "ws://127.0.0.1:9229/devtools/page/1",
            ),
            (
                "app://-/index.html?initialRoute=%2Fchatgpt%2Fquick-chat-prewarm",
                "ws://127.0.0.1:9229/devtools/page/1",
            ),
            ("app://-/index.html", "ws://127.0.0.1:9230/devtools/page/1"),
            (
                "app://-/index.html",
                "ws://203.0.113.1:9229/devtools/page/1",
            ),
        ] {
            let body = target_body("ChatGPT", url)
                .replace("ws://127.0.0.1:9229/devtools/page/1", websocket);
            let response = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{body}");
            assert!(!response_contains_codex_target(response.as_bytes(), 9229));
        }
    }

    #[test]
    fn response_parser_accepts_close_delimited_targets() {
        let body = target_body("Codex", "app://-/index.html");
        let response = format!("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n{body}");

        assert!(response_contains_codex_target(response.as_bytes(), 9229));
    }

    fn serve_once(build_body: impl FnOnce(u16) -> String) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = build_body(port);
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (port, handle)
    }

    #[test]
    fn endpoint_available_accepts_devtools_target_response() {
        let (port, server) = serve_once(|port| {
            format!(
                r#"[{{"id":"codex","type":"page","title":"Codex","url":"app://-/index.html","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#
            )
        });

        assert!(endpoint_available(port));
        server.join().unwrap();
    }

    #[test]
    fn endpoint_available_rejects_ordinary_http_response() {
        let (port, server) = serve_once(|_| r#"{"status":"ok"}"#.to_string());

        assert!(!endpoint_available(port));
        server.join().unwrap();
    }

    #[test]
    fn endpoint_available_rejects_non_codex_devtools_target() {
        let (port, server) = serve_once(|port| {
            format!(
                r#"[{{"id":"chrome","type":"page","title":"New Tab","url":"chrome://newtab","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#
            )
        });

        assert!(!endpoint_available(port));
        server.join().unwrap();
    }

    #[test]
    fn endpoint_available_rejects_chatgpt_web_target() {
        let (port, server) = serve_once(|port| {
            format!(
                r#"[{{"id":"chatgpt","type":"page","title":"ChatGPT","url":"https://chatgpt.com/","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#
            )
        });

        assert!(!endpoint_available(port));
        server.join().unwrap();
    }

    #[test]
    fn endpoint_available_rejects_quick_chat_only_target() {
        let (port, server) = serve_once(|port| {
            format!(
                r#"[{{"id":"quick-chat","type":"page","title":"Codex","url":"app://-/index.html?initialRoute=%2Fchatgpt%2Fquick-chat-prewarm","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#
            )
        });

        assert!(!endpoint_available(port));
        server.join().unwrap();
    }
}
