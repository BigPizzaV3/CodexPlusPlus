use codex_plus_core::protocol_proxy::{
    ChatSseToResponsesConverter, analyze_images_with_vl, apply_vl_with_fallback,
    chat_completion_to_response, chat_completion_to_response_with_request, chat_completions_url,
    chat_sse_to_responses_sse, chat_sse_to_responses_sse_with_request,
    is_chat_completions_proxy_path, is_models_proxy_path, is_responses_proxy_path,
    model_supports_image, model_supports_reasoning, models_url,
    open_chat_completions_proxy_request, open_models_proxy_request, open_responses_proxy_request,
    open_responses_proxy_request_with_settings, responses_error_from_upstream,
    responses_to_chat_completions, responses_to_chat_completions_with_image_support,
    send_upstream_request_with_header_timeout,
    strip_reasoning_in_place, upstream_header_timeout, upstream_http_client,
    upstream_request_parts_with_image_decision, upstream_stream_header_timeout,
};
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings,
    RelayMode, RelayProfile, VisionRelayConfig,
};
use serde_json::json;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 确保测试运行期间 `NO_PROXY` 含 127.0.0.1/localhost。
///
/// Bug 1 修复撤回了 `proxied_client()` 的 `.no_proxy()`，生产 client 恢复尊重系统代理。
/// 测试打 127.0.0.1 mock，需用 `NO_PROXY` env 绕过系统代理（只绕 localhost，公网照常）。
/// reqwest 在 client 构建时读 env，故须在构建 client 前设置。`Once` 保证全进程只设一次。
static NO_PROXY_INIT: std::sync::Once = std::sync::Once::new();
fn ensure_no_proxy_for_localhost() {
    NO_PROXY_INIT.call_once(|| {
        if std::env::var("NO_PROXY").is_err() {
            // SAFETY: 测试单线程初始化阶段设置；NO_PROXY 只增 localhost 绕过，不影响公网
            unsafe { std::env::set_var("NO_PROXY", "127.0.0.1,localhost") };
        }
    });
}

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
            "stream_options": { "include_usage": true },
            "tools": [
                {
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "description": "Lookup data",
                        "parameters": { "type": "object", "properties": {}, "required": [] }
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
    assert!(non_reasoning.get("tool_choice").is_none());

    let reasoning = responses_to_chat_completions(json!({
        "model": "gpt-5.4",
        "reasoning": { "effort": "high" },
        "tool_choice": { "type": "function", "name": "lookup" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(reasoning["reasoning_effort"], "high");
    assert!(reasoning.get("tool_choice").is_none());

    let minimal = responses_to_chat_completions(json!({
        "model": "gpt-5.4",
        "reasoning": { "effort": "minimal" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(minimal["reasoning_effort"], "minimal");
}

#[test]
fn proxy_route_matchers_accept_ccswitch_codex_aliases() {
    for path in [
        "/responses",
        "/v1/responses",
        "/v1/v1/responses",
        "/codex/v1/responses",
        "/responses/compact",
        "/v1/responses/compact",
        "/v1/v1/responses/compact",
        "/codex/v1/responses/compact",
    ] {
        assert!(is_responses_proxy_path(path), "{path}");
    }

    for path in [
        "/chat/completions",
        "/v1/chat/completions",
        "/v1/v1/chat/completions",
        "/codex/v1/chat/completions",
    ] {
        assert!(is_chat_completions_proxy_path(path), "{path}");
    }

    for path in ["/models", "/v1/models", "/v1/v1/models", "/codex/v1/models"] {
        assert!(is_models_proxy_path(path), "{path}");
    }
}

#[test]
fn responses_request_applies_ccswitch_reasoning_dialects() {
    let deepseek = responses_to_chat_completions(json!({
        "model": "deepseek-reasoner",
        "reasoning": { "effort": "xhigh" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(deepseek["reasoning_effort"], "max");

    let openrouter = responses_to_chat_completions(json!({
        "model": "openrouter/deepseek/deepseek-r1",
        "reasoning": { "effort": "max" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(openrouter["reasoning"]["effort"], "xhigh");
    assert!(openrouter.get("reasoning_effort").is_none());

    let openrouter_off = responses_to_chat_completions(json!({
        "model": "openrouter/deepseek/deepseek-r1",
        "reasoning": { "effort": "none" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(openrouter_off["reasoning"]["effort"], "none");

    let kimi = responses_to_chat_completions(json!({
        "model": "kimi-k2-thinking",
        "reasoning": { "effort": "high" },
        "input": "hi"
    }))
    .unwrap();
    assert_eq!(kimi["thinking"]["type"], "enabled");
    assert!(kimi.get("reasoning_effort").is_none());
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
fn responses_request_collapses_system_messages_to_head_for_strict_chat_upstreams() {
    let converted = responses_to_chat_completions(json!({
        "model": "MiniMax-M2.7",
        "instructions": "root system",
        "input": [
            {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello" }]
            },
            {
                "type": "message",
                "role": "developer",
                "content": [{ "type": "input_text", "text": "late developer" }]
            },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "ok" }]
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "system");
    assert_eq!(
        converted["messages"][0]["content"],
        "root system\n\nlate developer"
    );
    let system_count = converted["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "system")
        .count();
    assert_eq!(system_count, 1);
    assert_eq!(converted["messages"][1]["role"], "user");
    assert_eq!(converted["messages"][2]["role"], "assistant");
}

#[test]
fn responses_request_maps_latest_reminder_to_user_like_ccswitch() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "message",
                "role": "latest_reminder",
                "content": [
                    { "type": "input_text", "text": "remember this" }
                ]
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "user");
    assert_eq!(converted["messages"][0]["content"], "remember this");
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
fn responses_request_normalizes_empty_assistant_messages_for_chat_upstream() {
    let converted = responses_to_chat_completions(json!({
        "model": "deepseek-chat",
        "input": [
            {
                "type": "message",
                "role": "assistant",
                "content": null
            },
            {
                "type": "message",
                "role": "assistant",
                "content": []
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "assistant");
    assert_eq!(converted["messages"][0]["content"], "");
    assert_eq!(converted["messages"][1]["role"], "assistant");
    assert_eq!(converted["messages"][1]["content"], "");
}

#[test]
fn responses_request_drops_tool_controls_when_no_chat_tools_survive() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": "hi",
        "tools": [
            { "type": "unknown_builtin", "name": "unsupported" }
        ],
        "tool_choice": { "type": "required" },
        "parallel_tool_calls": true
    }))
    .unwrap();

    assert!(converted.get("tools").is_none());
    assert!(converted.get("tool_choice").is_none());
    assert!(converted.get("parallel_tool_calls").is_none());
}

#[test]
fn responses_request_normalizes_function_tool_parameters() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": "hi",
        "tools": [
            {
                "type": "function",
                "name": "lookup",
                "parameters": {}
            }
        ]
    }))
    .unwrap();

    let params = &converted["tools"][0]["function"]["parameters"];
    assert_eq!(params["type"], "object");
    assert_eq!(params["properties"], json!({}));
    assert_eq!(params["required"], json!([]));
}

#[test]
fn responses_request_maps_codex_custom_and_namespace_tools_to_chat_functions() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": "hi",
        "tools": [
            {
                "type": "custom",
                "name": "exec",
                "description": "Run a command"
            },
            {
                "type": "namespace",
                "name": "mcp__vscode_mcp__",
                "description": "VS Code MCP",
                "tools": [
                    {
                        "type": "function",
                        "name": "open_file",
                        "description": "Open a file",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "path": { "type": "string" }
                            },
                            "required": ["path"]
                        }
                    }
                ]
            },
            {
                "type": "web_search"
            }
        ],
        "tool_choice": {
            "type": "function",
            "namespace": "mcp__vscode_mcp__",
            "name": "open_file"
        },
        "parallel_tool_calls": true
    }))
    .unwrap();

    let names: Vec<_> = converted["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"exec"));
    assert!(names.contains(&"mcp__vscode_mcp__open_file"));
    assert!(names.contains(&"web_search"));
    assert_eq!(
        converted["tools"][0]["function"]["parameters"]["properties"]["input"]["type"],
        "string"
    );
    assert_eq!(converted["parallel_tool_calls"], true);
    assert_eq!(
        converted["tool_choice"]["function"]["name"],
        "mcp__vscode_mcp__open_file"
    );
}

#[test]
fn responses_request_stream_includes_usage_and_apply_patch_proxy_tools() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": "hi",
        "stream": true,
        "tools": [
            {
                "type": "custom",
                "name": "apply_patch",
                "description": "Patch files"
            }
        ],
        "tool_choice": { "type": "custom", "name": "apply_patch" }
    }))
    .unwrap();

    assert_eq!(converted["stream_options"]["include_usage"], true);
    let names: Vec<_> = converted["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "apply_patch_add_file",
            "apply_patch_delete_file",
            "apply_patch_update_file",
            "apply_patch_replace_file",
            "apply_patch_batch"
        ]
    );
    assert_eq!(
        converted["tools"][2]["function"]["parameters"]["properties"]["hunks"]["items"]["properties"]
            ["lines"]["items"]["required"],
        json!(["op", "text"])
    );
    assert_eq!(
        converted["tool_choice"]["function"]["name"],
        "apply_patch_batch"
    );
}

#[test]
fn responses_input_replays_custom_and_legacy_tool_history() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "custom_tool_call",
                "call_id": "call_custom",
                "name": "exec",
                "input": "ls -la"
            },
            {
                "type": "custom_tool_call_output",
                "call_id": "call_custom",
                "output": "ok"
            },
            {
                "type": "tool_call",
                "tool_use": {
                    "id": "call_legacy",
                    "name": "lookup",
                    "input": { "query": "rust" }
                }
            },
            {
                "type": "tool_result",
                "content": {
                    "tool_use_id": "call_legacy",
                    "content": { "result": "found" }
                }
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "assistant");
    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["id"],
        "call_custom"
    );
    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["function"]["name"],
        "exec"
    );
    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["function"]["arguments"],
        "{\"input\":\"ls -la\"}"
    );
    assert_eq!(converted["messages"][1]["role"], "tool");
    assert_eq!(converted["messages"][1]["content"], "ok");
    assert_eq!(
        converted["messages"][2]["tool_calls"][0]["id"],
        "call_legacy"
    );
    assert_eq!(
        converted["messages"][3]["content"],
        "{\"result\":\"found\"}"
    );
}

#[test]
fn responses_input_flattens_namespace_function_history_and_skips_invalid_tool_items() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "function_call",
                "call_id": "call_ns",
                "namespace": "mcp__vscode_mcp__",
                "name": "execute_command",
                "arguments": "{\"command\":\"save\"}"
            },
            {
                "type": "function_call_output",
                "call_id": "call_ns",
                "output": "saved"
            },
            {
                "type": "function_call",
                "call_id": "missing_name",
                "arguments": "{}"
            },
            {
                "type": "function_call_output",
                "output": "orphan"
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["function"]["name"],
        "mcp__vscode_mcp__execute_command"
    );
    assert_eq!(converted["messages"][1]["tool_call_id"], "call_ns");
    assert_eq!(converted["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn responses_input_sanitizes_invalid_function_call_arguments_history() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "function_call",
                "call_id": "bad_object",
                "name": "broken_args",
                "arguments": "{foo: \"bar\"}"
            },
            {
                "type": "function_call",
                "call_id": "plain_text",
                "name": "plain_args",
                "arguments": "raw text with \"quotes\" and \\slashes"
            },
            {
                "type": "function_call",
                "call_id": "array_args",
                "name": "array_args",
                "arguments": "[1,2,3]"
            },
            {
                "type": "tool_call",
                "tool_use": {
                    "id": "object_args",
                    "name": "object_args",
                    "input": { "ok": true }
                }
            }
        ]
    }))
    .unwrap();

    let calls = converted["messages"][0]["tool_calls"].as_array().unwrap();
    for call in calls {
        let arguments = call["function"]["arguments"].as_str().unwrap();
        serde_json::from_str::<serde_json::Value>(arguments)
            .expect("chat tool call arguments must always be valid JSON");
    }
    assert_eq!(
        calls[0]["function"]["arguments"],
        "{\"input\":\"{foo: \\\"bar\\\"}\"}"
    );
    assert_eq!(
        calls[1]["function"]["arguments"],
        "{\"input\":\"raw text with \\\"quotes\\\" and \\\\slashes\"}"
    );
    assert_eq!(calls[2]["function"]["arguments"], "{\"input\":[1,2,3]}");
    assert_eq!(calls[3]["function"]["arguments"], "{\"ok\":true}");
}

#[test]
fn responses_input_downgrades_orphan_tool_outputs_to_user_messages() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "reasoning",
                "summary": [{ "type": "summary_text", "text": "I need the previous tool result." }]
            },
            {
                "type": "function_call_output",
                "call_id": "missing_call",
                "output": "tool output without a matching call"
            },
            {
                "type": "custom_tool_call_output",
                "call_id": "missing_custom",
                "output": "custom output without a matching call"
            }
        ]
    }))
    .unwrap();

    assert_eq!(converted["messages"][0]["role"], "assistant");
    assert!(converted["messages"][0].get("tool_calls").is_none());
    assert_eq!(converted["messages"][1]["role"], "user");
    assert_eq!(
        converted["messages"][1]["content"],
        "Function call output (missing_call): tool output without a matching call"
    );
    assert_eq!(converted["messages"][2]["role"], "user");
    assert_eq!(
        converted["messages"][2]["content"],
        "Function call output (missing_custom): custom output without a matching call"
    );
}

#[test]
fn responses_input_replays_apply_patch_custom_history_as_proxy_tool() {
    let converted = responses_to_chat_completions(json!({
        "model": "gpt-5-mini",
        "input": [
            {
                "type": "custom_tool_call",
                "call_id": "call_patch",
                "name": "apply_patch",
                "input": "*** Begin Patch\n*** Add File: docs/test.md\n+# Test\n*** End Patch"
            }
        ],
        "tools": [{ "type": "custom", "name": "apply_patch" }]
    }))
    .unwrap();

    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["function"]["name"],
        "apply_patch_add_file"
    );
    assert_eq!(
        converted["messages"][0]["tool_calls"][0]["function"]["arguments"],
        "{\"content\":\"# Test\",\"path\":\"docs/test.md\"}"
    );
}

#[test]
fn responses_request_preserves_input_image_for_multimodal_model() {
    // 路径 A：当 supports_image = true（默认多模态模型）时，input_image 应正常转成 image_url
    let converted = responses_to_chat_completions_with_image_support(
        json!({
            "model": "gpt-5.5",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "看下这张图" },
                        { "type": "input_image", "image_url": "https://example.com/cat.png" }
                    ]
                }
            ]
        }),
        true,
    )
    .unwrap();

    // 验证：content 应为数组（包含 image_url）
    let content = &converted["messages"][0]["content"];
    assert!(
        content.is_array(),
        "supports_image=true 时 content 应为数组以保留图片，实际为: {content}"
    );
    let parts = content.as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["type"], "text");
    assert_eq!(parts[0]["text"], "看下这张图");
    assert_eq!(parts[1]["type"], "image_url");
    assert_eq!(parts[1]["image_url"]["url"], "https://example.com/cat.png");
}

#[test]
fn responses_request_strips_input_image_for_text_only_model() {
    // 路径 A：MVP 核心场景 — 当 supports_image = false（纯文本模型）时，
    // input_image 应被静默移除，content 自动坍缩为纯文本字符串。
    // 这是修复 issue #1194 的关键测试。
    let converted = responses_to_chat_completions_with_image_support(
        json!({
            "model": "deepseek-v4-pro",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "看下这张图" },
                        { "type": "input_image", "image_url": "https://example.com/cat.png" }
                    ]
                }
            ]
        }),
        false,
    )
    .unwrap();

    // 验证：content 应坍缩为纯文本字符串，不含 image_url
    let content = &converted["messages"][0]["content"];
    assert!(
        content.is_string(),
        "supports_image=false 时 content 应坍缩为字符串，实际为: {content}"
    );
    assert_eq!(content.as_str().unwrap(), "看下这张图");

    // 验证：消息中不包含 image_url 任何痕迹
    let serialized = serde_json::to_string(&converted).unwrap();
    assert!(
        !serialized.contains("image_url"),
        "纯文本模式转换结果不应包含 image_url 字段"
    );
}

#[test]
fn responses_request_strips_input_image_alone_leaves_placeholder_text() {
    // 边界：用户只发了图片，没发文字，strip 后 content 应为空字符串而不是被丢弃
    let converted = responses_to_chat_completions_with_image_support(
        json!({
            "model": "deepseek-v4-pro",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_image", "image_url": "https://example.com/cat.png" }
                    ]
                }
            ]
        }),
        false,
    )
    .unwrap();

    // content 应是空字符串（不报错、不丢消息）
    let content = &converted["messages"][0]["content"];
    assert!(
        content.is_string(),
        "纯图片被 strip 后 content 应为字符串，实际为: {content}"
    );
    assert_eq!(content.as_str().unwrap(), "");
}

#[test]
fn responses_request_preserves_input_image_with_object_url() {
    // 多模态路径上 image_url 也可能是对象形式（{url, detail}），需保证两种格式都通过
    let converted = responses_to_chat_completions_with_image_support(
        json!({
            "model": "gpt-5.5",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "image_url": { "url": "data:image/png;base64,abc", "detail": "high" }
                        }
                    ]
                }
            ]
        }),
        true,
    )
    .unwrap();

    let parts = converted["messages"][0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0]["type"], "image_url");
    assert_eq!(parts[0]["image_url"]["url"], "data:image/png;base64,abc");
    assert_eq!(parts[0]["image_url"]["detail"], "high");
}

#[test]
fn model_supports_image_returns_true_when_strip_images_disabled() {
    let mut profile = RelayProfile::default();
    profile.strip_images = false;
    assert!(model_supports_image(&profile, "deepseek-v4-pro"));
    assert!(model_supports_image(&profile, "gpt-5.5"));
}

#[test]
fn model_supports_image_returns_false_when_strip_images_enabled() {
    let mut profile = RelayProfile::default();
    profile.strip_images = true;
    assert!(!model_supports_image(&profile, "deepseek-v4-pro"));
    assert!(!model_supports_image(&profile, "gpt-5.5"));
}

// ==========================================================================
// 路径 A 进阶 + 路径 C 进阶：per-model 图片能力 map
//
// spec 进阶方案：RelayProfile.modelImageSupport 存 JSON map
// { "deepseek-v4-pro": false, "gpt-5.5": true }，覆盖 stripImages 的全局开关，
// 让同供应商下纯文本模型和视觉模型可共存。
// 查询优先级：map 命中 → 用 map；map 未命中 → 走 stripImages 默认值。
// ==========================================================================

#[test]
fn model_supports_image_uses_per_model_map_when_configured() {
    // map 明确写 deepseek-v4-pro: false，stripImages=false 也不应被 strip
    let mut profile = RelayProfile::default();
    profile.strip_images = false;
    profile.model_image_support = r#"{"deepseek-v4-pro": false}"#.to_string();
    assert!(!model_supports_image(&profile, "deepseek-v4-pro"));
}

#[test]
fn model_supports_image_per_model_map_overrides_strip_images() {
    // map 写 deepseek=true, gpt=false，strip_images=true 应被覆盖
    let mut profile = RelayProfile::default();
    profile.strip_images = true;
    profile.model_image_support = r#"{"deepseek-v4-pro": true, "gpt-5.5": false}"#.to_string();
    assert!(
        model_supports_image(&profile, "deepseek-v4-pro"),
        "map 显式 true 应覆盖 strip_images=true"
    );
    assert!(
        !model_supports_image(&profile, "gpt-5.5"),
        "map 显式 false 应被尊重"
    );
}

#[test]
fn model_supports_image_per_model_map_defaults_to_support_when_map_nonempty_and_model_missing() {
    // per-model map 非空时：未列出的模型默认「支持图片」，
    // 让视觉模型（kimi / minimax）与纯文本模型（map 里标 false 的）在同一中转共存。
    // 用户反馈：stripImages=true + 非空 map 时，kimi（不在 map）被误 strip，
    // 必须取消 stripImages 才正常。改成 map 非空时 strip_images 不再作用于未列出模型。
    let mut profile = RelayProfile::default();
    profile.strip_images = true;
    profile.model_image_support = r#"{"deepseek-v4-flash": false}"#.to_string();
    // 未列出的视觉模型即使在 strip_images=true 时也应支持图片
    assert!(
        model_supports_image(&profile, "kimi-k2.6"),
        "map 非空时未列出模型应默认支持图片，不受 strip_images 影响"
    );
    assert!(model_supports_image(&profile, "minimax-m3"), "minimax 同理");
    // map 里显式标 false 的纯文本模型仍应被 strip
    assert!(
        !model_supports_image(&profile, "deepseek-v4-flash"),
        "map 显式 false 仍应被尊重"
    );

    // strip_images=false 时同样：未列出 -> 支持
    profile.strip_images = false;
    assert!(model_supports_image(&profile, "kimi-k2.6"));
}

#[test]
fn model_supports_image_empty_map_falls_back_to_strip_images() {
    // map 为空时：退化为 profile 级 strip_images 开关（MVP 行为，向后兼容）
    let mut profile = RelayProfile::default();
    profile.strip_images = true;
    profile.model_image_support = String::new();
    assert!(!model_supports_image(&profile, "deepseek-v4-pro"));

    profile.strip_images = false;
    assert!(model_supports_image(&profile, "deepseek-v4-pro"));
}

#[test]
fn model_supports_image_matches_case_insensitively() {
    // 模型名大小写匹配：Codex 发送的 model 名可能与 map key 大小写不一致
    // （如 map 里 "GLM-5.2"，Codex 发 "glm-5.2"）。查询应不区分大小写。
    let mut profile = RelayProfile::default();
    profile.model_image_support = r#"{"GLM-5.2": false, "deepseek-v4-flash": false}"#.to_string();
    assert!(
        !model_supports_image(&profile, "glm-5.2"),
        "小写查询应匹配大写 key"
    );
    assert!(
        !model_supports_image(&profile, "GLM-5.2"),
        "精确匹配仍应工作"
    );
    assert!(
        !model_supports_image(&profile, "DeepSeek-V4-Flash"),
        "大小写混写也应匹配"
    );
}

#[test]
fn model_supports_image_per_model_map_handles_empty_and_invalid_gracefully() {
    // 空 map / 非法 JSON 不能崩，回落到 strip_images
    let mut profile = RelayProfile::default();
    profile.strip_images = true;
    profile.model_image_support = String::new();
    assert!(!model_supports_image(&profile, "deepseek-v4-pro"));

    profile.model_image_support = "not json".to_string();
    assert!(!model_supports_image(&profile, "deepseek-v4-pro"));

    profile.model_image_support = r#"{"deepseek-v4-pro": true}"#.to_string();
    assert!(model_supports_image(&profile, "deepseek-v4-pro"));
}

// ==========================================================================
// 路径 A 续：Responses 透传剥离 reasoning（不支持推理的模型）
//
// 用户反馈：kimi-2.6 在 Ark 走 Responses 透传，Codex 默认带 reasoning 参数，
// Ark 的 kimi 端点拒绝 "reasoning is not supported by current model"。
// 与 input_image 同类：部分模型不支持的字段，透传前需剥离。
// per-model map 控制（同 modelImageSupport 模式），无 profile 级开关，
// 避免和 stripImages 一样的「全局开关压过 per-model」冲突。
// ==========================================================================

#[test]
fn model_supports_reasoning_defaults_to_true_when_map_empty() {
    // 空 map / 未配置 -> 默认支持 reasoning（不误伤推理模型）
    let profile = RelayProfile::default();
    assert!(model_supports_reasoning(&profile, "minimax-m3"));
    assert!(model_supports_reasoning(&profile, "deepseek-v4-flash"));
}

#[test]
fn model_supports_reasoning_uses_per_model_map() {
    let mut profile = RelayProfile::default();
    profile.model_reasoning_support = r#"{"kimi-k2.6": false}"#.to_string();
    // map 命中 false -> 不支持
    assert!(!model_supports_reasoning(&profile, "kimi-k2.6"));
    // 未列出 -> 默认支持（不误伤 minimax 等推理模型）
    assert!(model_supports_reasoning(&profile, "minimax-m3"));
    assert!(model_supports_reasoning(&profile, "deepseek-v4-flash"));
}

#[test]
fn model_supports_reasoning_matches_case_insensitively() {
    let mut profile = RelayProfile::default();
    profile.model_reasoning_support = r#"{"Kimi-K2.6": false}"#.to_string();
    assert!(!model_supports_reasoning(&profile, "kimi-k2.6"));
    assert!(!model_supports_reasoning(&profile, "KIMI-K2.6"));
}

#[test]
fn strip_reasoning_in_place_removes_reasoning_when_unsupported() {
    let mut body = json!({
        "model": "kimi-k2.6",
        "reasoning": { "effort": "high" },
        "input": [
            { "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }
        ]
    });
    strip_reasoning_in_place(&mut body, false);
    assert!(body.get("reasoning").is_none(), "reasoning 应被移除");
    // 其余字段不动
    assert_eq!(body["model"], "kimi-k2.6");
    assert_eq!(body["input"][0]["content"][0]["text"], "hi");
}

#[test]
fn strip_reasoning_in_place_preserves_reasoning_when_supported() {
    let mut body = json!({
        "model": "minimax-m3",
        "reasoning": { "effort": "high" },
        "input": []
    });
    strip_reasoning_in_place(&mut body, true);
    assert!(
        body.get("reasoning").is_some(),
        "支持推理时 reasoning 应保留"
    );
    assert_eq!(body["reasoning"]["effort"], "high");
}

#[test]
fn strip_reasoning_in_place_noop_when_reasoning_absent() {
    let mut body = json!({
        "model": "kimi-k2.6",
        "input": [{ "type": "message", "role": "user", "content": [{ "type": "input_text", "text": "hi" }] }]
    });
    strip_reasoning_in_place(&mut body, false);
    // 无 reasoning 字段时不崩，body 不变
    assert!(body.get("reasoning").is_none());
    assert_eq!(body["input"][0]["content"][0]["text"], "hi");
}

#[test]
fn chat_path_strips_reasoning_when_model_unsupported() {
    // Bug 2：Chat 路径转换前剥离 reasoning（模型不支持时）。
    // kimi-k2.6 走 Thinking 风格；未剥离时转换会注入 thinking 字段，剥离后不注入。
    let mut profile = RelayProfile::default();
    profile.protocol = codex_plus_core::settings::RelayProtocol::ChatCompletions;
    profile.model_reasoning_support = r#"{"kimi-k2.6": false}"#.to_string();
    let body = json!({
        "model": "kimi-k2.6",
        "reasoning": { "effort": "high" },
        "input": []
    });
    let (_endpoint, upstream_body, wire_api) =
        upstream_request_parts_with_image_decision(&profile, body, true).unwrap();
    assert_eq!(
        wire_api,
        codex_plus_core::protocol_proxy::UpstreamWireApi::ChatCompletions
    );
    assert!(upstream_body.get("reasoning").is_none(), "reasoning 应被剥离");
    assert!(
        upstream_body.get("thinking").is_none(),
        "reasoning 剥离后不应注入 thinking，实际：{upstream_body}"
    );
    assert!(upstream_body.get("reasoning_effort").is_none());
}

#[test]
fn chat_path_preserves_reasoning_when_supported() {
    // Bug 2 回归保护：模型支持 reasoning（map 显式 true）时不误伤，转换正常注入。
    let mut profile = RelayProfile::default();
    profile.protocol = codex_plus_core::settings::RelayProtocol::ChatCompletions;
    profile.model_reasoning_support = r#"{"kimi-k2.6": true}"#.to_string();
    let body = json!({
        "model": "kimi-k2.6",
        "reasoning": { "effort": "high" },
        "input": []
    });
    let (_e, upstream_body, _w) =
        upstream_request_parts_with_image_decision(&profile, body, true).unwrap();
    // kimi Thinking 风格 + reasoning effort high -> 注入 thinking
    assert!(
        upstream_body.get("thinking").is_some(),
        "支持 reasoning 时应保留并注入 thinking，实际：{upstream_body}"
    );
}

#[test]
fn responses_path_preserves_reasoning_passthrough() {
    // Bug 2 边界：Responses 协议纯透传，不剥离 reasoning（已知局限，用户接受）。
    let mut profile = RelayProfile::default();
    profile.protocol = codex_plus_core::settings::RelayProtocol::Responses;
    profile.model_reasoning_support = r#"{"kimi-k2.6": false}"#.to_string();
    let body = json!({
        "model": "kimi-k2.6",
        "reasoning": { "effort": "high" },
        "input": []
    });
    let (_e, upstream_body, _w) =
        upstream_request_parts_with_image_decision(&profile, body, true).unwrap();
    assert!(
        upstream_body.get("reasoning").is_some(),
        "Responses 透传应保留 reasoning，实际：{upstream_body}"
    );
}

#[test]
fn upstream_responses_passthrough_preserves_images_and_reasoning_as_is_for_text_only_model()
 {
    // 集成：Responses 透传分支不剥离任何内容，原样转发。
    // 即使模型不支持图片/推理，Response 格式也不干预。
    let mut profile = RelayProfile::default();
    profile.protocol = codex_plus_core::settings::RelayProtocol::Responses;
    profile.strip_images = false;
    profile.model_image_support = r#"{"kimi-k2.6": false}"#.to_string();
    profile.model_reasoning_support = r#"{"kimi-k2.6": false}"#.to_string();

    let body = json!({
        "model": "kimi-k2.6",
        "reasoning": { "effort": "high" },
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                { "type": "input_text", "text": "看这张图" },
                { "type": "input_image", "image_url": "https://x.com/a.png" }
            ]
        }]
    });

    let (endpoint, upstream_body, wire_api) =
        upstream_request_parts_with_image_decision(&profile, body, false).unwrap();

    assert!(
        endpoint.ends_with("/responses"),
        "Responses 透传 endpoint 应以 /responses 结尾"
    );
    assert_eq!(
        wire_api,
        codex_plus_core::protocol_proxy::UpstreamWireApi::Responses
    );

    let serialized = serde_json::to_string(&upstream_body).unwrap();
    // Response 格式纯透传，图片和 reasoning 都保留原样
    assert!(serialized.contains("input_image"), "Response 格式应保留 input_image");
    assert!(serialized.contains("a.png"), "Response 格式应保留图片 URL");
    assert!(serialized.contains("reasoning"), "Response 格式应保留 reasoning");
    assert!(serialized.contains("看这张图"), "input_text 应保留");
}

#[test]
fn upstream_responses_passthrough_preserves_images_and_reasoning_for_full_capability_model() {
    // 反向回归：支持图片 + 推理的模型（如 minimax-m3），两者都保留。
    let mut profile = RelayProfile::default();
    profile.protocol = codex_plus_core::settings::RelayProtocol::Responses;
    profile.model_image_support = r#"{"kimi-k2.6": false}"#.to_string();
    profile.model_reasoning_support = r#"{"kimi-k2.6": false}"#.to_string();

    let body = json!({
        "model": "minimax-m3",
        "reasoning": { "effort": "high" },
        "input": [{
            "type": "message",
            "role": "user",
            "content": [
                { "type": "input_text", "text": "看这张图" },
                { "type": "input_image", "image_url": "https://x.com/a.png" }
            ]
        }]
    });

    let (_endpoint, upstream_body, _wire_api) =
        upstream_request_parts_with_image_decision(&profile, body, true).unwrap();

    let serialized = serde_json::to_string(&upstream_body).unwrap();
    assert!(
        serialized.contains("input_image"),
        "视觉模型的 input_image 应保留"
    );
    assert!(
        serialized.contains("reasoning"),
        "推理模型的 reasoning 应保留"
    );
}
// ==========================================================================
// 路径 B1：analyze_images_with_vl — 视觉模型中转
//
// spec 第四章路径 B1：纯文本模型请求中遇到 input_image 时，
// 调 VL API 拿文字描述，替换为 input_text 后再走协议转换。
// 测试用本地 mock HTTP server 模拟 VL 上游。
// ==========================================================================

/// 启动一个 mock VL 服务端：收到请求后回写 `response_body`，并把收到的请求体存到 `captured`。
async fn mock_vl_server(
    listener: tokio::net::TcpListener,
    response_body: &'static str,
    captured: std::sync::Arc<std::sync::Mutex<String>>,
) {
    let (mut stream, _) = listener.accept().await.unwrap();
    // 先读 header 直到 \r\n\r\n
    let mut header_buf = Vec::with_capacity(512);
    let mut body_len: Option<usize> = None;
    let mut tmp = [0u8; 1];
    loop {
        let n = stream.read(&mut tmp).await.unwrap();
        if n == 0 {
            break;
        }
        header_buf.push(tmp[0]);
        if header_buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if header_buf.len() > 16384 {
            break;
        }
    }
    let header_str = String::from_utf8_lossy(&header_buf).to_string();
    if let Some(cl_line) = header_str
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
    {
        body_len = cl_line
            .split(':')
            .nth(1)
            .and_then(|s| s.trim().parse::<usize>().ok());
    }
    // 读 body 精确字节数
    let mut body_buf = Vec::new();
    if let Some(len) = body_len {
        while body_buf.len() < len {
            let need = len - body_buf.len();
            let mut chunk = vec![0u8; need.min(4096)];
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            body_buf.extend_from_slice(&chunk[..n]);
        }
    }
    // 捕获完整原始请求（请求行 + 头 + 体），便于断言 endpoint path 等请求级细节；
    // 现有测试按 body 内容 `.contains(...)` 断言依然成立（body 仍包含在内）。
    let mut raw = String::new();
    raw.push_str(&header_str);
    raw.push_str(&String::from_utf8_lossy(&body_buf));
    *captured.lock().unwrap() = raw;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body,
    );
    stream.write_all(response.as_bytes()).await.unwrap();
    let _ = stream.shutdown().await;
}

fn vl_response(description: &str) -> String {
    format!(
        r#"{{"id":"vl-1","object":"chat.completion","model":"qwen-vl-plus","choices":[{{"index":0,"message":{{"role":"assistant","content":"{}"}},"finish_reason":"stop"}}]}}"#,
        description.replace('"', "\\\"")
    )
}

#[tokio::test]
async fn analyze_images_with_vl_is_noop_when_disabled() {
    // VL 关闭时直接返回，不发任何 HTTP 请求
    let mut config = VisionRelayConfig::default();
    config.enabled = false;
    config.model = "qwen-vl-plus".to_string();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://x.com/a.png"}]}]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    // input_image 必须保留（未启用 VL）
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(serialized.contains("input_image"), "VL 关闭时不应改动 body");
}

#[tokio::test]
async fn analyze_images_with_vl_is_noop_when_no_images() {
    // 没有 input_image 时不调 VL
    let mut config = VisionRelayConfig::default();
    config.enabled = true;
    config.model = "qwen-vl-plus".to_string();
    // 故意指向无效端口 —— 如果函数错误地发起调用，会失败
    config.base_url = "http://127.0.0.1:1/v1".to_string();
    config.api_key = "sk-test".to_string();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [{"type":"message","role":"user","content":[{"type":"input_text","text":"纯文本问题"}]}]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    assert_eq!(body["input"][0]["content"][0]["text"], "纯文本问题");
}

#[tokio::test]
async fn analyze_images_with_vl_replaces_input_image_with_description() {
    // 核心场景：input_image 被 VL 描述替换为 input_text
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("图片中是一只橘猫");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "qwen-vl-plus".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_text","text":"看下这张图"},
                {"type":"input_image","image_url":"https://example.com/cat.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    // 验证：input_image 已被替换为 input_text，文字内容是 VL 返回的描述
    let parts = body["input"][0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0]["type"], "input_text");
    assert_eq!(parts[0]["text"], "看下这张图");
    assert_eq!(parts[1]["type"], "input_text");
    assert!(parts[1]["text"].as_str().unwrap().contains("橘猫"));
    // 不应残留 image_url
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(
        !serialized.contains("image_url"),
        "strip 后不应残留 image_url"
    );
    // VL 收到的请求体应包含图片 URL
    let vl_request_body = captured.lock().unwrap().clone();
    assert!(vl_request_body.contains("https://example.com/cat.png"));
    assert!(vl_request_body.contains("qwen-vl-plus"));
    // 用户提问文字应被转发给 VL 模型（带问题识图）
    assert!(
        vl_request_body.contains("看下这张图"),
        "用户提问应转发给 VL 模型"
    );
}

#[tokio::test]
async fn analyze_images_with_vl_forwards_user_question_as_prompt() {
    // 用户提问文字应作为 prompt 传给 VL 模型，带问题识图
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("截图显示一个登录表单");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "mimo-v2.5".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "xopdeepseekv4flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_text","text":"这个登录表单的报错信息是什么？"},
                {"type":"input_image","image_url":"https://example.com/login.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    let vl_request_body = captured.lock().unwrap().clone();
    // 用户的具体问题应出现在 VL 请求体里
    assert!(
        vl_request_body.contains("这个登录表单的报错信息是什么？"),
        "VL 请求应包含用户提问文字，实际：{vl_request_body}"
    );
}

#[tokio::test]
async fn analyze_images_with_vl_uses_configured_max_tokens() {
    // max_tokens 应从 config 读取，不硬编码 256
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("图片描述");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "mimo-v2.5".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 1024,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "xopdeepseekv4flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_image","image_url":"https://example.com/a.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    let vl_request_body = captured.lock().unwrap().clone();
    assert!(
        vl_request_body.contains(r#""max_tokens":1024"#),
        "VL 请求应使用配置的 max_tokens=1024，实际：{vl_request_body}"
    );
    assert!(
        !vl_request_body.contains(r#""max_tokens":256"#),
        "不应硬编码 256"
    );
}

#[tokio::test]
async fn analyze_images_with_vl_falls_back_to_generic_prompt_without_user_text() {
    // 消息只有图片没有文字时，退回固定提示词（不崩）
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("一张图片");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "mimo-v2.5".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "xopdeepseekv4flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_image","image_url":"https://example.com/a.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    let vl_request_body = captured.lock().unwrap().clone();
    // 无用户文字时应用固定提示词
    assert!(
        vl_request_body.contains("简要描述这张图片"),
        "无用户文字时应退回固定提示词，实际：{vl_request_body}"
    );
}

#[tokio::test]
async fn analyze_images_with_vl_uses_responses_api_when_protocol_is_responses() {
    // protocol=Responses：请求体用 `input` 数组 + input_text/input_image，
    // 响应解析 `output[*].content[*].text`
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = r#"{"id":"resp-1","object":"response","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"图片中是一只橘猫"}]}]}"#;
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.to_string().into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "gpt-5.5".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::Responses,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_image","image_url":"https://example.com/cat.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    // 替换为 input_text
    let content_text = body["input"][0]["content"][0]["text"].as_str().unwrap();
    assert!(content_text.contains("橘猫"));
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(!serialized.contains("image_url"));
    // 验证请求体格式：Responses API 应该用 `input` 数组 + input_text/input_image
    let vl_request_body = captured.lock().unwrap().clone();
    assert!(
        vl_request_body.contains("\"input\":"),
        "Responses 请求体应含 input 数组"
    );
    assert!(
        vl_request_body.contains("input_text"),
        "Responses 请求体应含 input_text part"
    );
    assert!(
        vl_request_body.contains("input_image"),
        "Responses 请求体应含 input_image part"
    );
    assert!(
        !vl_request_body.contains("\"messages\":"),
        "Responses 协议不应含 Chat Completions 的 messages 字段"
    );
    // image_url 应为字符串（Responses API 格式），不是对象 {url:...}
    // Ark 等上游收到对象会报 "invalid url scheme, parse map[url:...]"
    assert!(
        vl_request_body.contains(r#""image_url":"https://example.com/cat.png""#),
        "Responses 协议 image_url 应为字符串，实际：{vl_request_body}"
    );
    assert!(
        !vl_request_body.contains(r#""image_url":{"url""#),
        "Responses 协议 image_url 不应为对象 {{url:...}}，实际：{vl_request_body}"
    );
}

#[tokio::test]
async fn analyze_images_with_vl_strips_old_images_outside_context_window() {
    // context_window 限制：超出窗口的老图直接 strip，不调 VL（省成本）
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("最近一张图的描述");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "mimo-v2.5".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        // 只覆盖最近一条消息（~25 token），两条老图 strip
        context_window: 30,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let long_text = "x".repeat(100); // ~25 token
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_text","text": long_text.clone()},
                {"type":"input_image","image_url":"https://example.com/old1.png"}
            ]},
            {"type":"message","role":"user","content":[
                {"type":"input_text","text": long_text.clone()},
                {"type":"input_image","image_url":"https://example.com/old2.png"}
            ]},
            {"type":"message","role":"user","content":[
                {"type":"input_text","text": long_text.clone()},
                {"type":"input_image","image_url":"https://example.com/recent.png"}
            ]}
        ]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    server.await.unwrap();

    // 只有窗口内的最近一张图被 VL 处理
    let vl_request_body = captured.lock().unwrap().clone();
    assert!(
        vl_request_body.contains("recent.png"),
        "窗口内的图应被 VL 处理，实际：{vl_request_body}"
    );
    assert!(
        !vl_request_body.contains("old1.png") && !vl_request_body.contains("old2.png"),
        "窗口外的老图不应调 VL，实际：{vl_request_body}"
    );

    // 窗口外的图被 strip（input_image 移除），最近图的 VL 描述保留
    let serialized = serde_json::to_string(&body).unwrap();
    assert!(!serialized.contains("old1.png"), "老图 URL 应被移除");
    assert!(!serialized.contains("old2.png"), "老图 URL 应被移除");
    assert!(serialized.contains("最近一张图"), "最近图的 VL 描述应保留");
}

#[tokio::test]
async fn analyze_images_with_vl_returns_error_when_vl_call_fails() {
    // VL 不可用时必须返回 Err（让上游决定是否降级为 strip）
    let config = VisionRelayConfig {
        enabled: true,
        model: "qwen-vl-plus".to_string(),
        api_key: "sk-test".to_string(),
        base_url: "http://127.0.0.1:1/v1".to_string(), // 不可达
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_millis(200))
        .build()
        .unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [
            {"type":"message","role":"user","content":[
                {"type":"input_image","image_url":"https://example.com/cat.png"}
            ]}
        ]
    });
    let result = analyze_images_with_vl(&mut body, &config, &client).await;
    assert!(result.is_err(), "VL 不可达时必须返回 Err 让上游降级");
}

#[test]
fn upstream_chat_error_is_regularized_as_responses_error_envelope() {
    let json_error = responses_error_from_upstream(
        400,
        "application/json",
        br#"{"error":{"message":"bad request","type":"invalid_request_error","code":"bad_model","param":"model"}}"#,
    );
    assert_eq!(json_error["error"]["message"], "bad request");
    assert_eq!(json_error["error"]["type"], "invalid_request_error");
    assert_eq!(json_error["error"]["code"], "bad_model");
    assert_eq!(json_error["error"]["param"], "model");

    let text_error = responses_error_from_upstream(502, "text/html", b"<html>bad gateway</html>");
    assert_eq!(text_error["error"]["message"], "<html>bad gateway</html>");
    assert_eq!(text_error["error"]["type"], "upstream_error");
    assert_eq!(text_error["error"]["code"], "502");
}

#[tokio::test]
async fn apply_vl_with_fallback_returns_supports_image_true_when_base_already_true() {
    // 多模态模型：base supports_image=true，VL 不会触发
    let mut relay = RelayProfile::default();
    relay.strip_images = false; // supports_image = true
    let mut config = VisionRelayConfig::default();
    config.enabled = true;
    config.model = "qwen-vl-plus".to_string();
    config.base_url = "http://127.0.0.1:1/v1".to_string(); // 不可达，证明不会调
    let body = json!({"model":"gpt-5.5","input":[]});
    let (supports_image, returned_body) = apply_vl_with_fallback(&relay, body.clone(), &config, "")
        .await
        .unwrap();
    assert!(supports_image);
    assert_eq!(returned_body, body);
}

#[tokio::test]
async fn apply_vl_with_fallback_returns_supports_image_false_when_vl_disabled() {
    // 纯文本模型但 VL 未启用：返回 (false, body) 让 strip 处理
    let mut relay = RelayProfile::default();
    relay.strip_images = true; // supports_image = false
    let config = VisionRelayConfig::default(); // enabled = false
    let body = json!({
        "model":"deepseek-v4-flash",
        "input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://x.com/a.png"}]}]
    });
    let (supports_image, returned_body) = apply_vl_with_fallback(&relay, body.clone(), &config, "")
        .await
        .unwrap();
    assert!(!supports_image);
    assert_eq!(
        returned_body, body,
        "VL 未启用时 body 必须原样返回（由 strip 处理）"
    );
}

#[tokio::test]
async fn apply_vl_with_fallback_falls_back_to_strip_when_vl_fails() {
    // 纯文本模型 + VL 启用 + VL 不可达：返回 (false, 原 body) 让 strip 处理
    ensure_no_proxy_for_localhost();
    let mut relay = RelayProfile::default();
    relay.strip_images = true;
    let mut config = VisionRelayConfig::default();
    config.enabled = true;
    config.model = "qwen-vl-plus".to_string();
    config.base_url = "http://127.0.0.1:1/v1".to_string(); // 不可达
    config.api_key = "sk-test".to_string();
    let body = json!({
        "model":"deepseek-v4-flash",
        "input":[{"type":"message","role":"user","content":[{"type":"input_image","image_url":"https://x.com/a.png"}]}]
    });
    let (supports_image, returned_body) = apply_vl_with_fallback(&relay, body.clone(), &config, "")
        .await
        .unwrap();
    assert!(!supports_image, "VL 失败时必须降级为 strip，不能阻断用户");
    assert_eq!(
        returned_body, body,
        "VL 失败时 body 必须是原版（让 strip 处理）"
    );
}

#[tokio::test]
async fn apply_vl_with_fallback_returns_preprocessed_body_when_vl_succeeds() {
    // 纯文本模型 + VL 启用 + VL 可用：返回 (true, 预处理后 body)
    ensure_no_proxy_for_localhost();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("一只橘猫坐在窗台");
    let server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let mut relay = RelayProfile::default();
    relay.strip_images = true;
    let config = VisionRelayConfig {
        enabled: true,
        model: "qwen-vl-plus".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let body = json!({
        "model":"deepseek-v4-flash",
        "input":[{"type":"message","role":"user","content":[
            {"type":"input_image","image_url":"https://example.com/cat.png"}
        ]}]
    });
    let (supports_image, returned_body) = apply_vl_with_fallback(&relay, body, &config, "")
        .await
        .unwrap();
    server.await.unwrap();
    assert!(
        supports_image,
        "VL 成功后 supports_image 必须 true（让 strip 走 no-op）"
    );
    let content = returned_body["input"][0]["content"][0].clone();
    assert_eq!(content["type"], "input_text");
    assert!(content["text"].as_str().unwrap().contains("橘猫"));
}

#[tokio::test]
async fn vl_endpoint_normalizes_bare_domain_with_v1() {
    // Bug 5：裸域名 base_url（无 /v1）-> VL 请求应打到 /v1/chat/completions
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("一只猫");
    let _server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "qwen-vl-plus".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}"), // 裸域名，无 /v1
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [{"type":"message","role":"user","content":[
            {"type":"input_image","image_url":"https://example.com/a.png"}
        ]}]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();

    let raw = captured.lock().unwrap().clone();
    assert!(
        raw.contains("POST /v1/chat/completions"),
        "裸域名应补 /v1，实际请求行：{raw}"
    );
}

#[tokio::test]
async fn vl_endpoint_does_not_duplicate_path() {
    // Bug 5：完整 endpoint base_url -> 不重复拼 /chat/completions
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response("一只猫");
    let _server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    let config = VisionRelayConfig {
        enabled: true,
        model: "qwen-vl-plus".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1/chat/completions"), // 完整 endpoint
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [{"type":"message","role":"user","content":[
            {"type":"input_image","image_url":"https://example.com/a.png"}
        ]}]
    });
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();

    let raw = captured.lock().unwrap().clone();
    assert!(
        raw.contains("POST /v1/chat/completions "),
        "完整 endpoint 不应重复拼路径，实际请求行：{raw}"
    );
    assert!(
        !raw.contains("/chat/completions/chat/completions"),
        "不应出现重复路径，实际请求行：{raw}"
    );
}

#[tokio::test]
async fn vl_log_does_not_panic_on_chinese_description_and_omits_body() {
    // Bug 3 + Bug 6：VL 返回 >200 字节的中文描述（含唯一标记）。
    // 旧代码 `&description[..200]` 字节截断在汉字中间会 panic；且 description_preview
    // 把描述正文写进 diagnostic_log（泄露截图内容）。修复后：不 panic + 日志只记元数据。
    use codex_plus_core::diagnostic_log;

    let marker = "VL_BODY_SECRET_";
    let description = format!("{marker}{}", "中".repeat(100)); // 15 + 300 = 315 字节

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let response = vl_response(&description);
    let _server = tokio::spawn(async move {
        mock_vl_server(
            listener,
            Box::leak(response.into_boxed_str()),
            captured_clone,
        )
        .await;
    });

    // 重定向 diagnostic_log 到临时文件，便于断言日志内容
    let log_file = tempfile::NamedTempFile::new().unwrap();
    let log_path = log_file.path().to_path_buf();
    diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path.clone()));

    let config = VisionRelayConfig {
        enabled: true,
        model: "vl-bug36-unique-marker".to_string(),
        api_key: "sk-test".to_string(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        max_tokens: 256,
        context_window: 0,
    };
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let mut body = json!({
        "model": "deepseek-v4-flash",
        "input": [{"type":"message","role":"user","content":[
            {"type":"input_image","image_url":"https://example.com/a.png"}
        ]}]
    });
    // 不应 panic（旧代码在此会 panic: byte index 200 is not a char boundary）
    analyze_images_with_vl(&mut body, &config, &client)
        .await
        .unwrap();
    // 立即恢复默认日志路径，避免影响其他测试
    diagnostic_log::set_diagnostic_log_path_for_tests(None);

    let log_content = std::fs::read_to_string(&log_path).unwrap_or_default();
    // 用唯一 vlModel 定位本测试的 vl_described 行（并行测试可能共享临时日志文件）
    let vl_line = log_content
        .lines()
        .find(|l| {
            l.contains(r#""event":"protocol_proxy.vl_described""#)
                && l.contains(r#""vlModel":"vl-bug36-unique-marker""#)
        })
        .expect("应有本测试的 vl_described 日志记录");
    // Bug 6：日志只记元数据，不含描述正文标记
    assert!(
        !vl_line.contains(marker),
        "日志不应含描述正文标记，实际：{vl_line}"
    );
    assert!(
        vl_line.contains("description_len") && vl_line.contains("description_chars"),
        "日志应记 description_len/description_chars 元数据，实际：{vl_line}"
    );
    // 元数据值正确：315 字节、115 字符（15 ASCII + 100 中文）
    assert!(vl_line.contains(r#""description_len":315"#), "实际：{vl_line}");
    assert!(
        vl_line.contains(r#""description_chars":115"#),
        "实际：{vl_line}"
    );
}

#[tokio::test]
async fn proxied_client_respects_http_proxy_after_no_proxy_revert() {
    // Bug 1：撤回 .no_proxy() 后，proxied_client 应尊重 HTTP_PROXY env，
    // 把非 NO_PROXY 主机的请求转发给代理（而非直连）。
    // 区分性：若仍带 .no_proxy()，HTTP_PROXY 被忽略 -> 请求直连 .invalid 主机 ->
    // DNS 失败 -> 代理 mock 收不到任何请求 -> 断言失败（RED）。
    //
    // 并行安全：先调 ensure_no_proxy_for_localhost 设置 NO_PROXY=127.0.0.1,localhost，
    // 再设 HTTP_PROXY；故并发测试的 127.0.0.1 请求始终被 NO_PROXY 绕过，不受影响。
    ensure_no_proxy_for_localhost();

    // 代理 mock：接受连接，读请求行，回 502（只需验证请求到达代理）
    let proxy_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_clone = captured.clone();
    let proxy_task = tokio::spawn(async move {
        let Ok((mut stream, _)) = proxy_listener.accept().await else {
            return;
        };
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        if n > 0 {
            *captured_clone.lock().unwrap() = String::from_utf8_lossy(&buf[..n]).to_string();
        }
        let _ = stream
            .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await;
    });

    // 临时设置 HTTP_PROXY 指向代理 mock（NO_PROXY 已含 127.0.0.1，不影响并发 localhost 测试）
    let saved_http_proxy = std::env::var("HTTP_PROXY").ok();
    // SAFETY: HTTP_PROXY 仅在本次测试窗口内设置，结束后立即恢复；并发 localhost 测试受 NO_PROXY 保护
    unsafe { std::env::set_var("HTTP_PROXY", format!("http://{proxy_addr}")) };
    let client = codex_plus_core::http_client::proxied_client("test").unwrap();
    // .invalid TLD（RFC 6761）不解析；带代理时 reqwest 把请求转给代理 mock，不解析 DNS
    let _ = client
        .get("http://bug1-test.invalid/")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;
    // 立即恢复 HTTP_PROXY
    match saved_http_proxy {
        Some(v) => unsafe { std::env::set_var("HTTP_PROXY", v) },
        None => unsafe { std::env::remove_var("HTTP_PROXY") },
    }
    proxy_task.abort();

    let raw = captured.lock().unwrap().clone();
    assert!(
        !raw.is_empty(),
        "proxied_client 应尊重 HTTP_PROXY 把请求转发给代理；若仍带 .no_proxy() 则请求直连 .invalid 失败，代理收不到"
    );
    assert!(
        raw.contains("bug1-test.invalid"),
        "代理应收到转发请求行，实际：{raw}"
    );
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
fn chat_completion_response_extracts_reasoning_details_like_ccswitch() {
    let converted = chat_completion_to_response(json!({
        "id": "chatcmpl_reasoning_details",
        "created": 123,
        "model": "MiniMax-M2.7",
        "choices": [{
            "finish_reason": "stop",
            "message": {
                "role": "assistant",
                "reasoning_details": [
                    { "summary": "Step one." },
                    { "parts": [{ "text": "Step two." }] }
                ],
                "content": "final"
            }
        }]
    }))
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "reasoning");
    assert_eq!(
        converted["output"][0]["summary"][0]["text"],
        "Step one.\n\nStep two."
    );
    assert_eq!(converted["output"][1]["content"][0]["text"], "final");
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
    assert_eq!(converted["usage"]["total_tokens"], 15);
    assert!(converted["usage"].get("input_tokens_details").is_none());
    assert_eq!(converted["usage"]["cache_read_input_tokens"], 1);
    assert_eq!(converted["usage"]["cache_creation_input_tokens"], 4);
}

#[test]
fn chat_completion_response_maps_custom_and_namespace_calls_with_request_context() {
    let request = json!({
        "model": "gpt-5-mini",
        "input": "hi",
        "tools": [
            { "type": "custom", "name": "exec" },
            {
                "type": "namespace",
                "name": "mcp__vscode_mcp__",
                "tools": [
                    { "type": "function", "name": "open_file", "parameters": {} }
                ]
            }
        ]
    });
    let converted = chat_completion_to_response_with_request(
        json!({
            "id": "chatcmpl_tools",
            "created": 123,
            "model": "gpt-5-mini",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "id": "call_custom",
                            "type": "function",
                            "function": {
                                "name": "exec",
                                "arguments": "{\"input\":\"ls -la\"}"
                            }
                        },
                        {
                            "id": "call_ns",
                            "type": "function",
                            "function": {
                                "name": "mcp__vscode_mcp__open_file",
                                "arguments": "{\"path\":\"src/main.rs\"}"
                            }
                        }
                    ]
                }
            }]
        }),
        &request,
    )
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "custom_tool_call");
    assert_eq!(converted["output"][0]["name"], "exec");
    assert_eq!(converted["output"][0]["input"], "ls -la");
    assert_eq!(converted["output"][1]["type"], "function_call");
    assert_eq!(converted["output"][1]["name"], "open_file");
    assert_eq!(converted["output"][1]["namespace"], "mcp__vscode_mcp__");
}

#[test]
fn chat_completion_response_reconstructs_apply_patch_proxy_call() {
    let converted = chat_completion_to_response_with_request(
        json!({
            "id": "chatcmpl_patch",
            "created": 123,
            "model": "gpt-5-mini",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_patch",
                        "type": "function",
                        "function": {
                            "name": "apply_patch_add_file",
                            "arguments": "{\"path\":\"README.md\",\"content\":\"hello\"}"
                        }
                    }]
                }
            }]
        }),
        &json!({
            "model": "gpt-5-mini",
            "tools": [{ "type": "custom", "name": "apply_patch" }]
        }),
    )
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "custom_tool_call");
    assert_eq!(converted["output"][0]["name"], "apply_patch");
    assert_eq!(
        converted["output"][0]["input"],
        "*** Begin Patch\n*** Add File: README.md\n+hello\n*** End Patch"
    );
}

#[test]
fn chat_completion_response_remaps_string_apply_patch_proxy_tools() {
    let converted = chat_completion_to_response_with_request(
        json!({
            "id": "chatcmpl_patch_string_tool",
            "created": 123,
            "model": "gpt-5-mini",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call_patch",
                        "type": "function",
                        "function": {
                            "name": "apply_patch_add_file",
                            "arguments": "{\"path\":\"docs/test.md\",\"content\":\"# Test\\n\"}"
                        }
                    }]
                }
            }]
        }),
        &json!({
            "model": "gpt-5-mini",
            "tools": ["apply_patch_add_file", "apply_patch_batch"]
        }),
    )
    .unwrap();

    assert_eq!(converted["output"][0]["type"], "custom_tool_call");
    assert_eq!(converted["output"][0]["name"], "apply_patch");
    assert_eq!(
        converted["output"][0]["input"],
        "*** Begin Patch\n*** Add File: docs/test.md\n+# Test\n*** End Patch"
    );
}

#[test]
fn chat_completion_response_maps_gemini_and_claude_cache_usage_like_ccx() {
    let gemini = chat_completion_to_response(json!({
        "id": "chatcmpl_gemini_usage",
        "created": 123,
        "model": "gemini-proxy",
        "choices": [{ "finish_reason": "stop", "message": { "role": "assistant", "content": "ok" } }],
        "usage": {
            "promptTokenCount": 20,
            "cachedContentTokenCount": 5,
            "candidatesTokenCount": 7
        }
    }))
    .unwrap();
    assert_eq!(gemini["usage"]["input_tokens"], 15);
    assert_eq!(gemini["usage"]["output_tokens"], 7);
    assert_eq!(gemini["usage"]["total_tokens"], 27);
    assert_eq!(gemini["usage"]["input_tokens_details"]["cached_tokens"], 5);

    let claude = chat_completion_to_response(json!({
        "id": "chatcmpl_claude_usage",
        "created": 123,
        "model": "claude-proxy",
        "choices": [{ "finish_reason": "stop", "message": { "role": "assistant", "content": "ok" } }],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 3,
            "cache_read_input_tokens": 2,
            "cache_creation_5m_input_tokens": 4,
            "cache_creation_1h_input_tokens": 6
        }
    }))
    .unwrap();
    assert_eq!(claude["usage"]["input_tokens"], 10);
    assert_eq!(claude["usage"]["total_tokens"], 25);
    assert_eq!(claude["usage"]["cache_read_input_tokens"], 2);
    assert_eq!(claude["usage"]["cache_creation_5m_input_tokens"], 4);
    assert_eq!(claude["usage"]["cache_creation_1h_input_tokens"], 6);
    assert_eq!(claude["usage"]["cache_ttl"], "mixed");
    assert!(claude["usage"].get("input_tokens_details").is_none());
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
fn chat_sse_maps_custom_tool_call_with_request_context() {
    let converted = chat_sse_to_responses_sse_with_request(
        r#"data: {"id":"chatcmpl_custom","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_custom","type":"function","function":{"name":"exec"}}]}}]}

data: {"id":"chatcmpl_custom","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"input\":"}}]}}]}

data: {"id":"chatcmpl_custom","model":"gpt-5.4","choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"ls -la\"}"}}]},"finish_reason":"tool_calls"}]}

data: [DONE]

"#,
        &json!({
            "model": "gpt-5.4",
            "tools": [{ "type": "custom", "name": "exec" }]
        }),
    );

    assert!(converted.contains("response.custom_tool_call_input.delta"));
    assert_eq!(
        converted
            .matches("event: response.custom_tool_call_input.delta")
            .count(),
        1
    );
    assert!(converted.contains("\"type\":\"custom_tool_call\""));
    assert!(converted.contains("\"name\":\"exec\""));
    assert!(converted.contains("\"input\":\"ls -la\""));
    assert!(converted.contains("data: [DONE]"));
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

#[test]
fn upstream_header_timeout_is_bounded_for_hung_providers() {
    assert!(upstream_header_timeout() >= Duration::from_secs(30));
    assert!(upstream_header_timeout() <= Duration::from_secs(60));
    assert!(upstream_stream_header_timeout() >= Duration::from_secs(120));
}

#[tokio::test]
async fn upstream_request_returns_when_provider_accepts_but_never_sends_headers() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let Ok((_stream, _addr)) = listener.accept().await else {
            return;
        };
        tokio::time::sleep(Duration::from_secs(2)).await;
    });

    let started = Instant::now();
    let result = send_upstream_request_with_header_timeout(
        upstream_http_client()
            .unwrap()
            .get(format!("http://{addr}/v1/models")),
        Duration::from_millis(100),
    )
    .await;

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
}

#[tokio::test]
async fn aggregate_proxy_fails_over_to_next_member_in_same_request() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let first = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let first_addr = first.local_addr().unwrap();
    let second = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let second_addr = second.local_addr().unwrap();
    let first_server = tokio::spawn(respond_once(
        first,
        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 11\r\ncontent-type: application/json\r\n\r\n{\"error\":1}",
    ));
    let second_server = tokio::spawn(respond_once(
        second,
        "HTTP/1.1 200 OK\r\ncontent-length: 35\r\ncontent-type: application/json\r\n\r\n{\"id\":\"resp_1\",\"object\":\"response\"}",
    ));
    let settings = aggregate_proxy_settings(
        "failover",
        format!("http://{first_addr}/v1"),
        format!("http://{second_addr}/v1"),
    );

    let result = open_responses_proxy_request_with_settings(
        r#"{"model":"gpt-5-mini","input":"hi","stream":false}"#,
        settings,
    )
    .await
    .unwrap();
    let body = result.response.bytes().await.unwrap();

    assert_eq!(result.status_code, 200);
    assert_eq!(body.as_ref(), br#"{"id":"resp_1","object":"response"}"#);
    first_server.await.unwrap();
    second_server.await.unwrap();
}

#[tokio::test]
async fn aggregate_stream_request_sends_sse_accept_header() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let fallback = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let fallback_addr = fallback.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let read = stream.read(&mut buffer).await.unwrap();
        let request = String::from_utf8_lossy(&buffer[..read]).to_string();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-length: 14\r\ncontent-type: text/event-stream\r\n\r\ndata: [DONE]\n\n",
            )
            .await
            .unwrap();
        request
    });
    let fallback_server = tokio::spawn(respond_once(
        fallback,
        "HTTP/1.1 200 OK\r\ncontent-length: 14\r\ncontent-type: text/event-stream\r\n\r\ndata: [DONE]\n\n",
    ));
    let settings = aggregate_proxy_settings(
        "stream",
        format!("http://{addr}/v1"),
        format!("http://{fallback_addr}/v1"),
    );

    let result = open_responses_proxy_request_with_settings(
        r#"{"model":"gpt-5-mini","input":"hi","stream":true}"#,
        settings,
    )
    .await
    .unwrap();
    let request = server.await.unwrap();

    assert_eq!(result.status_code, 200);
    assert!(result.is_stream);
    assert!(
        request
            .to_ascii_lowercase()
            .contains("accept: text/event-stream")
    );
    fallback_server.abort();
}

async fn respond_once(listener: tokio::net::TcpListener, response: &'static str) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buffer = [0; 1024];
    let _ = stream.read(&mut buffer).await.unwrap();
    stream.write_all(response.as_bytes()).await.unwrap();
}

fn aggregate_proxy_settings(
    id_suffix: &str,
    first_base_url: String,
    second_base_url: String,
) -> BackendSettings {
    let first_id = format!("proxy-{id_suffix}-a");
    let second_id = format!("proxy-{id_suffix}-b");
    let aggregate_id = format!("proxy-{id_suffix}-agg");
    BackendSettings {
        relay_profiles: vec![
            RelayProfile {
                id: first_id.clone(),
                name: "first".to_string(),
                base_url: first_base_url,
                api_key: "sk-first".to_string(),
                ..RelayProfile::default()
            },
            RelayProfile {
                id: second_id.clone(),
                name: "second".to_string(),
                base_url: second_base_url,
                api_key: "sk-second".to_string(),
                ..RelayProfile::default()
            },
            RelayProfile {
                id: aggregate_id.clone(),
                name: "aggregate".to_string(),
                relay_mode: RelayMode::Aggregate,
                ..RelayProfile::default()
            },
        ],
        active_relay_id: aggregate_id.clone(),
        active_aggregate_relay_id: aggregate_id.clone(),
        aggregate_relay_profiles: vec![AggregateRelayProfile {
            id: aggregate_id,
            name: "aggregate".to_string(),
            strategy: AggregateRelayStrategy::RequestRoundRobin,
            members: vec![
                AggregateRelayMember {
                    relay_id: first_id,
                    weight: 1,
                },
                AggregateRelayMember {
                    relay_id: second_id,
                    weight: 1,
                },
            ],
        }],
        ..BackendSettings::default()
    }
}
#[tokio::test]
async fn chat_completions_proxy_uses_configured_user_agent() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _guard = SettingsPathGuard::set(temp.path().join("settings.json"));
    let server = spawn_chat_server();
    write_chat_relay_settings(temp.path(), &server.base_url, "Configured-Codex-UA/1.0");

    let upstream = open_chat_completions_proxy_request(
        r#"{"model":"gpt-5.5","messages":[{"role":"user","content":"hello"}]}"#,
        Some("Original-Codex-UA/1.0"),
    )
    .await
    .unwrap();
    assert_eq!(upstream.status_code, 200);

    let request = server.finish();
    assert_eq!(request.user_agent, "Configured-Codex-UA/1.0");
}

#[tokio::test]
async fn chat_completions_proxy_passes_through_original_user_agent_when_unconfigured() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _guard = SettingsPathGuard::set(temp.path().join("settings.json"));
    let server = spawn_chat_server();
    write_chat_relay_settings(temp.path(), &server.base_url, "");

    let upstream = open_chat_completions_proxy_request(
        r#"{"model":"gpt-5.5","messages":[{"role":"user","content":"hello"}]}"#,
        Some("Original-Codex-UA/1.0"),
    )
    .await
    .unwrap();
    assert_eq!(upstream.status_code, 200);

    let request = server.finish();
    assert_eq!(request.user_agent, "Original-Codex-UA/1.0");
}

#[tokio::test]
async fn responses_proxy_passes_through_original_user_agent_when_unconfigured() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _guard = SettingsPathGuard::set(temp.path().join("settings.json"));
    let server = spawn_chat_server();
    write_chat_relay_settings(temp.path(), &server.base_url, "");

    let upstream = open_responses_proxy_request(
        r#"{"model":"gpt-5.5","input":"hello","stream":false}"#,
        Some("Original-Codex-UA/1.0"),
    )
    .await
    .unwrap();
    assert_eq!(upstream.status_code, 200);

    let request = server.finish();
    assert_eq!(request.user_agent, "Original-Codex-UA/1.0");
}

#[tokio::test]
async fn models_proxy_passes_through_original_user_agent_when_unconfigured() {
    let _lock = settings_path_test_lock().lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let _guard = SettingsPathGuard::set(temp.path().join("settings.json"));
    let server = spawn_chat_server();
    write_chat_relay_settings(temp.path(), &server.base_url, "");

    let upstream = open_models_proxy_request(Some("Original-Codex-UA/1.0"))
        .await
        .unwrap();
    assert_eq!(upstream.status_code, 200);

    let request = server.finish();
    assert_eq!(request.user_agent, "Original-Codex-UA/1.0");
}

fn write_chat_relay_settings(settings_dir: &Path, base_url: &str, user_agent: &str) {
    let settings = json!({
        "relayProfiles": [{
            "id": "chat",
            "name": "Chat",
            "baseUrl": base_url,
            "upstreamBaseUrl": base_url,
            "apiKey": "sk-test",
            "protocol": "chatCompletions",
            "relayMode": "mixedApi",
            "userAgent": user_agent
        }],
        "activeRelayId": "chat"
    });
    std::fs::write(
        settings_dir.join("settings.json"),
        serde_json::to_vec_pretty(&settings).unwrap(),
    )
    .unwrap();
}

struct SettingsPathGuard {
    previous: Option<PathBuf>,
}

fn settings_path_test_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

impl SettingsPathGuard {
    fn set(path: PathBuf) -> Self {
        let previous = codex_plus_core::paths::set_settings_path_for_tests(Some(path));
        Self { previous }
    }
}

impl Drop for SettingsPathGuard {
    fn drop(&mut self) {
        codex_plus_core::paths::set_settings_path_for_tests(self.previous.take());
    }
}

struct ChatServer {
    base_url: String,
    handle: thread::JoinHandle<ChatRequest>,
}

impl ChatServer {
    fn finish(self) -> ChatRequest {
        self.handle.join().unwrap()
    }
}

struct ChatRequest {
    user_agent: String,
}

fn spawn_chat_server() -> ChatServer {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let base_url = format!("http://{address}/v1");
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < std::time::Duration::from_secs(5),
                        "test upstream did not receive a request"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("failed to accept test request: {error}"),
            }
        };
        let mut buffer = [0u8; 4096];
        let bytes = loop {
            match stream.read(&mut buffer) {
                Ok(0) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Ok(bytes) => break bytes,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("failed to read test request: {error}"),
            }
        };
        let request = String::from_utf8_lossy(&buffer[..bytes]).to_string();
        let user_agent = request
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("user-agent")
                        .then(|| value.trim().to_string())
                })
            })
            .unwrap_or_default();
        let body = r#"{"id":"chatcmpl-test","object":"chat.completion","choices":[]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).unwrap();
        ChatRequest { user_agent }
    });
    ChatServer { base_url, handle }
}
