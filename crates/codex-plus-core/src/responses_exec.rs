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

/// 一次请求里做过的工具改写，响应按它还原。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolAdaptation {
    /// 转成 function 的 custom 工具名。
    pub(crate) custom: BTreeSet<String>,
    /// 展开到顶层的 namespace 子工具：扁平名 → (namespace, 子工具名)。
    pub(crate) namespaced: BTreeMap<String, (String, String)>,
    /// 客户端执行的 tool_search 以名为 `tool_search` 的 function 发出，调用要还原成 tool_search_call。
    pub(crate) tool_search: bool,
}

impl ToolAdaptation {
    /// 兼容重试只把 custom `exec` 转成 function。
    pub(crate) fn exec() -> Self {
        Self {
            custom: BTreeSet::from([EXEC_TOOL_NAME.to_string()]),
            ..Self::default()
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.custom.is_empty() && self.namespaced.is_empty() && !self.tool_search
    }
}

/// Codex 把自己的内置函数放在这个 namespace 里，客户端当作没有 namespace 处理，展开时不加前缀。
pub(crate) const LITE_NAMESPACE: &str = "functions";
pub(crate) const TOOL_SEARCH_NAME: &str = "tool_search";
/// 函数名长度上限（多数上游按 64 字符校验）。
const MAX_TOOL_NAME_LEN: usize = 64;

/// namespace 子工具展开后的名字：`namespace__name`，超长时截断并缀上整体哈希保证唯一。
pub(crate) fn flat_tool_name(namespace: &str, name: &str) -> String {
    if namespace == LITE_NAMESPACE {
        return name.to_owned();
    }
    let flat = crate::protocol_proxy::flatten_namespace_tool_name(namespace, name);
    if flat.len() <= MAX_TOOL_NAME_LEN {
        return flat;
    }
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{namespace}\0{name}").as_bytes());
    let mut cut = MAX_TOOL_NAME_LEN - 9;
    while !flat.is_char_boundary(cut) {
        cut -= 1;
    }
    let hash = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}_{hash}", &flat[..cut])
}

/// 只认 function 等内置工具类型的上游（xAI）：custom 工具转 function，
/// namespace（MCP 工具分组）展开成顶层 function，历史里带 namespace 的调用也按扁平名还原。
pub(crate) fn tool_adaptation_without_custom_types(request: &Value) -> ToolAdaptation {
    let custom = custom_tool_names(request);
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    // 与顶层工具重名的子工具不展开：响应里分不清调的是哪个。
    let taken = |flat: &str| {
        custom.contains(flat)
            || tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) != Some("namespace")
                    && tool.get("name").and_then(Value::as_str) == Some(flat)
            })
    };
    let mut namespaced = BTreeMap::new();
    let mut add = |namespace: &str, name: &str| {
        if namespace.is_empty() || namespace == LITE_NAMESPACE || name.is_empty() {
            return;
        }
        let flat = flat_tool_name(namespace, name);
        if !taken(&flat) {
            namespaced
                .entry(flat)
                .or_insert_with(|| (namespace.to_owned(), name.to_owned()));
        }
    };
    for tool in tools {
        if tool.get("type").and_then(Value::as_str) != Some("namespace") {
            continue;
        }
        let namespace = tool.get("name").and_then(Value::as_str).unwrap_or_default();
        for child in namespace_function_children(tool) {
            add(
                namespace,
                child
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
    }
    for item in request
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
    {
        if let Some(namespace) = item.get("namespace").and_then(Value::as_str) {
            add(
                namespace,
                item.get("name").and_then(Value::as_str).unwrap_or_default(),
            );
        }
    }
    ToolAdaptation {
        custom,
        namespaced,
        tool_search: false,
    }
}

fn namespace_function_children(tool: &Value) -> impl Iterator<Item = &Value> {
    tool.get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|child| {
            child.get("type").and_then(Value::as_str) == Some("function")
                && child
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.is_empty())
        })
}

/// 请求里出现的全部 custom 工具名（声明 + 历史调用），给完全不认 custom 类型的上游用。
/// 与 function 工具同名的跳过：响应里同名的 function_call 分不清该还原成哪种。
fn custom_tool_names(request: &Value) -> BTreeSet<String> {
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let declared = tools
        .iter()
        .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("custom"));
    let history = request
        .get("input")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("custom_tool_call"));
    declared
        .chain(history)
        .filter_map(|value| value.get("name").and_then(Value::as_str))
        .filter(|name| !name.is_empty())
        .filter(|name| !tools.iter().any(|tool| is_named_function_tool(tool, name)))
        .map(str::to_owned)
        .collect()
}

pub(crate) fn adapt_request(request: Value) -> Result<Value> {
    let declares_exec = request
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| tools.iter().any(is_custom_exec_tool));
    if !declares_exec {
        return Ok(request);
    }
    adapt_request_for(request, &ToolAdaptation::exec())
}

/// 按 `adaptation` 改写工具声明、tool_choice 和历史调用。
pub(crate) fn adapt_request_for(request: Value, adaptation: &ToolAdaptation) -> Result<Value> {
    let mut request = adapt_custom_request(request, &adaptation.custom)?;
    flatten_namespace_request(&mut request, &adaptation.namespaced);
    Ok(request)
}

/// namespace 声明展开成顶层 function，tool_choice 与历史里带 namespace 的调用改用扁平名。
/// 没有 namespace 的请求原样不动。
fn flatten_namespace_request(request: &mut Value, namespaced: &BTreeMap<String, (String, String)>) {
    if let Some(tools) = request.get("tools").and_then(Value::as_array).cloned()
        && tools
            .iter()
            .any(|tool| tool.get("type").and_then(Value::as_str) == Some("namespace"))
    {
        let mut translated: Vec<Value> = Vec::with_capacity(tools.len());
        let top_level = tools
            .iter()
            .filter(|tool| tool.get("type").and_then(Value::as_str) != Some("namespace"))
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        for tool in tools {
            if tool.get("type").and_then(Value::as_str) != Some("namespace") {
                translated.push(tool);
                continue;
            }
            let namespace = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            let namespace_description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            for child in namespace_function_children(&tool) {
                let name = child
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let flat = flat_tool_name(namespace, name);
                // `functions` 里的内置函数按原名上；其余只展开登记过的（未与顶层工具重名）。
                let offered = if namespace == LITE_NAMESPACE {
                    !top_level.contains(&flat)
                } else {
                    namespaced.get(&flat) == Some(&(namespace.to_owned(), name.to_owned()))
                };
                if !offered
                    || translated
                        .iter()
                        .any(|tool| tool.get("name").and_then(Value::as_str) == Some(flat.as_str()))
                {
                    continue;
                }
                let mut function = child.clone();
                function["name"] = json!(flat);
                let description = crate::protocol_proxy::combine_namespace_description(
                    namespace_description,
                    child
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                if let Some(object) = function.as_object_mut() {
                    object.remove("defer_loading");
                    if description.is_empty() {
                        object.remove("description");
                    } else {
                        object.insert("description".to_string(), json!(description));
                    }
                }
                translated.push(function);
            }
        }
        request["tools"] = Value::Array(translated);
    }
    if let Some(choice) = request.get_mut("tool_choice") {
        flatten_namespaced_call(choice);
    }
    if let Some(Value::Array(items)) = request.get_mut("input") {
        for item in items
            .iter_mut()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
        {
            flatten_namespaced_call(item);
        }
    }
}

/// `{name, namespace}` → `{name: 扁平名}`；没有 namespace 的原样不动。
fn flatten_namespaced_call(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let Some(namespace) = object
        .get("namespace")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !namespace.is_empty() && !name.is_empty() {
        let flat = flat_tool_name(&namespace, name);
        object.insert("name".to_string(), json!(flat));
    }
    object.remove("namespace");
}

/// 把上游的调用还原成 Codex 认的形式：namespace 子工具回到 `name` + `namespace`，
/// `tool_search` 函数回到客户端执行的 tool_search_call。
pub(crate) fn restore_adapted_item(item: &mut Value, adaptation: &ToolAdaptation) {
    if item.get("type").and_then(Value::as_str) != Some("function_call")
        || item.get("namespace").is_some()
    {
        return;
    }
    let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
    if adaptation.tool_search && name == TOOL_SEARCH_NAME {
        let arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .map(tool_search_arguments)
            .unwrap_or_else(|| json!({}));
        let Some(object) = item.as_object_mut() else {
            return;
        };
        object.remove("name");
        object.insert("type".to_string(), json!("tool_search_call"));
        object.insert("execution".to_string(), json!("client"));
        object.insert("arguments".to_string(), arguments);
        return;
    }
    let Some((namespace, name)) = adaptation.namespaced.get(name).cloned() else {
        return;
    };
    item["name"] = json!(name);
    item["namespace"] = json!(namespace);
    // 客户端把不带这个列表的 namespace 调用当作参数加密过（如 spawn_agent 的 message），
    // 上游从不加密参数，必须显式声明为空。
    item["encrypted_function_args"] = json!([]);
}

/// tool_search_call 的参数是 JSON 对象；上游给的不是对象时按原文放进 `input`。
fn tool_search_arguments(arguments: &str) -> Value {
    let arguments = arguments.trim();
    if arguments.is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(arguments) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({ "input": arguments }),
    }
}

/// SSE 事件里的 `item` 与终态 `response.output` 都要还原。
pub(crate) fn restore_adapted_event(event: &mut Value, adaptation: &ToolAdaptation) {
    if adaptation.namespaced.is_empty() && !adaptation.tool_search {
        return;
    }
    if let Some(item) = event.get_mut("item") {
        restore_adapted_item(item, adaptation);
    }
    if let Some(items) = event
        .pointer_mut("/response/output")
        .and_then(Value::as_array_mut)
    {
        for item in items {
            restore_adapted_item(item, adaptation);
        }
    }
}

/// 把 `names` 里的 custom 工具声明、tool_choice 和历史调用都改写成 function 形式。
fn adapt_custom_request(mut request: Value, names: &BTreeSet<String>) -> Result<Value> {
    if names.is_empty() {
        return Ok(request);
    }
    if let Some(tools) = request.get("tools").and_then(Value::as_array).cloned() {
        let mut declared = BTreeSet::new();
        let mut translated_tools = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            if tool.get("type").and_then(Value::as_str) == Some("custom") && names.contains(name) {
                ensure!(
                    declared.insert(name.to_owned()),
                    "ambiguous custom {name} declarations"
                );
                translated_tools.push(function_custom_tool(&tool));
            } else {
                ensure!(
                    !(names.contains(name) && is_named_function_tool(&tool, name)),
                    "custom {name} conflicts with a function of the same name"
                );
                translated_tools.push(tool);
            }
        }
        request["tools"] = Value::Array(translated_tools);
    }

    if let Some(tool_choice) = request.get_mut("tool_choice") {
        if let Some(name) =
            custom_tool_choice_name(tool_choice).filter(|name| names.contains(*name))
        {
            *tool_choice = json!({"type": "function", "name": name});
        } else if tool_choice_references_any(tool_choice, names) {
            bail!("unsupported tool_choice shape references an adapted custom tool");
        }
    }
    adapt_request_history(&mut request, names)?;
    Ok(request)
}

fn function_custom_tool(tool: &Value) -> Value {
    let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
    let mut translated = Map::new();
    translated.insert("type".to_string(), json!("function"));
    translated.insert("name".to_string(), json!(name));
    if name == EXEC_TOOL_NAME {
        if let Some(description) = tool.get("description") {
            translated.insert("description".to_string(), description.clone());
        }
    } else {
        translated.insert(
            "description".to_string(),
            json!(freeform_tool_description(tool, name)),
        );
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

/// 原工具是 freeform 文本输入，改成 function 后要告诉模型把原文整段放进 `input`，
/// 有语法定义（如 apply_patch 的 lark 文法）就一并带上。
fn freeform_tool_description(tool: &Value, name: &str) -> String {
    let description = tool
        .get("description")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    let mut text = if description.is_empty() {
        format!("Custom tool `{name}`.")
    } else {
        description.to_owned()
    };
    text.push_str(
        "\n\nThis tool takes raw text. Put the complete raw input in the `input` string \
         argument exactly as the original tool expects; do not add any other fields.",
    );
    if let Some(format) = tool.get("format").filter(|format| !format.is_null()) {
        text.push_str("\n\nThe `input` text must follow this format:\n```json\n");
        text.push_str(&format.to_string());
        text.push_str("\n```");
    }
    text
}

fn custom_tool_choice_name(choice: &Value) -> Option<&str> {
    let object = choice.as_object()?;
    (object.len() == 2 && choice.get("type").and_then(Value::as_str) == Some("custom"))
        .then(|| choice.get("name").and_then(Value::as_str))
        .flatten()
}

fn tool_choice_references_any(choice: &Value, names: &BTreeSet<String>) -> bool {
    match choice {
        Value::Object(object) => {
            object
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| names.contains(name))
                || object
                    .values()
                    .any(|value| tool_choice_references_any(value, names))
        }
        Value::Array(items) => items
            .iter()
            .any(|value| tool_choice_references_any(value, names)),
        _ => false,
    }
}

fn adapt_request_history(request: &mut Value, names: &BTreeSet<String>) -> Result<()> {
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
                    && item
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| names.contains(name))
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
                if item
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| names.contains(name)) =>
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
                translate_history_item_id(item, FUNCTION_CALL_PREFIX)?;
                item["type"] = json!("function_call");
                item["arguments"] = json!(serde_json::to_string(&json!({"input": input}))?);
                item.as_object_mut()
                    .expect("history item remains object")
                    .remove("input");
            }
            Some("custom_tool_call_output") => {
                let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or("");
                ensure!(all_calls.contains(call_id), "orphan custom tool output");
                if exec_calls.contains(call_id) {
                    translate_history_item_id(item, FUNCTION_OUTPUT_PREFIX)?;
                    item["type"] = json!("function_call_output");
                }
            }
            _ => {}
        }
    }
    validate_item_ids(items)?;
    remap_item_references(items, &original_ids)
}

pub(crate) fn adapt_response(response: Value) -> Result<Value> {
    adapt_response_for(response, &ToolAdaptation::exec())
}

/// 按 `adaptation` 把响应里的调用还原成 Codex 认的形式。
pub(crate) fn adapt_response_for(response: Value, adaptation: &ToolAdaptation) -> Result<Value> {
    let mut response = restore_custom_response(response, &adaptation.custom)?;
    if let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) {
        for item in items {
            restore_adapted_item(item, adaptation);
        }
    }
    Ok(response)
}

/// 把 `names` 里工具的 function_call 还原成 Codex 认的 custom_tool_call。
fn restore_custom_response(mut response: Value, names: &BTreeSet<String>) -> Result<Value> {
    let adapted_name = |item: &Value, item_type: &str| {
        item.get("type").and_then(Value::as_str) == Some(item_type)
            && item
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| names.contains(name))
    };
    let response_status = response
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let Some(items) = response.get_mut("output").and_then(Value::as_array_mut) else {
        return Ok(response);
    };
    ensure!(
        !items
            .iter()
            .any(|item| adapted_name(item, "custom_tool_call")),
        "adapted response contains unvalidated native exec input"
    );
    let matching_call_ids = items
        .iter()
        .filter(|item| adapted_name(item, "function_call"))
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
    let mut all_call_ids = BTreeSet::new();
    for item in items.iter().filter(|item| {
        matches!(
            item.get("type").and_then(Value::as_str),
            Some("function_call") | Some("custom_tool_call")
        )
    }) {
        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| anyhow!("tool call lacks call_id"))?;
        ensure!(
            all_call_ids.insert(call_id.to_owned()),
            "duplicate tool call_id"
        );
    }
    let mut output_call_ids = BTreeSet::new();
    validate_item_ids(items)?;
    for item in items.iter() {
        if matches!(
            item.get("type").and_then(Value::as_str),
            Some("function_call_output") | Some("custom_tool_call_output")
        ) {
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("tool output lacks call_id"))?;
            ensure!(all_call_ids.contains(call_id), "orphan tool output");
            ensure!(
                output_call_ids.insert(call_id.to_owned()),
                "duplicate tool output call_id"
            );
        }
    }
    for item in items.iter_mut() {
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") if adapted_name(item, "function_call") => {
                let arguments = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow!("exec arguments must be string"))?;
                let input = decode_exec_input(arguments)?;
                let id = translated_item_id(item, CUSTOM_CALL_PREFIX)?;
                item["type"] = json!("custom_tool_call");
                item["id"] = json!(id);
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

fn translate_history_item_id(item: &mut Value, target_prefix: &str) -> Result<()> {
    if item.get("id").is_some() {
        item["id"] = json!(translated_item_id(item, target_prefix)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_tools_are_flattened_and_restored() {
        let request = json!({
            "input": [
                {"type": "function_call", "id": "fc_1", "call_id": "c1",
                 "namespace": "mcp__fs", "name": "read", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "c1", "output": "ok"}
            ],
            "tools": [
                {"type": "function", "name": "shell", "parameters": {"type": "object"}},
                {"type": "function", "name": "mcp__x__y", "parameters": {"type": "object"}},
                {"type": "namespace", "name": "mcp__fs", "description": "Filesystem", "tools": [
                    {"type": "function", "name": "read", "description": "read file",
                     "defer_loading": true, "parameters": {"type": "object"}}
                ]},
                {"type": "namespace", "name": "mcp__x", "tools": [
                    {"type": "function", "name": "y", "parameters": {"type": "object"}}
                ]}
            ],
            "tool_choice": {"type": "function", "namespace": "mcp__fs", "name": "read"}
        });
        let adaptation = tool_adaptation_without_custom_types(&request);
        assert!(adaptation.custom.is_empty());
        assert_eq!(
            adaptation.namespaced,
            BTreeMap::from([(
                "mcp__fs__read".to_string(),
                ("mcp__fs".to_string(), "read".to_string())
            )]),
            "与顶层 function 重名的子工具不展开"
        );

        let adapted = adapt_request_for(request, &adaptation).unwrap();
        let tools = adapted["tools"].as_array().unwrap();
        assert!(tools.iter().all(|tool| tool["type"] == "function"));
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["shell", "mcp__x__y", "mcp__fs__read"]
        );
        assert_eq!(tools[2]["description"], "Filesystem\n\nread file");
        assert!(tools[2].get("defer_loading").is_none());
        assert_eq!(
            adapted["tool_choice"],
            json!({"type": "function", "name": "mcp__fs__read"})
        );
        assert_eq!(adapted["input"][0]["name"], "mcp__fs__read");
        assert!(adapted["input"][0].get("namespace").is_none());

        let response = adapt_response_for(
            json!({"status": "completed", "output": [
                {"type": "function_call", "id": "fc_2", "call_id": "c2",
                 "name": "mcp__fs__read", "arguments": "{}"},
                {"type": "function_call", "id": "fc_3", "call_id": "c3",
                 "name": "shell", "arguments": "{}"}
            ]}),
            &adaptation,
        )
        .unwrap();
        assert_eq!(response["output"][0]["name"], "read");
        assert_eq!(response["output"][0]["namespace"], "mcp__fs");
        assert_eq!(response["output"][1]["name"], "shell");
        assert!(response["output"][1].get("namespace").is_none());
    }

    #[test]
    fn all_custom_tools_are_adapted_for_upstreams_without_custom_support() {
        let request = json!({
            "input": [
                {"type": "message", "id": "msg_1", "role": "user", "content": "hi"},
                {"type": "custom_tool_call", "id": "ctc_1", "call_id": "c1",
                 "name": "apply_patch", "input": "*** Begin Patch\n*** End Patch"},
                {"type": "custom_tool_call_output", "id": "ctco_1",
                 "call_id": "c1", "output": "done"},
                {"type": "custom_tool_call", "id": "ctc_2", "call_id": "c2",
                 "name": "dup", "input": "x"},
                {"type": "function_call_output", "call_id": "c2", "output": "x"}
            ],
            "tools": [
                {"type": "function", "name": "shell", "parameters": {"type": "object"}},
                {"type": "function", "name": "dup", "parameters": {"type": "object"}},
                {"type": "custom", "name": "apply_patch", "description": "patch",
                 "format": {"type": "grammar", "syntax": "lark", "definition": "start: begin_patch"}},
                {"type": "custom", "name": "exec", "description": "run"}
            ],
            "tool_choice": "auto"
        });
        let adaptation = tool_adaptation_without_custom_types(&request);
        assert!(adaptation.namespaced.is_empty());
        let names = adaptation.custom.clone();
        assert_eq!(
            names,
            BTreeSet::from(["apply_patch".to_string(), "exec".to_string()]),
            "与 function 同名的不转，免得响应里分不清"
        );

        let adapted = adapt_request_for(request, &adaptation).unwrap();
        let tools = adapted["tools"].as_array().unwrap();
        assert!(tools.iter().all(|tool| tool["type"] == "function"));
        let patch = tools
            .iter()
            .find(|tool| tool["name"] == "apply_patch")
            .unwrap();
        assert_eq!(patch["parameters"]["required"], json!(["input"]));
        let description = patch["description"].as_str().unwrap();
        assert!(description.starts_with("patch"));
        assert!(
            description.contains("start: begin_patch"),
            "文法定义要带给模型"
        );
        assert_eq!(
            tools.iter().find(|tool| tool["name"] == "exec").unwrap()["description"],
            "run",
            "exec 沿用兼容重试的原样描述"
        );
        assert_eq!(adapted["tool_choice"], "auto");

        let input = adapted["input"].as_array().unwrap();
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["id"], "fc_1");
        assert_eq!(input[1]["name"], "apply_patch");
        assert_eq!(
            input[1]["arguments"],
            serde_json::to_string(&json!({"input": "*** Begin Patch\n*** End Patch"})).unwrap()
        );
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["id"], "fco_1");
        assert_eq!(input[3]["type"], "custom_tool_call", "不在名单里的保持原样");

        let response = adapt_response_for(
            json!({"status": "completed", "output": [
                {"type": "function_call", "id": "fc_9", "call_id": "c9",
                 "name": "apply_patch", "arguments": "{\"input\":\"*** Begin Patch\"}"},
                {"type": "function_call", "id": "fc_8", "call_id": "c8",
                 "name": "shell", "arguments": "{}"}
            ]}),
            &adaptation,
        )
        .unwrap();
        assert_eq!(response["output"][0]["type"], "custom_tool_call");
        assert_eq!(response["output"][0]["id"], "ctc_9");
        assert_eq!(response["output"][0]["name"], "apply_patch");
        assert_eq!(response["output"][0]["input"], "*** Begin Patch");
        assert_eq!(response["output"][1]["type"], "function_call");
    }

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

    #[test]
    fn history_preserves_optional_call_and_output_ids() {
        for omit_call_id in [false, true] {
            let mut native = request();
            native["input"][2].as_object_mut().unwrap().remove("id");
            if omit_call_id {
                native["input"][1].as_object_mut().unwrap().remove("id");
            }
            let adapted = adapt_request(native).unwrap();
            assert_eq!(adapted["input"][1]["type"], "function_call");
            assert_eq!(adapted["input"][2]["type"], "function_call_output");
            assert!(adapted["input"][2].get("id").is_none());
            assert_eq!(adapted["input"][2]["call_id"], "old");
            assert_eq!(adapted["input"][2]["output"], "ok");
            assert_eq!(adapted["input"][1].get("id").is_none(), omit_call_id);
        }
    }

    #[test]
    fn response_rejects_call_ids_shared_by_function_and_custom_tools() {
        for call_id in ["exec", "patch"] {
            let response = json!({
                "status": "completed",
                "output": [
                    {"type": "function_call", "id": "fc_exec", "call_id": "exec",
                     "name": "exec", "arguments": "{\"input\":\"text('ok')\"}"},
                    {"type": "custom_tool_call", "id": "ctc_patch", "call_id": "patch",
                     "name": "apply_patch", "input": "*** Begin Patch\n*** End Patch"},
                    {"type": "function_call", "id": "fc_lookup", "call_id": call_id,
                     "name": "lookup", "arguments": "{}"}
                ]
            });
            assert!(adapt_response(response).is_err(), "{call_id}");
        }
        let response = json!({
            "status": "completed",
            "output": [
                {"type": "function_call", "id": "fc_exec", "call_id": "same",
                 "name": "exec", "arguments": "{\"input\":\"text('ok')\"}"},
                {"type": "custom_tool_call", "id": "ctc_patch", "call_id": "same",
                 "name": "apply_patch", "input": "*** Begin Patch\n*** End Patch"}
            ]
        });
        assert!(adapt_response(response).is_err());
    }
}
