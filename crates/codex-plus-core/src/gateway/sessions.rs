//! 各 Agent 的本地会话：Claude Code（`~/.claude/projects/*/*.jsonl`）、Grok（`~/.grok/sessions/session_search.sqlite`）。
//! Codex 的会话沿用现有的会话库读取（`list_local_sessions`）。
//!
//! Claude Desktop 的 Code 会话另有一份会话记录（`<Desktop 数据目录>/claude-code-sessions/<账号>/<组织>/local_<id>.json`，
//! 标题、目录、归档状态都在里面），对话内容仍是 `~/.claude/projects` 下的 jsonl（记录里的 `cliSessionId`，
//! 改写 / 回退过的还有 `priorCliSessionIds`）。列表里这些 jsonl 合并成一条 Desktop 会话，不再按 jsonl 各出一条。
//!
//! 「继续」在终端里执行 Agent 自己的恢复命令（`claude --resume <id>`、`grok --resume <id>`、`codex resume <id>`）。
//! 「删除」把会话文件 / 目录移进废纸篓（可找回），Grok 另从它的会话索引里删掉这一行。

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::Value;

/// 读 Claude 会话文件头部找标题和目录时最多读多少字节。
const CLAUDE_HEAD_BYTES: u64 = 256 * 1024;

/// Desktop 的会话记录正常只有几十 KB，超过这个大小不读（与 Desktop 自己的上限同量级）。
const DESKTOP_RECORD_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// Desktop 会话 id 的前缀：`local_<uuid>`。
const DESKTOP_ID_PREFIX: &str = "local_";

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
    /// 会话来自哪个客户端：Claude Desktop 的会话是 `desktop`，命令行的不带。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
    /// Desktop 里已归档。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// 在终端恢复时用的 id（Desktop 会话是它的 `cliSessionId`，与列表 id 不同）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_id: Option<String>,
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
        surface: None,
        archived: false,
        resume_id: None,
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
            surface: None,
            archived: false,
            resume_id: None,
        })
        .collect()
}

/// Claude Desktop 可能用到的数据目录（只返回已存在的）：第三方网关模式在 `Claude-3p`，官方登录在 `Claude`。
/// macOS `~/Library/Application Support`，Windows `%LOCALAPPDATA%` / `%APPDATA%` 以及 MSIX 包的虚拟化目录，
/// Linux `~/.config`。
pub fn claude_desktop_data_dirs() -> Vec<PathBuf> {
    let mut bases: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(PathBuf::from(home).join("Library/Application Support"));
    }
    #[cfg(target_os = "windows")]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        if let Some(local) = &local {
            bases.push(local.clone());
        }
        if let Some(roaming) = std::env::var_os("APPDATA") {
            bases.push(PathBuf::from(roaming));
        }
        if let Some(local) = &local
            && let Ok(packages) = std::fs::read_dir(local.join("Packages"))
        {
            let mut found: Vec<PathBuf> = packages
                .flatten()
                .filter(|entry| {
                    let name = entry.file_name().to_string_lossy().to_string();
                    name.starts_with("Claude_") || name.starts_with("AnthropicPBC.Claude_")
                })
                .map(|entry| entry.path().join("LocalCache"))
                .collect();
            found.sort();
            for cache in found {
                bases.push(cache.join("Local"));
                bases.push(cache.join("Roaming"));
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some(home) = std::env::var_os("HOME") {
        bases.push(PathBuf::from(home).join(".config"));
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for base in bases {
        for name in ["Claude-3p", "Claude"] {
            let dir = base.join(name);
            if dir.join("claude-code-sessions").is_dir() && !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// 一条 Desktop 会话记录：列表里显示的会话，加上它占用的命令行会话 id（对应 `~/.claude/projects` 下的 jsonl）。
struct DesktopRecord {
    session: AgentSession,
    cli_ids: Vec<String>,
}

/// Desktop 所有的会话记录文件：`<数据目录>/claude-code-sessions/<账号>/<组织>/local_*.json`。
fn desktop_record_files(desktop_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in desktop_dirs {
        let Ok(accounts) = std::fs::read_dir(dir.join("claude-code-sessions")) else {
            continue;
        };
        for account in accounts.flatten() {
            let Ok(orgs) = std::fs::read_dir(account.path()) else {
                continue;
            };
            for org in orgs.flatten() {
                let Ok(entries) = std::fs::read_dir(org.path()) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(DESKTOP_ID_PREFIX)
                        && name.ends_with(".json")
                        && entry.path().is_file()
                    {
                        files.push(entry.path());
                    }
                }
            }
        }
    }
    files
}

/// 记录里占用的命令行会话 id：当前的 `cliSessionId`、取消归档前的 `unarchivedCliSessionId`、改写过的 `priorCliSessionIds`。
fn desktop_cli_ids(value: &Value) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut push = |id: Option<&str>| {
        if let Some(id) = id.map(str::trim).filter(|id| !id.is_empty())
            && !ids.iter().any(|known| known == id)
        {
            ids.push(id.to_string());
        }
    };
    push(value.get("cliSessionId").and_then(Value::as_str));
    push(value.get("unarchivedCliSessionId").and_then(Value::as_str));
    for id in value
        .get("priorCliSessionIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        push(id.as_str());
    }
    ids
}

fn read_desktop_record(path: &Path) -> Option<DesktopRecord> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > DESKTOP_RECORD_MAX_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let file_id = path.file_stem()?.to_str()?.to_string();
    let id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| *id == file_id)
        .unwrap_or(&file_id)
        .to_string();
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default();
    let updated_at_ms = ["lastActivityAt", "createdAt"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
        .map(normalize_timestamp)
        .unwrap_or(modified);
    let cli_ids = desktop_cli_ids(&value);
    let title = text("title");
    Some(DesktopRecord {
        session: AgentSession {
            agent: "claude".to_string(),
            id,
            title: shorten(&title.split_whitespace().collect::<Vec<_>>().join(" "), 120),
            cwd: text("cwd"),
            updated_at_ms,
            path: path.display().to_string(),
            surface: Some("desktop".to_string()),
            archived: value.get("isArchived").and_then(Value::as_bool) == Some(true),
            resume_id: cli_ids.first().cloned(),
        },
        cli_ids,
    })
}

/// Claude Code 的会话，连同 Claude Desktop 里的：Desktop 占用的 jsonl 不再单独出现，
/// 由 Desktop 的记录代替（用 Desktop 里的标题、归档状态）。按更新时间倒序，最多 `limit` 个。
pub fn list_claude_sessions_with_desktop(
    claude_home: &Path,
    desktop_dirs: &[PathBuf],
    limit: usize,
) -> Vec<AgentSession> {
    let records: Vec<DesktopRecord> = desktop_record_files(desktop_dirs)
        .iter()
        .filter_map(|path| read_desktop_record(path))
        .collect();
    let owned: HashSet<&str> = records
        .iter()
        .flat_map(|record| record.cli_ids.iter().map(String::as_str))
        .collect();
    // 被 Desktop 占用的 jsonl 会被滤掉，多取一些，免得命令行会话不够 `limit` 个。
    let mut sessions: Vec<AgentSession> = list_claude_sessions(claude_home, limit + owned.len())
        .into_iter()
        .filter(|session| !owned.contains(session.id.as_str()))
        .collect();
    sessions.extend(records.into_iter().map(|record| record.session));
    sessions.sort_by(|left, right| right.updated_at_ms.cmp(&left.updated_at_ms));
    sessions.truncate(limit);
    sessions
}

/// 删除一个 Claude Desktop 会话，做法与 Desktop 自己删除时一致：在记录旁写 `deleted_<id>` 墓碑（内容是删除时刻），
/// 会话记录和它的对话 jsonl 移进废纸篓，再从归档索引里去掉。还被别的 Desktop 会话引用的 jsonl 不动。
/// Desktop 运行时内存里还有这个会话，可能把记录写回来，调用方要先确认 Desktop 没在运行。
/// 返回会话记录移到了哪里。
pub fn delete_claude_desktop_session(
    claude_home: &Path,
    desktop_dirs: &[PathBuf],
    id: &str,
    trash_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let id = safe_id(id)?;
    let Some(stem) = id.strip_prefix(DESKTOP_ID_PREFIX).filter(|stem| !stem.is_empty()) else {
        bail!("不是 Claude Desktop 会话：{id}");
    };
    let files = desktop_record_files(desktop_dirs);
    let file_name = format!("{id}.json");
    let Some(record_path) = files
        .iter()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(file_name.as_str()))
        .cloned()
    else {
        bail!("没有找到这个 Claude Desktop 会话：{id}");
    };
    let org_dir = record_path.parent().context("会话记录没有上级目录")?.to_path_buf();
    let value: Value = serde_json::from_slice(
        &std::fs::read(&record_path).with_context(|| format!("读取 {}", record_path.display()))?,
    )
    .with_context(|| format!("解析 {}", record_path.display()))?;
    // 其他会话（比如分叉出来的）还引用的 jsonl 保留。
    let shared: HashSet<String> = files
        .iter()
        .filter(|path| **path != record_path)
        .filter_map(|path| read_desktop_record(path))
        .flat_map(|record| record.cli_ids)
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let tombstone = org_dir.join(format!("deleted_{stem}"));
    std::fs::write(&tombstone, now.to_string())
        .with_context(|| format!("写入 {}", tombstone.display()))?;
    let moved = move_to_trash(&record_path, trash_dir)?;
    for cli_id in desktop_cli_ids(&value) {
        if shared.contains(&cli_id) || safe_id(&cli_id).is_err() {
            continue;
        }
        // 对话文件可能早就不在了（Desktop 标过 transcriptUnavailable），找不到不算失败。
        move_claude_transcript(claude_home, &cli_id, trash_dir)?;
    }
    drop_from_archived_index(&org_dir.join("archived-sessions.idx"), id)?;
    Ok(moved)
}

/// 从 Desktop 的归档索引（`{"v":1,"archived":[...]}`）里去掉一个会话。索引不存在、读不懂或不含这个会话时不动它。
fn drop_from_archived_index(path: &Path, id: &str) -> anyhow::Result<()> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let Some(archived) = value.get_mut("archived").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    let before = archived.len();
    archived.retain(|item| item.as_str() != Some(id));
    if archived.len() == before {
        return Ok(());
    }
    std::fs::write(path, serde_json::to_string(&value)?)
        .with_context(|| format!("写入 {}", path.display()))
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
        // 命令写进 .cmd 再交给 cmd /k：直接当参数传时 Rust 会把路径里的引号转义成 \"，cmd 不认，
        // cd 失败后 && 后面的恢复命令不会执行。cmd 边执行边读批处理文件，每次用新文件名，免得被下一次覆盖。
        // 先切到 UTF-8 代码页，中文路径才不会乱码。
        std::fs::create_dir_all(scratch_dir)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        let script = scratch_dir.join(format!("resume-session-{stamp}.cmd"));
        let mut text = String::from("@echo off\r\nchcp 65001 >nul\r\n");
        if !cwd.is_empty() {
            text.push_str(&format!("cd /d \"{cwd}\" || exit /b 1\r\n"));
        }
        text.push_str(&command.replace('%', "%%"));
        text.push_str("\r\n");
        std::fs::write(&script, text)?;
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "cmd", "/k"])
            .arg(&script)
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
pub fn delete_claude_session(
    claude_home: &Path,
    id: &str,
    trash_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let id = safe_id(id)?;
    let projects = claude_home.join("projects");
    std::fs::read_dir(&projects).with_context(|| format!("读取 {}", projects.display()))?;
    match move_claude_transcript(claude_home, id, trash_dir)? {
        Some(moved) => Ok(moved),
        None => bail!("没有找到这个 Claude Code 会话：{id}"),
    }
}

/// 把 `projects/*/<id>.jsonl`（以及同名的附属目录）移进废纸篓。没有这个文件时返回 `None`。
fn move_claude_transcript(
    claude_home: &Path,
    id: &str,
    trash_dir: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let Ok(dirs) = std::fs::read_dir(claude_home.join("projects")) else {
        return Ok(None);
    };
    for dir in dirs.flatten() {
        let file = dir.path().join(format!("{id}.jsonl"));
        if file.is_file() {
            let moved = move_to_trash(&file, trash_dir)?;
            let companion = dir.path().join(id);
            if companion.is_dir() {
                move_to_trash(&companion, trash_dir)?;
            }
            return Ok(Some(moved));
        }
    }
    Ok(None)
}

/// 删除一个 Grok 会话：会话目录 `sessions/<目录>/<id>/` 移进废纸篓，再从 Grok 的会话索引里删掉这一行
/// （索引库自带触发器，全文索引会一起更新）。返回会话目录移到了哪里。
pub fn delete_grok_session(
    grok_home: &Path,
    id: &str,
    trash_dir: &Path,
) -> anyhow::Result<PathBuf> {
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
        let connection = rusqlite::Connection::open(&index)
            .with_context(|| format!("打开 {}", index.display()))?;
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

    /// 造一份 Desktop 数据目录：`claude-code-sessions/<账号>/<组织>/`，返回组织目录。
    fn desktop_org_dir(root: &Path) -> PathBuf {
        let org = root
            .join("Claude-3p")
            .join("claude-code-sessions")
            .join("acct")
            .join("org");
        std::fs::create_dir_all(&org).unwrap();
        org
    }

    fn write_transcript(claude_home: &Path, id: &str, prompt: &str) {
        let project = claude_home.join("projects").join("-Users-me-app");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join(format!("{id}.jsonl")),
            format!(
                "{{\"type\":\"user\",\"cwd\":\"/Users/me/app\",\"message\":{{\"content\":\"{prompt}\"}}}}\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn desktop_sessions_replace_the_transcripts_they_own() {
        let root = tempfile::tempdir().unwrap();
        let claude_home = root.path().join(".claude");
        write_transcript(&claude_home, "cli-now", "第一句");
        write_transcript(&claude_home, "cli-before", "改写前");
        write_transcript(&claude_home, "cli-only", "命令行里开的");
        let org = desktop_org_dir(root.path());
        std::fs::write(
            org.join("local_d1.json"),
            r#"{"sessionId":"local_d1","cliSessionId":"cli-now","priorCliSessionIds":["cli-before"],
                "cwd":"/Users/me/app","title":"Desktop 里的  标题","isArchived":true,
                "createdAt":1700000000000,"lastActivityAt":1800000000000}"#,
        )
        .unwrap();
        // 墓碑、归档索引、坏文件都不算会话
        std::fs::write(org.join("deleted_d0"), "1").unwrap();
        std::fs::write(org.join("archived-sessions.idx"), r#"{"v":1,"archived":["local_d1"]}"#)
            .unwrap();
        std::fs::write(org.join("local_bad.json"), "not json").unwrap();
        let dirs = vec![root.path().join("Claude-3p")];

        let sessions = list_claude_sessions_with_desktop(&claude_home, &dirs, 10);
        let ids: Vec<&str> = sessions.iter().map(|session| session.id.as_str()).collect();
        assert_eq!(ids, vec!["local_d1", "cli-only"]);
        let desktop = &sessions[0];
        assert_eq!(desktop.title, "Desktop 里的 标题");
        assert_eq!(desktop.cwd, "/Users/me/app");
        assert_eq!(desktop.updated_at_ms, 1_800_000_000_000);
        assert_eq!(desktop.surface.as_deref(), Some("desktop"));
        assert!(desktop.archived);
        assert_eq!(desktop.resume_id.as_deref(), Some("cli-now"));
        assert_eq!(sessions[1].surface, None);
        // 前端靠这几个字段区分 Desktop 会话；命令行会话不带它们
        let json = serde_json::to_value(&sessions).unwrap();
        assert_eq!(json[0]["surface"], "desktop");
        assert_eq!(json[0]["resumeId"], "cli-now");
        assert_eq!(json[0]["archived"], true);
        assert!(json[1].get("surface").is_none() && json[1].get("archived").is_none());
        // 没有 Desktop 数据时与原来一样
        assert_eq!(list_claude_sessions_with_desktop(&claude_home, &[], 10).len(), 3);
        assert_eq!(list_claude_sessions_with_desktop(&claude_home, &dirs, 1).len(), 1);
    }

    #[test]
    fn deleting_a_desktop_session_tombstones_it_and_trashes_record_and_transcripts() {
        let root = tempfile::tempdir().unwrap();
        let claude_home = root.path().join(".claude");
        write_transcript(&claude_home, "cli-a", "a");
        write_transcript(&claude_home, "cli-shared", "shared");
        let org = desktop_org_dir(root.path());
        std::fs::write(
            org.join("local_d1.json"),
            r#"{"sessionId":"local_d1","cliSessionId":"cli-a","priorCliSessionIds":["cli-shared","cli-gone"]}"#,
        )
        .unwrap();
        // 分叉出来的会话还引用着 cli-shared
        std::fs::write(
            org.join("local_d2.json"),
            r#"{"sessionId":"local_d2","cliSessionId":"cli-shared"}"#,
        )
        .unwrap();
        std::fs::write(
            org.join("archived-sessions.idx"),
            r#"{"v":1,"archived":["local_d1","local_d2"]}"#,
        )
        .unwrap();
        let dirs = vec![root.path().join("Claude-3p")];
        let trash = root.path().join("Trash");

        let moved = delete_claude_desktop_session(&claude_home, &dirs, "local_d1", &trash).unwrap();
        assert_eq!(moved, trash.join("local_d1.json"));
        assert!(!org.join("local_d1.json").exists());
        let stamp: u128 = std::fs::read_to_string(org.join("deleted_d1"))
            .unwrap()
            .parse()
            .unwrap();
        assert!(stamp > 1_700_000_000_000);
        let project = claude_home.join("projects").join("-Users-me-app");
        assert!(!project.join("cli-a.jsonl").exists());
        assert!(trash.join("cli-a.jsonl").exists());
        assert!(project.join("cli-shared.jsonl").exists());
        let index = std::fs::read_to_string(org.join("archived-sessions.idx")).unwrap();
        assert!(!index.contains("local_d1") && index.contains("local_d2"));
        let left = list_claude_sessions_with_desktop(&claude_home, &dirs, 10);
        assert_eq!(
            left.iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["local_d2"]
        );

        assert!(delete_claude_desktop_session(&claude_home, &dirs, "local_d1", &trash).is_err());
        assert!(delete_claude_desktop_session(&claude_home, &dirs, "cli-a", &trash).is_err());
        assert!(delete_claude_desktop_session(&claude_home, &dirs, "local_../x", &trash).is_err());
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
        assert_eq!(
            left.iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            vec!["s-2"]
        );
        assert!(delete_grok_session(root.path(), "nope", &trash).is_err());
    }
}
