use codex_plus_core::models::{ImageOutput, ImageOutputResult, ImageOutputStatus, SessionRef};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const MAX_IMAGES: usize = 8;
const MAX_BASE64_CHARS: usize = 24 * 1024 * 1024;

pub fn image_outputs_from_paths(
    db_paths: impl IntoIterator<Item = PathBuf>,
    session: &SessionRef,
) -> ImageOutputResult {
    let thread_id = normalize_session_id(&session.session_id);
    let mut result = failed(&thread_id, "未找到对应会话");
    let mut saw_candidate = false;
    for db_path in db_paths {
        if !db_path.exists() {
            continue;
        }
        saw_candidate = true;
        let candidate = ImageOutputService::new(Some(db_path)).load(session);
        if matches!(candidate.status, ImageOutputStatus::Found) {
            if !candidate.images.is_empty() {
                return candidate;
            }
            result = candidate;
            continue;
        }
        if result.message == "未找到对应会话" || candidate.message != "未找到对应会话"
        {
            result = candidate;
        }
    }
    if saw_candidate {
        result
    } else {
        failed(&thread_id, "未配置本地 Codex 数据库")
    }
}

#[derive(Debug, Clone)]
pub struct ImageOutputService {
    db_path: Option<PathBuf>,
}

impl ImageOutputService {
    pub fn new(db_path: Option<impl Into<PathBuf>>) -> Self {
        Self {
            db_path: db_path.map(Into::into),
        }
    }

    pub fn load(&self, session: &SessionRef) -> ImageOutputResult {
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
        let result = (|| -> anyhow::Result<ImageOutputResult> {
            let db = Connection::open(db_path)?;
            let record = match lookup_thread_record(&db, db_path, &thread_id)? {
                ThreadLookup::Found(record) => record,
                ThreadLookup::Missing => return Ok(failed(&thread_id, "未找到对应会话")),
                ThreadLookup::Unsupported => {
                    return Ok(failed(&thread_id, "不支持当前本地存储结构"));
                }
            };
            let Some(rollout_path) = record
                .rollout_path
                .filter(|path| !path.as_os_str().is_empty())
            else {
                return Ok(failed(&thread_id, "会话缺少 rollout 文件路径"));
            };
            if !rollout_path.is_file() {
                return Ok(failed(
                    &thread_id,
                    format!("rollout 文件不存在：{}", rollout_path.to_string_lossy()),
                ));
            }
            let images = load_image_outputs(&rollout_path)?;
            let message = if images.is_empty() {
                "未找到图片生成结果".to_string()
            } else {
                format!("已找到 {} 张图片生成结果", images.len())
            };
            Ok(ImageOutputResult {
                status: ImageOutputStatus::Found,
                session_id: thread_id.clone(),
                message,
                images,
            })
        })();
        result.unwrap_or_else(|err| failed(&thread_id, format!("读取 rollout 失败：{err}")))
    }
}

#[derive(Debug)]
struct ThreadRecord {
    rollout_path: Option<PathBuf>,
}

#[derive(Debug)]
enum ThreadLookup {
    Found(ThreadRecord),
    Missing,
    Unsupported,
}

fn failed(session_id: &str, message: impl Into<String>) -> ImageOutputResult {
    ImageOutputResult {
        status: ImageOutputStatus::Failed,
        session_id: session_id.to_string(),
        message: message.into(),
        images: Vec::new(),
    }
}

fn lookup_thread_record(
    db: &Connection,
    db_path: &Path,
    thread_id: &str,
) -> anyhow::Result<ThreadLookup> {
    if has_columns(db, "threads", &["id", "rollout_path"])? {
        let row = db.query_row(
            "SELECT rollout_path FROM threads WHERE id = ?1",
            [thread_id],
            |row| {
                Ok(ThreadRecord {
                    rollout_path: row
                        .get::<_, Option<String>>(0)?
                        .filter(|path| !path.trim().is_empty())
                        .map(PathBuf::from),
                })
            },
        );
        return match row {
            Ok(row) => Ok(ThreadLookup::Found(row)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(ThreadLookup::Missing),
            Err(err) => Err(err.into()),
        };
    }

    if has_columns(db, "automation_runs", &["thread_id"])? {
        let row = db.query_row(
            "SELECT 1 FROM automation_runs WHERE thread_id = ?1",
            [thread_id],
            |_| Ok(()),
        );
        return match row {
            Ok(()) => Ok(ThreadLookup::Found(ThreadRecord {
                rollout_path: discover_rollout_path(db_path, thread_id)?,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(ThreadLookup::Missing),
            Err(err) => Err(err.into()),
        };
    }

    Ok(ThreadLookup::Unsupported)
}

fn discover_rollout_path(db_path: &Path, thread_id: &str) -> anyhow::Result<Option<PathBuf>> {
    for home in codex_home_candidates(db_path) {
        let mut candidates = Vec::new();
        collect_jsonl_files(&home.join("sessions"), &mut candidates)?;
        collect_jsonl_files(&home.join("archived_sessions"), &mut candidates)?;
        candidates.sort_by_key(|path| {
            std::cmp::Reverse(
                fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .ok(),
            )
        });
        for path in candidates {
            if rollout_matches_thread(&path, thread_id)? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn codex_home_candidates(db_path: &Path) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    for ancestor in db_path.ancestors().skip(1) {
        if ancestor.join("sessions").is_dir() || ancestor.join("archived_sessions").is_dir() {
            homes.push(ancestor.to_path_buf());
        }
    }
    if homes.is_empty() {
        if let Some(parent) = db_path.parent().and_then(Path::parent) {
            homes.push(parent.to_path_buf());
        }
    }
    homes
}

fn collect_jsonl_files(dir: &Path, output: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            collect_jsonl_files(&path, output)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl") {
            output.push(path);
        }
    }
    Ok(())
}

fn rollout_matches_thread(path: &Path, thread_id: &str) -> anyhow::Result<bool> {
    for raw in fs::read_to_string(path)?.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<Value>(raw) else {
            continue;
        };
        if event.get("type") != Some(&Value::String("session_meta".to_string())) {
            continue;
        }
        let id = event
            .get("payload")
            .and_then(|payload| payload.get("id"))
            .or_else(|| event.get("id"))
            .and_then(Value::as_str)
            .map(normalize_session_id)
            .unwrap_or_default();
        if id == thread_id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_columns(db: &Connection, table: &str, columns: &[&str]) -> anyhow::Result<bool> {
    let existing = table_columns(db, table)?;
    if existing.is_empty() {
        return Ok(false);
    }
    Ok(columns
        .iter()
        .all(|column| existing.iter().any(|existing| existing == column)))
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    if !db
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok()
    {
        return Ok(Vec::new());
    }
    let mut stmt = db.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    let columns = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(columns.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_image_outputs(path: &Path) -> anyhow::Result<Vec<ImageOutput>> {
    let mut assistant_text_by_turn: HashMap<String, String> = HashMap::new();
    let mut seen = HashSet::new();
    let mut images = Vec::new();
    for raw in fs::read_to_string(path)?.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(raw)?;
        if event.get("type") != Some(&Value::String("response_item".to_string())) {
            continue;
        }
        let payload = &event["payload"];
        if payload.get("type") == Some(&Value::String("message".to_string()))
            && payload.get("role") == Some(&Value::String("assistant".to_string()))
        {
            if let (Some(turn_id), Some(text)) = (
                turn_id_from_payload(payload),
                assistant_text_from_payload(payload),
            ) {
                assistant_text_by_turn.insert(turn_id.to_string(), text);
            }
            continue;
        }
        if payload.get("type") != Some(&Value::String("image_generation_call".to_string())) {
            continue;
        }
        let Some(result) = payload.get("result").and_then(Value::as_str) else {
            continue;
        };
        let result = result.trim();
        if result.is_empty() || result.len() > MAX_BASE64_CHARS {
            continue;
        }
        let id = payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("image-output-{}", images.len() + 1));
        if !seen.insert(id.clone()) {
            continue;
        }
        let output_format = payload
            .get("output_format")
            .and_then(Value::as_str)
            .map(normalize_image_format)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| infer_image_format(result).to_string());
        let data_url = image_data_url(result, &output_format);
        images.push(ImageOutput {
            id,
            turn_id: turn_id_from_payload(payload).map(ToString::to_string),
            assistant_text: None,
            data_url,
            output_format,
            revised_prompt: payload
                .get("revised_prompt")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string),
            timestamp: event
                .get("timestamp")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(ToString::to_string),
        });
    }
    if images.len() > MAX_IMAGES {
        images = images.split_off(images.len() - MAX_IMAGES);
    }
    for image in images.iter_mut() {
        image.assistant_text = image
            .turn_id
            .as_ref()
            .and_then(|turn_id| assistant_text_by_turn.get(turn_id))
            .cloned();
    }
    Ok(images)
}

fn turn_id_from_payload(payload: &Value) -> Option<&str> {
    payload
        .get("internal_chat_message_metadata_passthrough")
        .and_then(|metadata| metadata.get("turn_id"))
        .and_then(Value::as_str)
}

fn assistant_text_from_payload(payload: &Value) -> Option<String> {
    let text = payload
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn image_data_url(result: &str, output_format: &str) -> String {
    if result.starts_with("data:image/") {
        return result.to_string();
    }
    format!("data:image/{output_format};base64,{result}")
}

fn infer_image_format(result: &str) -> &'static str {
    if result.starts_with("iVBORw0KGgo") {
        "png"
    } else if result.starts_with("/9j/") {
        "jpeg"
    } else if result.starts_with("UklGR") {
        "webp"
    } else if result.starts_with("R0lGOD") {
        "gif"
    } else {
        "png"
    }
}

fn normalize_image_format(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("image/")
        .to_ascii_lowercase()
        .replace("jpg", "jpeg")
}

fn normalize_session_id(session_id: &str) -> String {
    session_id
        .strip_prefix("local:")
        .unwrap_or(session_id)
        .to_string()
}
