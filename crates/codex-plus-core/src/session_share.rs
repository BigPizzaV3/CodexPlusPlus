use anyhow::{Context, bail};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAX_ROLLOUT_BYTES: usize = 16 * 1024 * 1024;

pub fn export_rollout(home: &Path, session_id: &str) -> anyhow::Result<Value> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        bail!("会话 ID 为空");
    }
    let path = find_rollout(home, session_id).ok_or_else(|| anyhow::anyhow!("找不到原生会话文件"))?;
    let bytes = fs::read(&path).with_context(|| format!("读取会话文件失败：{}", path.display()))?;
    if bytes.len() > MAX_ROLLOUT_BYTES {
        bail!("会话文件超过分享大小限制");
    }
    let content = String::from_utf8(bytes).context("会话文件不是有效的 UTF-8")?;
    Ok(json!({
        "status": "ok",
        "kind": "codex-rollout",
        "session_id": session_id,
        "content": content,
        "filename": path.file_name().and_then(|value| value.to_str()).unwrap_or("rollout.jsonl"),
    }))
}

pub fn import_rollout(home: &Path, payload: &Value) -> anyhow::Result<Value> {
    if payload.get("kind").and_then(Value::as_str) != Some("codex-rollout") {
        bail!("不支持的会话文件格式");
    }
    let source_id = payload.get("session_id").and_then(Value::as_str).unwrap_or_default();
    let content = payload.get("content").and_then(Value::as_str).unwrap_or_default();
    let title = payload.get("title").and_then(Value::as_str).unwrap_or("导入的会话").trim();
    if source_id.is_empty() || content.is_empty() {
        bail!("会话文件内容不完整");
    }
    if content.len() > MAX_ROLLOUT_BYTES {
        bail!("会话文件超过导入大小限制");
    }
    let new_id = Uuid::new_v4().to_string();
    let rewritten = rewrite_rollout(content, source_id, &new_id)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let directory = home.join("sessions").join("imported");
    fs::create_dir_all(&directory).context("创建会话目录失败")?;
    let path = directory.join(format!("rollout-{now}-{new_id}.jsonl"));
    fs::write(&path, rewritten).context("写入导入会话失败")?;

    let index_path = home.join("session_index.jsonl");
    let mut index = OpenOptions::new().create(true).append(true).open(&index_path).context("打开会话索引失败")?;
    writeln!(index, "{}", serde_json::to_string(&json!({
        "id": new_id,
        "thread_name": if title.is_empty() { "导入的会话" } else { title },
        "updated_at": now.to_string(),
    }))?).context("更新会话索引失败")?;

    Ok(json!({ "status": "ok", "session_id": new_id, "title": if title.is_empty() { "导入的会话" } else { title } }))
}

fn find_rollout(home: &Path, session_id: &str) -> Option<PathBuf> {
    for root in [home.join("sessions"), home.join("archived_sessions")] {
        if let Some(path) = find_rollout_in(&root, session_id) {
            return Some(path);
        }
    }
    None
}

fn find_rollout_in(root: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_rollout_in(&path, session_id) {
                return Some(found);
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            && path.file_name().and_then(|value| value.to_str()).is_some_and(|name| name.contains(session_id))
        {
            return Some(path);
        }
    }
    None
}

fn rewrite_rollout(content: &str, old_id: &str, new_id: &str) -> anyhow::Result<String> {
    let mut lines = Vec::new();
    for line in content.lines() {
        let mut value: Value = serde_json::from_str(line).context("会话文件包含无效 JSON 行")?;
        replace_id(&mut value, old_id, new_id);
        lines.push(serde_json::to_string(&value)?);
    }
    if lines.is_empty() {
        bail!("会话文件没有内容");
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn replace_id(value: &mut Value, old_id: &str, new_id: &str) {
    match value {
        Value::String(text) if text == old_id => *text = new_id.to_string(),
        Value::Array(items) => items.iter_mut().for_each(|item| replace_id(item, old_id, new_id)),
        Value::Object(object) => object.values_mut().for_each(|item| replace_id(item, old_id, new_id)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_ids_in_rollout_lines() {
        let content = r#"{"type":"session_meta","payload":{"id":"old","session_id":"old"}}"#;
        let rewritten = rewrite_rollout(content, "old", "new").unwrap();
        assert!(rewritten.contains("\"id\":\"new\""));
        assert!(rewritten.contains("\"session_id\":\"new\""));
    }
}
