//! 对话与当前拓展的关联，只接受 MCP 调用携带的对话身份。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::user_scripts::{UserScriptManager, validate_extension_id};

const STATE_FILE: &str = "extension-creator-contexts.json";
const LOCK_FILE: &str = "extension-creator-contexts.json.lock";
const STATE_VERSION: u32 = 1;

/// 新版直接传 threadId，旧版放在请求元数据中；sessionId 是 MCP 会话，不能当对话用。
pub(super) fn thread_id(meta: &Value) -> anyhow::Result<Option<String>> {
    let metadata = match meta {
        Value::Null => return Ok(None),
        Value::Object(metadata) => metadata,
        _ => bail!("MCP _meta 必须是对象"),
    };
    let direct = metadata
        .get("threadId")
        .map(|value| parse_thread_id(value, "_meta.threadId"))
        .transpose()?;
    let header = match metadata.get("x-codex-turn-metadata") {
        None => None,
        Some(Value::Object(header)) => Some(Value::Object(header.clone())),
        Some(Value::String(header)) => Some(
            serde_json::from_str::<Value>(header)
                .context("_meta.x-codex-turn-metadata 不是有效 JSON")?,
        ),
        Some(_) => bail!("_meta.x-codex-turn-metadata 必须是对象或 JSON 字符串"),
    };
    let fallback = match header {
        None => None,
        Some(Value::Object(header)) => header
            .get("thread_id")
            .map(|value| parse_thread_id(value, "_meta.x-codex-turn-metadata.thread_id"))
            .transpose()?,
        Some(_) => bail!("_meta.x-codex-turn-metadata 的 JSON 必须是对象"),
    };
    if let (Some(direct), Some(fallback)) = (&direct, &fallback) {
        if direct != fallback {
            bail!("MCP 对话身份冲突：threadId 与 x-codex-turn-metadata.thread_id 不一致");
        }
    }
    Ok(direct.or(fallback))
}

fn parse_thread_id(value: &Value, field: &str) -> anyhow::Result<String> {
    let id = value.as_str().with_context(|| format!("{field} 必须是字符串"))?;
    validate_thread_id(id).with_context(|| format!("{field} 格式无效"))?;
    Ok(id.to_string())
}

fn validate_thread_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 256
        || id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        bail!("对话 ID 必须是 1~256 字节且不含空白或控制字符的字符串");
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    threads: BTreeMap<String, String>,
}

impl Default for State {
    fn default() -> Self {
        Self { version: STATE_VERSION, threads: BTreeMap::new() }
    }
}

/// guard 存活期间锁住读改写，多个 MCP 进程不会互相覆盖对话记录。
pub(super) struct Bindings {
    path: PathBuf,
    state: State,
    _lock: File,
}

impl Bindings {
    pub(super) fn lock(manager: &UserScriptManager) -> anyhow::Result<Self> {
        let directory = manager
            .config_path()
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(directory).context("无法创建拓展对话关联目录")?;
        let path = directory.join(STATE_FILE);
        let lock_path = directory.join(LOCK_FILE);
        reject_non_file(&lock_path)?;
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).context("无法打开拓展对话关联锁")?;
        FileExt::lock_exclusive(&lock).context("无法锁定拓展对话关联")?;
        reject_non_file(&path)?;
        let state = match fs::read(&path) {
            Ok(bytes) => {
                let state: State =
                    serde_json::from_slice(&bytes).context("拓展对话关联记录已损坏，未覆盖")?;
                if state.version != STATE_VERSION {
                    bail!("不支持的拓展对话关联版本 {}，未覆盖", state.version);
                }
                for (thread, extension) in &state.threads {
                    validate_thread_id(thread).context("拓展对话关联包含无效对话 ID，未覆盖")?;
                    validate_extension_id(extension).context("拓展对话关联包含无效拓展 ID，未覆盖")?;
                }
                state
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(error) => return Err(error).context("无法读取拓展对话关联，未覆盖"),
        };
        Ok(Self { path, state, _lock: lock })
    }

    pub(super) fn get(&self, thread_id: &str) -> Option<&str> {
        self.state.threads.get(thread_id).map(String::as_str)
    }

    pub(super) fn bind(&mut self, thread_id: &str, extension_id: &str) -> anyhow::Result<()> {
        validate_thread_id(thread_id)?;
        validate_extension_id(extension_id)?;
        if self.get(thread_id) == Some(extension_id) {
            return Ok(());
        }
        reject_non_file(&self.path)?;
        let previous = self.state.threads.insert(thread_id.to_string(), extension_id.to_string());
        let saved = serde_json::to_vec_pretty(&self.state)
            .context("无法编码拓展对话关联")
            .and_then(|bytes| {
                crate::settings::atomic_write(&self.path, &bytes).context("无法保存拓展对话关联")
            });
        if let Err(error) = saved {
            match previous {
                Some(previous) => {
                    self.state.threads.insert(thread_id.to_string(), previous);
                }
                None => {
                    self.state.threads.remove(thread_id);
                }
            }
            return Err(error);
        }
        Ok(())
    }
}

fn reject_non_file(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("拓展对话关联文件不能是符号链接，未修改")
        }
        Ok(metadata) if !metadata.is_file() => bail!("拓展对话关联目标不是普通文件，未修改"),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("无法检查拓展对话关联文件"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
        UserScriptManager::new(
            temp.path().join("builtin"),
            temp.path().join("user"),
            temp.path().join("user_scripts.json"),
        )
    }

    #[test]
    fn accepts_direct_and_legacy_thread_metadata() {
        for metadata in [
            json!({"threadId": "thread-a", "sessionId": "mcp-session"}),
            json!({"x-codex-turn-metadata": {"thread_id": "thread-a"}}),
            json!({"x-codex-turn-metadata": "{\"thread_id\":\"thread-a\"}"}),
            json!({
                "threadId": "thread-a",
                "x-codex-turn-metadata": {"thread_id": "thread-a", "turn_id": "turn-1"},
            }),
        ] {
            assert_eq!(thread_id(&metadata).unwrap().as_deref(), Some("thread-a"));
        }
        for metadata in [
            Value::Null,
            json!({}),
            json!({"sessionId": "mcp-session", "session_id": "another-session"}),
            json!({"x-codex-turn-metadata": {"session_id": "mcp-session"}}),
        ] {
            assert_eq!(thread_id(&metadata).unwrap(), None);
        }
    }

    #[test]
    fn rejects_conflicting_and_malformed_metadata() {
        let conflict = json!({
            "threadId": "thread-a",
            "x-codex-turn-metadata": {"thread_id": "thread-b"},
        });
        assert!(thread_id(&conflict).unwrap_err().to_string().contains("身份冲突"));
        for metadata in [
            json!("not-an-object"),
            json!({"threadId": null}),
            json!({"threadId": 1}),
            json!({"threadId": ""}),
            json!({"threadId": " thread-a"}),
            json!({"threadId": "thread\n-a"}),
            json!({"x-codex-turn-metadata": null}),
            json!({"x-codex-turn-metadata": []}),
            json!({"x-codex-turn-metadata": "not-json"}),
            json!({"x-codex-turn-metadata": "[]"}),
            json!({"x-codex-turn-metadata": {"thread_id": false}}),
            json!({"threadId": "thread-a", "x-codex-turn-metadata": "broken"}),
        ] {
            assert!(thread_id(&metadata).is_err(), "{metadata}");
        }
    }

    #[test]
    fn persists_independent_bindings_and_skips_unchanged_writes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let path = temp.path().join(STATE_FILE);
        {
            let mut bindings = Bindings::lock(&manager).unwrap();
            assert_eq!(bindings.get("thread-a"), None);
            assert!(!path.exists(), "查询空记录不写状态文件");
            bindings.bind("thread-a", "ext-a").unwrap();
        }
        {
            let mut bindings = Bindings::lock(&manager).unwrap();
            assert_eq!(bindings.get("thread-a"), Some("ext-a"));
            bindings.bind("thread-b", "ext-b").unwrap();
            // 写成不同排版，重复绑定应完全不触碰它。
            let compact = serde_json::to_vec(&bindings.state).unwrap();
            fs::write(&path, &compact).unwrap();
            bindings.bind("thread-b", "ext-b").unwrap();
            assert_eq!(fs::read(&path).unwrap(), compact);
        }
        let bindings = Bindings::lock(&manager).unwrap();
        assert_eq!(bindings.get("thread-a"), Some("ext-a"));
        assert_eq!(bindings.get("thread-b"), Some("ext-b"));
        assert_eq!(bindings.get("thread-c"), None);
    }

    #[test]
    fn rejects_damaged_unsupported_and_invalid_state_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let path = temp.path().join(STATE_FILE);
        for contents in [
            "{",
            "{}",
            r#"{"version":2,"threads":{}}"#,
            r#"{"version":1,"threads":null}"#,
            r#"{"version":1,"threads":{"thread-a":1}}"#,
            r#"{"version":1,"threads":{"":"ext-a"}}"#,
            r#"{"version":1,"threads":{"thread-a":"../ext-a"}}"#,
        ] {
            fs::write(&path, contents).unwrap();
            assert!(Bindings::lock(&manager).is_err(), "{contents}");
            assert_eq!(fs::read_to_string(&path).unwrap(), contents);
        }
    }

    #[test]
    fn rejects_invalid_bindings_without_changing_existing_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut bindings = Bindings::lock(&manager).unwrap();
        bindings.bind("thread-a", "ext-a").unwrap();
        let before = fs::read(temp.path().join(STATE_FILE)).unwrap();
        assert!(bindings.bind("", "ext-b").is_err());
        assert!(bindings.bind("thread-a", "../ext-b").is_err());
        assert_eq!(bindings.get("thread-a"), Some("ext-a"));
        assert_eq!(fs::read(temp.path().join(STATE_FILE)).unwrap(), before);
    }

    #[test]
    fn holds_the_file_lock_until_bindings_are_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let bindings = Bindings::lock(&manager).unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path().join(LOCK_FILE))
            .unwrap();
        assert!(FileExt::try_lock_exclusive(&contender).is_err());
        drop(bindings);
        FileExt::try_lock_exclusive(&contender).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_state_and_lock_files() {
        use std::os::unix::fs::symlink;

        for target in [STATE_FILE, LOCK_FILE] {
            let temp = tempfile::tempdir().unwrap();
            let manager = manager(&temp);
            let outside = temp.path().join("outside.json");
            let contents = r#"{"version":1,"threads":{"thread-a":"ext-a"}}"#;
            fs::write(&outside, contents).unwrap();
            symlink(&outside, temp.path().join(target)).unwrap();
            assert!(Bindings::lock(&manager).is_err());
            assert_eq!(fs::read_to_string(&outside).unwrap(), contents);
        }
    }
}
