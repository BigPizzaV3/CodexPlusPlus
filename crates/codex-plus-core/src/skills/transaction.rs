//! Skill 目录及状态的可恢复事务。日志只保存受控位置和相对标识，不执行任意路径。
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum SkillPlace {
    Library,
    Codex,
    Agent(String),
}

pub(super) struct PreparedSkillChange {
    pub place: SkillPlace,
    pub staged: bool,
    pub preserve_backup: bool,
    pub expected_backup_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalChange {
    place: SkillPlace,
    staged: bool,
    had_original: bool,
    preserve_backup: bool,
    expected_backup_hash: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SkillJournal {
    version: u8,
    operation: String,
    id: String,
    committed: bool,
    before_state: Option<Vec<u8>>,
    after_state: Option<Vec<u8>>,
    changes: Vec<JournalChange>,
}

impl SkillsManager {
    fn transaction_dir(&self) -> anyhow::Result<PathBuf> {
        Ok(self
            .state_path
            .parent()
            .context("Skill 状态文件缺少父目录")?
            .join(".skill-transactions"))
    }

    fn place_root(&self, place: &SkillPlace) -> anyhow::Result<PathBuf> {
        match place {
            SkillPlace::Library => Ok(self.source_dir.clone()),
            SkillPlace::Codex => Ok(self.linked_dir()),
            SkillPlace::Agent(agent) => self.agent_dir(agent),
        }
    }

    pub(super) fn transaction_stage(
        &self,
        operation: &str,
        index: usize,
        place: &SkillPlace,
    ) -> anyhow::Result<PathBuf> {
        uuid::Uuid::parse_str(operation).context("Skill 事务标识无效")?;
        Ok(self
            .place_root(place)?
            .join(format!(".codex-plus-{operation}-{index}-stage")))
    }

    fn transaction_backup(&self, journal: &SkillJournal, index: usize) -> anyhow::Result<PathBuf> {
        uuid::Uuid::parse_str(&journal.operation).context("Skill 事务标识无效")?;
        Ok(self
            .place_root(&journal.changes[index].place)?
            .join(format!(".codex-plus-{}-{index}-backup", journal.operation)))
    }

    pub(super) fn stage_skill_copy(
        &self,
        operation: &str,
        index: usize,
        place: &SkillPlace,
        source: &Path,
        marker: bool,
    ) -> anyhow::Result<()> {
        let stage = self.transaction_stage(operation, index, place)?;
        anyhow::ensure!(
            std::fs::symlink_metadata(&stage).is_err(),
            "Skill 暂存位置已存在"
        );
        let before = skill_directory_hash(source)?;
        if let Err(error) = copy_dir_all(source, &stage) {
            let _ = remove_link(&stage);
            return Err(error);
        }
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                skill_directory_hash(source)? == before && skill_directory_hash(&stage)? == before,
                "Skill 在复制时发生变化，请重试"
            );
            if marker {
                std::fs::write(stage.join(MANAGED_COPY_MARKER), "")?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = remove_link(&stage);
        }
        result
    }

    /// 调用方须持有 operation_lock；日志写入后才移动任何原始目录。
    pub(super) fn commit_skill_transaction(
        &self,
        operation: &str,
        id: &str,
        changes: Vec<PreparedSkillChange>,
        after_state: Option<&SkillsState>,
    ) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        uuid::Uuid::parse_str(operation)?;
        let before_state = match std::fs::read(&self.state_path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut journal = SkillJournal {
            version: 1,
            operation: operation.into(),
            id: id.into(),
            committed: false,
            before_state,
            after_state: after_state.map(serde_json::to_vec_pretty).transpose()?,
            changes: Vec::new(),
        };
        for (index, change) in changes.into_iter().enumerate() {
            let root = self.place_root(&change.place)?;
            std::fs::create_dir_all(&root)?;
            let target = root.join(id);
            let had_original = match std::fs::symlink_metadata(&target) {
                Ok(_) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            anyhow::ensure!(
                had_original == change.expected_backup_hash.is_some(),
                "Skill 目标在准备操作期间发生变化，未覆盖：{}",
                target.display()
            );
            if change.staged {
                anyhow::ensure!(
                    self.transaction_stage(operation, index, &change.place)?
                        .is_dir(),
                    "Skill 暂存目录不存在"
                );
            }
            journal.changes.push(JournalChange {
                place: change.place,
                staged: change.staged,
                had_original,
                preserve_backup: change.preserve_backup,
                expected_backup_hash: change.expected_backup_hash,
            });
        }
        if journal
            .changes
            .iter()
            .any(|change| change.had_original && change.preserve_backup)
        {
            std::fs::create_dir_all(&self.backups_dir)
                .context("无法创建 Skill 备份目录，未修改原内容")?;
        }
        let journal_dir = self.transaction_dir()?;
        std::fs::create_dir_all(&journal_dir)?;
        let path = journal_dir.join(format!("{operation}.json"));
        anyhow::ensure!(!path.exists(), "Skill 事务日志已存在");
        crate::settings::atomic_write(&path, &serde_json::to_vec_pretty(&journal)?)?;
        let result = (|| -> anyhow::Result<()> {
            for (index, change) in journal.changes.iter().enumerate() {
                let target = self.place_root(&change.place)?.join(id);
                let backup = self.transaction_backup(&journal, index)?;
                anyhow::ensure!(
                    std::fs::symlink_metadata(&backup).is_err(),
                    "Skill 临时备份已存在"
                );
                if change.had_original {
                    std::fs::rename(&target, &backup)?;
                    if let Some(expected) = &change.expected_backup_hash {
                        anyhow::ensure!(
                            &skill_directory_hash(&backup)? == expected,
                            "Skill 在操作时发生变化，请重新确认"
                        );
                    }
                } else {
                    anyhow::ensure!(
                        std::fs::symlink_metadata(&target).is_err(),
                        "Skill 目标位置在操作期间被占用"
                    );
                }
                if change.staged {
                    std::fs::rename(
                        self.transaction_stage(operation, index, &change.place)?,
                        &target,
                    )?;
                }
            }
            if let Some(after) = &journal.after_state {
                anyhow::ensure!(
                    read_optional_state(&self.state_path)? == journal.before_state,
                    "Skill 状态在操作期间发生变化"
                );
                crate::settings::atomic_write(&self.state_path, after)?;
            }
            journal.committed = true;
            crate::settings::atomic_write(&path, &serde_json::to_vec_pretty(&journal)?)?;
            Ok(())
        })();
        if let Err(error) = result {
            // 读取磁盘上的日志：最后一次原子写入可能已经成功。
            if let Err(recovery) = self.recover_skill_journal(&path) {
                return Err(error.context(format!(
                    "Skill 事务恢复失败，日志和副本已保留：{recovery:#}"
                )));
            }
            return Err(error);
        }
        self.recover_skill_journal(&path)
    }

    pub(super) fn recover_skill_transactions(&self) -> anyhow::Result<()> {
        let dir = self.transaction_dir()?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                self.recover_skill_journal(&path)?;
            }
        }
        Ok(())
    }

    fn recover_skill_journal(&self, path: &Path) -> anyhow::Result<()> {
        let journal: SkillJournal = serde_json::from_slice(&std::fs::read(path)?)?;
        anyhow::ensure!(
            journal.version == 1
                && path.file_stem().and_then(|stem| stem.to_str())
                    == Some(journal.operation.as_str()),
            "Skill 事务日志身份无效"
        );
        validate_skill_id(&journal.id)?;
        uuid::Uuid::parse_str(&journal.operation)?;
        if !journal.committed && journal.after_state.is_some() {
            let current = read_optional_state(&self.state_path)?;
            anyhow::ensure!(
                current == journal.before_state || current == journal.after_state,
                "Skill 状态已被外部修改，无法自动恢复事务"
            );
        }
        for index in (0..journal.changes.len()).rev() {
            let change = &journal.changes[index];
            let target = self.place_root(&change.place)?.join(&journal.id);
            let stage = self.transaction_stage(&journal.operation, index, &change.place)?;
            let backup = self.transaction_backup(&journal, index)?;
            let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
            if journal.committed {
                if exists(&backup) {
                    if change.preserve_backup {
                        self.archive_transaction_backup(&journal.id, &backup)?;
                    } else {
                        remove_link(&backup)?;
                    }
                }
            } else if exists(&backup) {
                if exists(&target) {
                    anyhow::ensure!(
                        change.staged && !exists(&stage),
                        "Skill 目标在恢复期间被占用，已保留所有副本"
                    );
                    self.archive_transaction_backup(&journal.id, &target)?;
                }
                std::fs::rename(&backup, &target)?;
            } else if !change.had_original && change.staged && !exists(&stage) && exists(&target) {
                self.archive_transaction_backup(&journal.id, &target)?;
            }
            if exists(&stage) {
                remove_link(&stage)?;
            }
        }
        if !journal.committed
            && journal.after_state.is_some()
            && read_optional_state(&self.state_path)? != journal.before_state
        {
            match &journal.before_state {
                Some(bytes) => crate::settings::atomic_write(&self.state_path, bytes)?,
                None => match std::fs::remove_file(&self.state_path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                },
            }
        }
        std::fs::remove_file(path)?;
        Ok(())
    }

    fn archive_transaction_backup(&self, id: &str, backup: &Path) -> anyhow::Result<()> {
        // 软链移动到备份目录可能改变相对目标；先将内容物化成独立备份。
        let symlink = std::fs::symlink_metadata(backup)?.file_type().is_symlink();
        if !symlink && self.backup_directory(id, backup).is_ok() {
            return Ok(());
        }
        // 不同磁盘之间无法原子重命名；先完整复制，再移除事务暂存副本。
        std::fs::create_dir_all(&self.backups_dir)?;
        let operation = uuid::Uuid::new_v4();
        let staging = self.backups_dir.join(format!(".backup-{operation}"));
        copy_dir_all(backup, &staging)?;
        let mut timestamp = current_unix_timestamp_string().parse::<u128>()?;
        let destination = loop {
            let candidate = self.backups_dir.join(format!("{id}-{timestamp}"));
            if std::fs::symlink_metadata(&candidate).is_err() {
                break candidate;
            }
            timestamp += 1;
        };
        std::fs::rename(&staging, destination)?;
        remove_link(backup)
    }
}

fn read_optional_state(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interrupted_transaction(
        publish: bool,
        save_state: bool,
        committed: bool,
        via_repo_write: bool,
        via_launch_gate: bool,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("library");
        let state_path = temp.path().join("skills.json");
        let manager = SkillsManager::new(
            &source,
            temp.path().join("backups"),
            &state_path,
            temp.path().join("home"),
        );
        let target = source.join("alpha");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("SKILL.md"), "old").unwrap();
        let before = serde_json::to_vec(&SkillsState::default()).unwrap();
        std::fs::write(&state_path, &before).unwrap();
        let mut next = SkillsState::default();
        next.repos.clear();
        let after = serde_json::to_vec(&next).unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        let stage = manager
            .transaction_stage(&operation, 0, &SkillPlace::Library)
            .unwrap();
        std::fs::create_dir_all(&stage).unwrap();
        std::fs::write(stage.join("SKILL.md"), "new").unwrap();
        let journal = SkillJournal {
            version: 1,
            operation: operation.clone(),
            id: "alpha".into(),
            committed,
            before_state: Some(before.clone()),
            after_state: Some(after.clone()),
            changes: vec![JournalChange {
                place: SkillPlace::Library,
                staged: true,
                had_original: true,
                preserve_backup: true,
                expected_backup_hash: None,
            }],
        };
        let journal_dir = manager.transaction_dir().unwrap();
        std::fs::create_dir_all(&journal_dir).unwrap();
        let path = journal_dir.join(format!("{operation}.json"));
        std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
        std::fs::rename(&target, manager.transaction_backup(&journal, 0).unwrap()).unwrap();
        if publish {
            std::fs::rename(&stage, &target).unwrap();
        }
        if save_state {
            std::fs::write(&state_path, &after).unwrap();
        }

        // 模拟进程重启，使用新实例读取磁盘上的日志恢复。
        let reopened = SkillsManager::new(
            &source,
            temp.path().join("backups"),
            &state_path,
            temp.path().join("home"),
        );
        let added_repo = super::super::parse_repo_key("example/recovered@main").unwrap();
        if via_repo_write {
            reopened.save_repo(added_repo.clone(), None).unwrap();
        } else if via_launch_gate {
            let _guard = reopened.lock_recovered_transactions().unwrap();
        } else {
            reopened.recover_skill_transactions().unwrap();
        }
        reopened.recover_skill_transactions().unwrap();
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
            if committed { "new" } else { "old" }
        );
        let mut expected: SkillsState =
            serde_json::from_slice(if committed { &after } else { &before }).unwrap();
        if via_repo_write || via_launch_gate {
            if via_repo_write {
                expected.repos.push(added_repo);
            }
            let actual = reopened.load_state();
            assert_eq!(actual.repos, expected.repos);
            assert_eq!(
                actual.suppressed_imported_repos,
                expected.suppressed_imported_repos
            );
            assert_eq!(
                actual.independent_library_migrated,
                expected.independent_library_migrated
            );
            assert_eq!(actual.installed.len(), 1);
            assert_eq!(
                actual.installed["alpha"].local_baseline_hash,
                super::super::skill_directory_hash(&target).unwrap()
            );
        } else {
            assert_eq!(reopened.load_state(), expected);
        }
        if publish {
            let retained = reopened.list_backups().into_iter().any(|backup| {
                std::fs::read_to_string(reopened.backups_dir.join(backup.id).join("SKILL.md"))
                    .is_ok_and(|content| content == if committed { "old" } else { "new" })
            });
            assert!(retained, "恢复时必须保留被替换的版本");
        }
    }

    #[test]
    fn interruption_before_publish_restores_original() {
        interrupted_transaction(false, false, false, false, false);
    }
    #[test]
    fn interruption_after_publish_rolls_back_and_preserves_both() {
        interrupted_transaction(true, false, false, false, false);
    }
    #[test]
    fn interruption_after_state_write_rolls_back_state_and_files() {
        interrupted_transaction(true, true, false, false, false);
    }
    #[test]
    fn interruption_after_commit_finishes_backup() {
        interrupted_transaction(true, true, true, false, false);
    }

    #[test]
    fn repository_write_recovers_interrupted_skill_transaction_before_saving() {
        interrupted_transaction(true, true, false, true, false);
    }

    #[test]
    fn launch_gate_recovers_interrupted_transaction_before_action() {
        interrupted_transaction(true, true, false, false, true);
    }

    #[test]
    fn launch_gate_retains_invalid_journal_and_rejects_action() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("skills.json"),
            temp.path().join("home"),
        );
        let journal_dir = manager.transaction_dir().unwrap();
        std::fs::create_dir_all(&journal_dir).unwrap();
        let journal = journal_dir.join("invalid.json");
        std::fs::write(&journal, b"invalid journal").unwrap();
        let mut stopped_or_started = false;
        if let Ok(_guard) = manager.lock_recovered_transactions() {
            stopped_or_started = true;
        }
        assert!(!stopped_or_started);
        assert_eq!(std::fs::read(&journal).unwrap(), b"invalid journal");
    }

    #[test]
    fn launch_gate_blocks_other_skill_operations_until_released() {
        let temp = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("skills.json"),
            temp.path().join("home"),
        );
        let second = manager.clone();
        let guard = manager.lock_recovered_transactions().unwrap();
        assert!(second.operation_lock.try_lock().is_err());
        drop(guard);
        assert!(second.operation_lock.try_lock().is_ok());
    }

    #[test]
    fn launch_gate_rejects_corrupt_state_before_action() {
        let temp = tempfile::tempdir().unwrap();
        let state_path = temp.path().join("skills.json");
        std::fs::write(&state_path, b"invalid state").unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            &state_path,
            temp.path().join("home"),
        );
        let mut stopped_or_started = false;
        if let Ok(_guard) = manager.lock_recovered_transactions() {
            stopped_or_started = true;
        }
        assert!(!stopped_or_started);
        assert_eq!(std::fs::read(&state_path).unwrap(), b"invalid state");
        assert!(manager.operation_lock.try_lock().is_ok());
    }

    #[test]
    fn launch_gate_rejects_failed_legacy_migration_before_action() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy");
        std::fs::write(&legacy, b"not a directory").unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("skills.json"),
            temp.path().join("home"),
        )
        .with_legacy_source_dir(&legacy);
        let mut stopped_or_started = false;
        if let Ok(_guard) = manager.lock_recovered_transactions() {
            stopped_or_started = true;
        }
        assert!(!stopped_or_started);
        assert_eq!(std::fs::read(&legacy).unwrap(), b"not a directory");
        assert!(!manager.load_state().independent_library_migrated);
        assert!(manager.operation_lock.try_lock().is_ok());
    }

    #[test]
    fn process_exit_after_publish_recovers_on_restart() {
        const ENV: &str = "CODEX_PLUS_SKILL_CRASH_FIXTURE";
        if let Some(root) = std::env::var_os(ENV) {
            let root = PathBuf::from(root);
            let manager = SkillsManager::new(
                root.join("library"),
                root.join("backups"),
                root.join("skills.json"),
                root.join("home"),
            );
            let target = manager.source_dir.join("alpha");
            let operation = uuid::Uuid::new_v4().to_string();
            let stage = manager
                .transaction_stage(&operation, 0, &SkillPlace::Library)
                .unwrap();
            std::fs::create_dir_all(&stage).unwrap();
            std::fs::write(stage.join("SKILL.md"), "new").unwrap();
            let before = std::fs::read(&manager.state_path).unwrap();
            let after = serde_json::to_vec(&SkillsState {
                repos: Vec::new(),
                installed: BTreeMap::new(),
                independent_library_migrated: false,
                suppressed_imported_repos: Default::default(),
            })
            .unwrap();
            let journal = SkillJournal {
                version: 1,
                operation: operation.clone(),
                id: "alpha".into(),
                committed: false,
                before_state: Some(before),
                after_state: Some(after.clone()),
                changes: vec![JournalChange {
                    place: SkillPlace::Library,
                    staged: true,
                    had_original: true,
                    preserve_backup: true,
                    expected_backup_hash: None,
                }],
            };
            let dir = manager.transaction_dir().unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            crate::settings::atomic_write(
                &dir.join(format!("{operation}.json")),
                &serde_json::to_vec(&journal).unwrap(),
            )
            .unwrap();
            std::fs::rename(&target, manager.transaction_backup(&journal, 0).unwrap()).unwrap();
            std::fs::rename(&stage, &target).unwrap();
            crate::settings::atomic_write(&manager.state_path, &after).unwrap();
            std::process::exit(77);
        }
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library/alpha");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("SKILL.md"), "old").unwrap();
        let state = temp.path().join("skills.json");
        let before = serde_json::to_vec(&SkillsState::default()).unwrap();
        std::fs::write(&state, &before).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "skills::transaction::tests::process_exit_after_publish_recovers_on_restart",
                "--nocapture",
            ])
            .env(ENV, temp.path())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(77));
        let reopened = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            &state,
            temp.path().join("home"),
        );
        reopened.recover_skill_transactions().unwrap();
        assert_eq!(
            std::fs::read_to_string(library.join("SKILL.md")).unwrap(),
            "old"
        );
        assert_eq!(std::fs::read(&state).unwrap(), before);
        assert!(reopened.list_backups().iter().any(|backup| {
            std::fs::read_to_string(reopened.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|text| text == "new")
        }));
    }
}
