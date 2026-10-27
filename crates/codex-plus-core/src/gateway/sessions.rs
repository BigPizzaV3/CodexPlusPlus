//! 各 Agent 的本地会话：Claude Code（`~/.claude/projects/*/*.jsonl`）、Grok（`~/.grok/sessions/session_search.sqlite`）。
//! Codex 的会话沿用现有的会话库读取（`list_local_sessions`）。
//!
//! 「继续」在终端里执行 Agent 自己的恢复命令（`claude --resume <id>`、`grok --resume <id>`、`codex resume <id>`）。
//! 「删除」把会话文件 / 目录移进废纸篓（可找回），Grok 另从它的会话索引里删掉这一行。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Value;

/// 读 Claude 会话文件头部找标题和目录时最多读多少字节。
const CLAUDE_HEAD_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub agent: String,
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at_ms: u64,
    /// 会话文件（Claude）或会话库（Grok）的位置。
    pub path: String,
}

/// Claude Code 的会话：每个项目目录下一个 `<会话 id>.jsonl`。按修改时间倒序，最多 `limit` 个。
pub fn list_claude_sessions(claude_home: &Path, limit: usize) -> Vec<AgentSession> {
    let projects = claude_home.join("projects");
    let Ok(dirs) = std::fs::read_dir(&projects) else {
        return Vec::new();
    };
    let mut files: Vec<(u64, PathBuf)> = Vec::new();
    for dir in dirs.flatten() {
        let Ok(entries) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let modified = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_millis() as u64)
                .unwrap_or_default();
            files.push((modified, path));
        }
    }
    files.sort_by(|left, right| right.0.cmp(&left.0));
    files
        .into_iter()
        .take(limit)
        .filter_map(|(modified, path)| read_claude_session(&path, modified))
        .collect()
}

fn read_claude_session(path: &Path, modified: u64) -> Option<AgentSession> {
    let id = path.file_stem()?.to_str()?.to_string();
    let file = std::fs::File::open(path).ok()?;
    let reader = BufReader::new(file.take(CLAUDE_HEAD_BYTES));
    let mut cwd = String::new();
    let mut summary = String::new();
    let mut first_prompt = String::new();
    for line in reader.lines().map_while(Result::ok) {
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if cwd.is_empty()
            && let Some(dir) = value.get("cwd").and_then(Value::as_str)
        {
            cwd = dir.to_string();
        }
        match value.get("type").and_then(Value::as_str) {
            Some("summary") if summary.is_empty() => {
                summary = value
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
            }
            Some("user") if first_prompt.is_empty() => {
                first_prompt = user_text(&value);
            }
            _ => {}
        }
        if !cwd.is_empty() && !summary.is_empty() {
            break;
        }
    }
    let title = if !summary.is_empty() {
        summary
    } else {
        first_prompt
    };
    if title.is_empty() && cwd.is_empty() {
        return None;
    }
    Some(AgentSession {
        agent: "claude".to_string(),
        id,
        title: shorten(&title, 120),
        cwd,
        updated_at_ms: modified,
        path: path.display().to_string(),
    })
}

/// 用户消息的文字（跳过 `<command-name>` 这类系统插入的内容）。
fn user_text(value: &Value) -> String {
    let content = value.pointer("/message/content");
    let text = match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    };
    let text = text.trim();
    if text.starts_with('<') {
        return String::new();
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn shorten(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        text.to_string()
    } else {
        format!("{}…", chars[..max].iter().collect::<String>())
    }
}

/// Grok 的会话：Grok 自己维护的搜索库里有标题、目录和更新时间。
pub fn list_grok_sessions(grok_home: &Path, limit: usize) -> Vec<AgentSession> {
    let path = grok_home.join("sessions").join("session_search.sqlite");
    if !path.is_file() {
        return Vec::new();
    }
    let Ok(connection) = rusqlite::Connection::open_with_flags(
        &path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare(
        "SELECT session_id, cwd, updated_at, title FROM session_docs ORDER BY updated_at DESC LIMIT ?1",
    ) else {
        return Vec::new();
    };
    let rows = statement.query_map([limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
        ))
    });
    let Ok(rows) = rows else {
        return Vec::new();
    };
    rows.flatten()
        .map(|(id, cwd, updated, title)| AgentSession {
            agent: "grok".to_string(),
            id,
            title: shorten(title.trim(), 120),
            cwd,
            updated_at_ms: normalize_timestamp(updated),
            path: path.display().to_string(),
        })
        .collect()
}

/// 秒 / 毫秒 / 微秒统一成毫秒。
fn normalize_timestamp(value: i64) -> u64 {
    let value = value.max(0) as u64;
    if value > 100_000_000_000_000 {
        value / 1000
    } else if value > 100_000_000_000 {
        value
    } else {
        value * 1000
    }
}

/// 会话 id 只能是字母、数字、`-`、`_`，防止拼进命令时被注入。
fn safe_id(id: &str) -> anyhow::Result<&str> {
    let id = id.trim();
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("会话 id 不合法");
    }
    Ok(id)
}

/// 恢复会话的命令。
pub fn resume_command(agent: &str, id: &str) -> anyhow::Result<String> {
    let id = safe_id(id)?;
    Ok(match agent {
        "claude" => format!("claude --resume {id}"),
        "grok" => format!("grok --resume {id}"),
        "codex" => format!("codex resume {id}"),
        other => bail!("{other} 不支持恢复会话"),
    })
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// 在新的终端窗口里进入 `cwd` 执行 `command`。
pub fn open_in_terminal(cwd: &str, command: &str, scratch_dir: &Path) -> anyhow::Result<()> {
    let cwd = cwd.trim();
    if !cwd.is_empty() && !Path::new(cwd).is_dir() {
        bail!("会话目录已经不存在：{cwd}");
    }
    #[cfg(target_os = "macos")]
    {
        // `.command` 文件交给「终端」打开，不需要自动化授权。用登录 shell，PATH 与用户平时一致。
        std::fs::create_dir_all(scratch_dir)?;
        let script = scratch_dir.join("resume-session.command");
        let mut text = String::from("#!/bin/zsh -l\n");
        if !cwd.is_empty() {
            text.push_str(&format!("cd {} || exit 1\n", shell_quote(cwd)));
        }
        text.push_str(command);
        text.push('\n');
        std::fs::write(&script, text)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
        let status = std::process::Command::new("open")
            .arg(&script)
            .status()
            .context("无法打开终端")?;
        if !status.success() {
            bail!("无法打开终端");
        }
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        let _ = scratch_dir;
        let inner = if cwd.is_empty() {
            command.to_string()
        } else {
            format!("cd /d \"{cwd}\" && {command}")
        };
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "cmd", "/k", &inner])
            .spawn()
            .context("无法打开终端")?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = scratch_dir;
        let inner = if cwd.is_empty() {
            format!("{command}; exec $SHELL")
        } else {
            format!("cd {} && {command}; exec $SHELL", shell_quote(cwd))
        };
        std::process::Command::new("x-terminal-emulator")
            .args(["-e", "bash", "-lc", &inner])
            .spawn()
            .context("无法打开终端（找不到 x-terminal-emulator）")?;
        Ok(())
    }
}

/// 删除一个 Claude Code 会话：把 `projects/*/<id>.jsonl`（以及同名的附属目录）移进废纸篓，可以从废纸篓找回。
/// 返回会话文件移到了哪里。
pub fn delete_claude_session(claude_home: &Path, id: &str, trash_dir: &Path) -> anyhow::Result<PathBuf> {
    let id = safe_id(id)?;
    let projects = claude_home.join("projects");
    let dirs = std::fs::read_dir(&projects).with_context(|| format!("读取 {}", projects.display()))?;
    for dir in dirs.flatten() {
        let file = dir.path().join(format!("{id}.jsonl"));
        if file.is_file() {
            let moved = move_to_trash(&file, trash_dir)?;
            let companion = dir.path().join(id);
            if companion.is_dir() {
                move_to_trash(&companion, trash_dir)?;
            }
            return Ok(moved);
        }
    }
    bail!("没有找到这个 Claude Code 会话：{id}")
}

/// 删除一个 Grok 会话：会话目录 `sessions/<目录>/<id>/` 移进废纸篓，再从 Grok 的会话索引里删掉这一行
/// （索引库自带触发器，全文索引会一起更新）。返回会话目录移到了哪里。
pub fn delete_grok_session(grok_home: &Path, id: &str, trash_dir: &Path) -> anyhow::Result<PathBuf> {
    let id = safe_id(id)?;
    let sessions = grok_home.join("sessions");
    let mut moved = None;
    if let Ok(dirs) = std::fs::read_dir(&sessions) {
        for dir in dirs.flatten() {
            let candidate = dir.path().join(id);
            if candidate.is_dir() {
                moved = Some(move_to_trash(&candidate, trash_dir)?);
                break;
            }
        }
    }
    let index = sessions.join("session_search.sqlite");
    let mut removed = 0;
    if index.is_file() {
        let connection = rusqlite::Connection::open(&index).with_context(|| format!("打开 {}", index.display()))?;
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        removed = connection.execute("DELETE FROM session_docs WHERE session_id = ?1", [id])?;
    }
    match moved {
        Some(path) => Ok(path),
        None if removed > 0 => Ok(index),
        None => bail!("没有找到这个 Grok 会话：{id}"),
    }
}

/// 移进废纸篓：同名时在文件名后加时间戳，不覆盖废纸篓里已有的东西。只做重命名，不删除任何内容。
fn move_to_trash(path: &Path, trash_dir: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(trash_dir).with_context(|| format!("创建 {}", trash_dir.display()))?;
    let name = path.file_name().context("路径没有文件名")?;
    let mut target = trash_dir.join(name);
    if target.exists() {
        let stamp = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or_default();
        target = trash_dir.join(format!("{} {stamp}", name.to_string_lossy()));
    }
    std::fs::rename(path, &target).with_context(|| format!("移动 {} 到废纸篓", path.display()))?;
    Ok(target)
}

/// 删除会话用的废纸篓：macOS 是用户的 `~/.Trash`，其他系统放在 Codex++ 状态目录里。
pub fn default_trash_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".Trash");
    }
    crate::paths::default_app_state_dir().join("deleted-sessions")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_sessions_use_summary_or_first_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("projects").join("-Users-me-app");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("aaaa-1111.jsonl"),
            concat!(
                "{\"type\":\"user\",\"cwd\":\"/Users/me/app\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/clear</command-name>\"}}\n",
                "{\"type\":\"user\",\"cwd\":\"/Users/me/app\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"修复  登录\\n的 bug\"}]}}\n",
            ),
        )
        .unwrap();
        std::fs::write(
            project.join("bbbb-2222.jsonl"),
            "{\"type\":\"summary\",\"summary\":\"重构网关\"}\n{\"type\":\"user\",\"cwd\":\"/Users/me/app\",\"message\":{\"content\":\"hi\"}}\n",
        )
        .unwrap();
        std::fs::write(project.join("notes.txt"), "x").unwrap();
        let sessions = list_claude_sessions(temp.path(), 10);
        assert_eq!(sessions.len(), 2);
        let first = sessions
            .iter()
            .find(|session| session.id == "aaaa-1111")
            .unwrap();
        assert_eq!(first.title, "修复 登录 的 bug");
        assert_eq!(first.cwd, "/Users/me/app");
        let second = sessions
            .iter()
            .find(|session| session.id == "bbbb-2222")
            .unwrap();
        assert_eq!(second.title, "重构网关");
        assert!(list_claude_sessions(&temp.path().join("missing"), 10).is_empty());
    }

    #[test]
    fn grok_sessions_come_from_its_search_index() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        let connection = rusqlite::Connection::open(dir.join("session_search.sqlite")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session_docs (session_id TEXT PRIMARY KEY, cwd TEXT NOT NULL, updated_at INTEGER NOT NULL, title TEXT NOT NULL, content TEXT NOT NULL, content_hash TEXT NOT NULL);
                 INSERT INTO session_docs VALUES ('s-old', '/tmp/a', 1700000000, '旧会话', '', '');
                 INSERT INTO session_docs VALUES ('s-new', '/tmp/b', 1800000000000, '新会话', '', '');",
            )
            .unwrap();
        drop(connection);
        let sessions = list_grok_sessions(temp.path(), 10);
        assert_eq!(
            sessions
                .iter()
                .map(|session| (session.id.as_str(), session.updated_at_ms))
                .collect::<Vec<_>>(),
            vec![("s-new", 1_800_000_000_000), ("s-old", 1_700_000_000_000)]
        );
    }

    #[test]
    fn resume_commands_reject_unsafe_ids() {
        assert_eq!(
            resume_command("claude", "aaaa-1111").unwrap(),
            "claude --resume aaaa-1111"
        );
        assert_eq!(resume_command("codex", "abc").unwrap(), "codex resume abc");
        assert!(resume_command("claude", "x; rm -rf ~").is_err());
        assert!(resume_command("other", "abc").is_err());
        assert_eq!(shell_quote("/a b/it's"), "'/a b/it'\\''s'");
    }

    #[test]
    fn deleting_a_claude_session_moves_its_file_and_companion_dir_to_trash() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("projects").join("-Users-me");
        std::fs::create_dir_all(project.join("abc-123")).unwrap();
        std::fs::write(project.join("abc-123.jsonl"), "{}\n").unwrap();
        std::fs::write(project.join("keep.jsonl"), "{}\n").unwrap();
        let trash = root.path().join("Trash");
        let moved = delete_claude_session(root.path(), "abc-123", &trash).unwrap();
        assert_eq!(moved, trash.join("abc-123.jsonl"));
        assert!(!project.join("abc-123.jsonl").exists());
        assert!(!project.join("abc-123").exists());
        assert!(trash.join("abc-123").is_dir());
        assert!(project.join("keep.jsonl").exists());
        // 同名再删一次不会覆盖废纸篓里已有的
        std::fs::write(project.join("abc-123.jsonl"), "{}\n").unwrap();
        let again = delete_claude_session(root.path(), "abc-123", &trash).unwrap();
        assert_ne!(again, moved);
        assert!(moved.exists() && again.exists());
        assert!(delete_claude_session(root.path(), "../etc", &trash).is_err());
        assert!(delete_claude_session(root.path(), "missing", &trash).is_err());
    }

    #[test]
    fn deleting_a_grok_session_moves_its_dir_and_drops_the_index_row() {
        let root = tempfile::tempdir().unwrap();
        let sessions = root.path().join("sessions");
        std::fs::create_dir_all(sessions.join("%2FUsers%2Fme").join("s-1")).unwrap();
        let index = sessions.join("session_search.sqlite");
        let connection = rusqlite::Connection::open(&index).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session_docs (session_id TEXT PRIMARY KEY, cwd TEXT NOT NULL, updated_at INTEGER NOT NULL, title TEXT NOT NULL);
                 INSERT INTO session_docs VALUES ('s-1', '/Users/me', 1, 'one'), ('s-2', '/Users/me', 2, 'two');",
            )
            .unwrap();
        drop(connection);
        let trash = root.path().join("Trash");
        let moved = delete_grok_session(root.path(), "s-1", &trash).unwrap();
        assert_eq!(moved, trash.join("s-1"));
        assert!(!sessions.join("%2FUsers%2Fme").join("s-1").exists());
        let left = list_grok_sessions(root.path(), 10);
        assert_eq!(left.iter().map(|session| session.id.as_str()).collect::<Vec<_>>(), vec!["s-2"]);
        assert!(delete_grok_session(root.path(), "nope", &trash).is_err());
    }
}
