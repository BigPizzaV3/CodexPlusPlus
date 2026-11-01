//! 拓展调模型：请求体拼装，以及 Responses / Chat Completions、流式 / 整段回复的解析。

use codex_plus_core::extension_llm::{SseDeltas, parse_reply, request_body};
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

#[test]
fn tools_are_sent_and_tool_calls_parsed() {
    let body = request_body(&json!({ "model": "m", "input": "查天气", "tools": [{ "name": "weather", "description": "查天气", "parameters": { "type": "object" } }], "toolChoice": "auto" }), None).unwrap();
    assert_eq!(body["tools"], json!([{ "type": "function", "name": "weather", "description": "查天气", "parameters": { "type": "object" } }]));
    assert_eq!(body["tool_choice"], "auto");
    assert!(request_body(&json!({ "model": "m", "input": "x", "tools": [{ "description": "没名字" }] }), None).is_err());
    // Responses 流：output_item.done 里的 function_call，加上推理摘要。
    let sse = concat!(
        "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"想一想\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"weather\",\"arguments\":\"{\\\"city\\\":\\\"bj\\\"}\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"weather\",\"arguments\":\"{}\"}]}}\n\n",
    );
    let reply = parse_reply(sse.as_bytes());
    assert_eq!(reply["toolCalls"], json!([{ "callId": "c1", "name": "weather", "arguments": "{\"city\":\"bj\"}" }]), "同一个调用只记一次");
    assert_eq!(reply["reasoning"], "想一想");
    // Chat 流：tool_calls 按 index 拼参数。
    let chat = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"t1\",\"function\":{\"name\":\"weather\",\"arguments\":\"{\\\"ci\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"ty\\\":1}\"}}]}}]}\n\n",
        "data: [DONE]\n\n",
    );
    assert_eq!(parse_reply(chat.as_bytes())["toolCalls"], json!([{ "callId": "t1", "name": "weather", "arguments": "{\"city\":1}" }]));
    assert!(parse_reply(b"{\"object\":\"response\",\"output\":[]}").get("toolCalls").is_none());
}

/// 流式增量：字节在任意处断开（包括多字节字符中间）也能拼回完整事件；Responses 与 Chat 的文字 / 推理增量都交出来。
#[test]
fn stream_deltas_are_emitted_as_lines_complete() {
    let sse = "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"想\"}\n\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\ndata: {\"choices\":[{\"delta\":{\"reasoning_content\":\"r\",\"content\":\"c\"}}]}\n\ndata: [DONE]\n\n";
    let bytes = sse.as_bytes();
    let mut parser = SseDeltas::default();
    let mut deltas = Vec::new();
    // 一次喂 5 个字节：中文字符必然被切开。
    for chunk in bytes.chunks(5) {
        deltas.extend(parser.push(chunk));
    }
    assert_eq!(deltas, vec![
        json!({ "type": "reasoning", "text": "想" }),
        json!({ "type": "text", "text": "你好" }),
        json!({ "type": "reasoning", "text": "r" }),
        json!({ "type": "text", "text": "c" }),
    ]);
    // 半行不交：等换行到了才算一个事件。
    let mut parser = SseDeltas::default();
    assert!(parser.push(b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}").is_empty());
    assert_eq!(parser.push(b"\n"), vec![json!({ "type": "text", "text": "x" })]);
}
