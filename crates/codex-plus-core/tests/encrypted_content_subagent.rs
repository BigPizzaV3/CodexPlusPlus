use codex_plus_core::protocol_proxy::responses_to_chat_completions;
use serde_json::json;

#[test]
fn responses_request_preserves_encrypted_content_for_subagent_tasks() {
    let converted = responses_to_chat_completions(json!({
        "model": "deepseek-v4-flash",
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Message Type: NEW_TASK\nTask name: /root/probe_none\nSender: /root\nPayload:\n"
                    },
                    {
                        "type": "encrypted_content",
                        "encrypted_content": "请回显标记：PROBE-NONE-7F3A"
                    }
                ]
            }
        ]
    }))
    .unwrap();

    let content = converted["messages"][0]["content"].as_str().unwrap();
    assert!(content.contains("Payload:"));
    assert!(content.contains("PROBE-NONE-7F3A"));
}
