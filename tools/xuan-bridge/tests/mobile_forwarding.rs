use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::thread;

use serde_json::Value;
use tempfile::tempdir;

#[test]
fn json_line_mobile_status_forwards_to_loopback_with_bearer_token() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = vec![0_u8; 4096];
        let read = stream.read(&mut request).unwrap();
        let request = String::from_utf8_lossy(&request[..read]);
        assert!(request.starts_with("GET /v1/mobile/status HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer test-token")
        );
        let body = r#"{"enabled":true,"connected":false,"bound":true}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
    });

    let home = tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xuan-bridge"))
        .env("XUAN_HOME", home.path())
        .env("XUAN_MOBILE_BRIDGE_URL", format!("http://{address}"))
        .env("XUAN_MOBILE_BRIDGE_TOKEN", "test-token")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"{\"id\":1,\"method\":\"mobile.status\",\"params\":{}}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(output.stdout.trim_ascii()).unwrap();
    assert_eq!(response["result"]["enabled"], true);
    assert_eq!(response["result"]["bound"], true);
    server.join().unwrap();
}

#[test]
fn non_loopback_mobile_bridge_is_rejected_before_transport() {
    let previous = std::env::var_os("XUAN_MOBILE_BRIDGE_URL");
    unsafe { std::env::set_var("XUAN_MOBILE_BRIDGE_URL", "http://192.0.2.1:17421") };
    let mut state = xuan_bridge::BridgeState::new();
    let response = xuan_bridge::handle_request(
        &mut state,
        xuan_bridge::RpcRequest {
            id: serde_json::json!(1),
            method: "mobile.status".into(),
            params: serde_json::json!({}),
        },
    );
    match previous {
        Some(value) => unsafe { std::env::set_var("XUAN_MOBILE_BRIDGE_URL", value) },
        None => unsafe { std::env::remove_var("XUAN_MOBILE_BRIDGE_URL") },
    }
    assert_eq!(response.error.unwrap().code, "configuration_error");
}
