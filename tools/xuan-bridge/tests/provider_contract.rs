use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use serde_json::{Value, json};
use tempfile::TempDir;

fn serve_json_once(body: Value) -> (SocketAddr, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut header_end = None;
        let mut chunk = [0_u8; 1024];
        while request.len() < 64 * 1024 {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                break;
            }
        }
        let header_end = header_end.unwrap();
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        sender
            .send(String::from_utf8_lossy(&request).to_string())
            .unwrap();
        let body = serde_json::to_vec(&body).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    });
    (address, receiver)
}

fn call_bridge(home: &TempDir, method: &str, params: Value, env: &[(&str, &str)]) -> Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xuan-bridge"));
    command
        .env("XUAN_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    writeln!(
        child.stdin.as_mut().unwrap(),
        "{}",
        json!({ "id": "contract", "method": method, "params": params })
    )
    .unwrap();
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "bridge failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(output.stdout.trim_ascii()).unwrap()
}

fn write_config(home: &TempDir, config: Value) {
    fs::write(
        home.path().join("xuan-plugins.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();
}

#[test]
fn usage_profile_calls_generic_provider_without_storing_the_secret() {
    let home = tempfile::tempdir().unwrap();
    let (address, request) = serve_json_once(json!({ "total": 12.5, "unit": "USD" }));
    write_config(
        &home,
        json!({
            "schemaVersion": 1,
            "plugins": {
                "xuan-usage": {
                    "defaultProfile": "primary",
                    "profiles": {
                        "primary": {
                            "name": "Primary Relay",
                            "provider": "generic",
                            "baseUrl": format!("http://{address}/v1"),
                            "apiKeyEnv": "XUAN_TEST_USAGE_KEY"
                        }
                    }
                }
            }
        }),
    );
    let response = call_bridge(
        &home,
        "usage.query",
        json!({ "startDate": "2026-09-01", "endDate": "2026-09-11" }),
        &[("XUAN_TEST_USAGE_KEY", "usage-secret")],
    );
    assert_eq!(response["result"]["profileRef"], "primary");
    assert_eq!(response["result"]["data"]["total"], 12.5);
    assert!(!response.to_string().contains("usage-secret"));
    let request = request.recv().unwrap();
    assert!(request.starts_with("GET /v1/usage?"));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer usage-secret")
    );
}

#[test]
fn polish_profile_calls_responses_and_preserves_context_boundaries() {
    let home = tempfile::tempdir().unwrap();
    let (address, request) = serve_json_once(json!({
        "output": [{
            "type": "message",
            "content": [{ "type": "output_text", "text": "```text\npolished result\n```" }]
        }]
    }));
    write_config(
        &home,
        json!({
            "schemaVersion": 1,
            "plugins": {
                "xuan-polish": {
                    "defaultProfile": "primary",
                    "profiles": {
                        "primary": {
                            "protocol": "responses",
                            "baseUrl": format!("http://{address}/v1"),
                            "apiKeyEnv": "XUAN_TEST_POLISH_KEY",
                            "model": "polish-test"
                        }
                    }
                }
            }
        }),
    );
    let response = call_bridge(
        &home,
        "polish.generate",
        json!({
            "text": "continue",
            "recentTurns": [{ "userText": "modify the project", "assistantText": "confirmed" }],
            "projectMap": "src/main.rs"
        }),
        &[("XUAN_TEST_POLISH_KEY", "polish-secret")],
    );
    assert_eq!(response["result"]["protocol"], "responses");
    assert_eq!(response["result"]["model"], "polish-test");
    assert_eq!(response["result"]["text"], "polished result");
    assert!(!response.to_string().contains("polish-secret"));
    let request = request.recv().unwrap();
    assert!(request.starts_with("POST /v1/responses "));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer polish-secret")
    );
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let body: Value = serde_json::from_str(body).unwrap();
    assert_eq!(body["model"], "polish-test");
    assert!(
        body["input"]
            .as_str()
            .unwrap()
            .contains("<draft>\ncontinue\n</draft>")
    );
}
