use codex_plus_core::protocol_proxy::{
    ChatSseToResponsesConverter, chat_completion_to_response, chat_completions_url,
    chat_sse_to_responses_sse, is_models_proxy_path, models_url, responses_to_chat_completions,
};
use serde_json::json;

#[test]
fn responses_request_converts_to_chat_completions() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "instructions": "You are helpful.",
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "hello" }
                ]
            }
        ],
        "max_output_tokens": 512,
        "temperature": 0.2,
        "stream": true,
        "tools": [
            {
                "type": "function",
                "name": "lookup",
                "description": "Lookup data",
                "parameters": { "type": "object" }
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        converted,
        json!({
            "model": "gpt-5-mini",
            "messages": [
                { "role": "system", "content": "You are helpful." },
                { "role": "user", "content": "hello" }
            ],
            "max_tokens": 512,
            "temperature": 0.2,
            "stream": true,
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "description": "Lookup data",
                        "parameters": { "type": "object" }
                    }
                }
            ]
        })
    );
}

#[test]
fn responses_request_matches_ccs_reasoning_and_tool_choice_edges() {
    let non_reasoning = responses_to_chat_completions(json!({
        "model": "gpt-4o",
        "reasoning": { "effort": "high" },
        "tool_choice": { "type": "required" },
        "input": "hi"
    }))
    .unwrap();
    assert!(non_reasoning.get("reasoning_effort").is_none());
    assert_eq!(non_reasoning["tool_choice"], json!({ "type": "required" }));

    let reasoning = responses_to_chat_completions(json!({
        "model": "gpt-5.4",
        "reasoning": { "effort": "high" },
        "tool_choice": { "type": "function", "name": "lookup" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(reasoning["reasoning_effort"], "high");
    assert_eq!(reasoning["tool_choice"]["type"], "function");
    assert_eq!(reasoning["tool_choice"]["function"]["name"], "lookup");
}

#[test]
fn responses_request_maps_developer_role_to_system_for_chat_upstream() {
    let converted = responses_to_chat_completions(json!({
        "model": "deepseek-chat",
        "input": [
            {
                "type": "message",
                "role": "developer",
                "content": [
                    { "type": "input_text", "text": "developer instructions" }
                ]
            },
            {
                "type": "message",
                "role": "user",
                "content": [
                    { "type": "input_text", "text": "hello" }
                ]
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "system");
    assert_eq!(
        converted["messages"][0]["content"],
        "developer instructions"
    );
    assert_eq!(converted["messages"][1]["role"], "user");
    assert!(
        !serde_json::to_string(&converted)
            .unwrap()
            .contains("\"developer\"")
    );
}

#[test]
fn responses_request_preserves_reasoning_content_for_thinking_followup() {
    let converted = responses_to_chat_completions(json!({
        "model": "deepseek-reasoner",
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "use the tool" }]
            },
            {
                "id": "rs_1",
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "Need to inspect files." }]
            },
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "shell",
                "arguments": "{\"cmd\":\"rg foo\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "result"
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][1]["role"], "assistant");
    assert_eq!(
        converted["messages"][1]["reasoning_content"],
        "Need to inspect files."
    );
    assert_eq!(converted["messages"][1]["tool_calls"][0]["id"], "call_1");
    assert_eq!(converted["messages"][2]["role"], "tool");
}

#[test]
fn responses_request_merges_reasoning_text_and_tool_calls_like_ccx() {
    let converted = responses_to_chat_completions(json!({
        "model": "deepseek-v4-pro",
        "input": [
            {
                "type": "reasoning",
                "status": "completed",
                "summary": [{ "type": "summary_text", "text": "I need to run go vet." }]
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "Let me run go vet." }]
            },
            {
                "type": "function_call",
                "call_id": "call_001",
                "name": "exec_command",
                "arguments": "{\"cmd\":\"go vet ./...\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_001",
                "output": "no issues found"
            },
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "run tests now" }]
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "assistant");
    assert_eq!(converted["messages"][0]["content"], "Let me run go vet.");
    assert_eq!(
        converted["messages"][0]["reasoning_content"],
        "I need to run go vet."
    );
    assert_eq!(converted["messages"][0]["tool_calls"][0]["id"], "call_001");
    assert_eq!(converted["messages"][1]["role"], "tool");
    assert_eq!(converted["messages"][1]["tool_call_id"], "call_001");
    assert_eq!(converted["messages"][2]["role"], "user");
}

#[test]
fn chat_completion_response_converts_to_responses_response() {
    let converted = chat_completion_to_response(json!({
        "id": "chatcmpl_123",
        "created": 1710000000,
        "model": "gpt-5-mini",
        "choices": [
            {
                "finish_reason": "stop",
                "message": {
                    "role": "assistant",
                    "content": "hi there"
                }
            }
        ],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    }))
    .unwrap();

    assert_eq!(converted["object"], "response");
    assert_eq!(converted["status"], "completed");
    assert_eq!(converted["model"], "gpt-5-mini");
    assert_eq!(converted["usage"]["input_tokens"], 10);
    assert_eq!(converted["usage"]["output_tokens"], 5);
    assert_eq!(converted["output"][0]["type"], "message");
    assert_eq!(converted["output"][0]["content"][0]["text"], "hi there");
}

#[test]
fn chat_completion_response_maps_reasoning_tool_calls_and_usage_details() {
    let converted = chat_completion_to_response(json!({
        "id": "chatcmpl_1",
        "created": 123,
        "model": "gpt-5.4",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "reasoning_content": "I should check first.",
                "content": "Let me check.",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "get_weather",
                        "arguments": "{\"city\":\"Tokyo\"}"
                    }
                }]
            }
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": { "cached_tokens": 3 },
            "completion_tokens_details": { "reasoning_tokens": 2 }
        }
    }))
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "reasoning");
    assert_eq!(
        converted["output"][0]["summary"][0]["text"],
        "I should check first."
    );
    assert_eq!(
        converted["output"][0]["reasoning_content"],
        "I should check first."
    );
    assert_eq!(converted["output"][1]["type"], "message");
    assert_eq!(converted["output"][2]["type"], "function_call");
    assert_eq!(converted["output"][2]["call_id"], "call_1");
    assert_eq!(
        converted["usage"]["input_tokens_details"]["cached_tokens"],
        3
    );
    assert_eq!(
        converted["usage"]["output_tokens_details"]["reasoning_tokens"],
        2
    );
}

#[test]
fn chat_completion_response_accepts_responses_style_usage_fields() {
    let converted = chat_completion_to_response(json!({
        "id": "chatcmpl_usage",
        "created": 123,
        "model": "gpt-5.4",
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": "ok"
            }
        }],
        "usage": {
            "input_tokens": 7,
            "output_tokens": 3,
            "input_tokens_details": { "cached_tokens": 2 },
            "cache_read_input_tokens": 1,
            "cache_creation_input_tokens": 4
        }
    }))
    .unwrap();

    assert_eq!(converted["usage"]["input_tokens"], 7);
    assert_eq!(converted["usage"]["output_tokens"], 3);
    assert_eq!(converted["usage"]["total_tokens"], 10);
    assert_eq!(
        converted["usage"]["input_tokens_details"]["cached_tokens"],
        2
    );
    assert_eq!(converted["usage"]["cache_read_input_tokens"], 1);
    assert_eq!(converted["usage"]["cache_creation_input_tokens"], 4);
}

#[test]
fn chat_completion_response_splits_inline_think_block() {
    let converted = chat_completion_to_response(json!({
        "id": "chatcmpl_think",
        "created": 123,
        "model": "MiniMax-M2.7",
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "content": "<think>\nNeed context.\n</think>\n\npong"
            }
        }]
    }))
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "reasoning");
    assert_eq!(
        converted["output"][0]["summary"][0]["text"],
        "Need context."
    );
    assert_eq!(converted["output"][1]["type"], "message");
    assert_eq!(converted["output"][1]["content"][0]["text"], "pong");
}

#[test]
fn chat_sse_converts_to_responses_sse_events() {
    let converted = chat_sse_to_responses_sse(
        r#"data: {"id":"chatcmpl_1","created":1710000000,"model":"gpt-5-mini","choices":[{"delta":{"content":"hel"},"finish_reason":null}]}

data: {"id":"chatcmpl_1","created":1710000000,"model":"gpt-5-mini","choices":[{"delta":{"content":"lo"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}

data: [DONE]

"#,
    );

    assert!(converted.contains("event: response.created"));
    assert!(converted.contains("event: response.output_text.delta"));
    assert!(converted.contains("\"delta\":\"hel\""));
    assert!(converted.contains("\"text\":\"hello\""));
    assert!(converted.contains("\"input_tokens\":3"));
    assert!(converted.contains("event: response.completed"));
    assert!(converted.contains("data: [DONE]"));
}

#[test]
fn chat_sse_converts_reasoning_inline_think_tools_and_errors_like_ccs() {
    let reasoning = chat_sse_to_responses_sse(
        r#"data: {"id":"chatcmpl_reason","created":123,"model":"deepseek-reasoner","choices":[{"delta":{"reasoning_content":"Need context. "}}]}

data: {"id":"chatcmpl_reason","created":123,"model":"deepseek-reasoner","choices":[{"delta":{"content":"Done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"completion_tokens_details":{"reasoning_tokens":3}}}

data: [DONE]

"#,
    );
    assert!(reasoning.contains("event: response.in_progress"));
    assert!(reasoning.contains("event: response.reasoning_summary_part.added"));
    assert!(reasoning.contains("event: response.reasoning_summary_text.delta"));
    assert!(reasoning.contains("event: response.reasoning_summary_text.done"));
    assert!(reasoning.contains("\"reasoning_content\":\"Need context. \""));
    assert!(reasoning.contains("\"type\":\"reasoning\""));
    assert!(reasoning.contains("\"text\":\"Done\""));
    assert!(reasoning.contains("\"reasoning_tokens\":3"));

    let inline_think = chat_sse_to_responses_sse(
        r#"data: {"id":"chatcmpl_minimax","created":123,"model":"MiniMax-M2.7","choices":[{"delta":{"content":"<think>\nNeed"}}]}

data: {"id":"chatcmpl_minimax","created":123,"model":"MiniMax-M2.7","choices":[{"delta":{"content":" context.</think>\n\npong"},"finish_reason":"stop"}]}

"#,
    );
    assert!(inline_think.contains("Need context."));
    assert!(inline_think.contains("\"text\":\"pong\""));
    assert!(!inline_think.contains("<think>"));
    assert!(!inline_think.contains("</think>"));

    let tool = chat_sse_to_responses_sse(
        r#"data: {"id":"chatcmpl_tool","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"get_weather"}}]}}]}

data: {"id":"chatcmpl_tool","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\":\"Tokyo\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#,
    );
    assert!(tool.contains("event: response.function_call_arguments.delta"));
    assert!(tool.contains("event: response.function_call_arguments.done"));
    assert!(tool.contains("\"type\":\"function_call\""));
    assert!(tool.contains("\"call_id\":\"call_1\""));

    let error = chat_sse_to_responses_sse(
        r#"event: error
data: {"error":{"message":"bad request","type":"invalid_request_error"}}

data: [DONE]

"#,
    );
    assert!(error.contains("event: response.failed"));
    assert!(error.contains("bad request"));
    assert!(error.contains("invalid_request_error"));
    assert!(!error.contains("event: response.completed"));
}

#[test]
fn chat_sse_converter_handles_partial_chunks_and_utf8_boundaries() {
    let sse = "data: {\"id\":\"chatcmpl_utf8\",\"created\":123,\"model\":\"gpt-5.4\",\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":\"stop\"}]}\r\n\r\n";
    let bytes = sse.as_bytes();
    let split = bytes
        .windows("好".len())
        .position(|window| window == "好".as_bytes())
        .unwrap()
        + 1;

    let mut converter = ChatSseToResponsesConverter::default();
    let mut output = converter.push_bytes(&bytes[..split]);
    output.extend(converter.push_bytes(&bytes[split..]));
    output.extend(converter.finish());
    let output = String::from_utf8(output).unwrap();

    assert!(output.contains("\"delta\":\"你好\""));
    assert!(output.contains("event: response.completed"));
}

#[test]
fn chat_completions_url_normalizes_common_base_urls() {
    assert_eq!(
        chat_completions_url("https://api.example.test"),
        "https://api.example.test/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/v1"),
        "https://api.example.test/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/openai"),
        "https://api.example.test/openai/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/v1/chat/completions"),
        "https://api.example.test/v1/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/v2"),
        "https://api.example.test/v2/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/v1beta"),
        "https://api.example.test/v1beta/chat/completions"
    );
    assert_eq!(
        chat_completions_url("https://api.example.test/openai#"),
        "https://api.example.test/openai/chat/completions"
    );
}

#[test]
fn models_url_normalizes_common_base_urls() {
    assert_eq!(
        models_url("https://api.example.test"),
        "https://api.example.test/v1/models"
    );
    assert_eq!(
        models_url("https://api.example.test/v1"),
        "https://api.example.test/v1/models"
    );
    assert_eq!(
        models_url("https://api.example.test/v1/chat/completions"),
        "https://api.example.test/v1/models"
    );
    assert_eq!(
        models_url("https://api.example.test/models"),
        "https://api.example.test/models"
    );
    assert_eq!(
        models_url("https://api.example.test/v2"),
        "https://api.example.test/v2/models"
    );
    assert_eq!(
        models_url("https://api.example.test/v1beta"),
        "https://api.example.test/v1beta/models"
    );
    assert_eq!(
        models_url("https://api.example.test/openai#"),
        "https://api.example.test/openai/models"
    );
}

#[test]
fn models_proxy_path_matches_v1_models() {
    assert!(is_models_proxy_path("/models"));
    assert!(is_models_proxy_path("/v1/models"));
    assert!(is_models_proxy_path("/v1/models?limit=10"));
    assert!(!is_models_proxy_path("/v1/responses"));
}
