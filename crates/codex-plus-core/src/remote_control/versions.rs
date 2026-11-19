//! Codex 桌面 IPC 的方法版本表。
//!
//! 每条 IPC 请求 / 广播都带版本号，与桌面不一致时会被拒（`request-version-mismatch`）或丢弃。
//! 版本表是主进程入口 `.vite/build/early-bootstrap.js` 里的一个对象字面量，所以从已安装
//! Codex 的 `app.asar` 里现读；读不到时退回内置的 26.1111 版本表。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::Value;

pub type IpcVersions = HashMap<String, u64>;

const MAIN_ENTRY: &str = ".vite/build/early-bootstrap.js";
/// 识别版本表用的锚点：只有这张表里有这个键。
const ANCHOR: &str = "\"thread-follower-start-turn\":";

/// Codex 26.1111（build 27194）里的版本表。
pub fn default_versions() -> IpcVersions {
    [
        ("thread-stream-state-changed", 13),
        ("thread-stream-following-changed", 1),
        ("thread-stream-following-status-requested", 1),
        ("ipc-connection-reset", 1),
        ("thread-read-state-changed", 3),
        ("thread-archived", 2),
        ("thread-unarchived", 1),
        ("thread-owner-discovery", 1),
        ("thread-follower-start-turn", 2),
        ("thread-follower-load-complete-history", 1),
        ("thread-follower-compact-thread", 1),
        ("thread-follower-steer-turn", 1),
        ("thread-follower-interrupt-turn", 4),
        ("thread-follower-update-thread-settings", 2),
        ("thread-follower-update-daybreak", 1),
        ("thread-follower-read-model-settings", 1),
        ("thread-follower-edit-last-user-turn", 2),
        ("thread-follower-command-approval-decision", 1),
        ("thread-follower-file-approval-decision", 1),
        ("thread-follower-permissions-request-approval-response", 1),
        ("thread-follower-submit-user-input", 1),
        ("thread-follower-submit-mcp-server-elicitation-response", 1),
        ("thread-follower-set-queued-follow-ups-state", 3),
        ("thread-follower-remove-queued-message", 1),
        ("thread-follower-clear-queued-messages", 1),
        ("thread-queued-followups-changed", 3),
    ]
    .into_iter()
    .map(|(method, version)| (method.to_string(), version))
    .collect()
}

/// 已安装 Codex 的 `app.asar` 路径。
pub fn app_asar_path(app_dir: &Path) -> PathBuf {
    if app_dir.extension().is_some_and(|extension| extension == "app") {
        app_dir.join("Contents").join("Resources").join("app.asar")
    } else {
        app_dir.join("resources").join("app.asar")
    }
}

/// 从 `app.asar` 读版本表；失败时返回错误，由调用方决定是否退回默认表。
pub fn read_versions_from_asar(asar: &Path) -> anyhow::Result<IpcVersions> {
    let source = read_asar_file(asar, MAIN_ENTRY)?;
    extract_versions(&String::from_utf8_lossy(&source))
}

/// 从主进程源码里抽出版本表：锚点所在的那个不含嵌套的对象字面量。
pub fn extract_versions(source: &str) -> anyhow::Result<IpcVersions> {
    let anchor = source.find(ANCHOR).context("没找到 IPC 版本表")?;
    let start = source[..anchor].rfind('{').context("IPC 版本表缺少起始括号")?;
    let end = anchor + source[anchor..].find('}').context("IPC 版本表缺少结束括号")?;
    let literal = &source[start..=end];
    let table: serde_json::Map<String, Value> =
        serde_json::from_str(literal).context("IPC 版本表不是 JSON 对象")?;
    let versions: IpcVersions = table
        .into_iter()
        .filter_map(|(method, version)| version.as_u64().map(|version| (method, version)))
        .collect();
    if versions.len() < 10 {
        bail!("IPC 版本表条目过少：{}", versions.len());
    }
    Ok(versions)
}

/// 读 asar 里的单个文件。
///
/// 头部：u32 魔数、u32 headerSize、u32、u32 jsonSize，JSON 目录从偏移 16 开始，
/// 文件数据从 `8 + headerSize` 开始（不是 16 + headerSize）。
pub fn read_asar_file(asar: &Path, inner_path: &str) -> anyhow::Result<Vec<u8>> {
    let mut file = File::open(asar).with_context(|| format!("打不开 {}", asar.display()))?;
    let mut head = [0u8; 16];
    file.read_exact(&mut head)?;
    let header_size = u32::from_le_bytes(head[4..8].try_into()?) as u64;
    let json_size = u32::from_le_bytes(head[12..16].try_into()?) as usize;
    if json_size > 64 * 1024 * 1024 {
        bail!("asar 目录过大：{json_size}");
    }
    let mut json = vec![0u8; json_size];
    file.read_exact(&mut json)?;
    let mut node: Value = serde_json::from_slice(&json).context("asar 目录不是 JSON")?;
    for part in inner_path.split('/') {
        node = node
            .get_mut("files")
            .and_then(|files| files.get_mut(part))
            .map(Value::take)
            .with_context(|| format!("asar 里没有 {inner_path}"))?;
    }
    if node.get("unpacked").and_then(Value::as_bool) == Some(true) {
        bail!("{inner_path} 在 app.asar.unpacked 里");
    }
    let size = node.get("size").and_then(Value::as_u64).context("asar 条目缺少 size")?;
    let offset = match node.get("offset") {
        Some(Value::String(text)) => text.parse::<u64>().context("asar offset 无效")?,
        Some(Value::Number(number)) => number.as_u64().context("asar offset 无效")?,
        _ => bail!("asar 条目缺少 offset"),
    };
    if size > 256 * 1024 * 1024 {
        bail!("{inner_path} 过大：{size}");
    }
    file.seek(SeekFrom::Start(8 + header_size + offset))?;
    let mut data = vec![0u8; size as usize];
    file.read_exact(&mut data)?;
    Ok(data)
}
