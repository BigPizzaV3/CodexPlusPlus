use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const MAX_REPLY_HISTORY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OfficialTask {
    pub id: String,
    pub name: String,
    pub workspace_name: String,
    #[serde(skip)]
    pub(super) cwd: PathBuf,
    #[serde(skip)]
    pub path: PathBuf,
    #[serde(skip)]
    updated_at: f64,
}

pub fn list_tasks(home: &Path) -> anyhow::Result<Vec<OfficialTask>> {
    query_tasks(home, None)
}

#[cfg(test)]
fn find_task(home: &Path, id: &str) -> anyhow::Result<Option<OfficialTask>> {
    Ok(find_tasks(home, &BTreeSet::from([id.to_owned()]))?
        .into_iter()
        .next())
}

pub(super) fn find_tasks(home: &Path, ids: &BTreeSet<String>) -> anyhow::Result<Vec<OfficialTask>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    query_tasks(home, Some(ids))
}

fn open_task_index(path: &Path) -> anyhow::Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // 同步是旁路读取，官方写入繁忙时立即让步，不等待锁或修改数据库配置。
    db.busy_timeout(std::time::Duration::ZERO)?;
    Ok(db)
}

fn query_tasks(home: &Path, ids: Option<&BTreeSet<String>>) -> anyhow::Result<Vec<OfficialTask>> {
    let mut tasks = BTreeMap::new();
    let mut available = false;
    let paths = crate::codex_sqlite::codex_db_candidate_paths_from_home(home);
    let selected = ids.map(serde_json::to_string).transpose()?;
    for path in &paths {
        if !path.is_file() {
            continue;
        }
        let db = open_task_index(path)?;
        let has_threads: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='threads')",
            [],
            |r| r.get(0),
        )?;
        if !has_threads {
            continue;
        }
        available = true;
        let mut statement = db
            .prepare(
                "SELECT id, title, cwd, rollout_path, updated_at FROM threads
             WHERE archived=0 AND rollout_path IS NOT NULL
               AND (?1 IS NULL OR id IN (SELECT value FROM json_each(?1)))
             ORDER BY updated_at DESC LIMIT 500",
            )
            .context("当前官方任务索引格式暂不支持")?;
        for task in statement.query_map([selected.as_deref()], |row| {
            let cwd: String = row.get(2)?;
            Ok(OfficialTask {
                id: row.get(0)?,
                name: clip(&row.get::<_, String>(1)?, 256),
                workspace_name: clip(
                    Path::new(&cwd)
                        .file_name()
                        .and_then(|v| v.to_str())
                        .unwrap_or("本地工作区"),
                    256,
                ),
                cwd: PathBuf::from(&cwd),
                path: PathBuf::from(row.get::<_, String>(3)?),
                updated_at: row.get(4)?,
            })
        })? {
            let task = task?;
            if super::opaque_id(&task.id) {
                match tasks.entry(task.id.clone()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(task);
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        if task.updated_at > entry.get().updated_at {
                            entry.insert(task);
                        }
                    }
                }
            }
        }
    }
    if ids.is_some() && !available {
        bail!("官方任务索引暂不可用");
    }
    apply_display_titles(&paths, selected.as_deref(), &mut tasks)?;
    let mut tasks: Vec<_> = tasks.into_values().collect();
    tasks.sort_by(|left, right| {
        right
            .updated_at
            .total_cmp(&left.updated_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(tasks)
}

fn apply_display_titles(
    paths: &[PathBuf],
    selected: Option<&str>,
    tasks: &mut BTreeMap<String, OfficialTask>,
) -> anyhow::Result<()> {
    // 桌面生成的标题保存在独立目录中；threads.title 可能仍是包含附件信息的首条消息。
    for path in paths {
        if !path.is_file() {
            continue;
        }
        let db = open_task_index(path)?;
        let has_catalog: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type='table' AND name='local_thread_catalog')",
            [],
            |row| row.get(0),
        )?;
        if !has_catalog {
            continue;
        }
        let mut statement = db.prepare(
            "SELECT thread_id, display_title FROM local_thread_catalog
             WHERE host_id='local' AND missing_candidate=0
               AND (?1 IS NULL OR thread_id IN (SELECT value FROM json_each(?1)))",
        )?;
        let titles = statement.query_map([selected], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for title in titles {
            let (task_id, title) = title?;
            // 目录仅补充显示名称，不扩大同步范围或替换已验证的会话路径。
            if let Some(task) = tasks.get_mut(&task_id)
                && !title.trim().is_empty()
            {
                task.name = clip(&title, 256);
            }
        }
    }
    Ok(())
}

pub(super) fn clip(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[derive(Default)]
pub(super) struct TaskReader {
    path: PathBuf,
    offset: u64,
    created: Option<std::time::SystemTime>,
    verified: bool,
    pub reply: String,
    reply_truncated: bool,
    pub model: String,
    pub outcome: String,
    pub last_item_id: String,
    turn_id: String,
    last_message_digest: Option<[u8; 32]>,
    last_message_source: &'static str,
}

impl TaskReader {
    pub fn validate(home: &Path, task: &OfficialTask) -> anyhow::Result<()> {
        let (_, file) = Self::open(home, task)?;
        let mut line = Vec::new();
        let read = BufReader::new(file)
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut line)?;
        if read > 8 * 1024 * 1024 || line.last() != Some(&b'\n') {
            bail!("任务记录尚未就绪");
        }
        let value: Value = serde_json::from_slice(&line).context("任务记录格式暂不支持")?;
        Self::default().apply(&value, &task.id)
    }

    fn open(home: &Path, task: &OfficialTask) -> anyhow::Result<(PathBuf, std::fs::File)> {
        let path = task.path.canonicalize().context("任务记录暂不可用")?;
        let allowed = ["sessions", "archived_sessions"].iter().any(|directory| {
            home.join(directory)
                .canonicalize()
                .is_ok_and(|root| path.starts_with(root))
        });
        if !allowed || path.extension().and_then(|v| v.to_str()) != Some("jsonl") {
            bail!("任务记录不在官方会话目录内");
        }
        let file = std::fs::File::open(&path).context("任务记录暂不可用")?;
        Ok((path, file))
    }

    pub fn read(&mut self, home: &Path, task: &OfficialTask) -> anyhow::Result<Value> {
        let (path, mut file) = Self::open(home, task)?;
        let metadata = file.metadata().context("任务记录暂不可用")?;
        let created = metadata.created().ok();
        if self.path != path || metadata.len() < self.offset || self.created != created {
            *self = Self {
                path,
                created,
                ..Self::default()
            };
        }
        file.seek(SeekFrom::Start(self.offset))
            .context("任务记录暂不可用")?;
        let initial_offset = self.offset;
        // 只消费本次打开时已有的内容；长历史分轮续读，不发布尚未追平的旧状态。
        let mut reader = BufReader::new(file.take(metadata.len() - self.offset));
        loop {
            if self.offset < metadata.len() && self.offset - initial_offset >= 1024 * 1024 {
                bail!("任务历史正在分批读取，稍后继续同步");
            }
            let mut line = Vec::new();
            // 工具输出可能很大；限制单行分配，且不消费尚未写完的行。
            let read = reader
                .by_ref()
                .take(8 * 1024 * 1024 + 1)
                .read_until(b'\n', &mut line)?;
            if read > 8 * 1024 * 1024 {
                bail!("任务记录单条内容过大，已暂停同步");
            }
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            let value: Value = serde_json::from_slice(&line).context("任务记录格式暂不支持")?;
            self.apply(&value, &task.id)?;
            self.offset += read as u64;
        }
        if !self.verified {
            bail!("任务记录尚未就绪");
        }
        let (task_status, turn_status, outcome) = match self.outcome.as_str() {
            "running" => ("running", "running", "none"),
            "completed" => ("stopped", "completed", "completed"),
            "interrupted" => ("stopped", "interrupted", "interrupted"),
            "failed" => ("failed", "failed", "failed"),
            _ => ("reconciling", "reconciling", "reconciling"),
        };
        let text = &self.reply;
        let state = if text.is_empty() {
            "absent"
        } else if self.reply_truncated {
            "truncated"
        } else {
            "available"
        };
        Ok(json!({
            "name": task.name, "workspaceName": task.workspace_name,
            "modelLabel": if self.model.is_empty() { "Codex" } else { &self.model },
            "taskStatus": task_status, "turnStatus": turn_status, "lastTurnOutcome": outcome,
            "lastReply": if text.is_empty() { Value::Null } else {
                json!({"state": state, "text": text, "byteLength": text.len(),
                    "truncated": self.reply_truncated})
            },
            "lastReplyState": state, "lastError": null, "pcConnectionState": "online"
        }))
    }

    fn apply(&mut self, value: &Value, expected_id: &str) -> anyhow::Result<()> {
        let payload = &value["payload"];
        let kind = value["type"].as_str().unwrap_or("");
        if !self.verified {
            if kind != "session_meta"
                || payload["id"] != expected_id
                || payload["source"].is_object()
            {
                bail!("任务身份不匹配或属于子任务，未同步");
            }
            self.verified = true;
            return Ok(());
        }
        match kind {
            "turn_context" => {
                if let Some(model) = payload["model"].as_str() {
                    self.model = clip(model, 256);
                }
            }
            "event_msg" => match payload["type"].as_str().unwrap_or("") {
                "task_started" => {
                    self.turn_id = payload["turn_id"].as_str().unwrap_or("").to_owned();
                    self.last_message_digest = None;
                    self.last_message_source = "";
                    self.outcome = "running".into();
                }
                "agent_message" => {
                    if self.matches_turn(payload)
                        && let Some(text) = payload["message"].as_str()
                    {
                        self.append_reply(text, "event_msg");
                    }
                }
                "task_complete" | "task_completed" => {
                    if !self.matches_turn(payload) {
                        return Ok(());
                    }
                    if let Some(text) = payload["last_agent_message"].as_str()
                        && self.last_message_digest != Some(reply_digest(text))
                    {
                        self.append_reply(text, "completion");
                    }
                    self.outcome = "completed".into();
                }
                "turn_aborted" => {
                    if self.matches_turn(payload) {
                        self.outcome = "interrupted".into();
                    }
                }
                _ => {}
            },
            "response_item"
                if payload["type"] == "message"
                    && payload["role"] == "assistant"
                    && payload["channel"]
                        .as_str()
                        .is_none_or(|channel| matches!(channel, "commentary" | "final")) =>
            {
                let text = payload["content"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter(|item| item["type"] == "output_text")
                            .filter_map(|item| item["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                if self.matches_turn(payload) {
                    self.last_item_id = payload["id"].as_str().unwrap_or("").to_owned();
                    self.append_reply(&text, "response_item");
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn append_reply(&mut self, text: &str, source: &'static str) {
        if text.is_empty() {
            return;
        }
        let digest = reply_digest(text);
        // 同一条可见回复常同时写入事件和响应记录；仅合并相邻的双写副本，不去掉真实重复回复。
        if self.last_message_digest == Some(digest)
            && matches!(
                (self.last_message_source, source),
                ("event_msg", "response_item") | ("response_item", "event_msg")
            )
        {
            self.last_message_source = "";
            return;
        }
        if !self.reply.is_empty() {
            self.reply.push_str("\n\n---\n\n");
        }
        self.reply.push_str(text);
        if retain_utf8_suffix(&mut self.reply, MAX_REPLY_HISTORY_BYTES) {
            self.reply_truncated = true;
        }
        self.last_message_digest = Some(digest);
        self.last_message_source = source;
    }

    fn matches_turn(&self, payload: &Value) -> bool {
        payload["turn_id"]
            .as_str()
            .is_none_or(|id| self.turn_id.is_empty() || id == self.turn_id)
    }

    pub(super) fn turn_id(&self) -> &str {
        &self.turn_id
    }
}

fn reply_digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

fn retain_utf8_suffix(text: &mut String, limit: usize) -> bool {
    if text.len() <= limit {
        return false;
    }
    let mut start = text.len() - limit;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    *text = text[start..].to_owned();
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "仅显式核验当前任务的官方记录，不上传或修改记录"]
    fn current_official_task_read_only() {
        let id = std::env::var("CODEX_THREAD_ID").expect("当前任务标识不可用");
        let home = crate::codex_home::default_codex_home_dir();
        let task = find_task(&home, &id)
            .unwrap()
            .expect("官方索引中没有当前任务");
        let snapshot = TaskReader::default().read(&home, &task).unwrap();
        assert!(snapshot["lastReplyState"].is_string());
        println!(
            "当前官方任务只读核验通过，回复字节数：{}",
            snapshot["lastReply"]["byteLength"].as_u64().unwrap_or(0)
        );
    }

    #[test]
    fn official_reply_preserves_unicode_and_ignores_tools_reasoning_and_stale_completion() {
        let mut reader = TaskReader::default();
        let id = "official_task_0001";
        for value in [
            json!({"type":"session_meta","payload":{"id":id,"source":"vscode"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"turn-two"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","output":"不应同步"}}),
            json!({"type":"event_msg","payload":{"type":"agent_reasoning","text":"不应同步"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"type":"output_text","text":"不应同步"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完整中文回复\n第二行"}]}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-one","last_agent_message":"旧回复"}}),
        ] {
            reader.apply(&value, id).unwrap();
        }
        assert_eq!(reader.reply, "完整中文回复\n第二行");
        assert_eq!(reader.outcome, "running");
        reader.apply(&json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-two"}}), id).unwrap();
        assert_eq!(reader.outcome, "completed");
    }

    #[test]
    fn partial_line_is_retried_and_new_turn_preserves_reply_history() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let path = sessions.join("task.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"session_meta","payload":{"id":"official_task_0001","source":"vscode"}})
        )
        .unwrap();
        let task = OfficialTask {
            id: "official_task_0001".into(),
            name: "任务".into(),
            workspace_name: "工作区".into(),
            cwd: PathBuf::from("workspace"),
            path,
            updated_at: 1.0,
        };
        let mut reader = TaskReader::default();
        write!(
            file,
            "{}",
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"回复"}})
        )
        .unwrap();
        assert_eq!(
            reader.read(dir.path(), &task).unwrap()["lastReplyState"],
            "absent"
        );
        writeln!(file).unwrap();
        assert_eq!(
            reader.read(dir.path(), &task).unwrap()["lastReply"]["byteLength"],
            6
        );
        writeln!(
            file,
            "{}",
            json!({"type":"event_msg","payload":{"type":"task_started"}})
        )
        .unwrap();
        assert_eq!(
            reader.read(dir.path(), &task).unwrap()["lastReply"]["text"],
            "回复"
        );
    }

    #[test]
    fn reply_history_preserves_turns_and_only_deduplicates_mirrored_messages() {
        let mut reader = TaskReader::default();
        let id = "official_task_0001";
        for value in [
            json!({"type":"session_meta","payload":{"id":id,"source":"vscode"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"one"}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"正在检查"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"正在检查"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"完成"}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"one","last_agent_message":"完成"}}),
            json!({"type":"event_msg","payload":{"type":"task_started","turn_id":"two"}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","turn_id":"one","message":"过期内容"}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"完成"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}}),
            json!({"type":"event_msg","payload":{"type":"agent_message","message":"完成"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"完成"}]}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"one","last_agent_message":"过期内容"}}),
            json!({"type":"event_msg","payload":{"type":"task_complete","turn_id":"two","last_agent_message":"完成"}}),
        ] {
            reader.apply(&value, id).unwrap();
        }
        assert_eq!(
            reader.reply,
            "正在检查\n\n---\n\n完成\n\n---\n\n完成\n\n---\n\n完成"
        );
        assert_eq!(reader.outcome, "completed");
    }

    #[test]
    fn reply_history_keeps_recent_two_megabytes_and_marks_truncated() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let path = sessions.join("task.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let id = "official_task_0001";
        let text = "完整回复".repeat(180_000);
        writeln!(
            file,
            "{}",
            json!({"type":"session_meta","payload":{"id":id,"source":"vscode"}})
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({"type":"event_msg","payload":{"type":"agent_message","message":text}})
        )
        .unwrap();
        let task = OfficialTask {
            id: id.into(),
            name: "任务".into(),
            workspace_name: "工作区".into(),
            cwd: PathBuf::from("workspace"),
            path,
            updated_at: 1.0,
        };
        let mut reader = TaskReader::default();
        let snapshot = reader.read(dir.path(), &task).unwrap();
        assert!(text.len() > 2 * 1024 * 1024);
        let retained = snapshot["lastReply"]["text"].as_str().unwrap();
        assert!(retained.len() <= MAX_REPLY_HISTORY_BYTES);
        assert!(text.ends_with(retained));
        assert_eq!(snapshot["lastReply"]["byteLength"], retained.len());
        assert_eq!(snapshot["lastReply"]["truncated"], true);
        assert_eq!(snapshot["lastReplyState"], "truncated");
        assert!(reader.reply.len() <= MAX_REPLY_HISTORY_BYTES);
        assert_eq!(snapshot, reader.read(dir.path(), &task).unwrap());
    }

    #[test]
    fn reply_history_limit_preserves_utf8_and_latest_turns() {
        let mut reader = TaskReader::default();
        let id = "official_task_0001";
        reader
            .apply(
                &json!({"type":"session_meta","payload":{"id":id,"source":"vscode"}}),
                id,
            )
            .unwrap();
        reader.append_reply(&"旧".repeat(MAX_REPLY_HISTORY_BYTES / 3), "event_msg");
        reader.append_reply("最新回复", "event_msg");
        assert!(reader.reply_truncated);
        assert!(reader.reply.len() <= MAX_REPLY_HISTORY_BYTES);
        assert!(reader.reply.ends_with("最新回复"));
        assert!(std::str::from_utf8(reader.reply.as_bytes()).is_ok());
    }

    #[test]
    fn rejects_path_escape_and_mismatched_identity() {
        let mut reader = TaskReader::default();
        assert!(
            reader
                .apply(
                    &json!({"type":"session_meta","payload":{"id":"other"}}),
                    "selected_task_0001"
                )
                .is_err()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outside.jsonl");
        std::fs::write(&path, "{}\n").unwrap();
        let task = OfficialTask {
            id: "selected_task_0001".into(),
            name: "".into(),
            workspace_name: "".into(),
            cwd: PathBuf::from("workspace"),
            path,
            updated_at: 1.0,
        };
        assert!(reader.read(dir.path(), &task).is_err());
        assert_eq!(clip("中文测试", 7), "中文");
        assert!(find_task(dir.path(), "selected_task_0001").is_err());
    }

    #[test]
    fn task_catalog_uses_local_display_title_without_adding_unverified_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let db = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(
               id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
               archived INTEGER, updated_at INTEGER
             );
             INSERT INTO threads VALUES
               ('official_task_0001', 'original prompt', 'workspace', 'task.jsonl', 0, 10),
               ('official_task_0002', 'fallback title', 'workspace', 'other.jsonl', 0, 20),
               ('official_task_0003', 'archived title', 'workspace', 'archived.jsonl', 1, 30);",
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("sqlite")).unwrap();
        let catalog = Connection::open(dir.path().join("sqlite/codex-dev.db")).unwrap();
        catalog
            .execute_batch(
                "CREATE TABLE local_thread_catalog(
               host_id TEXT, thread_id TEXT, display_title TEXT, missing_candidate INTEGER
             );
             INSERT INTO local_thread_catalog VALUES
               ('local', 'official_task_0001', 'desktop title', 0),
               ('remote', 'official_task_0001', 'remote title', 0),
               ('local', 'official_task_0002', 'missing title', 1),
               ('local', 'official_task_0003', 'archived desktop title', 0),
               ('local', 'official_task_0004', 'catalog only', 0);",
            )
            .unwrap();

        let tasks = list_tasks(dir.path()).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].name, "fallback title");
        assert_eq!(tasks[1].name, "desktop title");
        let task = find_task(dir.path(), "official_task_0001")
            .unwrap()
            .unwrap();
        assert_eq!(task.name, "desktop title");
        assert_eq!(task.path, PathBuf::from("task.jsonl"));
        catalog
            .execute(
                "UPDATE local_thread_catalog SET display_title=' ' WHERE host_id='local'",
                [],
            )
            .unwrap();
        assert_eq!(
            find_task(dir.path(), "official_task_0001")
                .unwrap()
                .unwrap()
                .name,
            "original prompt"
        );
    }

    #[test]
    fn task_catalog_tracks_create_edit_archive_and_recent_order() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir(&sessions).unwrap();
        let first_path = sessions.join("first.jsonl");
        let second_path = sessions.join("second.jsonl");
        std::fs::write(&first_path, "{}\n").unwrap();
        std::fs::write(&second_path, "{}\n").unwrap();
        let db = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(
               id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
               archived INTEGER, updated_at INTEGER
             );",
        )
        .unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, '先创建', 'E:/项目甲', ?2, 0, 10)",
            rusqlite::params!["official_task_0001", first_path.to_str().unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, '后创建', 'E:/项目乙', ?2, 0, 20)",
            rusqlite::params!["official_task_0002", second_path.to_str().unwrap()],
        )
        .unwrap();

        let tasks = list_tasks(dir.path()).unwrap();
        assert_eq!(
            tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            ["official_task_0002", "official_task_0001",]
        );

        db.execute(
            "UPDATE threads SET title='已编辑', cwd='E:/项目丙', updated_at=30 WHERE id=?1",
            ["official_task_0001"],
        )
        .unwrap();
        let edited = list_tasks(dir.path()).unwrap();
        assert_eq!(edited[0].name, "已编辑");
        assert_eq!(edited[0].workspace_name, "项目丙");

        db.execute(
            "UPDATE threads SET archived=1, updated_at=40 WHERE id=?1",
            ["official_task_0001"],
        )
        .unwrap();
        let remaining = list_tasks(dir.path()).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "official_task_0002");
    }

    #[test]
    fn locked_index_defers_sync_without_waiting_or_hiding_tasks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sqlite")).unwrap();
        let path = dir.path().join("sqlite/codex-dev.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(
               id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
               archived INTEGER, updated_at INTEGER
             );
             INSERT INTO threads VALUES
               ('official_task_0001', 'original', 'workspace', 'task.jsonl', 0, 10);
             BEGIN EXCLUSIVE;",
        )
        .unwrap();
        let started = std::time::Instant::now();
        assert!(list_tasks(dir.path()).is_err());
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        db.execute_batch("COMMIT").unwrap();
        assert_eq!(list_tasks(dir.path()).unwrap().len(), 1);
        assert_eq!(
            open_task_index(&path)
                .unwrap()
                .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, u64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("BEGIN EXCLUSIVE; UPDATE threads SET title='updated'; COMMIT;")
            .unwrap();
        assert_eq!(list_tasks(dir.path()).unwrap()[0].name, "updated");
        assert!(
            open_task_index(&path)
                .unwrap()
                .execute("DELETE FROM threads", [])
                .is_err()
        );
    }

    #[test]
    fn batch_selection_keeps_tasks_outside_recent_list() {
        let dir = tempfile::tempdir().unwrap();
        let db = Connection::open(dir.path().join("state_5.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(
               id TEXT, title TEXT, cwd TEXT, rollout_path TEXT,
               archived INTEGER, updated_at INTEGER
             );
             WITH RECURSIVE numbers(n) AS (
               SELECT 1 UNION ALL SELECT n+1 FROM numbers WHERE n<501
             )
             INSERT INTO threads SELECT
               printf('official_task_%04d', n), 'title', 'workspace', 'task.jsonl', 0, n
             FROM numbers;",
        )
        .unwrap();
        let selected = BTreeSet::from([
            "official_task_0001".to_owned(),
            "official_task_0501".to_owned(),
        ]);
        assert_eq!(list_tasks(dir.path()).unwrap().len(), 500);
        let tasks = find_tasks(dir.path(), &selected).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[1].id, "official_task_0001");
        assert!(find_tasks(dir.path(), &BTreeSet::new()).unwrap().is_empty());
    }

    #[test]
    fn large_history_resumes_without_publishing_partial_state_or_blocking_append() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sessions")).unwrap();
        let path = dir.path().join("sessions/task.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let task = OfficialTask {
            id: "official_task_0001".into(),
            name: "任务".into(),
            workspace_name: "工作区".into(),
            cwd: PathBuf::from("workspace"),
            path,
            updated_at: 1.0,
        };
        writeln!(
            file,
            "{}",
            json!({"type":"session_meta","payload":{"id":task.id,"source":"vscode"}})
        )
        .unwrap();
        let output = json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(16 * 1024)}});
        for _ in 0..80 {
            writeln!(file, "{output}").unwrap();
        }
        let original_len = file.metadata().unwrap().len();
        TaskReader::validate(dir.path(), &task).unwrap();
        let mut reader = TaskReader::default();
        assert!(reader.read(dir.path(), &task).is_err());
        assert!(reader.offset >= 1024 * 1024 && reader.offset < original_len);
        writeln!(file, "{}", json!({"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"完整回复"}})).unwrap();
        let appended_len = file.metadata().unwrap().len();
        let snapshot = reader.read(dir.path(), &task).unwrap();
        assert_eq!(snapshot["lastReply"]["text"], "完整回复");
        assert_eq!(reader.offset, appended_len);
        assert_eq!(file.metadata().unwrap().len(), appended_len);
        assert_eq!(reader.read(dir.path(), &task).unwrap(), snapshot);
    }
}
