//! 发往 xAI Responses 接口（api.x.ai 与 SuperGrok 订阅代理）前的请求整理。
//!
//! xAI 的 Responses 只认自己那套工具类型和字段，Codex 的请求里有不少它会整包拒绝的东西：
//! custom 工具（apply_patch / exec）、namespace（MCP 工具分组）、客户端执行的 tool_search、
//! 根不是 object 的函数参数、`required: null`、带 `content: null` 的 reasoning、
//! agent_message 等。这里一次整理干净，并记下工具改写（[`ToolAdaptation`]），
//! 响应侧由 `responses_exec` / `responses_exec_sse` 按它还原成 Codex 认的形式。

use std::collections::BTreeSet;

use anyhow::Result;
use serde_json::{Map, Value, json};

use crate::responses_exec::{
    TOOL_SEARCH_NAME, ToolAdaptation, adapt_request_for, flat_tool_name,
    tool_adaptation_without_custom_types,
};

/// xAI Responses 接受的工具类型，取自它 422 报错里列出的 expected 列表。
const XAI_TOOL_TYPES: &[&str] = &[
    "function",
    "web_search",
    "x_search",
    "image_generation",
    "collections_search",
    "file_search",
    "code_execution",
    "code_interpreter",
    "mcp",
    "shell",
    "tool_search",
];

/// xAI 的 `include` 只认这一项。
const XAI_INCLUDE: &str = "reasoning.encrypted_content";

/// 联合类型嵌套时最多往里折叠几层。
const MAX_UNION_DEPTH: usize = 8;

/// SuperGrok 订阅代理按 `x-grok-conv-id` 认出同一段对话（命中缓存），
/// 官方 CLI 取请求里的 `prompt_cache_key`。api.x.ai 不加。
pub(crate) fn with_conversation_header(
    mut relay: crate::settings::RelayProfile,
    body: &Value,
) -> crate::settings::RelayProfile {
    const HEADER: &str = "x-grok-conv-id";
    let subscription_proxy = reqwest::Url::parse(relay.base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "grok.com" || host.ends_with(".grok.com"));
    let Some(key) = body
        .get("prompt_cache_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|key| !key.is_empty())
    else {
        return relay;
    };
    if subscription_proxy
        && !relay
            .custom_headers
            .iter()
            .any(|header| header.key.trim().eq_ignore_ascii_case(HEADER))
    {
        relay
            .custom_headers
            .push(crate::settings::RelayHeaderKeyValue {
                key: HEADER.to_string(),
                value: key.to_string(),
            });
    }
    relay
}

/// 整理请求体，返回整理后的请求和响应还原需要的工具改写记录。
pub(crate) fn shape_request(mut body: Value) -> Result<(Value, ToolAdaptation)> {
    let tool_search = tool_search_as_function(&mut body);
    let mut adaptation = tool_adaptation_without_custom_types(&body);
    adaptation.tool_search = tool_search;
    let mut body = adapt_request_for(body, &adaptation)?;
    let dropped = tidy_tools(&mut body);
    if !dropped.is_empty() {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "xai_responses.unsupported_tools_dropped",
            json!({ "toolTypes": dropped }),
        );
    }
    tidy_tool_choice(&mut body);
    tidy_include(&mut body);
    tidy_input(&mut body);
    Ok((body, adaptation))
}

/// 客户端执行的 tool_search（Codex 自己检索 MCP / 子代理工具）xAI 不支持，改成同名 function；
/// 历史里的检索调用和结果改成 function_call / function_call_output，检索到的工具补进 `tools`。
/// 本轮没声明检索时历史照样要改写（工具检索完或压缩请求不带工具），只是不补工具。
/// 返回本轮是否以 function 形式提供了检索。
fn tool_search_as_function(body: &mut Value) -> bool {
    let mut search = false;
    if let Some(Value::Array(tools)) = body.get_mut("tools") {
        for tool in tools.iter_mut() {
            if tool.get("type").and_then(Value::as_str) == Some(TOOL_SEARCH_NAME)
                && tool.get("execution").and_then(Value::as_str) == Some("client")
            {
                let mut function = Map::new();
                function.insert("type".to_string(), json!("function"));
                function.insert("name".to_string(), json!(TOOL_SEARCH_NAME));
                for key in ["description", "parameters"] {
                    if let Some(value) = tool.get(key) {
                        function.insert(key.to_string(), value.clone());
                    }
                }
                *tool = Value::Object(function);
                search = true;
            }
        }
    }
    let mut found = Vec::new();
    if let Some(Value::Array(items)) = body.get_mut("input") {
        for item in items.iter_mut() {
            match item.get("type").and_then(Value::as_str) {
                Some("tool_search_call") => {
                    let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    let arguments = match arguments {
                        Value::String(text) => text,
                        other => other.to_string(),
                    };
                    *item = json!({
                        "type": "function_call",
                        "call_id": item.get("call_id").cloned().unwrap_or(Value::Null),
                        "name": TOOL_SEARCH_NAME,
                        "arguments": arguments,
                    });
                }
                Some("tool_search_output") => {
                    let mut tools = item
                        .get("tools")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    tools.iter_mut().for_each(undefer);
                    let summary = search_found(&tools);
                    found.extend(tools);
                    *item = json!({
                        "type": "function_call_output",
                        "call_id": item.get("call_id").cloned().unwrap_or(Value::Null),
                        "output": summary,
                    });
                }
                _ => {}
            }
        }
    }
    if search && let Some(Value::Array(tools)) = body.get_mut("tools") {
        for tool in found {
            add_tool(tools, tool);
        }
    }
    search
}

/// 去掉检索到的工具及其 namespace 子工具上的 `defer_loading`。
fn undefer(tool: &mut Value) {
    if let Some(object) = tool.as_object_mut() {
        object.remove("defer_loading");
    }
    if let Some(Value::Array(children)) = tool.get_mut("tools") {
        for child in children.iter_mut().filter_map(Value::as_object_mut) {
            child.remove("defer_loading");
        }
    }
}

/// 检索结果给模型看的文字：现在能调的工具，按它在请求里的名字列出。
fn search_found(tools: &[Value]) -> String {
    let mut names = Vec::new();
    for tool in tools {
        let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        match tool.get("type").and_then(Value::as_str) {
            Some("function") if !name.is_empty() => names.push(name.to_owned()),
            Some("namespace") => {
                for child in tool
                    .get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let child_name = child
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if child.get("type").and_then(Value::as_str) == Some("function")
                        && !child_name.is_empty()
                    {
                        names.push(flat_tool_name(name, child_name));
                    }
                }
            }
            _ => {}
        }
    }
    if names.is_empty() {
        "No tools matched the search.".to_string()
    } else {
        format!(
            "These tools are now available to call: {}",
            names.join(", ")
        )
    }
}

/// 同名工具只加一次；同名 namespace 再出现时补上它缺的子工具。
fn add_tool(tools: &mut Vec<Value>, tool: Value) {
    let key = tool_key(&tool);
    let Some(existing) = tools.iter_mut().find(|existing| tool_key(existing) == key) else {
        tools.push(tool);
        return;
    };
    let both_namespaces = [&*existing, &tool]
        .iter()
        .all(|value| value.get("type").and_then(Value::as_str) == Some("namespace"));
    if !both_namespaces {
        return;
    }
    let more = tool
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(Value::Array(children)) = existing.get_mut("tools") {
        for child in more {
            let name = child.get("name");
            if !children.iter().any(|known| known.get("name") == name) {
                children.push(child);
            }
        }
    }
}

fn tool_key(tool: &Value) -> String {
    match tool.get("name").and_then(Value::as_str) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => format!(
            "#{}",
            tool.get("type").and_then(Value::as_str).unwrap_or_default()
        ),
    }
}

/// 工具声明整理：去掉 xAI 不认的类型、web_search 的 `external_web_access`，
/// 函数参数根折叠成普通 object，删掉 schema 里的 `required: null`。返回去掉的类型。
fn tidy_tools(body: &mut Value) -> Vec<String> {
    let Some(Value::Array(tools)) = body.get_mut("tools") else {
        return Vec::new();
    };
    let mut dropped = Vec::new();
    tools.retain(|tool| {
        let kind = tool.get("type").and_then(Value::as_str).unwrap_or_default();
        let supported = XAI_TOOL_TYPES.contains(&kind);
        if !supported {
            dropped.push(kind.to_string());
        }
        supported
    });
    for tool in tools.iter_mut() {
        let Some(object) = tool.as_object_mut() else {
            continue;
        };
        object.remove("external_web_access");
        if let Some(Value::Object(parameters)) = object.get_mut("parameters") {
            fold_object_root(parameters);
        }
        if let Some(parameters) = object.get_mut("parameters") {
            drop_null_required(parameters);
        }
    }
    dropped
}

/// tool_choice 指向 xAI 不认的类型，或请求里已经没有工具时，整个去掉（否则 400）。
fn tidy_tool_choice(body: &mut Value) {
    let no_tools = body
        .get("tools")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty);
    let unsupported = body
        .get("tool_choice")
        .and_then(|choice| choice.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|kind| !XAI_TOOL_TYPES.contains(&kind));
    if (no_tools || unsupported)
        && let Some(object) = body.as_object_mut()
    {
        object.remove("tool_choice");
    }
}

/// `include` 只保留 xAI 支持的 `reasoning.encrypted_content`（跨轮携带加密推理要用），
/// 其余（如 `output.encrypted_content`）会让整个请求 400；剩不下就整个去掉。
fn tidy_include(body: &mut Value) {
    let Some(object) = body.as_object_mut() else {
        return;
    };
    let kept = object
        .get("include")
        .and_then(Value::as_array)
        .map(|include| {
            include
                .iter()
                .filter(|value| value.as_str() == Some(XAI_INCLUDE))
                .cloned()
                .collect::<Vec<_>>()
        });
    match kept {
        Some(kept) if !kept.is_empty() => {
            object.insert("include".to_string(), Value::Array(kept));
        }
        _ => {
            object.remove("include");
        }
    }
}

/// 历史整理：reasoning 的 `content: null` 会让 xAI 解不开同条里的加密推理（第二轮起 400），
/// 去掉；agent_message（子代理的任务或回复）xAI 不认，改成 user 消息。
fn tidy_input(body: &mut Value) {
    let Some(Value::Array(items)) = body.get_mut("input") else {
        return;
    };
    for item in items.iter_mut() {
        if item.get("type").and_then(Value::as_str) == Some("reasoning")
            && item.get("content").is_some_and(Value::is_null)
            && let Some(object) = item.as_object_mut()
        {
            object.remove("content");
        }
        if let Some(message) = agent_message_as_user(item) {
            *item = message;
        }
    }
}

/// 明文的 agent_message 改成 user 消息，开头写明谁发给谁。
/// 加密的、空的或含其他类型内容的保持原样。
fn agent_message_as_user(item: &Value) -> Option<Value> {
    if item.get("type").and_then(Value::as_str) != Some("agent_message")
        || item.get("encrypted_content").is_some()
    {
        return None;
    }
    let parts = item.get("content").and_then(Value::as_array)?;
    if parts.is_empty() {
        return None;
    }
    let plain = parts.iter().all(|part| {
        part.get("encrypted_content").is_none()
            && match part.get("type").and_then(Value::as_str) {
                Some("input_text") => part.get("text").is_some_and(Value::is_string),
                Some("input_image") => part.get("image_url").is_some_and(Value::is_string),
                _ => false,
            }
    });
    if !plain {
        return None;
    }
    let from = item
        .get("author")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let to = item
        .get("recipient")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let header = match (from.is_empty(), to.is_empty()) {
        (false, false) => format!("From {from} to {to}"),
        (false, true) => format!("From {from}"),
        (true, false) => format!("To {to}"),
        (true, true) => String::new(),
    };
    let mut content = Vec::with_capacity(parts.len() + 1);
    if !header.is_empty() {
        content.push(json!({ "type": "input_text", "text": format!("{header}\n\n") }));
    }
    content.extend(parts.iter().cloned());
    Some(json!({ "type": "message", "role": "user", "content": content }))
}

/// xAI 要求函数参数根是不带 anyOf / oneOf / allOf 的普通 object
/// （Codex 桌面版的 automation_update 就是对象的 anyOf）。
/// allOf 各支全部合并；anyOf / oneOf 合并各对象分支的属性，各分支都必填的字段保持必填。
/// 分支本身是联合时先折叠，同一字段在不同分支 schema 不同时取它们的 anyOf。
fn fold_object_root(schema: &mut Map<String, Value>) -> bool {
    let root = Value::Object(schema.clone());
    fold_object_root_in(&root, schema, 0)
}

fn fold_object_root_in(root: &Value, schema: &mut Map<String, Value>, depth: usize) -> bool {
    let is_union = ["anyOf", "oneOf", "allOf"]
        .iter()
        .any(|key| schema.contains_key(*key));
    if schema.get("type").and_then(Value::as_str) == Some("object") && !is_union {
        return false;
    }
    let mut properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut required = schema
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if let Some(Value::Array(branches)) = schema.remove("allOf") {
        for branch in branches
            .iter()
            .filter_map(|branch| resolve_ref(root, branch))
        {
            for (key, value) in branch
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                properties
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            required.extend(
                branch
                    .get("required")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
        }
    }
    let mut branches = Vec::new();
    for key in ["anyOf", "oneOf"] {
        let Some(Value::Array(list)) = schema.remove(key) else {
            continue;
        };
        for branch in &list {
            let Some(mut branch) = resolve_ref(root, branch) else {
                continue;
            };
            let nested = ["anyOf", "oneOf", "allOf"]
                .iter()
                .any(|key| branch.contains_key(*key));
            if nested && depth < MAX_UNION_DEPTH {
                fold_object_root_in(root, &mut branch, depth + 1);
            }
            if branch.get("type").and_then(Value::as_str) != Some("object")
                && !branch.contains_key("properties")
            {
                continue;
            }
            branches.push(branch);
        }
    }
    // 各分支都必填的字段；以及根上没有、由分支带来的字段的各种 schema
    let mut common: Vec<Value> = Vec::new();
    let mut from_branches: Vec<(String, Vec<Value>)> = Vec::new();
    for (index, branch) in branches.iter().enumerate() {
        if let Some(branch_properties) = branch.get("properties").and_then(Value::as_object) {
            let mut keys = branch_properties.keys().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if properties.contains_key(key) {
                    continue;
                }
                let value = &branch_properties[key];
                match from_branches.iter_mut().find(|(name, _)| name == key) {
                    Some((_, values)) => {
                        if !values.contains(value) {
                            values.push(value.clone());
                        }
                    }
                    None => from_branches.push((key.clone(), vec![value.clone()])),
                }
            }
        }
        let branch_required = branch
            .get("required")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if index == 0 {
            common = branch_required;
        } else {
            common.retain(|name| branch_required.contains(name));
        }
    }
    for (key, mut values) in from_branches {
        let value = if values.len() == 1 {
            values.remove(0)
        } else {
            json!({ "anyOf": values })
        };
        properties.insert(key, value);
    }
    let mut seen = BTreeSet::new();
    let required = required
        .into_iter()
        .chain(common)
        .filter(|name| {
            name.as_str()
                .is_none_or(|name| seen.insert(name.to_owned()))
        })
        .collect::<Vec<_>>();
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), Value::Object(properties));
    if required.is_empty() {
        schema.remove("required");
    } else {
        schema.insert("required".to_string(), Value::Array(required));
    }
    true
}

/// 分支本身，或它的本地 `$ref` 指向根 `$defs` / `definitions` 里的定义。
fn resolve_ref(root: &Value, branch: &Value) -> Option<Map<String, Value>> {
    let branch = branch.as_object()?;
    let Some(reference) = branch.get("$ref").and_then(Value::as_str) else {
        return Some(branch.clone());
    };
    ["$defs", "definitions"].iter().find_map(|key| {
        let name = reference.strip_prefix(&format!("#/{key}/"))?;
        root.get(*key)?.get(name)?.as_object().cloned()
    })
}

/// 递归删掉 schema 里的 `"required": null`（xAI 按 schema 校验，要求它是数组）。
fn drop_null_required(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if object.get("required").is_some_and(Value::is_null) {
                object.remove("required");
            }
            object.values_mut().for_each(drop_null_required);
        }
        Value::Array(items) => items.iter_mut().for_each(drop_null_required),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::responses_exec::adapt_response_for;

    #[test]
    fn client_tool_search_becomes_a_function_and_its_history_too() {
        let request = json!({
            "tools": [
                {"type": "function", "name": "shell", "parameters": {"type": "object"}},
                {"type": "tool_search", "execution": "client", "description": "find tools",
                 "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}}
            ],
            "input": [
                {"type": "tool_search_call", "id": "tsc_1", "call_id": "s1",
                 "execution": "client", "arguments": {"query": "files"}},
                {"type": "tool_search_output", "call_id": "s1", "tools": [
                    {"type": "namespace", "name": "mcp__fs", "tools": [
                        {"type": "function", "name": "read", "defer_loading": true,
                         "parameters": {"type": "object"}}
                    ]}
                ]}
            ]
        });
        let (shaped, adaptation) = shape_request(request).unwrap();
        assert!(adaptation.tool_search);
        let tools = shaped["tools"].as_array().unwrap();
        assert_eq!(
            tools
                .iter()
                .map(|tool| (
                    tool["type"].as_str().unwrap(),
                    tool["name"].as_str().unwrap()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("function", "shell"),
                ("function", "tool_search"),
                ("function", "mcp__fs__read")
            ],
            "检索到的 namespace 工具补进 tools 并展开"
        );
        assert!(tools[2].get("defer_loading").is_none());
        assert_eq!(
            shaped["input"][0],
            json!({"type": "function_call", "call_id": "s1", "name": "tool_search",
                   "arguments": "{\"query\":\"files\"}"})
        );
        assert_eq!(
            shaped["input"][1],
            json!({"type": "function_call_output", "call_id": "s1",
                   "output": "These tools are now available to call: mcp__fs__read"})
        );

        let response = adapt_response_for(
            json!({"status": "completed", "output": [
                {"type": "function_call", "id": "fc_9", "call_id": "s2",
                 "name": "tool_search", "arguments": "{\"query\":\"git\"}"},
                {"type": "function_call", "id": "fc_8", "call_id": "c8",
                 "name": "mcp__fs__read", "arguments": "{}"}
            ]}),
            &adaptation,
        )
        .unwrap();
        assert_eq!(
            response["output"][0],
            json!({"type": "tool_search_call", "id": "fc_9", "call_id": "s2",
                   "execution": "client", "arguments": {"query": "git"}})
        );
        assert_eq!(response["output"][1]["name"], "read");
        assert_eq!(response["output"][1]["namespace"], "mcp__fs");
        assert_eq!(response["output"][1]["encrypted_function_args"], json!([]));
    }

    #[test]
    fn history_search_without_an_offered_search_is_rewritten_but_adds_nothing() {
        let request = json!({
            "tools": [],
            "tool_choice": "auto",
            "input": [
                {"type": "tool_search_call", "call_id": "s1", "arguments": {"query": "x"}},
                {"type": "tool_search_output", "call_id": "s1", "tools": []}
            ]
        });
        let (shaped, adaptation) = shape_request(request).unwrap();
        assert!(!adaptation.tool_search);
        assert_eq!(shaped["tools"], json!([]));
        assert!(
            shaped.get("tool_choice").is_none(),
            "没有工具时去掉 tool_choice"
        );
        assert_eq!(shaped["input"][0]["type"], "function_call");
        assert_eq!(shaped["input"][1]["output"], "No tools matched the search.");
    }

    #[test]
    fn tools_and_history_are_tidied_for_xai() {
        let request = json!({
            "tools": [
                {"type": "web_search", "external_web_access": true},
                {"type": "local_shell"},
                {"type": "function", "name": "automation_update", "parameters": {
                    "anyOf": [
                        {"type": "object", "properties": {"id": {"type": "string"},
                         "mode": {"const": "a"}}, "required": ["id", "mode"]},
                        {"$ref": "#/$defs/b"}
                    ],
                    "$defs": {"b": {"type": "object", "properties": {"id": {"type": "string"},
                     "mode": {"const": "b"}, "extra": {"type": "integer"}}, "required": ["id"]}}
                }},
                {"type": "function", "name": "lookup", "parameters": {
                    "type": "object", "required": null,
                    "properties": {"q": {"type": "object", "required": null}}
                }}
            ],
            "tool_choice": {"type": "custom", "name": "apply_patch"},
            "input": [
                {"type": "reasoning", "id": "rs_1", "content": null,
                 "encrypted_content": "sealed", "summary": []},
                {"type": "agent_message", "author": "lead", "recipient": "worker",
                 "content": [{"type": "input_text", "text": "do it"}]},
                {"type": "agent_message", "encrypted_content": "sealed", "content": []}
            ]
        });
        let (shaped, _) = shape_request(request).unwrap();
        let tools = shaped["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3, "local_shell 去掉");
        assert_eq!(tools[0], json!({"type": "web_search"}));
        let folded = &tools[1]["parameters"];
        assert_eq!(folded["type"], "object");
        assert!(folded.get("anyOf").is_none());
        assert_eq!(
            folded["required"],
            json!(["id"]),
            "只保留各分支都必填的字段"
        );
        assert_eq!(folded["properties"]["id"], json!({"type": "string"}));
        assert_eq!(
            folded["properties"]["mode"],
            json!({"anyOf": [{"const": "a"}, {"const": "b"}]})
        );
        assert_eq!(folded["properties"]["extra"], json!({"type": "integer"}));
        assert!(tools[2]["parameters"].get("required").is_none());
        assert!(
            tools[2]["parameters"]["properties"]["q"]
                .get("required")
                .is_none()
        );
        assert!(
            shaped.get("tool_choice").is_none(),
            "不认的 tool_choice 类型去掉"
        );

        let input = shaped["input"].as_array().unwrap();
        assert!(input[0].get("content").is_none());
        assert_eq!(input[0]["encrypted_content"], "sealed");
        assert_eq!(
            input[1],
            json!({"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": "From lead to worker\n\n"},
                {"type": "input_text", "text": "do it"}
            ]})
        );
        assert_eq!(input[2]["type"], "agent_message", "加密的保持原样");
    }

    #[test]
    fn subscription_proxy_gets_the_conversation_header() {
        let relay = |base_url: &str| crate::settings::RelayProfile {
            base_url: base_url.to_string(),
            ..Default::default()
        };
        let body = json!({"prompt_cache_key": "conv-1"});
        let headers = with_conversation_header(relay("https://cli-chat-proxy.grok.com/v1"), &body)
            .custom_headers;
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].key, "x-grok-conv-id");
        assert_eq!(headers[0].value, "conv-1");
        assert!(
            with_conversation_header(relay("https://api.x.ai/v1"), &body)
                .custom_headers
                .is_empty()
        );
        assert!(
            with_conversation_header(relay("https://cli-chat-proxy.grok.com/v1"), &json!({}))
                .custom_headers
                .is_empty()
        );
    }

    #[test]
    fn include_keeps_only_reasoning_encrypted_content() {
        let (shaped, _) = shape_request(json!({"include": [
            "reasoning.encrypted_content", "output.encrypted_content", "message.output_text.logprobs"
        ]}))
        .unwrap();
        assert_eq!(shaped["include"], json!(["reasoning.encrypted_content"]));

        let (shaped, _) = shape_request(json!({"include": ["output.encrypted_content"]})).unwrap();
        assert!(shaped.get("include").is_none(), "剩不下就整个去掉");

        let (shaped, _) = shape_request(json!({"include": "reasoning.encrypted_content"})).unwrap();
        assert!(shaped.get("include").is_none(), "不是数组的也去掉");

        let (shaped, _) = shape_request(json!({"model": "grok"})).unwrap();
        assert_eq!(shaped, json!({"model": "grok"}));
    }

    #[test]
    fn long_flat_names_are_cut_with_a_hash_and_lite_namespace_stays_plain() {
        let long = flat_tool_name(
            "mcp__a_really_long_server_name_for_testing",
            "an_even_longer_tool_name",
        );
        assert_eq!(long.len(), 64);
        assert_ne!(
            long,
            flat_tool_name(
                "mcp__a_really_long_server_name_for_testing",
                "an_even_longer_tool_name2"
            )
        );
        assert_eq!(flat_tool_name("functions", "exec_command"), "exec_command");

        let request = json!({
            "tools": [{"type": "namespace", "name": "functions", "tools": [
                {"type": "function", "name": "exec_command", "parameters": {"type": "object"}}
            ]}],
            "input": [{"type": "function_call", "call_id": "c1", "namespace": "functions",
                       "name": "exec_command", "arguments": "{}"}]
        });
        let (shaped, adaptation) = shape_request(request).unwrap();
        assert!(
            adaptation.namespaced.is_empty(),
            "functions 里的调用回来不带 namespace"
        );
        assert_eq!(shaped["tools"][0]["name"], "exec_command");
        assert_eq!(shaped["input"][0]["name"], "exec_command");
        assert!(shaped["input"][0].get("namespace").is_none());
    }
}
