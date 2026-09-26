//! Read-only native extension discovery, independent of the legacy runtime patch.
//! Only getInfo is sent: no session, tab, header-policy or runtime changes.

#[cfg(any(windows, test))]
use anyhow::{Result, bail, ensure};
use serde::Serialize;
#[cfg(any(windows, test))]
use serde_json::{Value, json};
#[cfg(any(windows, test))]
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[cfg(any(windows, test))]
const MAX_FRAME: usize = 64 * 1024;
#[cfg(any(windows, test))]
const PIPE_PREFIX: &str = r"\\.\pipe\codex-browser-use";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedBrowser {
    pub family: String,
    pub header_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub state: String,
    pub browsers: Vec<ConnectedBrowser>,
    pub failed_checks: usize,
}

impl ConnectionStatus {
    pub fn failed() -> Self {
        Self {
            state: "check_failed".into(),
            browsers: vec![],
            failed_checks: 1,
        }
    }
}

#[cfg(any(windows, test))]
fn summarize(mut browsers: Vec<ConnectedBrowser>, failed_checks: usize) -> ConnectionStatus {
    browsers.sort_by(|a, b| {
        a.family
            .cmp(&b.family)
            .then(a.header_enabled.cmp(&b.header_enabled))
    });
    browsers.dedup();
    ConnectionStatus {
        state: if !browsers.is_empty() {
            "available"
        } else if failed_checks > 0 {
            "check_failed"
        } else {
            "disconnected"
        }
        .into(),
        browsers,
        failed_checks,
    }
}

#[cfg(any(windows, test))]
fn browser_info(info: &Value) -> Result<Option<ConnectedBrowser>> {
    let kind = info["type"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing backend type"))?;
    if kind != "extension" {
        ensure!(matches!(kind, "iab" | "cdp"), "Unknown browser backend");
        return Ok(None);
    }
    let family = info["family"].as_str().unwrap_or_default();
    let extension = info["metadata"]["extensionId"].as_str().unwrap_or_default();
    ensure!(
        matches!(
            (family, extension),
            ("edge", "odlomjlbamekndcpllcnffbgeohgkmjh")
                | ("chrome", "hehggadaopoacecdllhhajmbjkdcmajg")
        ),
        "Unknown browser extension"
    );
    ensure!(
        info.get("agentRequestHeaderEnabled")
            .is_none_or(Value::is_boolean),
        "Invalid header status"
    );
    Ok(Some(ConnectedBrowser {
        family: family.into(),
        header_enabled: info["agentRequestHeaderEnabled"].as_bool(),
    }))
}

#[cfg(any(windows, test))]
fn candidate_pipe(path: &str) -> bool {
    path.strip_prefix(PIPE_PREFIX)
        .and_then(|suffix| {
            suffix
                .strip_prefix('-')
                .or_else(|| suffix.strip_prefix('\\'))
        })
        .is_some_and(|suffix| suffix.len() == 36 && uuid::Uuid::parse_str(suffix).is_ok())
}

#[cfg(any(windows, test))]
async fn query_info<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
) -> Result<Option<ConnectedBrowser>> {
    let request =
        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":"getInfo","params":{}}))?;
    stream.write_u32_le(request.len() as u32).await?;
    stream.write_all(&request).await?;
    stream.flush().await?;
    // Notifications may precede the response; bound both frame size and count.
    for _ in 0..8 {
        let length = stream.read_u32_le().await? as usize;
        ensure!(
            (1..=MAX_FRAME).contains(&length),
            "Invalid browser response size"
        );
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        let response: Value = serde_json::from_slice(&bytes)?;
        ensure!(response["jsonrpc"] == "2.0", "Invalid browser response");
        if response.get("method").is_some() || response["id"] != 1 {
            continue;
        }
        ensure!(
            response.get("error").is_none(),
            "Browser info request failed"
        );
        return browser_info(&response["result"]);
    }
    bail!("Missing browser info response")
}

#[cfg(windows)]
pub async fn check_connection() -> ConnectionStatus {
    use std::time::{Duration, Instant};
    static CHECK: tokio::sync::Mutex<Option<(Instant, ConnectionStatus)>> =
        tokio::sync::Mutex::const_new(None);
    let mut cached = CHECK.lock().await;
    if let Some((checked, result)) = cached.as_ref() {
        if checked.elapsed() < Duration::from_secs(5) {
            return result.clone();
        }
    }
    let result = probe_connections().await;
    *cached = Some((Instant::now(), result.clone()));
    result
}

#[cfg(windows)]
async fn probe_connections() -> ConnectionStatus {
    use futures_util::{StreamExt, stream};
    use std::time::Duration;
    use tokio::net::windows::named_pipe::ClientOptions;

    let listing = tokio::task::spawn_blocking(|| -> Result<Vec<String>> {
        let mut pipes = Vec::new();
        for entry in std::fs::read_dir(r"\\.\pipe\")? {
            let path = entry?.path().to_string_lossy().into_owned();
            if candidate_pipe(&path) {
                pipes.push(path);
            }
            ensure!(pipes.len() <= 32, "Too many browser connections");
        }
        Ok(pipes)
    });
    let Ok(Ok(Ok(pipes))) = tokio::time::timeout(Duration::from_secs(1), listing).await else {
        return ConnectionStatus::failed();
    };
    let checks = stream::iter(pipes)
        .map(|pipe| async move {
            tokio::time::timeout(Duration::from_millis(700), async {
                let mut client = ClientOptions::new().open(pipe)?;
                query_info(&mut client).await
            })
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Browser connection timed out")))
        })
        .buffer_unordered(4);
    tokio::pin!(checks);
    let mut browsers = Vec::new();
    let mut failed = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        match tokio::time::timeout_at(deadline, checks.next()).await {
            Ok(Some(Ok(Some(browser)))) => browsers.push(browser),
            Ok(Some(Ok(None))) => {}
            Ok(Some(Err(_))) => failed += 1,
            Ok(None) => break,
            Err(_) => {
                failed += 1;
                break;
            }
        }
    }
    summarize(browsers, failed)
}

#[cfg(not(windows))]
pub async fn check_connection() -> ConnectionStatus {
    ConnectionStatus {
        state: "unsupported".into(),
        browsers: vec![],
        failed_checks: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(header: Value) -> Value {
        json!({"type":"extension","family":"edge","metadata":{"extensionId":"odlomjlbamekndcpllcnffbgeohgkmjh"},"agentRequestHeaderEnabled":header})
    }

    #[test]
    fn recognizes_browsers_without_requiring_headers_or_a_patch() {
        for enabled in [true, false] {
            let browser = browser_info(&edge(json!(enabled))).unwrap().unwrap();
            assert_eq!(browser.header_enabled, Some(enabled));
            assert_eq!(summarize(vec![browser], 2).state, "available");
        }
        let mut info = edge(json!(true));
        info.as_object_mut()
            .unwrap()
            .remove("agentRequestHeaderEnabled");
        assert_eq!(browser_info(&info).unwrap().unwrap().header_enabled, None);
        info["family"] = json!("chrome");
        info["metadata"]["extensionId"] = json!("hehggadaopoacecdllhhajmbjkdcmajg");
        assert_eq!(browser_info(&info).unwrap().unwrap().family, "chrome");
    }

    #[test]
    fn rejects_unknown_extensions_and_malformed_status() {
        assert!(browser_info(&json!({})).is_err());
        assert!(browser_info(&edge(json!("true"))).is_err());
        let mut info = edge(json!(true));
        info["metadata"]["extensionId"] = json!("other");
        assert!(browser_info(&info).is_err());
        assert_eq!(browser_info(&json!({"type":"iab"})).unwrap(), None);
        assert_eq!(browser_info(&json!({"type":"cdp"})).unwrap(), None);
    }

    #[test]
    fn distinguishes_disconnect_failure_and_partial_success() {
        assert_eq!(summarize(vec![], 0).state, "disconnected");
        assert_eq!(summarize(vec![], 1).state, "check_failed");
        let browser = browser_info(&edge(json!(true))).unwrap().unwrap();
        let status = summarize(vec![browser.clone(), browser], 1);
        assert_eq!(status.state, "available");
        assert_eq!(status.browsers.len(), 1);
        assert_eq!(status.failed_checks, 1);
    }

    #[test]
    fn only_accepts_native_discovery_pipe_names() {
        let id = "b313cb4f-f8e4-4ff6-a5e0-78911c5fa387";
        assert!(candidate_pipe(&format!("{PIPE_PREFIX}-{id}")));
        assert!(candidate_pipe(&format!("{PIPE_PREFIX}\\{id}")));
        for suffix in [
            "",
            "-invalid",
            "-../other",
            "-b313cb4f-f8e4-4ff6-a5e0-78911c5fa387/other",
        ] {
            assert!(!candidate_pipe(&format!("{PIPE_PREFIX}{suffix}")));
        }
        assert!(!candidate_pipe(
            r"\\other\pipe\codex-browser-use-b313cb4f-f8e4-4ff6-a5e0-78911c5fa387"
        ));
    }

    async fn exchange(response: Value) -> Result<Option<ConnectedBrowser>> {
        let (mut client, mut server) = tokio::io::duplex(8192);
        let task = tokio::spawn(async move {
            let len = server.read_u32_le().await.unwrap();
            let mut bytes = vec![0; len as usize];
            server.read_exact(&mut bytes).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"jsonrpc":"2.0","id":1,"method":"getInfo","params":{}})
            );
            let notice = serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"notice"})).unwrap();
            let bytes = serde_json::to_vec(&response).unwrap();
            for frame in [notice, bytes] {
                server.write_u32_le(frame.len() as u32).await.unwrap();
                for chunk in frame.chunks(7) {
                    server.write_all(chunk).await.unwrap();
                }
            }
        });
        let result = query_info(&mut client).await;
        task.await.unwrap();
        result
    }

    #[tokio::test]
    async fn sends_only_read_only_get_info_and_handles_fragmented_frames() {
        let browser = exchange(json!({"jsonrpc":"2.0","id":1,"result":edge(json!(true))}))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(browser.family, "edge");
        assert_eq!(browser.header_enabled, Some(true));
    }

    #[tokio::test]
    async fn rejects_errors_wrong_ids_and_bad_payloads() {
        for response in [
            json!({"jsonrpc":"2.0","id":1,"error":{"message":"failed"}}),
            json!({"jsonrpc":"2.0","id":2,"result":edge(json!(true))}),
            json!({"jsonrpc":"2.0","id":1,"result":{}}),
        ] {
            assert!(exchange(response).await.is_err());
        }
    }

    #[tokio::test]
    async fn rejects_oversized_frames_before_allocating() {
        let (mut client, mut server) = tokio::io::duplex(8192);
        server.write_u32_le((MAX_FRAME + 1) as u32).await.unwrap();
        assert!(query_info(&mut client).await.is_err());
    }

    #[tokio::test]
    async fn silent_backend_can_be_cancelled_with_a_deadline() {
        let (mut client, _server) = tokio::io::duplex(8192);
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(20),
                query_info(&mut client)
            )
            .await
            .is_err()
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    #[ignore = "requires a running native Edge or Chrome extension; read-only getInfo only"]
    async fn live_native_browser_connection() {
        let result = check_connection().await;
        println!("{}", serde_json::to_string(&result).unwrap());
        assert_eq!(result.state, "available");
    }
}
