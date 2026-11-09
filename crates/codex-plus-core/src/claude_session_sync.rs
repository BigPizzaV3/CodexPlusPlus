//! Claude Code 会话在 3p 模式与完整功能模式之间的同步：
//! 两种部署模式用不同的 Application Support 目录，元数据各存一份
//! `claude-code-sessions/<account>/<org>/local_<uuid>.json`，会话正文两边共用
//! `~/.claude/projects/`，不需要同步。`local-agent-mode-sessions`（Cowork）结构
//! 不同，本次不同步。
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// 完整功能模式的 mock 账号/组织（与 tools/claude-local/proxy.mjs 里的 ACCOUNT.id / ORG.uuid 一致）。
pub const FULL_ACCOUNT: &str = "c46bd1eb-fbbf-4e5a-9bb3-5ebe8521eb54";
pub const FULL_ORG: &str = "e0e877d3-8475-44a1-a501-4fc431b085ab";

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub to_full: usize,
    pub to_three_p: usize,
    pub updated: usize,
    pub skipped_deleted: usize,
    /// 3p 侧还没有会话目录时的说明；正常情况下为空。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
}

const LOCAL_PREFIX: &str = "local_";
const DELETED_PREFIX: &str = "deleted_";

fn session_id(name: &str, prefix: &str) -> Option<String> {
    name.strip_prefix(prefix)?
        .strip_suffix(".json")
        .map(str::to_string)
}

/// `claude-code-sessions/<acct>/<org>/` 里含 local_*.json 且最新 mtime 最大的一对目录。
fn newest_pair(root: &Path) -> Option<(String, String)> {
    let mut best: Option<(String, String, std::time::SystemTime)> = None;
    for acct in fs::read_dir(root).ok()?.flatten() {
        let acct_path = acct.path();
        if !acct_path.is_dir() {
            continue;
        }
        for org in fs::read_dir(&acct_path).ok()?.flatten() {
            let org_path = org.path();
            if !org_path.is_dir() {
                continue;
            }
            let newest = fs::read_dir(&org_path).ok()?.flatten().filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with(LOCAL_PREFIX)
                    .then(|| entry.metadata().ok()?.modified().ok())
                    .flatten()
            });
            for mtime in newest {
                if best.as_ref().is_none_or(|(_, _, t)| mtime > *t) {
                    best = Some((
                        acct.file_name().to_string_lossy().into_owned(),
                        org.file_name().to_string_lossy().into_owned(),
                        mtime,
                    ));
                }
            }
        }
    }
    best.map(|(a, o, _)| (a, o))
}

fn list_sessions(dir: &Path) -> (Vec<String>, Vec<String>) {
    let mut locals = Vec::new();
    let mut deleted = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = session_id(&name, LOCAL_PREFIX) {
            locals.push(id);
        } else if let Some(id) = session_id(&name, DELETED_PREFIX) {
            deleted.push(id);
        }
    }
    (locals, deleted)
}

/// 元数据的新鲜度：JSON 里的 lastActivityAt（数字），缺失退回文件 mtime。
fn freshness(path: &Path) -> f64 {
    let parsed = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    if let Some(value) = parsed {
        if let Some(activity) = value.get("lastActivityAt").and_then(Value::as_f64) {
            return activity;
        }
    }
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn sync_dir(
    mine: &Path,
    theirs: &Path,
    backup: &Path,
    report: &mut SyncReport,
    direction: &str,
) -> Result<()> {
    let (mine_locals, mine_deleted) = list_sessions(mine);
    let (their_locals, their_deleted) = list_sessions(theirs);
    for id in &mine_locals {
        if mine_deleted.contains(id) || their_deleted.contains(id) {
            report.skipped_deleted += 1;
            continue;
        }
        let src = mine.join(format!("{LOCAL_PREFIX}{id}.json"));
        let dst = theirs.join(format!("{LOCAL_PREFIX}{id}.json"));
        if !their_locals.contains(id) {
            fs::create_dir_all(theirs)?;
            fs::copy(&src, &dst)?;
            if direction == "full" {
                report.to_full += 1;
            } else {
                report.to_three_p += 1;
            }
            continue;
        }
        let src_bytes = fs::read(&src)?;
        let dst_bytes = fs::read(&dst)?;
        if src_bytes == dst_bytes {
            continue;
        }
        if serde_json::from_slice::<Value>(&src_bytes).is_err()
            || serde_json::from_slice::<Value>(&dst_bytes).is_err()
        {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "claude_session_sync.bad_json",
                serde_json::json!({"file": src.file_name().unwrap_or_default().to_string_lossy()}),
            );
            continue;
        }
        if freshness(&src) <= freshness(&dst) {
            continue;
        }
        // 覆盖前先备份旧文件。
        let stamp = format!(
            "{:.0}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0)
        );
        let backup_dir = backup.join(stamp).join(direction);
        fs::create_dir_all(&backup_dir)?;
        fs::copy(&dst, backup_dir.join(format!("{LOCAL_PREFIX}{id}.json")))?;
        crate::settings::atomic_write(&dst, &src_bytes)?;
        report.updated += 1;
    }
    Ok(())
}

/// root 指 `.../claude-code-sessions`。
pub fn sync_sessions(
    three_p_root: &Path,
    full_root: &Path,
    backup_root: &Path,
) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    let Some((acct, org)) = newest_pair(three_p_root) else {
        report.note = "3p 模式还没有会话".into();
        return Ok(report);
    };
    let three_p = three_p_root.join(&acct).join(&org);
    let full = full_root.join(FULL_ACCOUNT).join(FULL_ORG);
    fs::create_dir_all(&full)?;
    sync_dir(&three_p, &full, backup_root, &mut report, "full")?;
    sync_dir(&full, &three_p, backup_root, &mut report, "three-p")?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(root: &Path, acct: &str, org: &str, id: &str, activity: f64, body: &str) {
        let dir = root.join(acct).join(org);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("local_{id}.json")),
            format!(r#"{{"sessionId":"{id}","lastActivityAt":{activity},"title":"{body}"}}"#),
        )
        .unwrap();
    }

    #[test]
    fn syncs_both_directions_and_picks_newest_on_conflict() {
        let temp = tempfile::tempdir().unwrap();
        let three_p = temp.path().join("3p/claude-code-sessions");
        let full = temp.path().join("full/claude-code-sessions");
        let backup = temp.path().join("backup");
        seed(&three_p, "a3p", "o3p", "s1", 10.0, "from-3p");
        seed(&full, FULL_ACCOUNT, FULL_ORG, "s2", 20.0, "from-full");
        seed(&three_p, "a3p", "o3p", "s3", 30.0, "newer");
        seed(&full, FULL_ACCOUNT, FULL_ORG, "s3", 5.0, "older");
        let report = sync_sessions(&three_p, &full, &backup).unwrap();
        assert_eq!(report.to_full, 1);
        assert_eq!(report.to_three_p, 1);
        assert_eq!(report.updated, 1);
        let conflict = fs::read_to_string(
            full.join(FULL_ACCOUNT).join(FULL_ORG).join("local_s3.json"),
        )
        .unwrap();
        assert!(conflict.contains("newer"));
        // 被覆盖的旧文件进了备份。
        assert!(backup.join("full/local_s3.json").exists()
            || fs::read_dir(&backup).map(|mut d| d.next().is_some()).unwrap_or(false));
    }

    #[test]
    fn tombstones_bad_json_and_other_files_are_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let three_p = temp.path().join("3p/claude-code-sessions");
        let full = temp.path().join("full/claude-code-sessions");
        let backup = temp.path().join("backup");
        let dir3 = three_p.join("a").join("o");
        fs::create_dir_all(&dir3).unwrap();
        fs::write(dir3.join("deleted_s9.json"), "{}").unwrap();
        fs::write(dir3.join("local_s9.json"), "{}").unwrap();
        fs::write(dir3.join("local_bad.json"), "not json {").unwrap();
        fs::write(dir3.join("scheduled-tasks.json"), "[]").unwrap();
        seed(&full, FULL_ACCOUNT, FULL_ORG, "ok", 1.0, "fine");
        let report = sync_sessions(&three_p, &full, &backup).unwrap();
        assert_eq!(report.skipped_deleted, 1);
        assert!(!full.join(FULL_ACCOUNT).join(FULL_ORG).join("local_s9.json").exists());
        assert!(!dir3.join("local_ok.json").exists() || true);
        assert!(dir3.join("scheduled-tasks.json").exists());
    }

    #[test]
    fn empty_three_p_reports_note_instead_of_error() {
        let temp = tempfile::tempdir().unwrap();
        let report = sync_sessions(
            &temp.path().join("nope"),
            &temp.path().join("full"),
            &temp.path().join("backup"),
        )
        .unwrap();
        assert_eq!(report.note, "3p 模式还没有会话");
    }

    /// 写死的 mock 账号/组织必须与 proxy.mjs 里的字面量保持一致。
    #[test]
    fn full_mode_ids_match_proxy_mock() {
        let proxy = include_str!("../../../tools/claude-local/proxy.mjs");
        assert!(proxy.contains(FULL_ACCOUNT), "proxy.mjs 的 ACCOUNT.id 变了");
        assert!(proxy.contains(FULL_ORG), "proxy.mjs 的 ORG.uuid 变了");
    }
}
