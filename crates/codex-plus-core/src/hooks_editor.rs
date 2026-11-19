//! Codex hooks 编辑器的后端（注入页 `96-hooks-editor.js`）。
//!
//! Codex 原生设置页只能列出 hooks、标记信任；hook 本身要手写 `hooks.json`。这里提供三件事：
//!
//! - 列出能放 hooks 的文件：用户层 `<CODEX_HOME>/hooks.json`，以及项目层 `<项目>/.codex/hooks.json`。
//!   项目层只有在 config.toml 里 `[projects."<路径>"] trust_level = "trusted"` 的项目才会被 Codex 加载，
//!   所以项目清单就从 config.toml 的 `[projects]` 读，再加上页面里手动添加的目录（标明是否受信任）。
//! - 按版本号写回某个文件：只接受上面两种位置，写前比对版本防止覆盖别处的改动，写入是原子替换。
//! - 试运行一个 command hook：和 Codex 一样用 `$SHELL -c <command>`，把示例事件 JSON 喂给 stdin，
//!   带超时，返回退出码与输出。Windows 上用 `cmd /C`，只能近似。
//!
//! 文件内容由页面按原始 JSON 编辑，后端只校验是合法的 JSON 对象，未知字段原样保留。
//! 写入后生效靠页面随后发一次 `config/batchWrite`（`reloadUserConfig: true`）让 app-server 重读。
//! 这几条路由都不在拓展白名单里：能写文件、能执行命令。

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// hooks.json 的上限；Codex 自己不限制，但正常文件不会这么大，防止误读大文件卡住页面。
const MAX_HOOKS_FILE_BYTES: u64 = 1024 * 1024;
/// 试运行输出各自最多保留这么多字节。
const MAX_TEST_OUTPUT_BYTES: usize = 64 * 1024;
/// 试运行的超时上限（秒）。hook 自己的 timeout 默认 600 秒，试运行没必要等那么久。
const MAX_TEST_TIMEOUT_SECS: u64 = 120;
const DEFAULT_TEST_TIMEOUT_SECS: u64 = 30;

/// 文件内容的版本号：内容的 sha256；文件不存在时为空串。
pub fn content_version(bytes: Option<&[u8]>) -> String {
    match bytes {
        Some(bytes) => format!("{:x}", Sha256::digest(bytes)),
        None => String::new(),
    }
}

pub fn user_hooks_path(codex_home: &Path) -> PathBuf {
    codex_home.join("hooks.json")
}

pub fn project_hooks_path(root: &Path) -> PathBuf {
    root.join(".codex").join("hooks.json")
}

/// config.toml 里登记过的项目：`(路径, 是否受信任)`，按路径排序。
pub fn configured_projects(codex_home: &Path) -> Vec<(String, bool)> {
    let Some(config) = read_config_toml(codex_home) else {
        return Vec::new();
    };
    let Some(projects) = config.get("projects").and_then(toml::Value::as_table) else {
        return Vec::new();
    };
    let mut out: Vec<(String, bool)> = projects
        .iter()
        .map(|(path, entry)| {
            let trusted = entry.get("trust_level").and_then(toml::Value::as_str) == Some("trusted");
            (path.clone(), trusted)
        })
        .collect();
    out.sort();
    out
}

/// config.toml 里是否也直接写了 hooks 事件（`[[hooks.<Event>]]`）。`[hooks.state]` 是开关与信任记录，不算。
/// 同一层既有 hooks.json 又有 config.toml 定义时 Codex 会提示「请只用一种写法」。
pub fn config_defines_hooks(codex_home: &Path) -> bool {
    read_config_toml(codex_home)
        .as_ref()
        .and_then(|config| config.get("hooks"))
        .and_then(toml::Value::as_table)
        .is_some_and(|hooks| hooks.keys().any(|key| key != "state"))
}

fn read_config_toml(codex_home: &Path) -> Option<toml::Value> {
    let text = std::fs::read_to_string(codex_home.join("config.toml")).ok()?;
    toml::from_str(&text).ok()
}

/// 一个 hooks 文件的现状。读不了（太大、不是 UTF-8）时 `error` 有值，`text` 为空。
fn describe_file(scope: &str, root: Option<&str>, trusted: bool, path: &Path) -> Value {
    let mut entry = json!({
        "scope": scope,
        "root": root,
        "path": path.to_string_lossy(),
        "canonicalPath": canonical_path(path).to_string_lossy(),
        "trusted": trusted,
        "exists": false,
        "text": "",
        "version": "",
        "error": Value::Null,
    });
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            entry["exists"] = json!(true);
            if meta.len() > MAX_HOOKS_FILE_BYTES {
                entry["error"] = json!("文件超过 1 MiB，不在这里编辑");
                return entry;
            }
            match std::fs::read(path) {
                Ok(bytes) => {
                    entry["version"] = json!(content_version(Some(&bytes)));
                    match String::from_utf8(bytes) {
                        Ok(text) => entry["text"] = json!(text),
                        Err(_) => entry["error"] = json!("文件不是 UTF-8 文本"),
                    }
                }
                Err(error) => entry["error"] = json!(format!("读取失败：{error}")),
            }
        }
        Ok(_) => {
            entry["exists"] = json!(true);
            entry["error"] = json!("同名路径不是普通文件");
        }
        Err(_) => {}
    }
    entry
}

/// Codex 的 hook key 以规范化后的文件路径开头（macOS 上 `/tmp` 会变成 `/private/tmp`）。
/// 文件还不存在时规范化父目录再接上文件名。
fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(path) = std::fs::canonicalize(path) {
        return path;
    }
    match (path.parent().and_then(|parent| std::fs::canonicalize(parent).ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

/// 项目目录必须是存在的绝对路径，且不含 `..`。
fn validate_project_root(raw: &str) -> anyhow::Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("项目目录为空");
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        bail!("项目目录必须是绝对路径");
    }
    if path.components().any(|part| matches!(part, Component::ParentDir)) {
        bail!("项目目录不能包含 ..");
    }
    if !path.is_dir() {
        bail!("项目目录不存在：{trimmed}");
    }
    Ok(path)
}

/// `/hooks/files`：`{ roots?: [额外的项目目录] }`。
///
/// 返回用户层文件、config.toml 里登记的项目、页面额外添加的项目（不存在的目录跳过并记进 `skipped`）。
pub fn list_files(codex_home: &Path, payload: &Value) -> Value {
    let mut files = vec![describe_file("user", None, true, &user_hooks_path(codex_home))];
    let projects = configured_projects(codex_home);
    let mut seen: Vec<String> = Vec::new();
    for (root, trusted) in &projects {
        if seen.contains(root) {
            continue;
        }
        seen.push(root.clone());
        let mut entry = describe_file("project", Some(root), *trusted, &project_hooks_path(Path::new(root)));
        entry["configured"] = json!(true);
        entry["rootExists"] = json!(Path::new(root).is_dir());
        files.push(entry);
    }
    let mut skipped = Vec::new();
    for raw in payload
        .get("roots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let root = match validate_project_root(raw) {
            Ok(path) => path.to_string_lossy().into_owned(),
            Err(error) => {
                skipped.push(json!({ "root": raw, "message": error.to_string() }));
                continue;
            }
        };
        if seen.contains(&root) {
            continue;
        }
        seen.push(root.clone());
        let mut entry = describe_file("project", Some(&root), false, &project_hooks_path(Path::new(&root)));
        entry["configured"] = json!(false);
        entry["rootExists"] = json!(true);
        files.push(entry);
    }
    json!({
        "status": "ok",
        "codexHome": codex_home.to_string_lossy(),
        "files": files,
        "skipped": skipped,
        "configDefinesHooks": config_defines_hooks(codex_home),
    })
}

/// 校验要写入的内容：合法 JSON 对象；有 `hooks` 时必须是对象，每个事件是数组。
pub fn validate_hooks_document(text: &str) -> anyhow::Result<()> {
    let value: Value = serde_json::from_str(text).map_err(|error| anyhow!("不是合法的 JSON：{error}"))?;
    let object = value.as_object().ok_or_else(|| anyhow!("顶层必须是 JSON 对象"))?;
    if let Some(hooks) = object.get("hooks") {
        let hooks = hooks.as_object().ok_or_else(|| anyhow!("hooks 必须是对象（事件名 → 分组数组）"))?;
        for (event, groups) in hooks {
            if !groups.is_array() {
                bail!("hooks.{event} 必须是数组");
            }
        }
    }
    Ok(())
}

/// `/hooks/file/write`：`{ scope: "user" | "project", root?, text, expectedVersion }`。
///
/// 只能写用户层或某个项目的 `.codex/hooks.json`；`expectedVersion` 与磁盘上的版本不一致时拒绝
/// （文件不存在时版本是空串）。成功返回新版本号与路径。
pub fn write_file(codex_home: &Path, payload: &Value) -> anyhow::Result<Value> {
    let path = match payload.get("scope").and_then(Value::as_str) {
        Some("user") => user_hooks_path(codex_home),
        Some("project") => {
            let root = payload.get("root").and_then(Value::as_str).unwrap_or_default();
            project_hooks_path(&validate_project_root(root)?)
        }
        _ => bail!("scope 只能是 user 或 project"),
    };
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("缺少 text"))?;
    if text.len() as u64 > MAX_HOOKS_FILE_BYTES {
        bail!("内容超过 1 MiB");
    }
    validate_hooks_document(text)?;
    let expected = payload
        .get("expectedVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("缺少 expectedVersion"))?;
    let current = match std::fs::read(&path) {
        Ok(bytes) => content_version(Some(&bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
    };
    if current != expected {
        bail!("文件已在别处被修改，请重新读取后再保存");
    }
    crate::settings::atomic_write(&path, text.as_bytes())?;
    Ok(json!({
        "status": "ok",
        "path": path.to_string_lossy(),
        "version": content_version(Some(text.as_bytes())),
    }))
}

fn truncate_output(bytes: &[u8]) -> (String, bool) {
    if bytes.len() <= MAX_TEST_OUTPUT_BYTES {
        return (String::from_utf8_lossy(bytes).into_owned(), false);
    }
    (String::from_utf8_lossy(&bytes[..MAX_TEST_OUTPUT_BYTES]).into_owned(), true)
}

/// 试运行用的 shell：`(程序, 参数前缀, 说明)`。
fn test_shell(command: &str, command_windows: Option<&str>) -> (String, Vec<String>, String) {
    if cfg!(windows) {
        let command = command_windows.filter(|value| !value.trim().is_empty()).unwrap_or(command);
        return (
            "cmd".into(),
            vec!["/C".into(), command.to_owned()],
            "cmd /C（近似：Codex 在 Windows 上的执行方式可能不同）".into(),
        );
    }
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/bin/sh".into());
    let label = format!("{shell} -c");
    (shell, vec!["-c".into(), command.to_owned()], label)
}

/// `/hooks/test-run`：`{ command, commandWindows?, cwd?, stdin?, timeoutSec? }`。
///
/// 和 Codex 一样起 `$SHELL -c <command>`，环境继承 Codex++ 进程并补上 `CODEX_HOME`，stdin 写入示例事件。
/// 超时后结束进程并标记 `timedOut`。输出各自最多 64 KiB。
pub async fn test_run(codex_home: &Path, payload: &Value) -> anyhow::Result<Value> {
    let command = payload
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("command 为空"))?;
    let command_windows = payload.get("commandWindows").and_then(Value::as_str);
    let stdin = payload.get("stdin").and_then(Value::as_str).unwrap_or_default().to_owned();
    let timeout_secs = payload
        .get("timeoutSec")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TEST_TIMEOUT_SECS)
        .min(MAX_TEST_TIMEOUT_SECS);
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())
        .or_else(|| codex_home.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("/"));

    let (program, args, shell_label) = test_shell(command, command_windows);
    let mut command = tokio::process::Command::new(&program);
    command
        .args(&args)
        .current_dir(&cwd)
        .env("CODEX_HOME", codex_home)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // 管理器是 GUI 进程，不设这个标志 cmd 会闪黑框。
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().with_context(|| format!("无法启动 {program}"))?;

    let started = Instant::now();
    if let Some(mut pipe) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        // 命令不读 stdin 时写入会遇到断管，不算失败。
        let _ = pipe.write_all(stdin.as_bytes()).await;
        drop(pipe);
    }
    let waited = tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    let base = json!({
        "status": "ok",
        "shell": shell_label,
        "cwd": cwd.to_string_lossy(),
        "durationMs": duration_ms,
        "timeoutSec": timeout_secs,
    });
    let mut result = base;
    match waited {
        Err(_) => {
            // wait_with_output 的 future 被丢弃时子进程随 kill_on_drop 结束。
            result["timedOut"] = json!(true);
            result["exitCode"] = Value::Null;
            result["stdout"] = json!("");
            result["stderr"] = json!("");
        }
        Ok(output) => {
            let output = output.context("等待进程结束失败")?;
            let (stdout, stdout_truncated) = truncate_output(&output.stdout);
            let (stderr, stderr_truncated) = truncate_output(&output.stderr);
            result["timedOut"] = json!(false);
            result["exitCode"] = json!(output.status.code());
            result["stdout"] = json!(stdout);
            result["stderr"] = json!(stderr);
            result["truncated"] = json!(stdout_truncated || stderr_truncated);
        }
    }
    Ok(result)
}
