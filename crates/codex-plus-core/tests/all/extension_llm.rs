//! 拓展调模型：请求体拼装，以及 Responses / Chat Completions、流式 / 整段回复的解析。

use codex_plus_core::extension_llm::{parse_reply, request_body};
use serde_json::json;

#[test]
fn request_body_accepts_text_or_messages() {
    let body = request_body(&json!({ "input": "你好", "instructions": "简短", "reasoningEffort": "low" }), Some("gpt-x".into())).unwrap();
    assert_eq!(body["model"], "gpt-x", "没给 model 用 Codex 的默认模型");
    assert_eq!(body["input"][0]["content"][0], json!({ "type": "input_text", "text": "你好" }));
    assert_eq!(body["reasoning"]["effort"], "low");
    assert_eq!(body["stream"], true);
    let body = request_body(&json!({ "model": "m", "input": [{ "role": "system", "content": "规则" }, { "role": "assistant", "text": "好" }] }), None).unwrap();
    assert_eq!(body["input"][0]["role"], "developer");
    assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    assert!(request_body(&json!({ "input": "x" }), None).is_err(), "没有模型可用");
    assert!(request_body(&json!({ "model": "m", "input": [{ "role": "tool", "content": "x" }] }), None).is_err());
}

#[test]
fn replies_are_parsed_across_protocols() {
    let sse = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"好\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"model\":\"gpt-x\",\"usage\":{\"total_tokens\":3},\"output\":[{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"你好\"}]}]}}\n\n";
    let reply = parse_reply(sse.as_bytes());
    assert_eq!(reply["text"], "你好");
    assert_eq!(reply["model"], "gpt-x");
    assert_eq!(reply["usage"]["total_tokens"], 3);
    let chat = "data: {\"choices\":[{\"delta\":{\"content\":\"O\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"K\"}}],\"usage\":{\"total_tokens\":2}}\n\ndata: [DONE]\n\n";
    assert_eq!(parse_reply(chat.as_bytes())["text"], "OK");
    let whole = json!({ "choices": [{ "message": { "content": "整段" } }], "model": "c" }).to_string();
    assert_eq!(parse_reply(whole.as_bytes())["text"], "整段");
    let failed = json!({ "error": { "message": "额度不足" } }).to_string();
    assert_eq!(parse_reply(failed.as_bytes())["error"]["message"], "额度不足");
}
