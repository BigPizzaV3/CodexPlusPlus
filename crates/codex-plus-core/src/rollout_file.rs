//! Codex 原生会话文件（rollout jsonl）的定位、读取与原子写入，供侧聊保存使用。
//!
//! 原先还有会话分享的导出 / 导入 / 加密链接，分享下线后只留文件读写这一块。

use anyhow::{Context, bail};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// 共用文件定位和读取；length 仅指定读取截止位置，不包含保存的大小策略。
pub(crate) fn read_rollout(
    home: &Path,
    session_id: &str,
    length: Option<u64>,
) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        bail!("会话 ID 为空");
    }
    let path =
        find_rollout(home, session_id).ok_or_else(|| anyhow::anyhow!("找不到原生会话文件"))?;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| {
            file.take(length.unwrap_or(u64::MAX))
                .read_to_end(&mut bytes)
        })
        .with_context(|| format!("读取会话文件失败：{}", path.display()))?;
    Ok((path, bytes))
}

/// 原子写入完整历史。索引登记由调用方或原生 thread/resume 完成。
pub(crate) fn write_rollout(
    directory: &Path,
    filename: &str,
    content: &str,
) -> anyhow::Result<PathBuf> {
    fs::create_dir_all(directory).context("创建会话目录失败")?;
    let path = directory.join(filename);
    if path.exists() {
        if fs::read_to_string(&path)? == content {
            return Ok(path);
        }
        bail!("保存目标已存在且内容不同，未覆盖原文件");
    }
    let temporary = directory.join(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        // 原子发布且不覆盖同名文件，避免并发重试时 rename 覆盖已有内容。
        fs::hard_link(&temporary, &path)?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result.context("写入会话文件失败")?;
    Ok(path)
}

fn find_rollout(home: &Path, session_id: &str) -> Option<PathBuf> {
    for root in [home.join("sessions"), home.join("archived_sessions")] {
        if let Some(path) = find_rollout_in(&root, session_id) {
            return Some(path);
        }
    }
    None
}

fn find_rollout_in(root: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_rollout_in(&path, session_id) {
                return Some(found);
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("jsonl")
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.contains(session_id))
        {
            return Some(path);
        }
    }
    None
}
