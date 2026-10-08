//! Stateless Responses compatibility transforms for a provider that rejects
//! the native custom `exec` tool.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow, bail, ensure};
use serde::de::{Deserializer, MapAccess, Visitor};
use serde_json::{Map, Value, json};

pub const EXEC_TOOL_NAME: &str = "exec";
pub const EXEC_REJECTION_MESSAGE: &str =
    "Unsupported custom tool: 'exec'. Only 'apply_patch' is supported.";
pub const MAX_REJECTION_BODY_BYTES: usize = 64 * 1024;
pub const MAX_EXEC_ARGUMENT_BYTES: usize = 1024 * 1024;

const CUSTOM_CALL_PREFIX: &str = "ctc_";
const CUSTOM_OUTPUT_PREFIX: &str = "ctco_";
const FUNCTION_CALL_PREFIX: &str = "fc_";
const FUNCTION_OUTPUT_PREFIX: &str = "fco_";

pub(crate) fn eligible(request: &Value, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    if path != "/responses" && !path.ends_with("/responses") {
        return false;
    }
    if request.get("previous_response_id").is_some()
        || request.get("conversation").is_some()
        || request.get("conversation_id").is_some()
        || request_has_compaction_trigger(request)
    {
        return false;
    }
    let Some(tools) = request.get("tools").and_then(Value::as_array) else {
        return false;
    };
    let exec_count = tools
        .iter()
        .filter(|tool| is_custom_exec_tool(tool))
        .count();
    exec_count == 1
        && !tools
            .iter()
            .any(|tool| is_named_function_tool(tool, EXEC_TOOL_NAME))
}

pub(crate) fn should_adapt_request(request: &Value, path: &str) -> bool {
    eligible(request, path)
}

fn request_has_compaction_trigger(request: &Value) -> bool {
    request
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("compaction_trigger") | Some("compaction")
                )
            })
        })
}

fn is_custom_exec_tool(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("custom")
        && tool.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME)
}

fn is_named_function_tool(tool: &Value, name: &str) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("function")
        && tool.get("name").and_then(Value::as_str) == Some(name)
}

pub(crate) fn rejection_matches(
    status: u16,
    content_type: &str,
    body: &[u8],
    request: &Value,
) -> bool {
    if status != 400
        || body.len() > MAX_REJECTION_BODY_BYTES
        || !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return false;
    }
    let Ok(body) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(error) = body.get("error").and_then(Value::as_object) else {
        return false;
    };
    if error.get("type").and_then(Value::as_str) != Some("invalid_request_error")
        || error.get("message").and_then(Value::as_str) != Some(EXEC_REJECTION_MESSAGE)
    {
        return false;
    }
    let Some(tools) = request.get("tools").and_then(Value::as_array) else {
        return false;
    };
    let unambiguous = tools
        .iter()
        .filter(|tool| is_custom_exec_tool(tool))
        .count()
        == 1
        && !tools
            .iter()
            .any(|tool| is_named_function_tool(tool, EXEC_TOOL_NAME));
    if !unambiguous {
        return false;
    }
    match error.get("param") {
        None | Some(Value::Null) => true,
        Some(Value::String(param)) => parse_tools_index(param)
            .and_then(|index| tools.get(index))
            .is_some_and(is_custom_exec_tool),
        _ => false,
    }
}

pub(crate) fn is_explicit_exec_rejection(
    status: u16,
    content_type: &str,
    body: &[u8],
    request: &Value,
) -> bool {
    rejection_matches(status, content_type, body, request)
}

fn parse_tools_index(param: &str) -> Option<usize> {
    let index = param.strip_prefix("tools[")?.strip_suffix(']')?;
    (!index.is_empty() && index.chars().all(|ch| ch.is_ascii_digit()))
        .then(|| index.parse().ok())
        .flatten()
}

pub(crate) fn adapt_request(mut request: Value) -> Result<Value> {
    let Some(tools) = request.get("tools").and_then(Value::as_array).cloned() else {
        return Ok(request);
    };
    let exec_indexes = tools
        .iter()
        .enumerate()
        .filter_map(|(index, tool)| is_custom_exec_tool(tool).then_some(index))
        .collect::<Vec<_>>();
    if exec_indexes.is_empty() {
        return Ok(request);
    }
    ensure!(
        exec_indexes.len() == 1,
        "ambiguous custom exec declarations"
    );
    ensure!(
        !tools
            .iter()
            .any(|tool| is_named_function_tool(tool, EXEC_TOOL_NAME)),
        "custom exec conflicts with a function named exec"
    );
    let exec_index = exec_indexes[0];
    let mut translated_tools = tools;
    translated_tools[exec_index] = function_exec_tool(&translated_tools[exec_index]);
    request["tools"] = Value::Array(translated_tools);

    if let Some(tool_choice) = request.get_mut("tool_choice") {
        if is_custom_exec_tool_choice(tool_choice) {
            *tool_choice = json!({"type": "function", "name": EXEC_TOOL_NAME});
        } else if tool_choice_references_exec(tool_choice) {
            bail!("unsupported tool_choice shape references exec");
        }
    }
    adapt_request_history(&mut request)?;
    Ok(request)
}

fn function_exec_tool(tool: &Value) -> Value {
    let mut translated = Map::new();
    translated.insert("type".to_string(), json!("function"));
    translated.insert("name".to_string(), json!(EXEC_TOOL_NAME));
    if let Some(description) = tool.get("description") {
        translated.insert("description".to_string(), description.clone());
    }
    translated.insert(
        "parameters".to_string(),
        json!({
            "type": "object",
            "properties": {"input": {"type": "string"}},
            "required": ["input"],
            "additionalProperties": false
        }),
    );
    translated.insert("strict".to_string(), json!(true));
    Value::Object(translated)
}

fn is_custom_exec_tool_choice(choice: &Value) -> bool {
    let Some(object) = choice.as_object() else {
        return false;
    };
    object.len() == 2
        && choice.get("type").and_then(Value::as_str) == Some("custom")
        && choice.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME)
}

fn tool_choice_references_exec(choice: &Value) -> bool {
    match choice {
        Value::Object(object) => {
            object
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == EXEC_TOOL_NAME)
                || object.values().any(tool_choice_references_exec)
        }
        Value::Array(items) => items.iter().any(tool_choice_references_exec),
        _ => false,
    }
}

fn adapt_request_history(request: &mut Value) -> Result<()> {
    let Some(Value::Array(items)) = request.get_mut("input") else {
        return Ok(());
    };
    validate_item_ids(items)?;
    let original_ids = collect_item_ids(items);
    let mut all_calls = BTreeSet::new();
    let mut exec_calls = BTreeSet::new();
    let mut output_calls = BTreeSet::new();
    for item in items.iter() {
        match item.get("type").and_then(Value::as_str) {
            Some("custom_tool_call") | Some("function_call") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom tool call lacks call_id"))?;
                ensure!(!call_id.is_empty(), "custom tool call has empty call_id");
                ensure!(
                    all_calls.insert(call_id.to_owned()),
                    "duplicate tool call_id"
                );
                if item.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                    && item.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME)
                {
                    ensure!(
                        exec_calls.insert(call_id.to_owned()),
                        "duplicate exec call_id"
                    );
                }
            }
            Some("custom_tool_call_output") => {
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("custom tool output lacks call_id"))?;
                ensure!(!call_id.is_empty(), "custom tool output has empty call_id");
                ensure!(
                    output_calls.insert(call_id.to_owned()),
                    "duplicate tool output call_id"
                );
            }
            _ => {}
        }
    }
    for item in items.iter_mut() {
        match item.get("type").and_then(Value::as_str) {
            Some("custom_tool_call")
                if item.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME) =>
            {
                let input = item
                    .get("input")
                    .or_else(|| item.get("arguments"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("exec history input must be a string"))?
                    .to_owned();
                ensure!(
                    input.len() <= MAX_EXEC_ARGUMENT_BYTES,
                    "exec input too large"
                );
                let id = translated_item_id(item, FUNCTION_CALL_PREFIX)?;
                item["type"] = json!("function_call");
                item["id"] = json!(id);
                item["name"] = json!(EXEC_TOOL_NAME);
                item["arguments"] = json!(serde_json::to_string(&json!({"input": input}))?);
                item.as_object_mut()
                    .expect("history item remains object")
                    .remove("input");
            }
            Some("custom_tool_call_output") => {
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                ensure!(all_calls.contains(call_id), "orphan custom tool output");
                if exec_calls.contains(call_id) {
                    let id = translated_item_id(item, FUNCTION_OUTPUT_PREFIX)?;
                    item["type"] = json!("function_call_output");
                    item["id"] = json!(id);
                }
            }
            _ => {}
        }
    }
    validate_item_ids(items)?;
    remap_item_references(items, &original_ids)
}

pub(crate) fn adapt_response(mut response: Value) -> Result<Value> {
    let response_status = response
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return Ok(response);
    };
    ensure!(
        !items.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("custom_tool_call")
                && item.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME)
        }),
        "adapted response contains unvalidated native exec input"
    );
    let matching_call_ids = items
        .iter()
        .filter(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call")
                && item.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME)
        })
        .map(|item| {
            item.get("call_id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("exec function_call lacks call_id"))
        })
        .collect::<Result<Vec<_>>>()?;
    if matching_call_ids.is_empty() {
        return Ok(response);
    }
    ensure!(
        response_status
            .as_deref()
            .is_none_or(|status| status == "completed"),
        "exec response did not complete successfully"
    );
    let original_ids = collect_item_ids(items);
    ensure!(
        matching_call_ids.iter().collect::<BTreeSet<_>>().len() == matching_call_ids.len(),
        "duplicate exec call_id"
    );
    let all_call_ids = items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        .filter_map(|item| {
            item.get("call_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    let mut function_call_ids = BTreeSet::new();
    for item in items
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
    {
        if let Some(call_id) = item.get("call_id").and_then(Value::as_str) {
            ensure!(
                function_call_ids.insert(call_id),
                "duplicate function call_id"
            );
        }
    }
    let mut output_call_ids = BTreeSet::new();
    validate_item_ids(items)?;
    for item in items.iter() {
        if item.get("type").and_then(Value::as_str) == Some("function_call_output") {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("function_call_output lacks call_id"))?;
            ensure!(
                all_call_ids.contains(call_id),
                "orphan function_call_output"
            );
            ensure!(
                output_call_ids.insert(call_id.to_owned()),
                "duplicate function output call_id"
            );
        }
    }
    for item in items.iter_mut() {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call")
                if item.get("name").and_then(Value::as_str) == Some(EXEC_TOOL_NAME) =>
            {
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("exec arguments must be string"))?;
                let input = decode_exec_input(arguments)?;
                let id = translated_item_id(item, CUSTOM_CALL_PREFIX)?;
                item["type"] = json!("custom_tool_call");
                item["id"] = json!(id);
                item["name"] = json!(EXEC_TOOL_NAME);
                item["input"] = json!(input);
                item.as_object_mut()
                    .expect("response item remains object")
                    .remove("arguments");
            }
            Some("function_call_output")
                if matching_call_ids.contains(
                    &item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                ) =>
            {
                let id = translated_item_id(item, CUSTOM_OUTPUT_PREFIX)?;
                item["type"] = json!("custom_tool_call_output");
                item["id"] = json!(id);
            }
            _ => {}
        }
    }
    validate_item_ids(items)?;
    remap_item_references(items, &original_ids)?;
    Ok(response)
}

pub(crate) fn decode_exec_input(arguments: &str) -> Result<String> {
    ensure!(
        arguments.len() <= MAX_EXEC_ARGUMENT_BYTES,
        "exec arguments exceed 1 MiB"
    );
    let mut deserializer = serde_json::Deserializer::from_str(arguments);
    let value = deserializer.deserialize_map(ExecArgumentsVisitor)?;
    deserializer
        .end()
        .map_err(|error| anyhow!("invalid exec arguments: {error}"))?;
    Ok(value)
}

struct ExecArgumentsVisitor;

impl<'de> Visitor<'de> for ExecArgumentsVisitor {
    type Value = String;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an object containing exactly one string input property")
    }

    fn visit_map<A>(self, mut map: A) -> std::result::Result<String, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut input = None;
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(serde::de::Error::custom("duplicate exec argument"));
            }
            if key != "input" {
                let _: Value = map.next_value()?;
                return Err(serde::de::Error::custom("unexpected exec argument"));
            }
            let value = map.next_value::<String>()?;
            if value.len() > MAX_EXEC_ARGUMENT_BYTES {
                return Err(serde::de::Error::custom("exec input too large"));
            }
            input = Some(value);
        }
        input.ok_or_else(|| serde::de::Error::custom("missing exec input"))
    }
}

fn validate_item_ids(items: &[Value]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) == Some("item_reference") {
            continue;
        }
        if let Some(id) = item.get("id").and_then(Value::as_str) {
            ensure!(ids.insert(id), "duplicate item id");
        }
    }
    Ok(())
}

fn collect_item_ids(items: &[Value]) -> Vec<Option<String>> {
    items
        .iter()
        .map(|item| {
            (item.get("type").and_then(Value::as_str) != Some("item_reference"))
                .then(|| item.get("id").and_then(Value::as_str).map(str::to_owned))
                .flatten()
        })
        .collect()
}

fn remap_item_references(items: &mut [Value], original_ids: &[Option<String>]) -> Result<()> {
    let mut map = BTreeMap::new();
    for (original_id, item) in original_ids.iter().zip(items.iter()) {
        if let Some(original_id) = original_id {
            let final_id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("item id lost during adaptation"))?;
            map.insert(original_id.clone(), final_id.to_owned());
        }
    }
    for item in items {
        remap_reference_fields(item, &map)?;
    }
    Ok(())
}

fn remap_reference_fields(value: &mut Value, ids: &BTreeMap<String, String>) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    let is_reference = object.get("type").and_then(Value::as_str) == Some("item_reference");
    for (key, value) in object.iter_mut() {
        if key == "item_id" || (is_reference && key == "id") {
            let original_id = value
                .as_str()
                .ok_or_else(|| anyhow!("item reference id must be a string"))?;
            let final_id = ids
                .get(original_id)
                .ok_or_else(|| anyhow!("unresolved item reference"))?;
            *value = json!(final_id);
        }
    }
    Ok(())
}

fn translated_item_id(item: &Value, target_prefix: &str) -> Result<String> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("tool item lacks id"))?;
    let suffix = [
        CUSTOM_OUTPUT_PREFIX,
        CUSTOM_CALL_PREFIX,
        FUNCTION_OUTPUT_PREFIX,
        FUNCTION_CALL_PREFIX,
        "item_",
        "resp_",
        "cp_",
    ]
    .iter()
    .find_map(|prefix| id.strip_prefix(prefix))
    .unwrap_or(id);
    ensure!(!suffix.is_empty(), "tool item id has no stable suffix");
    Ok(format!("{target_prefix}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({
            "input": [
                {"type": "message", "id": "msg_1", "role": "user", "content": "hi"},
                {"type": "custom_tool_call", "id": "ctc_old", "call_id": "old",
                 "name": "exec", "input": "text('old')"},
                {"type": "custom_tool_call_output", "id": "ctco_old",
                 "call_id": "old", "output": "ok"}
            ],
            "tools": [
                {"type": "custom", "name": "exec", "description": "run"},
                {"type": "custom", "name": "apply_patch", "description": "patch"}
            ],
            "tool_choice": {"type": "custom", "name": "exec"},
            "opaque": {"preserve": true}
        })
    }

    #[test]
    fn request_translation_preserves_unrelated_fields_and_apply_patch() {
        let translated = adapt_request(request()).unwrap();
        assert_eq!(translated["opaque"], json!({"preserve": true}));
        assert_eq!(translated["tools"][0]["type"], "function");
        assert!(translated["tools"][0].get("function").is_none());
        assert_eq!(translated["tools"][1]["name"], "apply_patch");
        assert_eq!(
            translated["tool_choice"],
            json!({"type": "function", "name": "exec"})
        );
        assert_eq!(translated["input"][1]["type"], "function_call");
        assert_eq!(translated["input"][1]["id"], "fc_old");
        assert_eq!(
            translated["input"][1]["arguments"],
            json!("{\"input\":\"text('old')\"}")
        );
        assert_eq!(translated["input"][2]["type"], "function_call_output");
        assert_eq!(translated["input"][2]["id"], "fco_old");
    }

    #[test]
    fn response_translation_is_strict_and_keeps_unrelated_calls() {
        let response = json!({
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": []},
                {"type": "function_call", "id": "fc_1", "call_id": "call_1",
                 "name": "exec", "arguments": "{\"input\":\"text('x')\"}"},
                {"type": "function_call_output", "id": "fco_1", "call_id": "call_1",
                 "output": "ok"},
                {"type": "function_call", "id": "fc_2", "call_id": "call_2",
                 "name": "lookup", "arguments": "{\"q\":\"x\"}"}
            ]
        });
        let translated = adapt_response(response).unwrap();
        assert_eq!(translated["output"][1]["type"], "custom_tool_call");
        assert_eq!(translated["output"][1]["id"], "ctc_1");
        assert_eq!(translated["output"][1]["input"], "text('x')");
        assert_eq!(translated["output"][2]["type"], "custom_tool_call_output");
        assert_eq!(translated["output"][2]["id"], "ctco_1");
        assert_eq!(translated["output"][3]["type"], "function_call");
    }

    #[test]
    fn strict_parser_rejects_duplicates_extra_keys_non_strings_and_truncation() {
        for arguments in [
            r#"{"input":"a","input":"b"}"#,
            r#"{"input":"a","extra":true}"#,
            r#"{"input":42}"#,
            r#"{"input":"unterminated""#,
        ] {
            assert!(decode_exec_input(arguments).is_err(), "{arguments}");
        }
        assert_eq!(decode_exec_input(r#"{"input":""}"#).unwrap(), "");
    }

    #[test]
    fn rejection_requires_exact_message_and_declared_tool_index() {
        let request = request();
        let body = json!({
            "error": {
                "type": "invalid_request_error",
                "message": EXEC_REJECTION_MESSAGE,
                "param": "tools[0]"
            }
        })
        .to_string();
        assert!(rejection_matches(
            400,
            "application/json; charset=utf-8",
            body.as_bytes(),
            &request
        ));
        assert!(!rejection_matches(
            400,
            "application/json",
            br#"{"error":{"type":"invalid_request_error","message":"Unsupported tools","param":"tools[0]"}}"#,
            &request
        ));
        assert!(!rejection_matches(
            400,
            "application/json",
            body.as_bytes(),
            &json!({"tools": [{"type": "custom", "name": "apply_patch"}]})
        ));
    }

    #[test]
    fn eligibility_bypasses_stateful_and_compaction_requests() {
        assert!(eligible(&request(), "/v1/responses"));
        assert!(!eligible(&request(), "/v1/responses/compact"));
        let mut stateful = request();
        stateful["previous_response_id"] = json!("resp");
        assert!(!eligible(&stateful, "/v1/responses"));
        let mut compact = request();
        compact["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "compaction_trigger"}));
        assert!(!eligible(&compact, "/v1/responses"));
    }

    #[test]
    fn transformed_ids_cannot_collide() {
        let response = json!({
            "output": [
                {"type": "function_call", "id": "fc_same", "call_id": "exec",
                 "name": "exec", "arguments": "{\"input\":\"x\"}"},
                {"type": "message", "id": "ctc_same", "content": []}
            ]
        });
        assert!(adapt_response(response).is_err());
    }

    #[test]
    fn history_references_follow_item_id_transforms_and_orphans_fail() {
        let mut native = request();
        native["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "item_reference", "id": "ctc_old"}));
        let translated = adapt_request(native).unwrap();
        assert_eq!(translated["input"][3]["id"], "fc_old");
        let mut orphan = request();
        orphan["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "item_reference", "id": "ctc_missing"}));
        assert!(adapt_request(orphan).is_err());
    }

    #[test]
    fn orphan_outputs_and_function_name_conflicts_disable_adaptation() {
        let mut orphan = request();
        orphan["input"][2]["call_id"] = json!("missing");
        assert!(adapt_request(orphan).is_err());
        let mut conflict = request();
        conflict["tools"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "function", "name": "exec"}));
        assert!(!eligible(&conflict, "/v1/responses"));
        assert!(adapt_request(conflict).is_err());
    }

    #[test]
    fn parser_enforces_bounds_and_accepts_escaped_unicode() {
        let oversized = format!("{{\"input\":\"{}\"}}", "x".repeat(MAX_EXEC_ARGUMENT_BYTES));
        assert!(decode_exec_input(&oversized).is_err());
        assert_eq!(
            decode_exec_input(r#"{"input":"a\\b\n\u96ea"}"#).unwrap(),
            "a\\b\n\u{96ea}"
        );
        assert!(decode_exec_input("{}").is_err());
        assert!(decode_exec_input(r#"{"input":"x"} trailing"#).is_err());
    }

    #[test]
    fn compaction_history_and_unknown_exec_choice_disable_adaptation() {
        let mut compact = request();
        compact["input"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "compaction", "id": "cmp_1"}));
        assert!(!eligible(&compact, "/v1/responses"));
        let mut choice = request();
        choice["tool_choice"] = json!({
            "type": "allowed_tools", "tools": [{"type": "custom", "name": "exec"}]
        });
        assert!(adapt_request(choice).is_err());
    }

    #[test]
    fn rejection_bounds_media_type_status_and_nonzero_index_are_enforced() {
        let native = json!({"tools": [
            {"type": "custom", "name": "apply_patch"},
            {"type": "custom", "name": "exec"}
        ]});
        let body = json!({"error": {
            "type": "invalid_request_error", "message": EXEC_REJECTION_MESSAGE,
            "param": "tools[1]"
        }})
        .to_string();
        assert!(rejection_matches(
            400,
            "application/json",
            body.as_bytes(),
            &native
        ));
        assert!(!rejection_matches(
            429,
            "application/json",
            body.as_bytes(),
            &native
        ));
        assert!(!rejection_matches(
            400,
            "text/jsonish",
            body.as_bytes(),
            &native
        ));
        assert!(!rejection_matches(
            400,
            "application/json",
            &vec![b' '; MAX_REJECTION_BODY_BYTES + 1],
            &native
        ));
    }

    #[test]
    fn rejection_without_param_requires_one_unambiguous_exec() {
        let mut error = json!({"error": {
            "type": "invalid_request_error", "message": EXEC_REJECTION_MESSAGE
        }});
        assert!(rejection_matches(
            400,
            "application/json",
            error.to_string().as_bytes(),
            &request()
        ));
        error["error"]["param"] = Value::Null;
        assert!(rejection_matches(
            400,
            "application/json",
            error.to_string().as_bytes(),
            &request()
        ));
        for param in [json!("tools[1]"), json!("tools[0].type"), json!(1)] {
            error["error"]["param"] = param;
            assert!(!rejection_matches(
                400,
                "application/json",
                error.to_string().as_bytes(),
                &request()
            ));
        }
        error["error"]["param"] = Value::Null;
        let mut ambiguous = request();
        ambiguous["tools"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type": "function", "name": "exec"}));
        assert!(!rejection_matches(
            400,
            "application/json",
            error.to_string().as_bytes(),
            &ambiguous
        ));
    }

    #[test]
    fn unsuccessful_json_response_never_becomes_custom_input() {
        for status in ["failed", "incomplete", "in_progress", "cancelled"] {
            let response = json!({
                "status": status,
                "output": [{
                    "type": "function_call", "name": "exec", "id": "fc_1",
                    "call_id": "call_1", "arguments": "{\"input\":\"unsafe\"}"
                }]
            });
            assert!(adapt_response(response).is_err(), "{status}");
        }
    }

    #[test]
    fn opaque_item_metadata_is_not_treated_as_an_item_reference() {
        let mut native = request();
        native["input"][0]["metadata"] = json!({"item_id": "vendor-opaque"});
        let adapted = adapt_request(native).unwrap();
        assert_eq!(adapted["input"][0]["metadata"]["item_id"], "vendor-opaque");
    }

    #[test]
    fn adapted_response_rejects_raw_custom_exec_even_without_function_calls() {
        let response = json!({
            "status": "completed",
            "output": [{
                "type": "custom_tool_call", "name": "exec",
                "id": "ctc_1", "call_id": "call_1", "input": "unvalidated code"
            }]
        });
        assert!(adapt_response(response).is_err());
    }
}
