//! Claude Code 会话在 3p 模式与完整功能模式之间的同步：
//! 两种部署模式用不同的 Application Support 目录，会话元数据各存一份
//! `claude-code-sessions/<account>/<org>/local_<uuid>.json`；对话记录（transcript）
//! 在各自的 CLAUDE_CONFIG_DIR 下 `projects/<escaped cwd>/<cliSessionId>.jsonl`，
//! 早期 3p 会话的记录还在 `~/.claude`。`local-agent-mode-sessions`（Cowork）不同步。
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
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
    /// 元数据有但两边都找不到的 transcript 条数。
    pub missing_transcripts: usize,
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

/// Claude Code 的 projects 目录转义：cwd 里每个非 ASCII 字母数字字符换成 `-`。
fn escape_cwd(cwd: &str) -> String {
    cwd.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

fn backup_stamp() -> String {
    format!(
        "{:.0}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0)
    )
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)?.flatten() {
        let to = dst.join(entry.file_name());
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            copy_dir(&entry.path(), &to)?;
        } else {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// 元数据里这条会话引用的 transcript id：cliSessionId + priorCliSessionIds。
fn transcript_ids(meta: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(id) = meta.get("cliSessionId").and_then(Value::as_str) {
        ids.push(id.to_string());
    }
    if let Some(list) = meta.get("priorCliSessionIds").and_then(Value::as_array) {
        for id in list.iter().filter_map(Value::as_str) {
            if !ids.iter().any(|known| known == id) {
                ids.push(id.to_string());
            }
        }
    }
    ids
}

fn find_transcript(configs: &[PathBuf], escaped: &str, id: &str) -> Option<PathBuf> {
    configs
        .iter()
        .map(|config| config.join("projects").join(escaped).join(format!("{id}.jsonl")))
        .find(|path| path.is_file())
}

/// 把某条会话的 transcript 从源侧候选目录补到目标侧第一个 config 目录。
/// 冲突保留字节数更大的一份（jsonl 只追加），被覆盖的先进备份。
fn sync_transcripts(
    meta: &Value,
    source_configs: &[PathBuf],
    target_configs: &[PathBuf],
    backup: &Path,
    report: &mut SyncReport,
) -> Result<()> {
    let Some(target) = target_configs.first() else {
        return Ok(());
    };
    let cwd = meta.get("cwd").and_then(Value::as_str).unwrap_or_default();
    if cwd.is_empty() {
        return Ok(());
    }
    let escaped = escape_cwd(cwd);
    for id in transcript_ids(meta) {
        let dst = target.join("projects").join(&escaped).join(format!("{id}.jsonl"));
        let src = find_transcript(source_configs, &escaped, &id)
            .or_else(|| find_transcript(target_configs, &escaped, &id));
        match (src, dst.is_file()) {
            (Some(src), false) => {
                fs::create_dir_all(dst.parent().unwrap())?;
                fs::copy(&src, &dst)?;
            }
            (Some(src), true) => {
                let src_bytes = fs::read(&src)?;
                let dst_bytes = fs::read(&dst)?;
                if src_bytes != dst_bytes && src_bytes.len() > dst_bytes.len() {
                    let backup_dir = backup.join(backup_stamp()).join("transcripts");
                    fs::create_dir_all(&backup_dir)?;
                    fs::copy(&dst, backup_dir.join(format!("{id}.jsonl")))?;
                    crate::settings::atomic_write(&dst, &src_bytes)?;
                }
            }
            (None, exists) if !exists => {
                report.missing_transcripts += 1;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "claude_session_sync.missing_transcript",
                    serde_json::json!({"cliSessionId": id, "cwd": cwd}),
                );
            }
            _ => {}
        }
        // 同名子目录（子代理等输出）整体跟随。
        for config in source_configs {
            let dir = config.join("projects").join(&escaped).join(&id);
            if dir.is_dir() {
                let out = target.join("projects").join(&escaped).join(&id);
                if !out.exists() {
                    copy_dir(&dir, &out)?;
                }
            }
        }
    }
    Ok(())
}

/// 单向合并 mine → theirs：复制缺席会话、按新鲜度覆盖、补 transcript。
fn merge_dir(
    mine: &Path,
    mine_configs: &[PathBuf],
    theirs: &Path,
    their_configs: &[PathBuf],
    tombstones: &HashSet<String>,
    backup: &Path,
    report: &mut SyncReport,
    direction: &str,
) -> Result<()> {
    let (mine_locals, _) = list_sessions(mine);
    let (their_locals, _) = list_sessions(theirs);
    for id in &mine_locals {
        if tombstones.contains(id) {
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
        } else {
            let src_bytes = fs::read(&src)?;
            let dst_bytes = fs::read(&dst)?;
            if src_bytes != dst_bytes {
                if serde_json::from_slice::<Value>(&src_bytes).is_err()
                    || serde_json::from_slice::<Value>(&dst_bytes).is_err()
                {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "claude_session_sync.bad_json",
                        serde_json::json!({"file": src.file_name().unwrap_or_default().to_string_lossy()}),
                    );
                    continue;
                }
                if freshness(&src) > freshness(&dst) {
                    let backup_dir = backup.join(backup_stamp()).join(direction);
                    fs::create_dir_all(&backup_dir)?;
                    fs::copy(&dst, backup_dir.join(format!("{LOCAL_PREFIX}{id}.json")))?;
                    crate::settings::atomic_write(&dst, &src_bytes)?;
                    report.updated += 1;
                }
            }
        }
        // 元数据到位后补 transcript（含之前同步过、当时没拷上的）。
        if let Ok(meta) = serde_json::from_slice::<Value>(&fs::read(&dst).unwrap_or_default()) {
            sync_transcripts(&meta, mine_configs, their_configs, backup, report)?;
        }
    }
    Ok(())
}

/// sessions root 指 `.../claude-code-sessions`；config 候选按优先级给出，
/// 写入目标是每个候选列表的第一个。
pub fn sync_sessions(
    three_p_root: &Path,
    three_p_configs: &[PathBuf],
    full_root: &Path,
    full_configs: &[PathBuf],
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
    // 任一侧的墓碑都阻止复制；同一会话只计一次。
    let (_, tp_deleted) = list_sessions(&three_p);
    let (_, f_deleted) = list_sessions(&full);
    let tombstones: HashSet<String> = tp_deleted.into_iter().chain(f_deleted).collect();
    let (tp_locals, _) = list_sessions(&three_p);
    let (f_locals, _) = list_sessions(&full);
    report.skipped_deleted = tp_locals
        .iter()
        .chain(f_locals.iter())
        .filter(|id| tombstones.contains(*id))
        .collect::<HashSet<_>>()
        .len();
    merge_dir(
        &three_p,
        three_p_configs,
        &full,
        full_configs,
        &tombstones,
        backup_root,
        &mut report,
        "full",
    )?;
    merge_dir(
        &full,
        full_configs,
        &three_p,
        three_p_configs,
        &tombstones,
        backup_root,
        &mut report,
        "three-p",
    )?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(
        root: &Path,
        acct: &str,
        org: &str,
        id: &str,
        activity: f64,
        extra: &str,
    ) {
        let dir = root.join(acct).join(org);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("local_{id}.json")),
            format!(
                r#"{{"sessionId":"{id}","cliSessionId":"cli-{id}","cwd":"/Users/mac/Desktop/reclaude2api","lastActivityAt":{activity}{extra}}}"#
            ),
        )
        .unwrap();
    }

    fn transcript(config: &Path, id: &str, lines: usize) -> PathBuf {
        let file = config
            .join("projects/-Users-mac-Desktop-reclaude2api")
            .join(format!("cli-{id}.jsonl"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "x\n".repeat(lines)).unwrap();
        file
    }

    fn transcript_exists(config: &Path, id: &str) -> bool {
        config
            .join("projects/-Users-mac-Desktop-reclaude2api")
            .join(format!("cli-{id}.jsonl"))
            .is_file()
    }

    fn copy_dir_prep(config: &Path, id: &str) {
        let dir = config
            .join("projects/-Users-mac-Desktop-reclaude2api")
            .join(format!("cli-{id}/sub"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("out.txt"), "o").unwrap();
    }

    #[test]
    fn escape_matches_claude_code_project_dirs() {
        assert_eq!(escape_cwd("/Users/mac/Desktop/reclaude2api"), "-Users-mac-Desktop-reclaude2api");
        assert_eq!(escape_cwd("/Users/mac/.tmp_cc"), "-Users-mac--tmp-cc");
    }

    #[test]
    fn sessions_and_transcripts_flow_both_ways() {
        let temp = tempfile::tempdir().unwrap();
        let t3 = temp.path().join("3p/sessions");
        let tf = temp.path().join("full/sessions");
        let c3 = vec![temp.path().join("3p/claude-code-3p"), temp.path().join("claude")];
        let cf = vec![temp.path().join("full/claude-code"), temp.path().join("claude")];
        let backup = temp.path().join("backup");
        seed(&t3, "a3p", "o3p", "s1", 10.0, "");
        // transcript 只在 ~/.claude 候选里也找得到。
        transcript(&c3[1], "s1", 3);
        seed(&tf, FULL_ACCOUNT, FULL_ORG, "s2", 20.0, "");
        transcript(&cf[0], "s2", 2);
        let report = sync_sessions(&t3, &c3, &tf, &cf, &backup).unwrap();
        assert_eq!((report.to_full, report.to_three_p), (1, 1));
        assert!(transcript_exists(&cf[0], "s1"));
        assert!(transcript_exists(&c3[0], "s2"));
        assert_eq!(report.missing_transcripts, 0);
    }

    #[test]
    fn prior_ids_subdirs_conflicts_and_backfill() {
        let temp = tempfile::tempdir().unwrap();
        let t3 = temp.path().join("3p/sessions");
        let tf = temp.path().join("full/sessions");
        let c3 = vec![temp.path().join("3p/claude-code-3p")];
        let cf = vec![temp.path().join("full/claude-code")];
        let backup = temp.path().join("backup");
        // priorCliSessionIds 的 transcript 也复制。
        seed(&t3, "a3p", "o3p", "s1", 30.0, r#","priorCliSessionIds":["cli-p1"]"#);
        transcript(&c3[0], "s1", 5);
        transcript(&c3[0], "p1", 1);
        // 子目录整体复制。
        copy_dir_prep(&c3[0], "s1");
        // 冲突保留大的一份 + 备份。
        transcript(&cf[0], "s1", 2);
        let dst = cf[0].join("projects/-Users-mac-Desktop-reclaude2api/cli-s1.jsonl");
        // 元数据已同步过去但 transcript 缺的会话补拷。
        seed(&t3, "a3p", "o3p", "s9", 1.0, "");
        seed(&tf, FULL_ACCOUNT, FULL_ORG, "s9", 1.0, "");
        transcript(&c3[0], "s9", 1);
        let report = sync_sessions(&t3, &c3, &tf, &cf, &backup).unwrap();
        assert!(transcript_exists(&cf[0], "p1"));
        assert!(cf[0].join("projects/-Users-mac-Desktop-reclaude2api/cli-s1/sub").is_dir());
        assert!(transcript_exists(&cf[0], "s9"), "已有元数据缺 transcript 的也要补");
        assert!(fs::read_to_string(&dst).unwrap().lines().count() == 5, "大的那份覆盖小的");
        let stamped: Vec<_> = fs::read_dir(&backup).unwrap().flatten().collect();
        assert!(stamped.iter().any(|e| e.path().join("transcripts/cli-s1.jsonl").is_file()), "被覆盖的进备份");
        // 缺源计 missing_transcripts。
        seed(&t3, "a3p", "o3p", "gone", 1.0, "");
        let report = sync_sessions(&t3, &c3, &tf, &cf, &backup).unwrap();
        assert!(report.missing_transcripts >= 1);
    }

    #[test]
    fn tombstones_dedupe_and_other_files_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let t3 = temp.path().join("3p/sessions");
        let tf = temp.path().join("full/sessions");
        let c3 = vec![temp.path().join("3p/claude-code-3p")];
        let cf = vec![temp.path().join("full/claude-code")];
        let dir3 = t3.join("a").join("o");
        fs::create_dir_all(&dir3).unwrap();
        fs::write(dir3.join("deleted_s9.json"), "{}").unwrap();
        fs::write(dir3.join("local_s9.json"), "{}").unwrap();
        fs::write(dir3.join("scheduled-tasks.json"), "[]").unwrap();
        let report = sync_sessions(&t3, &c3, &tf, &cf, &temp.path().join("backup")).unwrap();
        assert_eq!(report.skipped_deleted, 1, "墓碑按 id 去重");
        assert!(!tf.join(FULL_ACCOUNT).join(FULL_ORG).join("local_s9.json").exists());
        assert!(dir3.join("scheduled-tasks.json").exists());
    }

    #[test]
    fn empty_three_p_reports_note_instead_of_error() {
        let temp = tempfile::tempdir().unwrap();
        let report = sync_sessions(
            &temp.path().join("nope"),
            &[],
            &temp.path().join("full"),
            &[],
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
