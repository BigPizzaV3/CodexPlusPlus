use anyhow::Context;
use serde::Serialize;

use super::{metadata_is_link, SkillsManager};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBackupUsage {
    pub backup_count: usize,
    pub measured_count: usize,
    pub bytes: u64,
    pub errors: Vec<String>,
}

impl SkillsManager {
    /// 仅在用户请求时统计备份文件的逻辑大小，不跟随目录链接。
    pub fn backup_usage(&self) -> anyhow::Result<SkillBackupUsage> {
        let backups = self.try_list_backups()?;
        let mut usage = SkillBackupUsage {
            backup_count: backups.len(),
            measured_count: 0,
            bytes: 0,
            errors: Vec::new(),
        };
        for backup in backups {
            let result = (|| -> anyhow::Result<u64> {
                let (root, _) = self.resolve_backup(&backup.id)?;
                let metadata = std::fs::symlink_metadata(&root)
                    .with_context(|| format!("读取备份失败：{}", root.display()))?;
                anyhow::ensure!(metadata.is_dir() && !metadata_is_link(&metadata),
                    "备份不是实体目录：{}", root.display());
                let mut pending = vec![root];
                let mut bytes = 0;
                while let Some(dir) = pending.pop() {
                    for entry in std::fs::read_dir(&dir)
                        .with_context(|| format!("读取目录失败：{}", dir.display()))?
                    {
                        let entry = entry?;
                        let path = entry.path();
                        let metadata = std::fs::symlink_metadata(&path)
                            .with_context(|| format!("读取文件信息失败：{}", path.display()))?;
                        anyhow::ensure!(!metadata_is_link(&metadata),
                            "备份含有链接，未统计其目标：{}", path.display());
                        if metadata.is_dir() {
                            pending.push(path);
                        } else if metadata.is_file() {
                            bytes += metadata.len();
                        }
                    }
                }
                Ok(bytes)
            })();
            match result {
                Ok(bytes) => {
                    usage.measured_count += 1;
                    usage.bytes += bytes;
                }
                Err(error) => usage.errors.push(format!("{}：{error:#}", backup.backup_path)),
            }
        }
        Ok(usage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{skill_path_identity, InstalledSkill, SkillBackupOrigin, SkillPublishedSnapshot, SkillUpdateBackupGroup};

    #[test]
    fn usage_counts_nested_files_and_reports_missing_registered_backup() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let manager = SkillsManager::new(root.join("skills"), root.join("backups"), root.join("state.json"), root.join("codex"));
        let first = root.join("backups/alpha-123");
        std::fs::create_dir_all(first.join("nested")).unwrap();
        std::fs::write(first.join("SKILL.md"), "alpha").unwrap();
        std::fs::write(first.join("nested/data"), "1234567").unwrap();
        let second = root.join("backups/beta-456");
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("SKILL.md"), "beta").unwrap();

        let original = root.join("external/gamma");
        std::fs::create_dir_all(&original).unwrap();
        let missing_id = format!("@external/{}/gamma-789", uuid::Uuid::new_v4());
        let mut state = manager.load_state();
        state.backup_origins.insert(missing_id.clone(), SkillBackupOrigin {
            path: original.clone(), purpose: "update".into(),
        });
        state.update_backup_groups.insert("group".into(), SkillUpdateBackupGroup {
            installed: InstalledSkill { id: "gamma".into(), ..Default::default() },
            backup_ids: vec![missing_id],
            published: vec![SkillPublishedSnapshot {
                path: original.clone(), identity: skill_path_identity(&original).unwrap(), digest: [0; 32],
            }],
            instance_path: None,
            partially_deleted: false,
        });
        manager.save_state_unlocked(&state).unwrap();

        let usage = manager.backup_usage().unwrap();
        assert_eq!(usage.backup_count, 3);
        assert_eq!(usage.measured_count, 2);
        assert_eq!(usage.bytes, 16);
        assert_eq!(usage.errors.len(), 1);
        assert!(usage.errors[0].contains("gamma-789"));
    }

    #[cfg(unix)]
    #[test]
    fn usage_marks_a_backup_with_links_incomplete() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let manager = SkillsManager::new(root.join("skills"), root.join("backups"), root.join("state.json"), root.join("codex"));
        let backup = root.join("backups/alpha-123");
        std::fs::create_dir_all(&backup).unwrap();
        std::fs::write(backup.join("SKILL.md"), "alpha").unwrap();
        std::os::unix::fs::symlink(root, backup.join("link")).unwrap();

        let usage = manager.backup_usage().unwrap();
        assert_eq!(usage.backup_count, 1);
        assert_eq!(usage.measured_count, 0);
        assert_eq!(usage.bytes, 0);
        assert!(usage.errors[0].contains("未统计其目标"));
    }
}
