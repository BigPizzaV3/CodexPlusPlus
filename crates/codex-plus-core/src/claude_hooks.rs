//! Claude Code 的回合结束钩子。
//!
//! # 为什么需要它
//!
//! Codex 的浏览器扩展靠「回合结束」清理 agent 自己开的标签：没标记的关闭，标记过的保留。
//! 触发点只有一个 —— `cua-repl` 的 `turn_ended`。原生 Codex 是这么接的：
//!
//! ```toml
//! # ~/.codex/config.toml
//! notify = [".../SkyComputerUseClient", "turn-ended"]
//! ```
//!
//! Claude Code 没有对应的通知机制，但有 hooks。这里往 `~/.claude/settings.json`
//! 的 `Stop` / `SubagentStop` 里挂一条命令，把「回合结束了」写进 spool 文件；
//! 浏览器 MCP 进程轮询 spool，看到新事件就发 `turn_ended`。
//!
//! 钩子命令是 `CodexPlusPlus --codex-plus-browser-turn-end <spool>`，不依赖 shell、
//! 不依赖脚本路径，也不受 GUI 与 MCP 进程工作目录不同的影响。
//!
//! # 边界
//!
//! - 只给 `Target::ClaudeCode` 装：其它客户端有各自的回合语义。
//! - 幂等：同一条命令已在列表里就不再追加；`uninstall` 只删自己的那条。
//! - 改动前备份原文件，解析失败不做任何写入。

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use serde_json::{Value, json};

/// 钩子里调用自己的参数。
pub const TURN_END_FLAG: &str = "--codex-plus-browser-turn-end";
/// spool 文件名：每次钩子触发追加一行。
const SPOOL_FILE: &str = "turn-end.jsonl";

/// spool 目录：`<state>/browser`。
pub fn spool_dir() -> PathBuf {
    crate::paths::default_app_state_dir().join("browser")
}

/// 钩子命令里要写的 spool 绝对路径。管理器的悬浮预览也据此判断回合结束。
pub fn spool_path() -> PathBuf {
    spool_dir().join(SPOOL_FILE)
}

/// 钩子命令，带上当前的 state 目录。
///
/// Claude Code 执行命令钩子时只继承它自己的环境，不会有 `CODEX_PLUS_STATE_DIR`。
/// 开发实例（`dist/local-build/state`）与主实例（`~/.codex-plus`）靠这个变量分开，
/// 所以必须把目录直接写进命令，否则钩子写进主实例、MCP 进程在开发实例里等不到。
fn hook_command() -> anyhow::Result<String> {
    let exe = std::env::current_exe().context("无法确定自身可执行文件路径")?;
    let state_dir = crate::paths::default_app_state_dir();
    Ok(format!(
        "\"{}\" {TURN_END_FLAG} \"{}\"",
        exe.display(),
        state_dir.display()
    ))
}

/// 命令里状态目录那段前缀。卸载时按前缀匹配，换过路径的旧条目也能认出来。
fn hook_command_prefix() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" {TURN_END_FLAG}", exe.display()))
}

/// `~/.claude/settings.json`。允许用 `CLAUDE_CONFIG_DIR` 覆盖（Claude Code 自己认这个变量）。
pub fn claude_settings_path() -> PathBuf {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir).join("settings.json");
        }
    }
    home_dir().join(".claude").join("settings.json")
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// 钩子安装结果。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookReceipt {
    /// 是否改动了文件。
    pub changed: bool,
    /// settings.json 的位置。
    pub path: String,
    /// 改动前的备份（不存在时为空）。
    pub backup: Option<String>,
    /// 现在装了哪些事件。
    pub events: Vec<String>,
}

/// 把回合结束钩子装进 Claude Code 的 settings.json。
pub fn install() -> anyhow::Result<HookReceipt> {
    install_at(&claude_settings_path())
}

/// 装进指定的 settings.json：自定义配置目录（`CLAUDE_CONFIG_DIR` 布局）用。
pub fn install_at(path: &Path) -> anyhow::Result<HookReceipt> {
    let path = path.to_path_buf();
    let command = hook_command()?;
    let mut document = read_settings(&path)?;
    let backup = backup_settings(&path)?;
    let mut events = Vec::new();
    let mut changed = false;
    for event in ["Stop", "SubagentStop"] {
        if upsert_hook(&mut document, event, &command)? {
            changed = true;
        }
        events.push(event.to_string());
    }
    if changed {
        write_settings(&path, &document)?;
    }
    Ok(HookReceipt {
        changed,
        path: path.display().to_string(),
        backup,
        events,
    })
}

/// 移除我们装的钩子；别人的钩子不动。
pub fn uninstall() -> anyhow::Result<HookReceipt> {
    uninstall_at(&claude_settings_path())
}

/// 从指定的 settings.json 移除我们的钩子。
pub fn uninstall_at(path: &Path) -> anyhow::Result<HookReceipt> {
    let path = path.to_path_buf();
    if !path.is_file() {
        return Ok(HookReceipt {
            changed: false,
            path: path.display().to_string(),
            backup: None,
            events: Vec::new(),
        });
    }
    let command = hook_command()?;
    let mut document = read_settings(&path)?;
    let backup = backup_settings(&path)?;
    let mut changed = false;
    for event in ["Stop", "SubagentStop"] {
        if remove_hook(&mut document, event, &command) {
            changed = true;
        }
    }
    if changed {
        write_settings(&path, &document)?;
    }
    Ok(HookReceipt {
        changed,
        path: path.display().to_string(),
        backup,
        events: Vec::new(),
    })
}

/// 是否已经装过（至少 `Stop` 里有我们的命令）。
pub fn is_installed() -> bool {
    let Ok(document) = read_settings(&claude_settings_path()) else {
        return false;
    };
    let Ok(command) = hook_command() else {
        return false;
    };
    has_hook(&document, "Stop", &command)
}

fn read_settings(path: &Path) -> anyhow::Result<Value> {
    if !path.is_file() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("读取 {} 失败", path.display()))?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).with_context(|| format!("{} 不是合法 JSON，未做任何改动", path.display()))
}

fn backup_settings(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let backup = path.with_extension("json.codex-plus-backup");
    std::fs::copy(path, &backup)
        .with_context(|| format!("备份 {} 失败", path.display()))?;
    Ok(Some(backup.display().to_string()))
}

fn write_settings(path: &Path, document: &Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(document)?;
    let temporary = path.with_extension("json.codex-plus-tmp");
    std::fs::write(&temporary, format!("{text}\n"))
        .with_context(|| format!("写入 {} 失败", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("替换 {} 失败", path.display()))?;
    Ok(())
}

/// 每个事件下的钩子条目长这样：
///
/// ```json
/// { "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "..." } ] } ] } }
/// ```
fn hook_entries<'a>(document: &'a Value, event: &str) -> Option<&'a Vec<Value>> {
    document
        .get("hooks")?
        .get(event)?
        .as_array()
}

fn has_hook(document: &Value, event: &str, command: &str) -> bool {
    hook_entries(document, event).is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry
                .get("hooks")
                .and_then(Value::as_array)
                .is_some_and(|hooks| {
                    hooks.iter().any(|hook| {
                        hook.get("command").and_then(Value::as_str) == Some(command)
                    })
                })
        })
    })
}

fn upsert_hook(document: &mut Value, event: &str, command: &str) -> anyhow::Result<bool> {
    if has_hook(document, event, command) {
        return Ok(false);
    }
    let root = document
        .as_object_mut()
        .context("settings.json 顶层不是对象，未做任何改动")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !hooks.is_object() {
        // `hooks: null` 是常见值（用户没配过钩子），直接换成对象。
        *hooks = Value::Object(serde_json::Map::new());
    }
    let hooks = hooks.as_object_mut().expect("刚确认过是对象");
    let list = hooks
        .entry(event)
        .or_insert_with(|| Value::Array(Vec::new()));
    if !list.is_array() {
        return Err(anyhow::anyhow!("hooks.{event} 不是数组，未做任何改动"));
    }
    let list = list.as_array_mut().expect("刚确认过是数组");
    list.push(json!({
        "hooks": [{
            "type": "command",
            "command": command,
            "timeout": 5
        }]
    }));
    Ok(true)
}

fn remove_hook(document: &mut Value, event: &str, command: &str) -> bool {
    let Some(root) = document.as_object_mut() else {
        return false;
    };
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(list) = hooks.get_mut(event).and_then(Value::as_array_mut) else {
        return false;
    };
    let before = list.len();
    list.retain_mut(|entry| {
        let Some(child) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        child.retain(|hook| hook.get("command").and_then(Value::as_str) != Some(command));
        // 整条被我们清空的记录一并删掉，别留空壳。
        !child.is_empty()
    });
    let removed = list.len() != before;
    let empty = list.is_empty();
    if empty {
        hooks.remove(event);
    }
    removed
}

/// 钩子入口：把一次回合结束追加进 spool。
///
/// 由 `CodexPlusPlus --codex-plus-browser-turn-end` 调用。任何失败都只写 stderr：
/// 钩子的退出码会显示给用户，不该因为磁盘问题打断对话。
pub fn record_turn_end(state_dir: Option<&std::ffi::OsStr>) {
    // 钩子命令里带了 state 目录（见 `hook_command`）：开发实例与主实例各写各的 spool。
    // 没带就退回本进程的默认目录，用于手动排查。
    let path = match state_dir.filter(|value| !value.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("browser").join(SPOOL_FILE),
        None => spool_path(),
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 这个文件只用来「计数」新事件，不需要留历史。超过 1MB 就从头写，
    // 避免用户的 state 目录里攒出一份只增不减的日志。
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 1024 * 1024) {
        let _ = std::fs::write(&path, "");
    }
    let payload = json!({
        "at": SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or_default(),
        "cwd": std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    });
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "{payload}");
        }
        Err(error) => eprintln!("[codex-plus-browser] 无法写入回合事件：{error}"),
    }
}

/// 在后台线程里盯 spool，每来一条新事件就回调一次。
pub fn spawn_turn_watcher(on_turn_end: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        // 先把已有内容记账，避免把上一次会话留下的事件当成本次回合结束。
        let mut seen = spool_len(&spool_path());
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let path = spool_path();
            let current = spool_len(&path);
            if current > seen {
                seen = current;
                on_turn_end();
            } else if current < seen {
                // 文件被清空或轮转：重新对齐，不补发。
                seen = current;
            }
        }
    });
}

/// 用「行数 + 修改时间」判断有没有新事件。
pub fn spool_len(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::metadata(path) else {
        return 0;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos() as u64)
        .unwrap_or_default();
    // 同一纳秒内追加两行几乎不可能；用长度兜底。
    metadata.len().wrapping_mul(1_000_003).wrapping_add(modified)
}

/// 供管理器展示：当前装了哪些事件。
pub fn installed_events() -> Vec<String> {
    let Ok(document) = read_settings(&claude_settings_path()) else {
        return Vec::new();
    };
    let Ok(command) = hook_command() else {
        return Vec::new();
    };
    let mut events = BTreeSet::new();
    for event in ["Stop", "SubagentStop", "SessionEnd"] {
        if has_hook(&document, event, &command) {
            events.insert(event.to_string());
        }
    }
    events.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_into_an_empty_document_and_is_idempotent() {
        let mut document = json!({});
        assert!(upsert_hook(&mut document, "Stop", "cmd").unwrap());
        assert!(!upsert_hook(&mut document, "Stop", "cmd").unwrap());
        let entries = document["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["hooks"][0]["command"], "cmd");
    }

    #[test]
    fn hooks_null_becomes_an_object() {
        let mut document = json!({ "hooks": null, "model": "opus" });
        assert!(upsert_hook(&mut document, "Stop", "cmd").unwrap());
        assert_eq!(document["model"], "opus");
        assert!(document["hooks"]["Stop"].is_array());
    }

    #[test]
    fn keeps_other_peoples_hooks() {
        let mut document = json!({
            "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "mine" } ] } ] }
        });
        assert!(upsert_hook(&mut document, "Stop", "cmd").unwrap());
        assert!(remove_hook(&mut document, "Stop", "cmd"));
        let entries = document["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["hooks"][0]["command"], "mine");
        // 别人的钩子还在，所以事件键要保留。
        assert!(document["hooks"]["Stop"].is_array());
    }

    #[test]
    fn removing_the_last_hook_drops_the_event_key() {
        let mut document = json!({});
        upsert_hook(&mut document, "Stop", "cmd").unwrap();
        assert!(remove_hook(&mut document, "Stop", "cmd"));
        assert!(document["hooks"].get("Stop").is_none());
    }

    #[test]
    fn refuses_to_touch_a_non_array_event() {
        let mut document = json!({ "hooks": { "Stop": "nope" } });
        assert!(upsert_hook(&mut document, "Stop", "cmd").is_err());
    }

    #[test]
    fn broken_settings_are_reported_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_settings(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn hook_command_points_at_this_binary() {
        let command = hook_command().unwrap();
        assert!(command.contains(TURN_END_FLAG));
        assert!(command.starts_with('"'));
    }
}
