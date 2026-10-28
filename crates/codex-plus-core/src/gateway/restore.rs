//! 接入网关前 Agent 配置的存档，与断开 / 退出时的还原。
//!
//! 接入时记两样：接入前的文件原文，和网关这次改了哪些键（键路径，最多两层，例如
//! `model_providers.codex-plus-gateway`、`env.ANTHROPIC_BASE_URL`）。之后网关每次重写配置，
//! 把那次改到的键也记进来。还原时只把这些键改回接入前的值，接入期间的其它改动
//! （Codex 加的 MCP、Claude Code 写的权限……）都保留；没有其它改动时直接写回原文，一字不差。
//!
//! 存档放在备份目录（`<状态目录>/backups/<agent>-before-gateway.json`），里面有凭据，只给本人读写。

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 配置里的一个键：从根开始的键名序列，最多 [`MAX_DEPTH`] 层。
pub type KeyPath = Vec<String>;

/// 键路径最多记到第二层：`[model_providers.x]`、`[mcp_servers.x]`、`env.X` 这一级。
const MAX_DEPTH: usize = 2;

/// 网关改过的一个键，以及网关写进去的值（还原 auth.json 时用来判断它之后有没有被 Codex 改过）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedKey {
    pub path: KeyPath,
    /// 网关写进去的值；网关删掉了这个键时为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<Value>,
}

/// 一个配置文件接入网关前的存档。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStash {
    /// 接入前的原文；文件不存在时为 None。
    #[serde(default)]
    pub before: Option<String>,
    /// 网关改过的键。
    #[serde(default)]
    pub changed: Vec<ChangedKey>,
}

impl FileStash {
    pub fn new(before: Option<String>) -> Self {
        Self {
            before,
            changed: Vec::new(),
        }
    }

    /// 记下一次写入改到的键（`before` / `after` 是那次写入前后的内容，已转成 JSON 值）。
    /// 同一个键再次被改时，更新网关写进去的值。
    pub fn record_write(&mut self, before: &Value, after: &Value) {
        for path in changed_paths(before, after) {
            let written = value_at(after, &path).cloned();
            match self.changed.iter_mut().find(|known| known.path == path) {
                Some(known) => known.written = written,
                None => self.changed.push(ChangedKey { path, written }),
            }
        }
    }

    pub fn paths(&self) -> Vec<KeyPath> {
        self.changed.iter().map(|key| key.path.clone()).collect()
    }
}

/// 读存档；不存在或读不出来时为 None。
pub fn load_stash<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}

/// 写存档（只给本人读写，里面有 Key / token）。
pub fn save_stash<T: Serialize>(path: &Path, stash: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    crate::settings::atomic_write(path, serde_json::to_string_pretty(stash)?.as_bytes())
        .with_context(|| format!("无法写入接入前的存档 {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 删存档；本来就没有不算错。
pub fn remove_stash(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// 读一个可能不存在的文件。
pub fn read_optional(path: &Path) -> anyhow::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}

/// 写入内容；`None` 表示文件本来就不存在，删掉它。
pub fn write_or_remove(path: &Path, contents: Option<&str>) -> anyhow::Result<()> {
    match contents {
        Some(contents) => crate::settings::atomic_write(path, contents.as_bytes())
            .with_context(|| format!("无法写入 {}", path.display())),
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("无法删除 {}", path.display())),
        },
    }
}

/// TOML 文本 → JSON 值（只用来比较内容，不管格式）。空文本 / 不存在视作空表。
pub fn toml_value(text: Option<&str>) -> anyhow::Result<Value> {
    let text = text.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: toml::Value =
        toml::from_str(text).map_err(|error| toml_syntax_error("配置", text, error.span()))?;
    Ok(serde_json::to_value(value)?)
}

/// TOML 解析失败的错误：只报出错的行列，不带出错那一行的原文。
///
/// toml / toml_edit 的错误信息会把出错的那行原文带出来，那一行可能就是 Key、token；
/// 这些错误会写进诊断日志，所以统一换成「第几行第几列」。`what` 是出错的文件（或「配置」）。
pub fn toml_syntax_error(
    what: &str,
    text: &str,
    span: Option<std::ops::Range<usize>>,
) -> anyhow::Error {
    let Some(span) = span else {
        return anyhow::anyhow!("{what} 不是合法的 TOML");
    };
    let bytes = text.as_bytes();
    let before = &bytes[..span.start.min(bytes.len())];
    let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
    let line_start = before
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let column = String::from_utf8_lossy(&before[line_start..])
        .chars()
        .count()
        + 1;
    anyhow::anyhow!("{what} 第 {line} 行第 {column} 列不是合法的 TOML")
}

/// JSON 文本 → JSON 值。空文本 / 不存在视作空对象。
pub fn json_value(text: Option<&str>) -> anyhow::Result<Value> {
    let text = text.unwrap_or_default();
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    serde_json::from_str(text).context("配置不是合法的 JSON")
}

/// 两份内容里取值不同的键路径（两边都是表时往下一层，最多 [`MAX_DEPTH`] 层）。
pub fn changed_paths(before: &Value, after: &Value) -> Vec<KeyPath> {
    let mut out = Vec::new();
    walk(Some(before), Some(after), &mut Vec::new(), &mut out);
    out
}

fn walk(
    before: Option<&Value>,
    after: Option<&Value>,
    prefix: &mut KeyPath,
    out: &mut Vec<KeyPath>,
) {
    if before == after {
        return;
    }
    let both_objects = matches!(
        (before, after),
        (Some(Value::Object(_)) | None, Some(Value::Object(_)) | None)
    );
    if prefix.len() < MAX_DEPTH && both_objects {
        let empty = Map::new();
        let before = before.and_then(Value::as_object).unwrap_or(&empty);
        let after = after.and_then(Value::as_object).unwrap_or(&empty);
        let mut keys = before.keys().collect::<Vec<_>>();
        for key in after.keys() {
            if !before.contains_key(key) {
                keys.push(key);
            }
        }
        for key in keys {
            prefix.push(key.clone());
            walk(before.get(key), after.get(key), prefix, out);
            prefix.pop();
        }
        return;
    }
    out.push(prefix.clone());
}

/// 取 `value` 里路径上的值。
pub fn value_at<'a>(value: &'a Value, path: &[String]) -> Option<&'a Value> {
    path.iter().try_fold(value, |node, key| node.get(key))
}

/// 还原 TOML 配置：`current` 里 `paths` 上的键改回 `before` 的值（`before` 没有的删掉），其余不动。
/// 结果与 `before` 内容相同时直接返回 `before` 原文（`None` 表示文件本来就不存在）。
pub fn restore_toml(
    before: Option<&str>,
    current: Option<&str>,
    paths: &[KeyPath],
) -> anyhow::Result<Option<String>> {
    let before_doc = parse_toml(before)?;
    let mut doc = parse_toml(current)?;
    for path in paths {
        let original = toml_item_at(before_doc.as_item(), path).cloned();
        set_toml_path(&mut doc, &before_doc, path, original);
    }
    let restored = doc.to_string();
    if toml_value(Some(&restored))? == toml_value(before)? {
        return Ok(before.map(str::to_string));
    }
    Ok(Some(restored))
}

fn parse_toml(text: Option<&str>) -> anyhow::Result<toml_edit::DocumentMut> {
    let text = text.unwrap_or_default();
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|error| toml_syntax_error("配置", text, error.span()))
}

fn toml_item_at<'a>(item: &'a toml_edit::Item, path: &[String]) -> Option<&'a toml_edit::Item> {
    path.iter()
        .try_fold(item, |node, key| node.get(key.as_str()))
}

fn set_toml_path(
    doc: &mut toml_edit::DocumentMut,
    before_doc: &toml_edit::DocumentMut,
    path: &[String],
    value: Option<toml_edit::Item>,
) {
    match path {
        [] => {}
        [key] => match value {
            Some(item) => {
                doc.insert(key, item);
            }
            None => {
                doc.remove(key);
            }
        },
        [parent, key, ..] => {
            let parent_is_table = doc
                .get(parent)
                .is_some_and(|item| item.as_table_like().is_some());
            if !parent_is_table {
                // 父级不是表（被改成了别的类型）或不存在：整块用接入前的。
                match before_doc.get(parent) {
                    Some(item) if value.is_none() => {
                        doc.insert(parent, item.clone());
                    }
                    _ if value.is_none() => {}
                    _ => {
                        let mut table = toml_edit::Table::new();
                        table.set_implicit(true);
                        doc.insert(parent, toml_edit::Item::Table(table));
                    }
                }
            }
            let Some(table) = doc
                .get_mut(parent)
                .and_then(|item| item.as_table_like_mut())
            else {
                return;
            };
            match value {
                Some(item) => {
                    table.insert(key, item);
                }
                None => {
                    table.remove(key);
                }
            }
            // 删空了、接入前也没有这一节：整节去掉，不留一个空的表头。
            if table.is_empty() && before_doc.get(parent).is_none() {
                doc.remove(parent);
            }
        }
    }
}

/// 还原 JSON 配置，规则同 [`restore_toml`]。`keep` 返回 true 的键保留当前值。
pub fn restore_json(
    before: Option<&str>,
    current: Option<&str>,
    changed: &[ChangedKey],
    keep: impl Fn(&ChangedKey, &Value, &Value) -> bool,
) -> anyhow::Result<Option<String>> {
    let before_value = json_value(before)?;
    let mut value = json_value(current)?;
    if !value.is_object() {
        value = Value::Object(Map::new());
    }
    let mut modified = false;
    for key in changed {
        if keep(key, &before_value, &value) {
            continue;
        }
        let original = value_at(&before_value, &key.path).cloned();
        set_json_path(&mut value, &before_value, &key.path, original);
        modified = true;
    }
    if value == before_value {
        return Ok(before.map(str::to_string));
    }
    // 一个键都不用改：保持当前文件原样（不重排格式）。
    if !modified {
        return Ok(current.map(str::to_string));
    }
    if before.is_none() && value.as_object().is_some_and(Map::is_empty) {
        return Ok(None);
    }
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    Ok(Some(text))
}

fn set_json_path(value: &mut Value, before: &Value, path: &[String], original: Option<Value>) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    match path {
        [] => {}
        [key] => match original {
            Some(item) => {
                root.insert(key.clone(), item);
            }
            None => {
                root.remove(key);
            }
        },
        [parent, key, ..] => {
            if !root.get(parent).is_some_and(Value::is_object) {
                if original.is_none() {
                    if let Some(item) = before.get(parent) {
                        root.insert(parent.clone(), item.clone());
                    }
                    return;
                }
                root.insert(parent.clone(), Value::Object(Map::new()));
            }
            let Some(object) = root.get_mut(parent).and_then(Value::as_object_mut) else {
                return;
            };
            match original {
                Some(item) => {
                    object.insert(key.clone(), item);
                }
                None => {
                    object.remove(key);
                }
            }
            if object.is_empty() && before.get(parent).is_none() {
                root.remove(parent);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changed_paths_go_two_levels_deep() {
        let before = json!({ "model": "a", "keep": 1, "providers": { "x": { "url": "u" } } });
        let after = json!({ "model": "b", "keep": 1, "providers": { "x": { "url": "v" }, "gw": { "url": "g" } } });
        let mut paths = changed_paths(&before, &after);
        paths.sort();
        assert_eq!(
            paths,
            vec![
                vec!["model".to_string()],
                vec!["providers".to_string(), "gw".to_string()],
                vec!["providers".to_string(), "x".to_string()],
            ]
        );
    }

    #[test]
    fn toml_restore_reverts_only_changed_keys_and_keeps_new_sections() {
        let before = "model = \"a\"\n\n[model_providers.x]\nbase_url = \"https://x\"\n";
        let after = "model = \"gw\"\nmodel_provider = \"gw\"\n\n[model_providers.gw]\nbase_url = \"http://127.0.0.1/gw\"\n";
        let mut stash = FileStash::new(Some(before.to_string()));
        stash.record_write(
            &toml_value(Some(before)).unwrap(),
            &toml_value(Some(after)).unwrap(),
        );
        // 接入期间多了一个 MCP。
        let current = format!("{after}\n[mcp_servers.x]\ncommand = \"node\"\n");
        let restored = restore_toml(Some(before), Some(&current), &stash.paths())
            .unwrap()
            .unwrap();
        let value = toml_value(Some(&restored)).unwrap();
        assert_eq!(value["model"], "a");
        assert!(value.get("model_provider").is_none(), "{restored}");
        assert_eq!(value["model_providers"]["x"]["base_url"], "https://x");
        assert!(value["model_providers"].get("gw").is_none(), "{restored}");
        assert_eq!(value["mcp_servers"]["x"]["command"], "node");
        // 没有别的改动：原文写回。
        assert_eq!(
            restore_toml(Some(before), Some(after), &stash.paths()).unwrap(),
            Some(before.to_string())
        );
        // 接入前没有这个文件、之后也没有别的改动：删掉。
        let mut created = FileStash::new(None);
        created.record_write(
            &toml_value(None).unwrap(),
            &toml_value(Some(after)).unwrap(),
        );
        assert_eq!(
            restore_toml(None, Some(after), &created.paths()).unwrap(),
            None
        );
    }

    #[test]
    fn toml_errors_report_position_without_source_text() {
        let text = "model = \"a\"\napi_key = \"sk-secret-123\n";
        for error in [
            toml_value(Some(text)).unwrap_err(),
            restore_toml(Some(text), None, &[]).unwrap_err(),
        ] {
            let message = format!("{error:#}");
            assert!(!message.contains("sk-secret"), "{message}");
            assert!(message.contains("第 2 行"), "{message}");
        }
    }

    #[test]
    fn json_restore_respects_keep() {
        let before = r#"{"OPENAI_API_KEY":"sk-user"}"#;
        let after = r#"{"OPENAI_API_KEY":"cpx-gw-codex-1","auth_mode":"apikey"}"#;
        let mut stash = FileStash::new(Some(before.to_string()));
        stash.record_write(
            &json_value(Some(before)).unwrap(),
            &json_value(Some(after)).unwrap(),
        );
        let current = r#"{"OPENAI_API_KEY":"cpx-gw-codex-1","auth_mode":"chatgpt","tokens":{"access_token":"new"}}"#;
        // 网关写的值之后被改过的键保留当前值。
        let restored = restore_json(
            Some(before),
            Some(current),
            &stash.changed,
            |key, _, now| value_at(now, &key.path) != key.written.as_ref(),
        )
        .unwrap()
        .unwrap();
        let value: Value = serde_json::from_str(&restored).unwrap();
        assert_eq!(value["OPENAI_API_KEY"], "sk-user");
        assert_eq!(value["auth_mode"], "chatgpt");
        assert_eq!(value["tokens"]["access_token"], "new");
    }
}
