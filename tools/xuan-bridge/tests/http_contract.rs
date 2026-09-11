use std::fs;
use std::net::TcpListener;
use std::thread;

use reqwest::blocking::Client;
use serde_json::{Value, json};
use tempfile::tempdir;
use xuan_bridge::{BRIDGE_PROTOCOL_VERSION, serve_http_listener};

#[test]
fn loopback_http_routes_and_cors_follow_the_bridge_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || serve_http_listener(listener, Some(5)).unwrap());
    let client = Client::builder().build().unwrap();

    let health: Value = client
        .get(format!("http://{address}/v1/health"))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(health["protocolVersion"], BRIDGE_PROTOCOL_VERSION);

    let preflight = client
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{address}/v1/polish"),
        )
        .header("Origin", "https://chatgpt.com")
        .send()
        .unwrap();
    assert_eq!(preflight.status(), 204);
    assert_eq!(
        preflight.headers()["access-control-allow-origin"],
        "https://chatgpt.com"
    );

    let desktop_preflight = client
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{address}/v1/usage"),
        )
        .header("Origin", "app://-")
        .send()
        .unwrap();
    assert_eq!(desktop_preflight.status(), 204);
    assert_eq!(
        desktop_preflight.headers()["access-control-allow-origin"],
        "app://-"
    );

    let workspace = tempdir().unwrap();
    fs::write(
        workspace.path().join("sample.txt"),
        "alpha\nneedle\nomega\n",
    )
    .unwrap();
    let search: Value = client
        .post(format!("http://{address}/v1/search/start"))
        .json(&json!({
            "root": workspace.path(),
            "query": "needle",
            "maxResults": 10
        }))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(search["state"], "complete");
    assert_eq!(search["result"]["results"][0]["line"], 2);

    let wrong_method = client
        .post(format!("http://{address}/v1/health"))
        .send()
        .unwrap();
    assert_eq!(wrong_method.status(), 405);
    server.join().unwrap();
}

#[test]
fn public_http_bind_is_rejected() {
    let error = xuan_bridge::serve_http("0.0.0.0:0").unwrap_err();
    assert!(error.contains("loopback"));
}
