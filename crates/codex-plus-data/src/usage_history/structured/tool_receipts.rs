//! 只保留执行状态和技能加载证据，不把工具正文转发给扩展。
use super::*;

pub(super) const MAX_JSON_TEXT_BYTES: usize = 64 * 1024;

// 只解析分类所用的字段。未知字段可含稠密数组或很长的正文。
pub(super) struct ArgumentFields(pub(super) Value);

impl<'de> Deserialize<'de> for ArgumentFields {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::{IgnoredAny, MapAccess, Visitor};
        struct Fields;
        impl<'de> Visitor<'de> for Fields {
            type Value = ArgumentFields;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("tool arguments")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                #[derive(Deserialize)]
                struct Subcall {
                    recipient_name: String,
                    parameters: ArgumentFields,
                }
                let mut fields = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    let value = match key.as_str() {
                        "cmd" | "command" | "path" | "file_path" | "resource" | "code"
                        | "js_code" => json!(map.next_value::<String>()?),
                        "session_id" => json!(map.next_value::<u64>()?),
                        "tool_uses" => {
                            let calls = map.next_value::<Vec<Subcall>>()?;
                            Value::Array(calls.into_iter().map(|call| json!({
                                "recipient_name":call.recipient_name,"parameters":call.parameters.0
                            })).collect())
                        }
                        _ => {
                            map.next_value::<IgnoredAny>()?;
                            if !matches!(
                                key.as_str(),
                                "offset" | "limit" | "start_line" | "end_line"
                            ) {
                                fields.insert("_extra".into(), Value::Null);
                            }
                            continue;
                        }
                    };
                    fields.insert(key, value);
                }
                Ok(ArgumentFields(Value::Object(fields)))
            }
        }
        de.deserialize_map(Fields)
    }
}

/// 只提供分类元数据，不把命令、路径或调用方附带的其他字段传给扩展。
pub(super) fn call(payload: &Value) -> Value {
    let mut result = serde_json::Map::new();
    for key in ["type", "name", "namespace"] {
        if let Some(text) = payload[key].as_str() {
            result.insert(key.into(), json!(text));
        }
    }
    if let Some(id) = payload["call_id"]
        .as_str()
        .or_else(|| payload["id"].as_str())
    {
        result.insert("call_id".into(), json!(id));
    }
    let name = payload["name"].as_str().unwrap_or("");
    if !may_read_skill(name) {
        result.insert(
            "arguments".into(),
            json!({"skillNames":[],"startsWithRead":false,"singleRead":false}),
        );
        return Value::Object(result);
    }
    let arguments = payload
        .get("arguments")
        .or_else(|| payload.get("input"))
        .unwrap_or(&Value::Null);
    let raw = arguments.as_str().unwrap_or("");
    let parsed = raw
        .trim_start()
        .starts_with('{')
        .then(|| serde_json::from_str::<ArgumentFields>(raw).ok())
        .flatten();
    let args = parsed.as_ref().map_or(arguments, |parsed| &parsed.0);
    let fallback = if raw.trim_start().starts_with(['{', '[']) {
        ""
    } else {
        raw
    };
    let mut names = std::collections::BTreeSet::new();
    let starts_with_read = skill_names(name, args, fallback, &mut names);
    result.insert(
        "arguments".into(),
        json!({
            "skillNames":names,
            "startsWithRead":starts_with_read,
            "singleRead":single_read(name, args, fallback),
        }),
    );
    Value::Object(result)
}

fn single_read(name: &str, args: &Value, fallback: &str) -> bool {
    let shell = ["exec", "shell", "terminal", "commandExecution"]
        .iter()
        .any(|part| name.contains(part));
    if !shell {
        return ["read_file", "read", "open_file"].iter().any(|suffix| {
            name.strip_suffix(suffix)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with(['.', '_']))
        }) && args.as_object().is_some_and(|args| {
            let paths: Vec<_> = ["path", "file_path", "resource"]
                .iter()
                .filter_map(|key| args.get(*key).and_then(Value::as_str))
                .collect();
            args.keys().all(|key| {
                [
                    "path",
                    "file_path",
                    "resource",
                    "offset",
                    "limit",
                    "start_line",
                    "end_line",
                ]
                .contains(&key.as_str())
            }) && paths.len() == 1
                && (paths[0].to_ascii_lowercase().ends_with("/skill.md")
                    || paths[0].to_ascii_lowercase().ends_with("\\skill.md"))
        });
    }
    let command = args["cmd"]
        .as_str()
        .or_else(|| args["command"].as_str())
        .unwrap_or(fallback);
    if command.chars().any(|c| "`$;\r\n&<>(){}*?[]".contains(c)) {
        return false;
    }
    let mut tokens = Vec::new();
    let (mut token, mut quote) = (String::new(), None);
    for ch in command.chars().chain(std::iter::once(' ')) {
        if let Some(expected) = quote {
            if ch == expected {
                quote = None;
            } else {
                token.push(ch);
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch.is_whitespace() || ch == '|' {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            if ch == '|' {
                tokens.push("|".into());
            }
        } else {
            token.push(ch);
        }
    }
    if quote.is_some() || tokens.is_empty() {
        return false;
    }
    let pipe = tokens
        .iter()
        .position(|token| token == "|")
        .unwrap_or(tokens.len());
    if pipe < tokens.len() {
        let tail = &tokens[pipe + 1..];
        if tail.len() < 3
            || !tail[0].eq_ignore_ascii_case("Select-Object")
            || tail[1..].len() % 2 != 0
            || !tail[1..].chunks(2).all(|pair| {
                ["-skip", "-first", "-last"].contains(&pair[0].to_ascii_lowercase().as_str())
                    && pair[1].parse::<u64>().is_ok()
            })
        {
            return false;
        }
    }
    let program = tokens[0].to_ascii_lowercase();
    if !["get-content", "cat", "head", "tail", "more", "type"].contains(&program.as_str()) {
        return false;
    }
    let mut paths = 0;
    let mut iter = tokens[1..pipe].iter();
    while let Some(token) = iter.next() {
        let lower = token.to_ascii_lowercase();
        match (program.as_str(), lower.as_str()) {
            ("get-content", "-literalpath" | "-path" | "-raw") => {}
            ("get-content", "-totalcount" | "-tail") | ("head" | "tail", "-n") => {
                if iter.next().and_then(|n| n.parse::<u64>().ok()).is_none() {
                    return false;
                }
            }
            ("get-content", "-encoding") => {
                if !iter.next().is_some_and(|encoding| {
                    [
                        "utf8",
                        "utf8nobom",
                        "utf8bom",
                        "unicode",
                        "ascii",
                        "default",
                    ]
                    .contains(&encoding.to_ascii_lowercase().as_str())
                }) {
                    return false;
                }
            }
            ("cat" | "head" | "tail", "--") => {}
            ("head" | "tail", count)
                if count.starts_with('-') && count[1..].parse::<u64>().is_ok() => {}
            _ if lower.ends_with("/skill.md") || lower.ends_with("\\skill.md") => paths += 1,
            _ => return false,
        }
    }
    paths == 1
}

pub(super) fn may_read_skill(name: &str) -> bool {
    [
        "exec",
        "shell",
        "terminal",
        "commandExecution",
        "read",
        "open_file",
        "run",
        "js",
    ]
    .iter()
    .any(|part| name.contains(part))
        || name.ends_with("parallel")
}

fn skill_names(
    name: &str,
    args: &Value,
    fallback: &str,
    names: &mut std::collections::BTreeSet<String>,
) -> bool {
    if name.ends_with("parallel") {
        if let Some(calls) = args["tool_uses"].as_array() {
            for call in calls {
                skill_names(
                    call["recipient_name"].as_str().unwrap_or(""),
                    &call["parameters"],
                    "",
                    names,
                );
            }
        }
        return false;
    }
    let command = [
        "cmd",
        "command",
        "path",
        "file_path",
        "resource",
        "code",
        "js_code",
    ]
    .iter()
    .find_map(|key| args[*key].as_str())
    .unwrap_or(fallback);
    let shell = ["exec", "shell", "terminal", "commandExecution"]
        .iter()
        .any(|part| name.contains(part));
    let read = [
        "Get-Content",
        "cat ",
        "head ",
        "tail ",
        "sed ",
        "more ",
        "type ",
        "readFile",
    ]
    .iter()
    .any(|part| command.contains(part));
    if shell && !read
        || !may_read_skill(name)
        || command
            .split([';', '\n'])
            .any(|part| part.trim_start().starts_with("echo "))
    {
        return false;
    }
    for token in command.split(|c: char| c.is_whitespace() || "\"'`;|<>(){},=".contains(c)) {
        let token = token.replace('\\', "/");
        if let Some(parent) = token
            .strip_suffix("/SKILL.md")
            .or_else(|| token.strip_suffix("/skill.md"))
        {
            if let Some(name) = parent.rsplit('/').next().filter(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_alphanumeric() || "._-:".contains(c))
            }) {
                names.insert(name.to_owned());
            }
        }
    }
    ["Get-Content ", "cat ", "head ", "more ", "type "]
        .iter()
        .any(|prefix| command.trim_start().starts_with(prefix))
}

pub(super) fn failed_text(text: &str) -> bool {
    let text = text.trim_start().to_ascii_lowercase();
    [
        "error",
        "tool call error:",
        "tool execution failed",
        "tool call failed",
        "failed to call",
        "failed to execute",
        "mcp error",
        "error calling tool",
        "filenotfound",
        "cannot find path",
        "no such file",
        "access denied",
        "access is denied",
    ]
    .iter()
    .any(|prefix| text.starts_with(prefix))
}

pub(super) fn compact(value: &Value, depth: usize, skill_read: bool) -> Value {
    match value {
        Value::String(text) => {
            if text.len() <= MAX_JSON_TEXT_BYTES {
                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                    return compact(&parsed, depth + 1, skill_read);
                }
            } else if text.trim_start().starts_with(['{', '['])
                && let Some(projected) = oversized::structured_output(text, skill_read)
            {
                return if projected.as_array().is_some_and(Vec::is_empty) {
                    json!("")
                } else {
                    projected
                };
            }
            let mut evidence = Vec::new();
            // 仅识别执行器回执头；正文中的同名行不能伪造退出状态。
            if exec_receipt(value).is_some() {
                for line in text.trim().lines() {
                    if let Some(seconds) = line.strip_prefix("Wall time: ") {
                        if let Some(seconds) = seconds.strip_suffix(" seconds") {
                            if let Ok(seconds) = seconds.parse::<f64>() {
                                if seconds.is_finite() && seconds >= 0.0 {
                                    evidence.push(format!("Wall time: {seconds} seconds"));
                                }
                            }
                        }
                    } else if let Some(code) = line.strip_prefix("Process exited with code ") {
                        if let Some(code) = code
                            .split_whitespace()
                            .next()
                            .and_then(|v| v.parse::<i32>().ok())
                        {
                            evidence.push(format!("Process exited with code {code}"));
                        }
                        break;
                    } else if let Some(id) = line.strip_prefix("Process running with session ID ") {
                        if let Some(id) = id
                            .split_whitespace()
                            .next()
                            .filter(|v| v.bytes().all(|c| c.is_ascii_digit()))
                        {
                            evidence.push(format!("Process running with session ID {id}"));
                        }
                        break;
                    }
                }
            } else if failed_text(text) {
                evidence.push("Error: tool execution failed".into());
            } else if [
                "status is unknown",
                "outcome is unknown",
                "did not complete",
            ]
            .iter()
            .any(|status| text.trim().to_ascii_lowercase().starts_with(status))
            {
                return json!({"status":"unknown"});
            }
            // 用于批量/异步技能读取的结果判定，仅返回名称和“见过文档”标志。
            for line in text.lines().filter(|_| skill_read) {
                if let Some(name) = line.trim().strip_prefix("name:") {
                    let name = name.trim().trim_matches(['"', '\'']);
                    if !name.is_empty()
                        && name
                            .chars()
                            .all(|c| c.is_alphanumeric() || "._-:".contains(c))
                    {
                        evidence.push(format!("name: {name}"));
                    }
                }
            }
            if skill_read
                && text.lines().any(|line| {
                    let line = line.trim_start();
                    line.starts_with("# ") || line.starts_with("## ") || line == "---"
                })
            {
                evidence.push("# Skill content observed".into());
            }
            json!(evidence.join("\n"))
        }
        Value::Array(parts) => json!(
            parts
                .iter()
                .map(|part| compact(part, depth + 1, skill_read))
                .collect::<Vec<_>>()
        ),
        Value::Object(object) => {
            let mut result = serde_json::Map::new();
            for key in ["isError", "success"] {
                if let Some(value) = object.get(key).and_then(Value::as_bool) {
                    result.insert(key.into(), json!(value));
                }
            }
            if object
                .get("error")
                .is_some_and(|v| !v.is_null() && *v != false)
            {
                result.insert("error".into(), json!(true));
            }
            for key in ["exit_code", "exitCode"] {
                if let Some(value) = object.get(key).and_then(Value::as_i64) {
                    result.insert(key.into(), json!(value));
                }
            }
            if let Some(status) = object
                .get("status")
                .and_then(Value::as_str)
                .filter(|status| {
                    matches!(
                        *status,
                        "success"
                            | "completed"
                            | "failed"
                            | "error"
                            | "cancelled"
                            | "canceled"
                            | "interrupted"
                            | "unknown"
                            | "pending"
                            | "running"
                            | "in_progress"
                    )
                })
            {
                result.insert("status".into(), json!(status));
            }
            if let Some(kind) = object
                .get("type")
                .and_then(Value::as_str)
                .filter(|kind| matches!(*kind, "text" | "input_text" | "output_text"))
            {
                result.insert("type".into(), json!(kind));
                result.insert(
                    "text".into(),
                    compact(&object["text"], depth + 1, skill_read),
                );
            }
            if let Some(content) = object.get("content").filter(|v| v.is_array()) {
                result.insert("content".into(), compact(content, depth + 1, skill_read));
            }
            Value::Object(result)
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_arguments_ids_and_poll_metadata_survive_plain_and_archived_rollouts()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let at = Local::now();
        let session = "00000000-0000-4000-8000-000000000001";
        let id = "id".repeat(800);
        let namespace = format!("mcp__{}", "n".repeat(2100));
        let name = "reader".repeat(200);
        for size in [72_000, (1 << 20) + 512] {
            let padding = "x".repeat(size);
            let skill = json!({"body":padding,"cmd":"cat /skills/example/SKILL.md"});
            let stdin = json!({"chars":padding,"session_id":42});
            let wrap = |payload| json!({"type":"response_item","timestamp":at.to_rfc3339(),"payload":payload});
            let rows = [
                json!({"type":"session_meta","timestamp":at.to_rfc3339(),"payload":{"id":session}}),
                wrap(
                    json!({"type":"function_call","name":name,"namespace":namespace,"call_id":id,"arguments":json!({"body":padding}).to_string()}),
                ),
                wrap(
                    json!({"type":"function_call_output","call_id":id,"output":json!({
                        "body":padding,"success":false,"content":[
                            {"type":"text","text":"Error: private failure details"},
                            {"type":"text","text":padding}
                        ]}).to_string()}),
                ),
                wrap(
                    json!({"type":"function_call","name":"functions.exec_command","call_id":"s","arguments":skill.to_string()}),
                ),
                wrap(
                    json!({"type":"function_call_output","call_id":"s","output":format!("Chunk ID: {padding}\nProcess running with session ID 42\nOutput:\nname: example")}),
                ),
                wrap(
                    json!({"type":"function_call","name":"functions.write_stdin","call_id":"p","arguments":stdin.to_string()}),
                ),
                wrap(
                    json!({"type":"function_call_output","call_id":"p","output":{
                        "exit_code":0,"content":[{"type":"text","text":format!("{padding}\nname: example\n# Skill content")}]
                    }}),
                ),
                wrap(
                    json!({"type":"function_call","name":"functions.exec_command","call_id":"fake","arguments":"{}"}),
                ),
                wrap(
                    json!({"type":"function_call_output","call_id":"fake","output":format!("Chunk ID: x\n  Process running with session ID 84\n{padding}")}),
                ),
            ];
            let text = rows
                .iter()
                .map(|row| format!("{row}\n"))
                .collect::<String>();
            for archived in [false, true] {
                let path = root.join(if archived {
                    "rollout.jsonl.zst"
                } else {
                    "rollout.jsonl"
                });
                if archived {
                    fs::write(&path, zstd::stream::encode_all(text.as_bytes(), 1)?)?;
                } else {
                    fs::write(&path, &text)?;
                }
                let mut state = FileState::default();
                state.executions = Some(execution_observations::Batch {
                    now_ms: at.timestamp_millis(),
                    rows: vec![],
                });
                let mut complete = false;
                for _ in 0..8 {
                    complete = state.read_with_budget(
                        &path,
                        at.date_naive(),
                        &mut index_budget::Budget::new(std::iter::empty()),
                    )?;
                    if complete {
                        break;
                    }
                }
                assert!(complete, "archived={archived}, size={size}");
                assert_eq!(state.issues, 0);
                assert!(!state.activity_truncated);
                assert_eq!(state.tools.len(), 2, "archived={archived}, size={size}");
                let mcp = &state.tools[0];
                assert_eq!(mcp.id, id);
                assert_eq!(mcp.call.payload["name"], name);
                assert_eq!(mcp.call.payload["namespace"], namespace);
                assert_eq!(mcp.output["output"]["success"], false);
                assert_eq!(
                    mcp.output["output"]["content"][0]["text"],
                    "Error: tool execution failed"
                );
                assert!(!mcp.call.payload.to_string().contains(&padding));
                assert_eq!(
                    state.tools[1].call.payload["arguments"]["skillNames"],
                    json!(["example"])
                );
                assert_eq!(state.tools[1].output["call_id"], "s");
                assert_eq!(
                    state.tools[1].output["output"]["content"][0]["text"],
                    "name: example\n# Skill content observed"
                );
                let observations = &state.executions.as_ref().unwrap().rows;
                assert_eq!(observations.len(), 8);
                assert_eq!(observations[0].name.as_deref(), Some(name.as_str()));
                assert_eq!(
                    observations[0].namespace.as_deref(),
                    Some(namespace.as_str())
                );
                assert_eq!(observations[4].process.as_deref(), Some("42"));
                assert_eq!(observations[3].process.as_deref(), Some("42"));
                assert!(observations[7].process.is_none());
            }
        }
        Ok(())
    }

    #[test]
    fn single_and_parallel_calls_expose_only_whitelisted_skill_metadata() {
        let single = json!({
            "type":"function_call", "call_id":"single", "name":"functions.exec_command",
            "arguments":json!({"cmd":"Get-Content C:/private-user/skills/pdf/SKILL.md; secret-key",
                "workdir":"private-folder"}).to_string(),
            "input":"private-input", "extra":"private-extra"
        });
        assert_eq!(
            call(&single),
            json!({
                "type":"function_call", "call_id":"single", "name":"functions.exec_command",
                "arguments":{"skillNames":["pdf"],"startsWithRead":true,"singleRead":false}
            })
        );

        let batch = json!({
            "type":"function_call", "id":"batch", "name":"multi_tool_use.parallel",
            "arguments":json!({"tool_uses":[
                {"recipient_name":"functions.exec_command", "parameters":{
                    "cmd":"cat /private-user/skills/pdf/SKILL.md", "workdir":"private-folder"}},
                {"recipient_name":"functions.read_file", "parameters":{
                    "path":"/private-user/skills/latex/SKILL.md"}},
                {"recipient_name":"functions.read_file", "parameters":{
                    "path":"/private-user/skills/pdf/SKILL.md"}}
            ]}).to_string(),
            "extra":"private-extra"
        });
        assert_eq!(
            call(&batch),
            json!({
                "type":"function_call", "call_id":"batch", "name":"multi_tool_use.parallel",
                "arguments":{"skillNames":["latex","pdf"],"startsWithRead":false,"singleRead":false}
            })
        );
        assert_eq!(
            call(&json!({"name":"functions.exec_command",
                "arguments":json!({"cmd":"echo /skills/fake/SKILL.md"}).to_string()
            }))["arguments"]["skillNames"],
            json!([])
        );
    }

    #[test]
    fn only_literal_single_skill_reads_are_marked_as_single() {
        for command in [
            "Get-Content -LiteralPath 'C:/skills/pdf/SKILL.md' -Raw",
            "Get-Content C:/skills/pdf/SKILL.md | Select-Object -Skip 1 -First 20",
            "cat /skills/pdf/SKILL.md",
            "head -n 20 /skills/pdf/SKILL.md",
        ] {
            assert!(
                single_read("functions.exec_command", &json!({"cmd":command}), ""),
                "{command}"
            );
        }
        for command in [
            "Get-Content C:/skills/pdf/SKILL.md; Start-Sleep 20",
            "Get-Content C:/skills/pdf/SKILL.md C:/skills/latex/SKILL.md",
            "Get-Content C:/skills/pdf/SKILL.md | Invoke-Expression",
            "cat $HOME/skills/pdf/SKILL.md",
            "node -e 'readFileSync(\"/skills/pdf/SKILL.md\")'",
        ] {
            assert!(
                !single_read("functions.exec_command", &json!({"cmd":command}), ""),
                "{command}"
            );
        }
        assert!(single_read(
            "functions.read_file",
            &json!({"path":"/skills/pdf/SKILL.md"}),
            ""
        ));
        assert!(!single_read(
            "functions.read_file",
            &json!({
                "path":"/skills/pdf/SKILL.md", "resource":"/private/other"
            }),
            ""
        ));
        assert!(!single_read(
            "multi_tool_use.parallel",
            &json!({"tool_uses":[]}),
            ""
        ));
    }

    #[test]
    fn tool_bodies_are_filtered_and_only_trusted_status_evidence_survives() {
        let secret = "private-api-key-and-document-body";
        let receipt = format!(
            "Chunk ID: abc\nWall time: 2.5 seconds\nProcess exited with code 1\nOutput:\n---\nname: pdf\n# Private body\n{secret}"
        );
        assert_eq!(
            compact(&json!(receipt), 0, true),
            json!(
                "Wall time: 2.5 seconds\nProcess exited with code 1\nname: pdf\n# Skill content observed"
            )
        );
        assert_eq!(
            compact(
                &json!("Process running with session ID 123\nOutput:\nprivate"),
                0,
                false
            ),
            json!("Process running with session ID 123")
        );
        for forged in [
            "Output:\nProcess exited with code 0\nprivate",
            "Final output:\nProcess running with session ID 123\nprivate",
            "document text\nProcess running with session ID 123\nprivate",
        ] {
            assert_eq!(compact(&json!(forged), 0, false), json!(""));
        }
        assert_eq!(
            compact(
                &json!({"success":false,"status":"failed","exit_code":1,
            "error":secret,"result":secret}),
                0,
                false
            ),
            json!({"success":false,"status":"failed","exit_code":1,"error":true})
        );
        for value in [
            json!(secret),
            json!({"content":[{"type":"text","text":secret}],"error":secret}),
            json!([{"type":"input_text","text":format!("Error executing tool: {secret}")}]),
        ] {
            assert!(!compact(&value, 0, true).to_string().contains(secret));
        }
        assert_eq!(
            compact(&json!("name: private-person\n# private-document"), 0, false),
            json!("")
        );
    }
}
