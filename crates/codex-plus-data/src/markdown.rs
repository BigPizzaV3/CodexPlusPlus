use codex_plus_core::models::{ExportResult, ExportStatus, SessionRef};
use rusqlite::Connection;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct MarkdownExportService {
    db_path: Option<PathBuf>,
}

impl MarkdownExportService {
    pub fn new(db_path: Option<impl Into<PathBuf>>) -> Self {
        Self {
            db_path: db_path.map(Into::into),
        }
    }

    pub fn export(&self, session: &SessionRef) -> ExportResult {
        let Some(db_path) = &self.db_path else {
            return failed(&session.session_id, "未配置本地 Codex 数据库");
        };
        if !db_path.exists() {
            return failed(
                &session.session_id,
                format!("数据库不存在：{}", db_path.to_string_lossy()),
            );
        }
        let thread_id = normalize_session_id(&session.session_id);
        let result = (|| -> anyhow::Result<ExportResult> {
            let db = Connection::open(db_path)?;
            if !supports_codex_threads(&db)? {
                return Ok(failed(&thread_id, "不支持当前本地存储结构"));
            }
            let row = db.query_row(
                "SELECT id, title, rollout_path FROM threads WHERE id = ?1",
                [&thread_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            );
            let (_, title, rollout_path) = match row {
                Ok(row) => row,
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Ok(failed(&thread_id, "未找到对应会话"));
                }
                Err(err) => return Err(err.into()),
            };
            let title = display_title(title.as_deref().unwrap_or(&session.title));
            let Some(rollout_path) = rollout_path.filter(|path| !path.is_empty()) else {
                return Ok(failed(&thread_id, "会话缺少 rollout 文件路径"));
            };
            if !Path::new(&rollout_path).is_file() {
                return Ok(failed(
                    &thread_id,
                    format!("rollout 文件不存在：{rollout_path}"),
                ));
            }
            let messages = load_messages(Path::new(&rollout_path))?;
            if messages.is_empty() {
                return Ok(failed(&thread_id, "未找到可导出的用户或助手消息"));
            }
            let filename = build_filename(&title, &thread_id);
            let markdown = render_markdown(&title, &messages);
            Ok(ExportResult {
                status: ExportStatus::Exported,
                session_id: thread_id.clone(),
                message: format!("已导出为 Markdown：{filename}"),
                filename: Some(filename),
                markdown: Some(markdown),
            })
        })();
        result.unwrap_or_else(|err| failed(&thread_id, format!("读取 rollout 失败：{err}")))
    }
}

#[derive(Debug)]
struct Message {
    speaker: &'static str,
    timestamp: Option<String>,
    body: String,
}

fn failed(session_id: &str, message: impl Into<String>) -> ExportResult {
    ExportResult {
        status: ExportStatus::Failed,
        session_id: session_id.to_string(),
        message: message.into(),
        filename: None,
        markdown: None,
    }
}

fn supports_codex_threads(db: &Connection) -> anyhow::Result<bool> {
    let has_threads = db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    if !has_threads {
        return Ok(false);
    }
    let mut stmt = db.prepare("PRAGMA table_info(\"threads\")")?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(["id", "title", "rollout_path"]
        .iter()
        .all(|column| columns.iter().any(|existing| existing == column)))
}

fn load_messages(path: &Path) -> anyhow::Result<Vec<Message>> {
    let mut messages = Vec::new();
    for raw in fs::read_to_string(path)?.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(raw)?;
        if event.get("type") != Some(&Value::String("response_item".to_string())) {
            continue;
        }
        let payload = &event["payload"];
        if payload.get("type") != Some(&Value::String("message".to_string())) {
            continue;
        }
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        let speaker = match role {
            "user" => "User",
            "assistant" => "Assistant",
            _ => continue,
        };
        let body = serialize_message_content(&payload["content"]);
        if body.is_empty() {
            continue;
        }
        messages.push(Message {
            speaker,
            timestamp: format_timestamp(event.get("timestamp")),
            body,
        });
    }
    Ok(messages)
}

fn serialize_message_content(content: &Value) -> String {
    let Some(items) = content.as_array() else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type").and_then(Value::as_str)?;
            match block_type {
                "input_text" | "output_text" => {
                    let text =
                        normalize_newlines(block.get("text").and_then(Value::as_str).unwrap_or(""))
                            .trim_matches('\n')
                            .to_string();
                    (!text.trim().is_empty()).then_some(text)
                }
                "input_image" => {
                    let image_url = block
                        .get("image_url")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim();
                    if image_url.is_empty() || image_url.starts_with("data:") {
                        Some("> Image attachment".to_string())
                    } else {
                        Some(format!("> Image attachment\n[Image link](<{image_url}>)"))
                    }
                }
                _ => None,
            }
        })
        .filter(|block| !block.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

fn format_timestamp(value: Option<&Value>) -> Option<String> {
    let raw = value?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw
        .strip_suffix('Z')
        .map_or_else(|| raw.to_string(), |prefix| format!("{prefix}+00:00"));
    let parsed = chrono::DateTime::parse_from_rfc3339(&normalized).ok()?;
    Some(
        parsed
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string(),
    )
}

fn display_title(value: &str) -> String {
    let normalized = normalize_newlines(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        "Untitled session".to_string()
    } else {
        normalized
    }
}

fn build_filename(title: &str, thread_id: &str) -> String {
    let cleaned = collapse_whitespace(&replace_windows_filename_chars(title, " "))
        .trim_matches([' ', '.'])
        .to_string();
    let mut safe_title = cleaned
        .chars()
        .take(80)
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_string();
    if safe_title.is_empty() {
        safe_title = "Untitled session".to_string();
    }
    let safe_thread_id = replace_windows_filename_chars(thread_id, "-");
    format!("{safe_title}-{}.md", safe_thread_id.trim())
}

fn render_markdown(title: &str, messages: &[Message]) -> String {
    let mut lines = vec![format!("# {title}"), String::new()];
    for message in messages {
        lines.push(format!("### {}", message.speaker));
        if let Some(timestamp) = &message.timestamp {
            lines.push(format!("_{timestamp}_"));
        }
        lines.push(String::new());
        lines.push(message.body.trim_end().to_string());
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n").trim_end())
}

fn normalize_session_id(session_id: &str) -> String {
    session_id
        .strip_prefix("local:")
        .unwrap_or(session_id)
        .to_string()
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

fn replace_windows_filename_chars(value: &str, replacement: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control() {
            output.push_str(replacement);
        } else {
            output.push(ch);
        }
    }
    output
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
