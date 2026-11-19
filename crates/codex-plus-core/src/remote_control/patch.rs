//! 把 Codex 桌面推送的会话补丁应用到本地副本上。
//!
//! owner 广播 `thread-stream-state-changed` 时，增量是 immer 生成的补丁：
//! `{ op: "add" | "replace" | "remove", path: [字符串或数字…], value }`，路径用数组。
//! 数组截断以 `{ op: "replace", path: [..., "length"], value: n }` 表示。

use anyhow::{Context, bail};
use serde_json::{Map, Value};

/// 依次应用补丁。中途失败时副本可能已部分修改，调用方应丢弃副本并重新要快照。
pub fn apply_patches(state: &mut Value, patches: &[Value]) -> anyhow::Result<()> {
    for patch in patches {
        apply_patch(state, patch)?;
    }
    Ok(())
}

fn apply_patch(state: &mut Value, patch: &Value) -> anyhow::Result<()> {
    let op = patch.get("op").and_then(Value::as_str).context("补丁缺少 op")?;
    let path = patch.get("path").and_then(Value::as_array).context("补丁缺少 path")?;
    let value = patch.get("value").cloned().unwrap_or(Value::Null);
    let Some((last, parents)) = path.split_last() else {
        if op != "remove" {
            *state = value;
        }
        return Ok(());
    };
    let mut parent = state;
    for (index, key) in parents.iter().enumerate() {
        let next_is_index = path.get(index + 1).is_some_and(Value::is_u64);
        parent = child_mut(parent, key, next_is_index)?;
    }
    match parent {
        Value::Array(items) => apply_to_array(items, op, last, value),
        Value::Object(map) => {
            let key = key_text(last)?;
            if op == "remove" {
                map.remove(&key);
            } else {
                map.insert(key, value);
            }
            Ok(())
        }
        other => bail!("补丁路径的父节点不是容器：{other}"),
    }
}

fn apply_to_array(items: &mut Vec<Value>, op: &str, key: &Value, value: Value) -> anyhow::Result<()> {
    if key.as_str() == Some("length") {
        if op == "replace" {
            let length = value.as_u64().context("数组 length 不是数字")? as usize;
            items.resize(length, Value::Null);
        }
        return Ok(());
    }
    let index = array_index(key)?;
    match op {
        "remove" => {
            if index < items.len() {
                items.remove(index);
            }
        }
        "add" => {
            if index > items.len() {
                bail!("数组插入越界：{index} > {}", items.len());
            }
            items.insert(index, value);
        }
        _ => {
            if index < items.len() {
                items[index] = value;
            } else if index == items.len() {
                items.push(value);
            } else {
                bail!("数组替换越界：{index} > {}", items.len());
            }
        }
    }
    Ok(())
}

/// 取子节点；缺失时按下一段路径的类型补一个空容器（immer 不会这样发，只是防御）。
fn child_mut<'a>(parent: &'a mut Value, key: &Value, next_is_index: bool) -> anyhow::Result<&'a mut Value> {
    let empty = || if next_is_index { Value::Array(Vec::new()) } else { Value::Object(Map::new()) };
    match parent {
        Value::Object(map) => {
            let entry = map.entry(key_text(key)?).or_insert_with(empty);
            if entry.is_null() {
                *entry = empty();
            }
            Ok(entry)
        }
        Value::Array(items) => {
            let index = array_index(key)?;
            items.get_mut(index).with_context(|| format!("数组下标越界：{index}"))
        }
        other => bail!("补丁路径经过了非容器节点：{other}"),
    }
}

fn key_text(key: &Value) -> anyhow::Result<String> {
    match key {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        other => bail!("补丁路径段无效：{other}"),
    }
}

fn array_index(key: &Value) -> anyhow::Result<usize> {
    match key {
        Value::Number(number) => number.as_u64().map(|index| index as usize).context("数组下标无效"),
        Value::String(text) => text.parse().context("数组下标无效"),
        other => bail!("数组下标无效：{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply(state: Value, patches: Value) -> Value {
        let mut state = state;
        apply_patches(&mut state, patches.as_array().unwrap()).unwrap();
        state
    }

    #[test]
    fn replaces_nested_object_fields() {
        let state = apply(
            json!({ "threadRuntimeStatus": { "type": "idle" }, "title": "a" }),
            json!([
                { "op": "replace", "path": ["threadRuntimeStatus"], "value": { "type": "active", "activeFlags": [] } },
                { "op": "replace", "path": ["title"], "value": "b" },
            ]),
        );
        assert_eq!(state["threadRuntimeStatus"]["type"], "active");
        assert_eq!(state["title"], "b");
    }

    #[test]
    fn inserts_and_replaces_array_items() {
        let state = apply(
            json!({ "items": [{ "id": 1 }] }),
            json!([
                { "op": "add", "path": ["items", 1], "value": { "id": 2, "text": "" } },
                { "op": "replace", "path": ["items", 1, "text"], "value": "收到" },
                { "op": "add", "path": ["items", 0], "value": { "id": 0 } },
            ]),
        );
        assert_eq!(state["items"], json!([{ "id": 0 }, { "id": 1 }, { "id": 2, "text": "收到" }]));
    }

    #[test]
    fn removes_keys_and_items_and_truncates_by_length() {
        let state = apply(
            json!({ "done": { "a": false, "b": true }, "list": [1, 2, 3, 4] }),
            json!([
                { "op": "remove", "path": ["done", "a"] },
                { "op": "remove", "path": ["list", 0] },
                { "op": "replace", "path": ["list", "length"], "value": 2 },
            ]),
        );
        assert_eq!(state, json!({ "done": { "b": true }, "list": [2, 3] }));
    }

    #[test]
    fn creates_missing_entity_by_string_key() {
        let state = apply(
            json!({ "turnHistory": { "history": { "entitiesByKey": {} } } }),
            json!([{ "op": "add", "path": ["turnHistory", "history", "entitiesByKey", "tail:0:local:x"], "value": { "items": [] } }]),
        );
        assert_eq!(state["turnHistory"]["history"]["entitiesByKey"]["tail:0:local:x"], json!({ "items": [] }));
    }

    #[test]
    fn empty_path_replaces_whole_state() {
        assert_eq!(apply(json!({ "a": 1 }), json!([{ "op": "replace", "path": [], "value": { "b": 2 } }])), json!({ "b": 2 }));
    }

    #[test]
    fn rejects_out_of_range_insert() {
        let mut state = json!({ "items": [] });
        let error = apply_patches(&mut state, &[json!({ "op": "add", "path": ["items", 3], "value": 1 })]).unwrap_err();
        assert!(error.to_string().contains("越界"));
    }
}
