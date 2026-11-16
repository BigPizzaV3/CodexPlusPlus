//! 加加的持久化：都放在 `<状态目录>/jiajia/` 下，一个文件一类东西。
//!
//! - `settings.json`：模型、推理强度、主动消息开关等；
//! - `chat.jsonl`：手机上看到的聊天记录（只追加，超长时截掉旧的）——界面以它为准，
//!   加加背后的 Codex 会话是临时的（不落盘），重开时靠这里和记忆把近况喂回去；
//! - `USER.md` / `NOTES.md`：加加记下的关于用户的事、它自己的笔记；
//! - `later.json`：约好的回访；`watch.json`：盯着的会话；`events.json`：最近的会话事件；
//! - `state.json`：已读位置。

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 内存里留多少条聊天记录（再早的只在文件里）。
pub const CHAT_KEEP: usize = 400;
/// 聊天文件超过这个大小就重写，只留最近 [`CHAT_KEEP`] 条。
const CHAT_FILE_LIMIT: u64 = 2 * 1024 * 1024;
/// 每份记忆的字数上限：超了让加加自己整理（replace）。
pub const MEMORY_LIMIT: usize = 6000;
/// 最多同时约几个回访。
pub const LATER_LIMIT: usize = 5;
/// 最多盯多少个会话。
pub const WATCH_LIMIT: usize = 40;
/// 事件留多少条、多久。
pub const EVENTS_KEEP: usize = 50;
pub const EVENTS_TTL_MS: i64 = 24 * 3600 * 1000;

/// 跑完的会话要不要报。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DoneReports {
    /// 只报加加派的活、它盯着的会话。
    #[default]
    Watched,
    All,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 第一次打开时选过模型了。
    pub configured: bool,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// 会话有动静时唤醒加加主动说（关掉就只发固定格式的提醒，不花模型调用）。
    pub proactive: bool,
    pub done_reports: DoneReports,
    /// 用户给加加定的性格（说话方式、口头禅……），空的就是默认的简短干练。
    pub persona: String,
    pub avatar: Avatar,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            configured: false,
            model: None,
            effort: None,
            proactive: true,
            done_reports: DoneReports::Watched,
            persona: String::new(),
            avatar: Avatar::default(),
        }
    }
}

/// 加加的头像：图片（存在 jiajia 目录里），或者一两个字 / emoji 配底色；都没有就是默认的「加」。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Avatar {
    pub text: Option<String>,
    /// `#rrggbb`
    pub color: Option<String>,
    /// 图片文件名（`avatar.jpg` 等），在 jiajia 目录下。
    pub image: Option<String>,
    /// 每换一次图片加一，网页拿它绕开缓存。
    pub version: u64,
}

/// 性格描述的字数上限。
pub const PERSONA_LIMIT: usize = 800;
/// 头像图片大小上限（网页会先缩到 256×256）。
pub const AVATAR_LIMIT: usize = 1024 * 1024;

/// 认头像图片的格式（看文件头，不信 Content-Type）。
pub fn avatar_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// 聊天记录的一条。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: u64,
    /// 毫秒时间戳。
    pub at: i64,
    /// `user` 用户；`bot` 加加；`event` 交给加加的会话动静（灰字）；`notice` 不经模型、由程序以加加名义发的提醒。
    pub role: String,
    #[serde(default)]
    pub text: String,
    /// 提到的会话（显示成卡片）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<String>,
    /// 审批 / 提问卡片：`{ threadId, title, request }`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<Value>,
    /// 审批卡片对应的请求已经处理（在哪儿处理的都算）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
    /// 谁发的（设备名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// 变更序号：新建或改动时取全局递增值，推送只发比上次大的。
    #[serde(default)]
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Later {
    pub id: u32,
    pub due_ms: i64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Watched {
    pub thread_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub reason: String,
    pub since_ms: i64,
}

/// 一条会话事件（等审批 / 出错 / 跑完）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRecord {
    pub at: i64,
    pub kind: String,
    pub thread_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub detail: String,
    /// 怎么处理的：tell 告诉加加 / notify 只发提醒 / ignore 只记录。
    #[serde(default)]
    pub routed: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ReadState {
    read_upto: u64,
}

/// 加加的全部持久状态（内存副本 + 写盘）。
pub struct Store {
    dir: PathBuf,
    pub settings: Settings,
    pub messages: VecDeque<ChatMessage>,
    next_id: u64,
    rev: u64,
    pub read_upto: u64,
    pub user_memory: String,
    pub notes: String,
    pub later: Vec<Later>,
    pub watched: Vec<Watched>,
    pub events: VecDeque<EventRecord>,
}

impl Store {
    pub fn open(dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(dir);
        let messages = load_chat(&dir.join("chat.jsonl"));
        let next_id = messages.back().map(|message| message.id + 1).unwrap_or(1);
        let rev = messages.iter().map(|message| message.rev).max().unwrap_or(0);
        Self {
            settings: read_json(&dir.join("settings.json")).unwrap_or_default(),
            messages,
            next_id,
            rev,
            read_upto: read_json::<ReadState>(&dir.join("state.json")).unwrap_or_default().read_upto,
            user_memory: std::fs::read_to_string(dir.join("USER.md")).unwrap_or_default(),
            notes: std::fs::read_to_string(dir.join("NOTES.md")).unwrap_or_default(),
            later: read_json(&dir.join("later.json")).unwrap_or_default(),
            watched: read_json(&dir.join("watch.json")).unwrap_or_default(),
            events: read_json::<Vec<EventRecord>>(&dir.join("events.json")).unwrap_or_default().into(),
            dir: dir.to_path_buf(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn save_settings(&self) {
        write_json(&self.dir.join("settings.json"), &self.settings);
    }

    pub fn next_rev(&mut self) -> u64 {
        self.rev += 1;
        self.rev
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// 追加一条聊天记录，返回它的 id。
    pub fn push(&mut self, mut message: ChatMessage) -> u64 {
        message.id = self.next_id;
        self.next_id += 1;
        message.rev = self.next_rev();
        if message.at == 0 {
            message.at = now_ms();
        }
        append_line(&self.dir.join("chat.jsonl"), &message);
        self.messages.push_back(message.clone());
        while self.messages.len() > CHAT_KEEP {
            self.messages.pop_front();
        }
        self.compact_chat_if_needed();
        message.id
    }

    /// 改一条已有的记录（比如审批卡片被处理了）。只改内存与重写文件。
    pub fn update(&mut self, id: u64, change: impl FnOnce(&mut ChatMessage)) -> bool {
        let rev = self.rev + 1;
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else { return false };
        change(message);
        message.rev = rev;
        self.rev = rev;
        self.rewrite_chat();
        true
    }

    /// 比 `rev` 新的记录（推送用）。
    pub fn changed_since(&self, rev: u64) -> Vec<ChatMessage> {
        self.messages.iter().filter(|message| message.rev > rev).cloned().collect()
    }

    /// `before` 之前的 `limit` 条（往上翻）。
    pub fn page_before(&self, before: Option<u64>, limit: usize) -> Vec<ChatMessage> {
        let older: Vec<&ChatMessage> = self.messages.iter().filter(|message| before.is_none_or(|before| message.id < before)).collect();
        older[older.len().saturating_sub(limit)..].iter().map(|message| (*message).clone()).collect()
    }

    pub fn unread(&self) -> usize {
        self.messages.iter().filter(|message| message.id > self.read_upto && matches!(message.role.as_str(), "bot" | "notice")).count()
    }

    pub fn mark_read(&mut self, upto: u64) {
        let upto = upto.min(self.next_id.saturating_sub(1));
        if upto > self.read_upto {
            self.read_upto = upto;
            write_json(&self.dir.join("state.json"), &ReadState { read_upto: upto });
        }
    }

    /// 清空聊天记录（记忆、回访、盯着的会话都保留）。
    pub fn clear_chat(&mut self) {
        self.messages.clear();
        self.read_upto = self.next_id.saturating_sub(1);
        let _ = std::fs::write(self.dir.join("chat.jsonl"), b"");
        write_json(&self.dir.join("state.json"), &ReadState { read_upto: self.read_upto });
    }

    pub fn save_memory(&self) {
        let _ = std::fs::write(self.dir.join("USER.md"), &self.user_memory);
        let _ = std::fs::write(self.dir.join("NOTES.md"), &self.notes);
    }

    pub fn save_later(&self) {
        write_json(&self.dir.join("later.json"), &self.later);
    }

    pub fn save_watched(&self) {
        write_json(&self.dir.join("watch.json"), &self.watched);
    }

    pub fn record_event(&mut self, event: EventRecord) {
        let cutoff = event.at - EVENTS_TTL_MS;
        self.events.retain(|existing| existing.at >= cutoff);
        self.events.push_back(event);
        while self.events.len() > EVENTS_KEEP {
            self.events.pop_front();
        }
        write_json(&self.dir.join("events.json"), &self.events);
    }

    pub fn is_watched(&self, thread_id: &str) -> bool {
        self.watched.iter().any(|watched| watched.thread_id == thread_id)
    }

    /// 加入盯着的会话；满了挤掉最早的。
    pub fn watch(&mut self, thread_id: &str, title: &str, reason: &str) {
        if let Some(existing) = self.watched.iter_mut().find(|watched| watched.thread_id == thread_id) {
            if !title.is_empty() {
                existing.title = title.to_string();
            }
            return self.save_watched();
        }
        self.watched.push(Watched { thread_id: thread_id.to_string(), title: title.to_string(), reason: reason.to_string(), since_ms: now_ms() });
        while self.watched.len() > WATCH_LIMIT {
            self.watched.remove(0);
        }
        self.save_watched();
    }

    pub fn unwatch(&mut self, thread_id: &str) -> bool {
        let before = self.watched.len();
        self.watched.retain(|watched| watched.thread_id != thread_id);
        let removed = self.watched.len() != before;
        if removed {
            self.save_watched();
        }
        removed
    }

    fn compact_chat_if_needed(&self) {
        let path = self.dir.join("chat.jsonl");
        if std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) > CHAT_FILE_LIMIT {
            self.rewrite_chat();
        }
    }

    fn rewrite_chat(&self) {
        let mut text = String::new();
        for message in &self.messages {
            if let Ok(line) = serde_json::to_string(message) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        let path = self.dir.join("chat.jsonl");
        let temp = path.with_extension("jsonl.tmp");
        if std::fs::write(&temp, text).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|duration| duration.as_millis() as i64).unwrap_or(0)
}

fn load_chat(path: &Path) -> VecDeque<ChatMessage> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut messages: VecDeque<ChatMessage> = VecDeque::new();
    for line in text.lines() {
        let Ok(message) = serde_json::from_str::<ChatMessage>(line) else { continue };
        messages.push_back(message);
        if messages.len() > CHAT_KEEP {
            messages.pop_front();
        }
    }
    messages
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) {
    let Ok(bytes) = serde_json::to_vec_pretty(value) else { return };
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, bytes).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

fn append_line<T: Serialize>(path: &Path, value: &T) {
    let Ok(line) = serde_json::to_string(value) else { return };
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}
