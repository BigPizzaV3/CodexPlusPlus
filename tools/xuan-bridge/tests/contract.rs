use std::io::Cursor;

use serde_json::json;
use xuan_bridge::{BRIDGE_PROTOCOL_VERSION, serve_json_lines};

#[test]
fn json_line_contract_returns_health_response() {
    let input = b"{\"id\":\"health-1\",\"method\":\"bridge.health\",\"params\":{}}\n";
    let mut output = Vec::new();
    serve_json_lines(Cursor::new(input), &mut output).unwrap();
    let response: serde_json::Value = serde_json::from_slice(output.trim_ascii()).unwrap();
    assert_eq!(response["id"], "health-1");
    assert_eq!(
        response["result"]["protocolVersion"],
        BRIDGE_PROTOCOL_VERSION
    );
}

#[test]
fn unknown_method_is_a_stable_error() {
    let input = b"{\"id\":7,\"method\":\"unknown\",\"params\":{}}\n";
    let mut output = Vec::new();
    serve_json_lines(Cursor::new(input), &mut output).unwrap();
    let response: serde_json::Value = serde_json::from_slice(output.trim_ascii()).unwrap();
    assert_eq!(response["error"]["code"], "method_not_found");
    assert_eq!(response["id"], json!(7));
}
