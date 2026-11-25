use serde::{Deserialize, Serialize};
use serde_json::Value;

const KEEP_TURNS: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Instructions,
    Tools,
    User,
    Assistant,
    ToolCalls,
    ToolResults,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextPart {
    kind: Kind,
    items: u64,
    estimated_tokens: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ContextTurn {
    number: u64,
    parts: Vec<ContextPart>,
}

/// Only numeric structure of the input; never serializes prompt text, names, or IDs.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestContextSummary {
    parts: Vec<ContextPart>,
    turns: Vec<ContextTurn>,
    omitted_turns: u64,
    partial: bool,
}

impl RequestContextSummary {
    pub(crate) fn from_responses(body: &Value) -> Option<Self> {
        let mut parts = Vec::new();
        if let Some(text) = body.get("instructions").and_then(Value::as_str) {
            add(&mut parts, Kind::Instructions, text.chars().count());
        }
        for tool in body
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add(&mut parts, Kind::Tools, schema_chars(tool));
        }
        let mut turns = std::collections::VecDeque::new();
        let mut number = 0;
        let mut current = Vec::new();
        let mut partial = [
            "previous_response_id",
            "conversation",
            "prompt",
            "item_reference",
        ]
        .iter()
        .any(|name| body.get(name).is_some_and(|value| !value.is_null()));
        if let Some(text) = body.get("input").and_then(Value::as_str) {
            number = 1;
            add(&mut parts, Kind::User, text.chars().count());
            add(&mut current, Kind::User, text.chars().count());
        }
        if let Some(input) = body.get("input").and_then(Value::as_array) {
            for item in input {
                let role = item.get("role").and_then(Value::as_str);
                let item_type = item.get("type").and_then(Value::as_str);
                let (kind, chars) = match (role, item_type) {
                    (Some("system" | "developer"), _) => {
                        (Kind::Instructions, message_chars(item, &mut partial))
                    }
                    (Some("user"), _) => {
                        if number > 0 {
                            if turns.len() == KEEP_TURNS {
                                turns.pop_front();
                            }
                            turns.push_back(ContextTurn {
                                number,
                                parts: std::mem::take(&mut current),
                            });
                        }
                        number += 1;
                        (Kind::User, message_chars(item, &mut partial))
                    }
                    (Some("assistant"), _) => (Kind::Assistant, message_chars(item, &mut partial)),
                    (_, Some("function_call" | "custom_tool_call")) => (
                        Kind::ToolCalls,
                        string_chars(item, "arguments") + string_chars(item, "input"),
                    ),
                    (_, Some("function_call_output" | "custom_tool_call_output")) => (
                        Kind::ToolResults,
                        text_blocks(item.get("output"), &mut partial),
                    ),
                    _ => {
                        partial = true;
                        (Kind::Other, 0)
                    }
                };
                add(&mut parts, kind, chars);
                if number > 0 && kind != Kind::Instructions {
                    add(&mut current, kind, chars);
                }
            }
        }
        if number > 0 {
            if turns.len() == KEEP_TURNS {
                turns.pop_front();
            }
            turns.push_back(ContextTurn {
                number,
                parts: current,
            });
        }
        (!parts.is_empty()).then(|| Self {
            parts,
            omitted_turns: number - turns.len() as u64,
            turns: turns.into(),
            partial,
        })
    }
}

fn add(parts: &mut Vec<ContextPart>, kind: Kind, chars: usize) {
    let estimate = crate::protocol_proxy::estimated_tokens_from_chars(chars);
    if let Some(part) = parts.iter_mut().find(|part| part.kind == kind) {
        part.items += 1;
        part.estimated_tokens += estimate;
    } else {
        parts.push(ContextPart {
            kind,
            items: 1,
            estimated_tokens: estimate,
        });
    }
}

fn string_chars(value: &Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(Value::as_str)
        .map_or(0, |text| text.chars().count())
}

fn schema_chars(value: &Value) -> usize {
    match value {
        Value::String(text) => text.chars().count(),
        Value::Array(items) => items.iter().map(schema_chars).sum(),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, val)| key.chars().count() + schema_chars(val))
            .sum(),
        _ => 0,
    }
}

fn message_chars(item: &Value, partial: &mut bool) -> usize {
    text_blocks(item.get("content"), partial)
}

fn text_blocks(value: Option<&Value>, partial: &mut bool) -> usize {
    match value {
        Some(Value::String(text)) => text.chars().count(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(|block| {
                if matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("text" | "input_text" | "output_text")
                ) {
                    string_chars(block, "text")
                } else {
                    *partial = true;
                    0
                }
            })
            .sum(),
        _ => {
            *partial = true;
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn records_turns_and_tool_types_without_payload_text() {
        let request = json!({"previous_response_id": "private-id", "instructions":"private-instruction", "tools":[{"name":"private-tool"}], "input":[
            {"role":"user","content":[{"type":"input_text","text":"private-user"},{"type":"input_image","image_url":"private-image"}]},
            {"type":"function_call","arguments":"private-call"}, {"type":"function_call_output","output":"private-output"},
            {"role":"user","content":"private-user"}, {"role":"assistant","content":"private-assistant"}
        ]});
        let summary = RequestContextSummary::from_responses(&request).unwrap();
        assert_eq!(summary.turns.len(), 2);
        assert_eq!(summary.turns[0].parts.len(), 3);
        assert_eq!(summary.turns[1].parts[1].kind, Kind::Assistant);
        assert!(summary.partial);
        assert!(!serde_json::to_string(&summary).unwrap().contains("private"));
    }

    #[test]
    fn retains_only_recent_turns_without_losing_totals() {
        let summary = RequestContextSummary::from_responses(
            &json!({"input": vec![json!({"role":"user", "content":"abcd"}); 30]}),
        )
        .unwrap();
        assert_eq!(summary.parts[0].items, 30);
        assert_eq!(summary.turns[0].number, 7);
        assert_eq!(summary.omitted_turns, 6);
        assert!(RequestContextSummary::from_responses(&json!({"model":"x"})).is_none());
    }
}
