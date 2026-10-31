//! 将已有 Skill 与已配置仓库中的候选来源关联，不替换任何 Skill 文件。
use super::*;
use std::collections::BTreeSet;

/// 名称相似不等于内容一致；该标记确保后续更新仍须确认覆盖。
const UNVERIFIED_ORIGIN_BASELINE: &str = "unverified-repository-origin";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatch {
    pub local_id: String,
    pub local_name: String,
    pub local_hash: String,
    pub snapshot: String,
    pub repo_key: String,
    pub repo_path: String,
    pub remote_name: String,
    pub remote_hash: String,
    pub exact: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchPreview {
    pub matches: Vec<SkillRepositoryMatch>,
    /// 仅单项扫描提供，可供用户手动选择名称和内容均不匹配的仓库路径。
    pub manual_candidates: Vec<SkillRepositoryMatch>,
    pub examined: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchSelection {
    pub local_id: String,
    pub local_hash: String,
    pub snapshot: String,
    pub repo_key: String,
    pub repo_path: String,
    pub remote_hash: String,
    /// 手动选择非匹配候选时显式传入；旧调用默认只接受名称或内容匹配。
    #[serde(default)]
    pub manual: bool,
}

fn source_missing(installed: Option<&InstalledSkill>) -> bool {
    installed
        .is_none_or(|item| item.repo_path.is_empty() || parse_repo_key(&item.repo_key).is_none())
}

pub(super) fn repository_match_origin(repo_key: &str, repo_path: &str) -> (String, String) {
    let key = parse_repo_key(repo_key)
        .and_then(|repo| normalize_repo(repo).ok())
        .map(|repo| {
            format!(
                "{}/{}@{}",
                repo.owner.to_ascii_lowercase(),
                repo.name.to_ascii_lowercase(),
                repo.branch
            )
        })
        .unwrap_or_else(|| repo_key.to_string());
    (key, repo_path.to_string())
}

fn comparable_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn plausible_match(id: &str, name: &str, remote: &RemoteSkill, local_hash: &str) -> (bool, bool) {
    let exact = !remote.content_hash.is_empty() && remote.content_hash == local_hash;
    let remote_dir = if remote.repo_path == "." {
        remote
            .repo_key
            .split('/')
            .nth(1)
            .unwrap_or("")
            .split('@')
            .next()
            .unwrap_or("")
    } else {
        remote.repo_path.rsplit('/').next().unwrap_or("")
    };
    let local_names = [comparable_name(id), comparable_name(name)];
    let remote_names = [comparable_name(remote_dir), comparable_name(&remote.name)];
    let similar = local_names
        .iter()
        .any(|local| !local.is_empty() && remote_names.contains(local));
    (exact || similar, exact)
}

impl SkillsManager {
    /// cc-switch 指明仓库但没有子路径时，只有内容一致才足以自动加入仓库。
    pub fn save_verified_imported_repo(
        &self,
        repo: SkillRepo,
        remote: &[RemoteSkill],
    ) -> anyhow::Result<bool> {
        let repo = normalize_repo(repo)?;
        let key = repo.key();
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _state = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        if state.suppressed_imported_repos.contains(&key) {
            return Ok(false);
        }
        if let Some(existing) = state.repos.iter().find(|item| item.key() == key) {
            return Ok(existing.enabled);
        }
        let evidence = state
            .installed
            .values()
            .filter(|item| item.import_source == "cc-switch" && item.repo_key == key)
            .any(|item| {
                if !item.repo_path.is_empty() {
                    return remote.iter().any(|candidate| {
                        candidate.repo_key == key && candidate.repo_path == item.repo_path
                    });
                }
                let Some(source) = self.matching_source(&state, &item.id) else {
                    return false;
                };
                let Ok(hash) = skill_git_tree_hash(&source) else {
                    return false;
                };
                remote.iter().any(|candidate| {
                    candidate.repo_key == key
                        && !candidate.content_hash.is_empty()
                        && candidate.content_hash == hash
                })
            });
        if !evidence {
            return Ok(false);
        }
        state.repos.push(repo);
        self.save_state_unlocked(&state)?;
        Ok(true)
    }

    fn matching_source(&self, state: &SkillsState, id: &str) -> Option<PathBuf> {
        std::iter::once(self.source_dir.join(id))
            .chain(
                self.legacy_source_dir
                    .iter()
                    .filter(|_| !state.independent_library_migrated)
                    .map(|root| root.join(id)),
            )
            .chain(std::iter::once(self.linked_dir().join(id)))
            .chain(self.agent_dirs.iter().map(|(_, root)| root.join(id)))
            .find(|path| path.join(SKILL_MANIFEST_FILE).is_file())
    }

    fn repository_match_codex_conflict(&self, id: &str, source: &Path) -> anyhow::Result<bool> {
        validate_skill_id(id)?;
        let codex = self.linked_dir().join(id);
        if std::fs::symlink_metadata(&codex).is_err() {
            return Ok(false);
        }
        if !codex.join(SKILL_MANIFEST_FILE).is_file() {
            return Ok(true);
        }
        Ok(skill_directory_hash(source)? != skill_directory_hash(&codex)?)
    }

    fn repository_match_snapshot_unlocked(
        &self,
        state: &SkillsState,
        id: &str,
    ) -> anyhow::Result<String> {
        let snapshot = self.import_target_snapshot_unlocked(state, id)?;
        let Some(legacy) = self.legacy_source_dir.as_ref().filter(|_| {
            !state.independent_library_migrated
                && !self.source_dir.join(id).join(SKILL_MANIFEST_FILE).is_file()
        }) else {
            return Ok(snapshot);
        };
        let path = legacy.join(id);
        if !path.join(SKILL_MANIFEST_FILE).is_file() {
            return Ok(snapshot);
        }
        let mut hash = Sha256::new();
        hash.update(snapshot.as_bytes());
        hash.update(b"legacy");
        hash.update(
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                &b"link"[..]
            } else {
                &b"copy"[..]
            },
        );
        hash.update(skill_directory_hash(&path)?.as_bytes());
        Ok(format!("{:x}", hash.finalize()))
    }

    fn recover_repository_match_transactions(&self) -> anyhow::Result<()> {
        let _state = self.state_lock.lock().unwrap();
        self.recover_skill_transactions()
    }

    /// 批量扫描只处理缺少来源的 Skill；指定 ID 时允许重新匹配并提供手动候选。
    pub fn preview_repository_matches(
        &self,
        repo: &SkillRepo,
        remote: &[RemoteSkill],
        only_id: Option<&str>,
    ) -> anyhow::Result<SkillRepositoryMatchPreview> {
        if let Some(id) = only_id {
            validate_skill_id(id)?;
        }
        let _operation = self.operation_lock.lock().unwrap();
        self.recover_repository_match_transactions()?;
        let _state = self.state_lock.lock().unwrap();
        let state = self.load_state_checked_unlocked()?;
        anyhow::ensure!(
            state.repos.iter().any(|configured| configured == repo),
            "仓库配置在扫描期间发生变化"
        );
        let mut ids: BTreeSet<String> = state.installed.keys().cloned().collect();
        ids.extend(
            scan_skill_dirs(&self.source_dir)
                .into_iter()
                .map(|(id, _)| id),
        );
        ids.extend(
            scan_skill_dirs(&self.linked_dir())
                .into_iter()
                .map(|(id, _)| id),
        );
        if !state.independent_library_migrated {
            if let Some(legacy) = &self.legacy_source_dir {
                ids.extend(scan_skill_dirs(legacy).into_iter().map(|(id, _)| id));
            }
        }
        for (_, root) in &self.agent_dirs {
            ids.extend(scan_skill_dirs(root).into_iter().map(|(id, _)| id));
        }
        let mut preview = SkillRepositoryMatchPreview::default();
        for id in ids {
            if only_id.is_some_and(|filter| filter != id)
                || (only_id.is_none() && !source_missing(state.installed.get(&id)))
            {
                continue;
            }
            let Some(source) = self.matching_source(&state, &id) else {
                continue;
            };
            preview.examined += 1;
            let result = (|| -> anyhow::Result<()> {
                anyhow::ensure!(
                    !self.repository_match_codex_conflict(&id, &source)?,
                    "Codex 副本与管理库不一致，请先选择版本"
                );
                let local_hash = skill_git_tree_hash(&source)?;
                let snapshot = self.repository_match_snapshot_unlocked(&state, &id)?;
                let (name, _) = read_skill_manifest(&source.join(SKILL_MANIFEST_FILE), &id);
                for candidate in remote.iter().filter(|item| item.repo_key == repo.key()) {
                    let (matches, exact) = plausible_match(&id, &name, candidate, &local_hash);
                    if matches || only_id.is_some() {
                        let item = SkillRepositoryMatch {
                            local_id: id.clone(),
                            local_name: name.clone(),
                            local_hash: local_hash.clone(),
                            snapshot: snapshot.clone(),
                            repo_key: candidate.repo_key.clone(),
                            repo_path: candidate.repo_path.clone(),
                            remote_name: candidate.name.clone(),
                            remote_hash: candidate.content_hash.clone(),
                            exact,
                        };
                        if matches {
                            preview.matches.push(item);
                        } else {
                            preview.manual_candidates.push(item);
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                preview.warnings.push(format!("{id}：{error:#}"));
            }
        }
        preview.matches.sort_by(|a, b| {
            a.local_id
                .cmp(&b.local_id)
                .then(b.exact.cmp(&a.exact))
                .then(a.repo_path.cmp(&b.repo_path))
        });
        preview
            .manual_candidates
            .sort_by(|a, b| a.repo_path.cmp(&b.repo_path));
        Ok(preview)
    }

    fn verify_repository_match_target(
        &self,
        state: &SkillsState,
        selection: &SkillRepositoryMatchSelection,
    ) -> anyhow::Result<()> {
        let source = self
            .matching_source(state, &selection.local_id)
            .context("本地 Skill 已不存在")?;
        anyhow::ensure!(
            !self.repository_match_codex_conflict(&selection.local_id, &source)?,
            "Codex 副本与管理库不一致，请先选择版本"
        );
        anyhow::ensure!(
            state
                .repos
                .iter()
                .any(|repo| repo.key() == selection.repo_key),
            "仓库已删除"
        );
        anyhow::ensure!(
            self.repository_match_snapshot_unlocked(state, &selection.local_id)?
                == selection.snapshot,
            "Skill 或 Agent 副本在确认后发生变化，请重新匹配"
        );
        let origin = repository_match_origin(&selection.repo_key, &selection.repo_path);
        anyhow::ensure!(
            !state
                .installed
                .values()
                .any(|item| item.id != selection.local_id
                    && repository_match_origin(&item.repo_key, &item.repo_path) == origin),
            "仓库路径已关联其他 Skill"
        );
        Ok(())
    }

    /// `remote` 必须是确认时重新取得的清单；状态、所有 Agent 副本和仓库哈希均重新校验。
    pub fn apply_repository_matches(
        &self,
        selections: &[SkillRepositoryMatchSelection],
        remote: &[RemoteSkill],
    ) -> anyhow::Result<(usize, Vec<String>)> {
        let mut ids = BTreeSet::new();
        let mut origins = BTreeSet::new();
        for item in selections {
            validate_skill_id(&item.local_id)?;
            anyhow::ensure!(
                !item.manual || selections.len() == 1,
                "手动选择仓库路径只能单项确认"
            );
            anyhow::ensure!(
                ids.insert(item.local_id.to_lowercase()),
                "同一个 Skill 只能选择一个来源"
            );
            anyhow::ensure!(
                origins.insert(repository_match_origin(&item.repo_key, &item.repo_path)),
                "同一个仓库路径不能关联多个 Skill"
            );
        }
        let _operation = self.operation_lock.lock().unwrap();
        self.recover_repository_match_transactions()?;
        let mut applied = 0;
        let mut errors = Vec::new();
        for selection in selections {
            let result = (|| -> anyhow::Result<()> {
                let candidate = remote
                    .iter()
                    .find(|item| {
                        item.repo_key == selection.repo_key
                            && item.repo_path == selection.repo_path
                            && item.content_hash == selection.remote_hash
                    })
                    .context("仓库内容已变化或来源不可用，请重新匹配")?;
                anyhow::ensure!(!candidate.content_hash.is_empty(), "仓库内容无法校验");
                let source = {
                    let _state = self.state_lock.lock().unwrap();
                    let state = self.load_state_checked_unlocked()?;
                    self.matching_source(&state, &selection.local_id)
                        .context("本地 Skill 已不存在")?
                };
                let local_hash = skill_git_tree_hash(&source)?;
                let local_directory_hash = skill_directory_hash(&source)?;
                let (name, description) =
                    read_skill_manifest(&source.join(SKILL_MANIFEST_FILE), &selection.local_id);
                let (matches, exact) =
                    plausible_match(&selection.local_id, &name, candidate, &local_hash);
                anyhow::ensure!(
                    (matches || selection.manual) && local_hash == selection.local_hash,
                    "本地 Skill 已变化，请重新匹配"
                );
                {
                    let _state = self.state_lock.lock().unwrap();
                    let state = self.load_state_checked_unlocked()?;
                    self.verify_repository_match_target(&state, selection)?;
                }
                let library = self.source_dir.join(&selection.local_id);
                let operation = if library.join(SKILL_MANIFEST_FILE).is_file() {
                    None
                } else {
                    anyhow::ensure!(
                        std::fs::symlink_metadata(&library).is_err(),
                        "Skill 管理库位置已存在其他内容，未关联"
                    );
                    std::fs::create_dir_all(&self.source_dir)?;
                    anyhow::ensure!(
                        !self
                            .source_dir
                            .canonicalize()?
                            .starts_with(source.canonicalize()?),
                        "Skill 目录不能包含受控源目录"
                    );
                    let operation = uuid::Uuid::new_v4().to_string();
                    self.stage_skill_copy(&operation, 0, &SkillPlace::Library, &source, false)?;
                    Some(operation)
                };
                let result = (|| -> anyhow::Result<()> {
                    let _state = self.state_lock.lock().unwrap();
                    let mut state = self.load_state_checked_unlocked()?;
                    self.verify_repository_match_target(&state, selection)?;
                    let checked = match &operation {
                        Some(operation) => {
                            self.transaction_stage(operation, 0, &SkillPlace::Library)?
                        }
                        None => library.clone(),
                    };
                    anyhow::ensure!(
                        skill_directory_hash(&checked)? == local_directory_hash
                            && skill_git_tree_hash(&checked)? == local_hash,
                        "Skill 在匹配期间发生变化，请重新匹配"
                    );
                    let item = state
                        .installed
                        .entry(selection.local_id.clone())
                        .or_insert_with(|| InstalledSkill {
                            id: selection.local_id.clone(),
                            name: name.clone(),
                            description: description.clone(),
                            repo_key: String::new(),
                            repo_path: String::new(),
                            import_source: String::new(),
                            local_baseline_hash: String::new(),
                            content_hash: String::new(),
                            installed_at: current_unix_timestamp_string(),
                        });
                    item.name = name;
                    item.description = description;
                    item.repo_key = selection.repo_key.clone();
                    item.repo_path = selection.repo_path.clone();
                    item.content_hash = local_hash;
                    item.local_baseline_hash = if exact {
                        local_directory_hash
                    } else {
                        UNVERIFIED_ORIGIN_BASELINE.into()
                    };
                    if let Some(operation) = &operation {
                        self.commit_skill_transaction(
                            operation,
                            &selection.local_id,
                            vec![PreparedSkillChange {
                                place: SkillPlace::Library,
                                staged: true,
                                preserve_backup: false,
                                expected_backup_hash: None,
                            }],
                            Some(&state),
                        )
                    } else {
                        self.save_state_unlocked(&state)
                    }
                })();
                if result.is_err() {
                    if let Some(operation) = &operation {
                        let stage = self.transaction_stage(operation, 0, &SkillPlace::Library)?;
                        let journal = self
                            .state_path
                            .parent()
                            .context("Skill 状态文件缺少父目录")?
                            .join(".skill-transactions")
                            .join(format!("{operation}.json"));
                        if stage.exists() && !journal.exists() {
                            remove_link(&stage).context("清理未提交的 Skill 暂存失败")?;
                        }
                    }
                }
                result
            })();
            match result {
                Ok(()) => applied += 1,
                Err(error) => errors.push(format!("{}：{error:#}", selection.local_id)),
            }
        }
        Ok((applied, errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> (tempfile::TempDir, SkillsManager, SkillRepo) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let manager = SkillsManager::new(
            root.join("library"),
            root.join("backups"),
            root.join("state/skills.json"),
            root.join("codex"),
        )
        .with_agent_dirs(vec![("claude".into(), root.join("claude/skills"))]);
        let repo = parse_repo_key("example/skills@main").unwrap();
        manager.upsert_repo(repo.clone()).unwrap();
        (temp, manager, repo)
    }

    fn write_skill(path: &Path, text: &str) {
        std::fs::create_dir_all(path).unwrap();
        std::fs::write(path.join(SKILL_MANIFEST_FILE), text).unwrap();
    }

    fn remote(repo: &SkillRepo, path: &str, name: &str, hash: &str) -> RemoteSkill {
        RemoteSkill {
            id: format!("remote-{name}"),
            name: name.into(),
            description: String::new(),
            repo_key: repo.key(),
            repo_path: path.into(),
            content_hash: hash.into(),
        }
    }

    fn same_named_skills_zip() -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for (path, content) in [
                (
                    "skills-main/a/alpha/SKILL.md",
                    "---\nname: Alpha\ndescription: Source A\n---\n",
                ),
                (
                    "skills-main/z/alpha/SKILL.md",
                    "---\nname: Alpha\ndescription: Source Z\n---\n",
                ),
                (
                    "skills-main/b/beta/SKILL.md",
                    "---\nname: Beta\ndescription: Source B\n---\n",
                ),
            ] {
                writer.start_file(path, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    fn selection(item: &SkillRepositoryMatch) -> SkillRepositoryMatchSelection {
        SkillRepositoryMatchSelection {
            local_id: item.local_id.clone(),
            local_hash: item.local_hash.clone(),
            snapshot: item.snapshot.clone(),
            repo_key: item.repo_key.clone(),
            repo_path: item.repo_path.clone(),
            remote_hash: item.remote_hash.clone(),
            manual: false,
        }
    }

    fn installed_with_origin(manager: &SkillsManager, repo: &SkillRepo) {
        let content = "---\nname: Alpha\ndescription: Original\n---\nuser content\n";
        for path in [
            manager.source_dir.join("alpha"),
            manager.linked_dir().join("alpha"),
            manager.agent_dirs[0].1.join("alpha"),
        ] {
            write_skill(&path, content);
            std::fs::write(path.join("extra.txt"), "user data").unwrap();
        }
        manager.prepare_managed_skills().unwrap();
        let _state = manager.state_lock.lock().unwrap();
        let mut state = manager.load_state_checked_unlocked().unwrap();
        let item = state.installed.get_mut("alpha").unwrap();
        item.repo_key = repo.key();
        item.repo_path = "original/alpha".into();
        item.import_source = "cc-switch".into();
        item.content_hash = skill_git_tree_hash(&manager.source_dir.join("alpha")).unwrap();
        manager.save_state_unlocked(&state).unwrap();
    }

    fn assert_alpha_files_unchanged(manager: &SkillsManager) {
        for path in [
            manager.source_dir.join("alpha"),
            manager.linked_dir().join("alpha"),
            manager.agent_dirs[0].1.join("alpha"),
        ] {
            assert_eq!(
                std::fs::read_to_string(path.join(SKILL_MANIFEST_FILE)).unwrap(),
                "---\nname: Alpha\ndescription: Original\n---\nuser content\n"
            );
            assert_eq!(
                std::fs::read_to_string(path.join("extra.txt")).unwrap(),
                "user data"
            );
        }
    }

    #[test]
    fn existing_origin_can_be_relinked_to_another_path_and_repository() {
        let (_temp, manager, first_repo) = fixture();
        installed_with_origin(&manager, &first_repo);
        let second_repo = parse_repo_key("other/skills@main").unwrap();
        manager.upsert_repo(second_repo.clone()).unwrap();
        let before = manager.load_state().installed["alpha"].clone();
        let hash = skill_git_tree_hash(&manager.source_dir.join("alpha")).unwrap();
        let first_remote = vec![
            remote(&first_repo, "original/alpha", "Alpha", &hash),
            remote(&first_repo, "relocated/renamed", "Renamed", &hash),
        ];
        let batch = manager
            .preview_repository_matches(&first_repo, &first_remote, None)
            .unwrap();
        assert_eq!(batch.examined, 0);
        assert!(batch.matches.is_empty());
        assert!(batch.manual_candidates.is_empty());

        let single = manager
            .preview_repository_matches(&first_repo, &first_remote, Some("alpha"))
            .unwrap();
        assert_eq!(single.examined, 1);
        assert_eq!(single.matches.len(), 2);
        let next = single
            .matches
            .iter()
            .find(|item| item.repo_path == "relocated/renamed")
            .unwrap();
        assert!(next.exact);
        assert_eq!(
            manager
                .apply_repository_matches(&[selection(next)], &first_remote)
                .unwrap(),
            (1, vec![])
        );
        let after = manager.load_state().installed["alpha"].clone();
        assert_eq!(after.repo_key, first_repo.key());
        assert_eq!(after.repo_path, "relocated/renamed");
        assert_eq!(after.import_source, before.import_source);
        assert_eq!(after.installed_at, before.installed_at);
        assert_eq!(after.name, before.name);
        assert_eq!(after.description, before.description);
        assert!(manager.update_local_snapshot("alpha").unwrap().is_none());
        let entries = manager.merge_entries(&first_remote);
        let installed = entries
            .iter()
            .find(|entry| {
                entry.id == "alpha" && entry.repo_path == "relocated/renamed" && entry.installed
            })
            .unwrap();
        assert_eq!(installed.name, before.name);
        assert_eq!(installed.description, before.description);
        assert!(
            entries
                .iter()
                .any(|entry| entry.repo_path == "original/alpha"
                    && !entry.installed
                    && entry.name == "Alpha")
        );

        let second_remote = vec![remote(
            &second_repo,
            "skills/alpha",
            "Alpha",
            &"a".repeat(64),
        )];
        let single = manager
            .preview_repository_matches(&second_repo, &second_remote, Some("alpha"))
            .unwrap();
        assert!(!single.matches[0].exact);
        assert_eq!(
            manager
                .apply_repository_matches(&[selection(&single.matches[0])], &second_remote)
                .unwrap(),
            (1, vec![])
        );
        let after = manager.load_state().installed["alpha"].clone();
        assert_eq!(after.repo_key, second_repo.key());
        assert_eq!(after.repo_path, "skills/alpha");
        assert_eq!(after.local_baseline_hash, UNVERIFIED_ORIGIN_BASELINE);
        assert_eq!(after.content_hash, hash);
        assert!(manager.update_local_snapshot("alpha").unwrap().is_some());
        assert!(
            manager
                .merge_entries(&second_remote)
                .iter()
                .any(|entry| entry.id == "alpha"
                    && entry.repo_key == second_repo.key()
                    && entry.update_available)
        );
        assert_alpha_files_unchanged(&manager);
    }

    #[test]
    fn relinking_keeps_installed_identity_when_remote_id_is_still_in_use() {
        for (old_path, new_path) in [("z/alpha", "a/alpha"), ("a/alpha", "z/alpha")] {
            for enabled in [false, true] {
                let (_temp, manager, repo) = fixture();
                let zip = same_named_skills_zip();
                let remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
                let old = remote
                    .iter()
                    .find(|item| item.repo_path == old_path)
                    .unwrap();
                let next = remote
                    .iter()
                    .find(|item| item.repo_path == new_path)
                    .unwrap();
                assert_ne!(old.id, next.id);
                manager.install_from_zip(old, &zip).unwrap();
                manager.set_enabled(&old.id, enabled).unwrap();

                let preview = manager
                    .preview_repository_matches(&repo, &remote, Some(&old.id))
                    .unwrap();
                let chosen = preview
                    .matches
                    .iter()
                    .find(|item| item.repo_path == new_path)
                    .unwrap();
                assert_eq!(
                    manager
                        .apply_repository_matches(&[selection(chosen)], &remote)
                        .unwrap(),
                    (1, vec![])
                );

                let original =
                    std::fs::read(manager.source_dir.join(&old.id).join(SKILL_MANIFEST_FILE))
                        .unwrap();
                let mut alias = None;
                for reversed in [false, true] {
                    let mut ordered = remote.clone();
                    if reversed {
                        ordered.reverse();
                    }
                    let entries = manager.merge_entries(&ordered);
                    let alpha: Vec<_> = entries
                        .iter()
                        .filter(|item| item.repo_key == repo.key() && item.id == old.id)
                        .collect();
                    assert_eq!(alpha.len(), 1, "{old_path} -> {new_path}");
                    assert!(alpha[0].installed, "{old_path} -> {new_path}");
                    assert_eq!(alpha[0].repo_path, new_path);
                    assert_eq!(alpha[0].name, old.name);
                    assert_eq!(alpha[0].description, old.description);
                    assert_eq!(alpha[0].remote_hash, next.content_hash);
                    assert_eq!(alpha[0].enabled, enabled);
                    assert_eq!(alpha[0].agents.contains(&"codex".into()), enabled);
                    assert!(alpha[0].managed);
                    let available = entries
                        .iter()
                        .find(|item| item.repo_key == repo.key() && item.repo_path == old_path)
                        .unwrap();
                    assert!(!available.installed);
                    assert!(!available.enabled);
                    assert_ne!(available.id, old.id);
                    assert_eq!(available.name, old.name);
                    assert_eq!(available.description, old.description);
                    if let Some(previous) = &alias {
                        assert_eq!(&available.id, previous);
                    } else {
                        alias = Some(available.id.clone());
                    }
                    let beta = entries
                        .iter()
                        .find(|item| item.repo_path == "b/beta")
                        .unwrap();
                    assert!(!beta.installed);
                    assert_eq!(
                        beta.id,
                        remote
                            .iter()
                            .find(|item| item.repo_path == "b/beta")
                            .unwrap()
                            .id
                    );
                }
                let alias = alias.unwrap();
                assert!(
                    manager
                        .resolve_remote_install(&remote, &repo.key(), &alias, None)
                        .is_err()
                );
                assert!(
                    manager
                        .resolve_remote_install(&remote, &repo.key(), &alias, Some(new_path))
                        .is_err()
                );
                assert!(
                    manager
                        .resolve_remote_install(&remote, &repo.key(), &old.id, Some(old_path))
                        .is_err()
                );
                let selected = manager
                    .resolve_remote_install(&remote, &repo.key(), &alias, Some(old_path))
                    .unwrap();
                assert_eq!(selected.id, alias);
                assert_eq!(selected.repo_path, old_path);
                manager.install_new_from_zip(&selected, &zip).unwrap();
                assert!(manager.install_new_from_zip(&selected, &zip).is_err());
                let state = manager.load_state();
                assert_eq!(state.installed[&old.id].repo_path, new_path);
                assert_eq!(state.installed[&alias].repo_path, old_path);
                assert_eq!(
                    std::fs::read(manager.source_dir.join(&old.id).join(SKILL_MANIFEST_FILE))
                        .unwrap(),
                    original
                );
                let (_, description) = read_skill_manifest(
                    &manager.source_dir.join(&alias).join(SKILL_MANIFEST_FILE),
                    &alias,
                );
                assert_eq!(description, old.description);
                let entries = manager.merge_entries(&remote);
                assert!(
                    entries.iter().any(|item| item.id == alias
                        && item.repo_path == old_path
                        && item.installed)
                );
                assert!(entries.iter().any(|item| item.id == old.id
                    && item.repo_path == new_path
                    && item.installed
                    && item.enabled == enabled));
            }
        }
    }

    #[test]
    fn colliding_alternative_id_is_reassigned_without_touching_existing_files() {
        let (_temp, manager, repo) = fixture();
        let zip = same_named_skills_zip();
        let remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        let old = remote
            .iter()
            .find(|item| item.repo_path == "z/alpha")
            .unwrap();
        manager.install_from_zip(old, &zip).unwrap();
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some(&old.id))
            .unwrap();
        let next = preview
            .matches
            .iter()
            .find(|item| item.repo_path == "a/alpha")
            .unwrap();
        manager
            .apply_repository_matches(&[selection(next)], &remote)
            .unwrap();
        let entries = manager.merge_entries(&remote);
        let first_id = entries
            .iter()
            .find(|item| item.repo_path == old.repo_path)
            .unwrap()
            .id
            .clone();
        write_skill(
            &manager.source_dir.join(&first_id),
            "existing local content",
        );
        let entries = manager.merge_entries(&remote);
        let second_id = &entries
            .iter()
            .find(|item| item.repo_path == old.repo_path)
            .unwrap()
            .id;
        assert_ne!(&first_id, second_id);
        assert!(
            manager
                .resolve_remote_install(&remote, &repo.key(), &first_id, Some(&old.repo_path))
                .is_err()
        );
        let selected = manager
            .resolve_remote_install(&remote, &repo.key(), second_id, Some(&old.repo_path))
            .unwrap();
        manager.install_new_from_zip(&selected, &zip).unwrap();
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join(first_id).join(SKILL_MANIFEST_FILE))
                .unwrap(),
            "existing local content"
        );
    }

    #[test]
    fn relinking_preserves_both_remote_paths_when_ids_do_not_collide() {
        for (old_path, new_path) in [("z/alpha", "a/alpha"), ("a/alpha", "z/alpha")] {
            for enabled in [false, true] {
                let (_temp, manager, repo) = fixture();
                installed_with_origin(&manager, &repo);
                if !enabled {
                    manager.set_enabled("alpha", false).unwrap();
                }
                let zip = same_named_skills_zip();
                let remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
                let old = remote
                    .iter()
                    .find(|item| item.repo_path == old_path)
                    .unwrap();
                let next = remote
                    .iter()
                    .find(|item| item.repo_path == new_path)
                    .unwrap();
                assert_ne!(old.id, "alpha");
                let preview = manager
                    .preview_repository_matches(&repo, &remote, Some("alpha"))
                    .unwrap();
                let chosen = preview
                    .matches
                    .iter()
                    .find(|item| item.repo_path == new_path)
                    .unwrap();
                assert_eq!(
                    manager
                        .apply_repository_matches(&[selection(chosen)], &remote)
                        .unwrap(),
                    (1, vec![])
                );
                for reversed in [false, true] {
                    let mut ordered = remote.clone();
                    if reversed {
                        ordered.reverse();
                    }
                    let entries = manager.merge_entries(&ordered);
                    let installed = entries
                        .iter()
                        .find(|item| item.repo_path == new_path)
                        .unwrap();
                    assert_eq!(installed.id, "alpha");
                    assert!(installed.installed);
                    assert_eq!(installed.enabled, enabled);
                    let available = entries
                        .iter()
                        .find(|item| item.repo_path == old_path)
                        .unwrap();
                    assert_eq!(available.id, old.id);
                    assert!(!available.installed);
                }

                manager.install_from_zip(old, &zip).unwrap();
                let mut update = next.clone();
                update.id = "alpha".into();
                let snapshot = manager.update_local_snapshot("alpha").unwrap().unwrap();
                manager
                    .install_from_zip_with_confirmation(&update, &zip, Some(&snapshot))
                    .unwrap();
                let state = manager.load_state();
                assert_eq!(state.installed[&old.id].repo_path, old_path);
                assert_eq!(state.installed["alpha"].repo_path, new_path);
                for (id, source) in [(old.id.as_str(), old), ("alpha", next)] {
                    let (_, description) = read_skill_manifest(
                        &manager.source_dir.join(id).join(SKILL_MANIFEST_FILE),
                        id,
                    );
                    assert_eq!(description, source.description);
                }
            }
        }
    }

    #[test]
    fn relinking_refreshes_metadata_from_current_local_manifest() {
        for enabled in [false, true] {
            let (_temp, manager, repo) = fixture();
            installed_with_origin(&manager, &repo);
            if !enabled {
                manager.set_enabled("alpha", false).unwrap();
            }
            let updated = "---\nname: Edited Alpha\ndescription: Edited locally\n---\nlocal\n";
            std::fs::write(manager.source_dir.join("alpha/SKILL.md"), updated).unwrap();
            if enabled {
                std::fs::write(manager.linked_dir().join("alpha/SKILL.md"), updated).unwrap();
            }
            let before = manager.load_state().installed["alpha"].clone();
            assert_eq!(before.name, "Alpha");
            assert_eq!(before.description, "Original");

            let mut candidate = remote(&repo, "new/alpha", "Remote Alpha", &"a".repeat(64));
            candidate.description = "Remote description".into();
            let remote = vec![candidate];
            let preview = manager
                .preview_repository_matches(&repo, &remote, Some("alpha"))
                .unwrap();
            assert_eq!(preview.matches[0].local_name, "Edited Alpha");
            assert_eq!(
                manager
                    .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
                    .unwrap(),
                (1, vec![])
            );
            let after = manager.load_state().installed["alpha"].clone();
            assert_eq!(after.name, "Edited Alpha");
            assert_eq!(after.description, "Edited locally");
            assert_eq!(after.installed_at, before.installed_at);
            assert_eq!(after.import_source, before.import_source);
            let entries = manager.merge_entries(&remote);
            let installed = entries.iter().find(|item| item.installed).unwrap();
            assert_eq!(installed.name, "Edited Alpha");
            assert_eq!(installed.description, "Edited locally");
            assert_eq!(installed.enabled, enabled);
            assert_eq!(
                std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
                updated
            );
        }
    }

    #[test]
    fn manual_nonmatching_candidate_requires_explicit_single_selection() {
        let (_temp, manager, repo) = fixture();
        installed_with_origin(&manager, &repo);
        let remote = vec![remote(
            &repo,
            "tools/unrelated",
            "Unrelated",
            &"b".repeat(64),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        assert!(preview.matches.is_empty());
        assert_eq!(preview.manual_candidates.len(), 1);
        let mut chosen = selection(&preview.manual_candidates[0]);
        let before = manager.load_state();
        let (count, errors) = manager
            .apply_repository_matches(&[chosen.clone()], &remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert_eq!(manager.load_state(), before);

        chosen.manual = true;
        assert!(
            manager
                .apply_repository_matches(&[chosen.clone(), chosen.clone()], &remote)
                .is_err()
        );
        assert_eq!(
            manager
                .apply_repository_matches(&[chosen], &remote)
                .unwrap(),
            (1, vec![])
        );
        let entries = manager.merge_entries(&remote);
        let installed = entries.iter().find(|entry| entry.id == "alpha").unwrap();
        assert_eq!(installed.name, "Alpha");
        assert_eq!(installed.description, "Original");
        assert_eq!(
            manager.load_state().installed["alpha"].repo_path,
            "tools/unrelated"
        );
        assert!(manager.update_local_snapshot("alpha").unwrap().is_some());
        assert_alpha_files_unchanged(&manager);
    }

    #[test]
    fn preview_and_selection_use_tauri_camel_case_json_with_backward_compatible_manual_flag() {
        let (_temp, manager, repo) = fixture();
        installed_with_origin(&manager, &repo);
        let remote = vec![remote(
            &repo,
            "tools/unrelated",
            "Unrelated",
            &"b".repeat(64),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        let json = serde_json::to_value(&preview).unwrap();
        assert!(json["matches"].as_array().unwrap().is_empty());
        let candidate = &json["manualCandidates"][0];
        assert_eq!(candidate["localId"], "alpha");
        assert_eq!(candidate["repoKey"], repo.key());
        assert_eq!(candidate["repoPath"], "tools/unrelated");
        assert_eq!(candidate["remoteHash"], "b".repeat(64));
        assert!(!candidate["snapshot"].as_str().unwrap().is_empty());
        assert_eq!(candidate["exact"], false);

        let mut request = candidate.clone();
        request["manual"] = true.into();
        let explicit: SkillRepositoryMatchSelection = serde_json::from_value(request).unwrap();
        assert!(explicit.manual);
        let mut request = candidate.clone();
        request.as_object_mut().unwrap().remove("manual");
        let old: SkillRepositoryMatchSelection = serde_json::from_value(request).unwrap();
        assert!(!old.manual);
        assert_eq!(
            manager
                .apply_repository_matches(&[explicit], &remote)
                .unwrap(),
            (1, vec![])
        );
    }

    #[test]
    fn relinking_rejects_stale_snapshots_and_leaves_existing_origins_unchanged() {
        for change in ["agent", "origin", "remote", "repo", "local"] {
            let (_temp, manager, repo) = fixture();
            installed_with_origin(&manager, &repo);
            let hash = skill_git_tree_hash(&manager.source_dir.join("alpha")).unwrap();
            let remote = vec![remote(&repo, "new/alpha", "Alpha", &hash)];
            let preview = manager
                .preview_repository_matches(&repo, &remote, Some("alpha"))
                .unwrap();
            let chosen = selection(&preview.matches[0]);
            let mut current_remote = remote.clone();
            match change {
                "agent" => std::fs::write(
                    manager.agent_dirs[0].1.join("alpha/extra.txt"),
                    "changed agent",
                )
                .unwrap(),
                "origin" => {
                    let _state = manager.state_lock.lock().unwrap();
                    let mut state = manager.load_state_checked_unlocked().unwrap();
                    state.installed.get_mut("alpha").unwrap().repo_path = "changed/alpha".into();
                    manager.save_state_unlocked(&state).unwrap();
                }
                "remote" => current_remote[0].content_hash = "c".repeat(64),
                "repo" => {
                    manager.delete_repo(&repo.key()).unwrap();
                }
                "local" => std::fs::write(
                    manager.source_dir.join("alpha/extra.txt"),
                    "changed library",
                )
                .unwrap(),
                _ => unreachable!(),
            }
            let before = manager.load_state();
            let library = std::fs::read(manager.source_dir.join("alpha/extra.txt")).unwrap();
            let codex = std::fs::read(manager.linked_dir().join("alpha/extra.txt")).unwrap();
            let agent = std::fs::read(manager.agent_dirs[0].1.join("alpha/extra.txt")).unwrap();
            let (count, errors) = manager
                .apply_repository_matches(&[chosen], &current_remote)
                .unwrap();
            assert_eq!(count, 0, "{change}");
            assert_eq!(errors.len(), 1, "{change}");
            assert_eq!(manager.load_state(), before, "{change}");
            assert_eq!(
                std::fs::read(manager.source_dir.join("alpha/extra.txt")).unwrap(),
                library
            );
            assert_eq!(
                std::fs::read(manager.linked_dir().join("alpha/extra.txt")).unwrap(),
                codex
            );
            assert_eq!(
                std::fs::read(manager.agent_dirs[0].1.join("alpha/extra.txt")).unwrap(),
                agent
            );
        }
    }

    #[test]
    fn relinking_rejects_a_path_already_claimed_after_preview() {
        let (_temp, manager, repo) = fixture();
        installed_with_origin(&manager, &repo);
        write_skill(&manager.source_dir.join("beta"), "---\nname: Beta\n---\n");
        manager.prepare_managed_skills().unwrap();
        let hash = skill_git_tree_hash(&manager.source_dir.join("alpha")).unwrap();
        let remote = vec![remote(&repo, "new/alpha", "Alpha", &hash)];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        let chosen = selection(&preview.matches[0]);
        {
            let _state = manager.state_lock.lock().unwrap();
            let mut state = manager.load_state_checked_unlocked().unwrap();
            let beta = state.installed.get_mut("beta").unwrap();
            beta.repo_key = repo.key();
            beta.repo_path = "new/alpha".into();
            manager.save_state_unlocked(&state).unwrap();
        }
        let before = manager.load_state();
        let (count, errors) = manager
            .apply_repository_matches(&[chosen], &remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("仓库路径已关联其他 Skill"));
        assert_eq!(manager.load_state(), before);
        assert_alpha_files_unchanged(&manager);
    }

    #[test]
    fn competing_relink_confirmations_cannot_overwrite_the_first_result() {
        let (_temp, manager, repo) = fixture();
        installed_with_origin(&manager, &repo);
        let hash = skill_git_tree_hash(&manager.source_dir.join("alpha")).unwrap();
        let remote = vec![
            remote(&repo, "one/alpha", "Alpha", &hash),
            remote(&repo, "two/alpha", "Alpha", &hash),
        ];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = preview
            .matches
            .iter()
            .map(|item| {
                let chosen = selection(item);
                let manager = manager.clone();
                let remote = remote.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    manager
                        .apply_repository_matches(&[chosen], &remote)
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().map(|(count, _)| count).sum::<usize>(), 1);
        assert_eq!(
            results
                .iter()
                .map(|(_, errors)| errors.len())
                .sum::<usize>(),
            1
        );
        assert!(
            ["one/alpha", "two/alpha"]
                .contains(&manager.load_state().installed["alpha"].repo_path.as_str())
        );
        assert_alpha_files_unchanged(&manager);
    }

    #[test]
    fn unmanaged_skill_with_occupied_library_path_is_not_partially_saved() {
        let (_temp, manager, repo) = fixture();
        let agent = manager.agent_dirs[0].1.join("alpha");
        write_skill(&agent, "---\nname: Alpha\n---\noriginal agent\n");
        std::fs::create_dir_all(manager.source_dir.join("alpha")).unwrap();
        std::fs::write(
            manager.source_dir.join("alpha/user-data.txt"),
            "do not replace",
        )
        .unwrap();
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&agent).unwrap(),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        let before = manager.load_state();
        let (count, errors) = manager
            .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert_eq!(manager.load_state(), before);
        assert!(!manager.load_state().installed.contains_key("alpha"));
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/user-data.txt")).unwrap(),
            "do not replace"
        );
        assert_eq!(
            std::fs::read_to_string(agent.join(SKILL_MANIFEST_FILE)).unwrap(),
            "---\nname: Alpha\n---\noriginal agent\n"
        );
    }

    #[test]
    fn preview_and_failed_confirmation_do_not_adopt_unrelated_skills() {
        let (_temp, manager, repo) = fixture();
        let alpha = manager.linked_dir().join("alpha");
        let beta = manager.linked_dir().join("beta");
        write_skill(&alpha, "---\nname: Alpha\n---\noriginal alpha\n");
        write_skill(&beta, "---\nname: Beta\n---\noriginal beta\n");
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&alpha).unwrap(),
        )];
        let before = std::fs::read(&manager.state_path).unwrap();
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(!manager.source_dir.join("alpha").exists());
        assert!(!manager.source_dir.join("beta").exists());

        let mut changed_remote = remote.clone();
        changed_remote[0].content_hash = "c".repeat(64);
        let (count, errors) = manager
            .apply_repository_matches(&[selection(&preview.matches[0])], &changed_remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(!manager.source_dir.join("alpha").exists());
        assert!(!manager.source_dir.join("beta").exists());
        assert_eq!(
            std::fs::read_to_string(beta.join(SKILL_MANIFEST_FILE)).unwrap(),
            "---\nname: Beta\n---\noriginal beta\n"
        );

        assert_eq!(
            manager
                .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
                .unwrap(),
            (1, vec![])
        );
        assert_eq!(
            std::fs::read_to_string(alpha.join(SKILL_MANIFEST_FILE)).unwrap(),
            "---\nname: Alpha\n---\noriginal alpha\n"
        );
        assert!(!manager.source_dir.join("beta").exists());
        assert!(!manager.load_state().installed.contains_key("beta"));
    }

    #[test]
    fn legacy_only_source_is_snapshotted_without_migration() {
        let (temp, manager, repo) = fixture();
        let legacy = temp.path().join("legacy");
        let manager = manager.with_legacy_source_dir(&legacy);
        let alpha = legacy.join("alpha");
        let original = "---\nname: Alpha\n---\nlegacy content\n";
        write_skill(&alpha, original);
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&alpha).unwrap(),
        )];
        let before = std::fs::read(&manager.state_path).unwrap();
        let preview = manager
            .preview_repository_matches(&repo, &remote, None)
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(!manager.source_dir.join("alpha").exists());

        std::fs::write(alpha.join("extra.txt"), "changed after preview").unwrap();
        let (count, errors) = manager
            .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(
            std::fs::read_to_string(alpha.join("extra.txt")).unwrap(),
            "changed after preview"
        );
    }

    #[test]
    fn legacy_and_codex_conflict_blocks_matching_without_migrating() {
        for codex_manifest in [Some("---\nname: Alpha\n---\nCodex edit\n"), None] {
            let (temp, manager, repo) = fixture();
            let manager = manager.with_legacy_source_dir(temp.path().join("legacy"));
            let legacy = temp.path().join("legacy/alpha");
            let codex = manager.linked_dir().join("alpha");
            let original = "---\nname: Alpha\n---\nlegacy content\n";
            write_skill(&legacy, original);
            if let Some(content) = codex_manifest {
                write_skill(&codex, content);
            } else {
                std::fs::create_dir_all(&codex).unwrap();
                std::fs::write(codex.join("user-data.txt"), "incomplete").unwrap();
            }
            let remote = vec![remote(
                &repo,
                "alpha",
                "Alpha",
                &skill_git_tree_hash(&legacy).unwrap(),
            )];
            let before = std::fs::read(&manager.state_path).unwrap();
            let preview = manager
                .preview_repository_matches(&repo, &remote, Some("alpha"))
                .unwrap();
            assert!(preview.matches.is_empty());
            assert!(preview.manual_candidates.is_empty());
            assert_eq!(preview.warnings.len(), 1);
            assert!(preview.warnings[0].contains("Codex 副本与管理库不一致"));
            assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
            assert!(!manager.source_dir.join("alpha").exists());
            assert_eq!(
                std::fs::read_to_string(legacy.join(SKILL_MANIFEST_FILE)).unwrap(),
                original
            );
            if let Some(content) = codex_manifest {
                assert_eq!(
                    std::fs::read_to_string(codex.join(SKILL_MANIFEST_FILE)).unwrap(),
                    content
                );
            } else {
                assert_eq!(
                    std::fs::read_to_string(codex.join("user-data.txt")).unwrap(),
                    "incomplete"
                );
            }
        }
    }

    #[test]
    fn codex_change_after_legacy_preview_is_rejected_without_migration() {
        let (temp, manager, repo) = fixture();
        let manager = manager.with_legacy_source_dir(temp.path().join("legacy"));
        let legacy = temp.path().join("legacy/alpha");
        let codex = manager.linked_dir().join("alpha");
        let original = "---\nname: Alpha\n---\nshared content\n";
        write_skill(&legacy, original);
        write_skill(&codex, original);
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&legacy).unwrap(),
        )];
        let before = std::fs::read(&manager.state_path).unwrap();
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        std::fs::write(codex.join(SKILL_MANIFEST_FILE), "changed Codex").unwrap();
        let (count, errors) = manager
            .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Codex 副本与管理库不一致"));
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(
            std::fs::read_to_string(legacy.join(SKILL_MANIFEST_FILE)).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read_to_string(codex.join(SKILL_MANIFEST_FILE)).unwrap(),
            "changed Codex"
        );
    }

    #[test]
    fn matching_a_legacy_only_skill_adopts_just_that_skill_after_confirmation() {
        let (temp, manager, repo) = fixture();
        let manager = manager.with_legacy_source_dir(temp.path().join("legacy"));
        let legacy = temp.path().join("legacy/alpha");
        let original = "---\nname: Alpha\n---\nlegacy content\n";
        write_skill(&legacy, original);
        write_skill(
            &temp.path().join("legacy/beta"),
            "---\nname: Beta\n---\nother content\n",
        );
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&legacy).unwrap(),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert_eq!(
            manager
                .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
                .unwrap(),
            (1, vec![])
        );
        let state = manager.load_state();
        assert!(!state.independent_library_migrated);
        assert_eq!(state.installed["alpha"].repo_path, "alpha");
        assert!(!state.installed.contains_key("beta"));
        assert!(!manager.source_dir.join("beta").exists());
        assert!(!manager.linked_dir().join("alpha").exists());
        assert_eq!(
            std::fs::read_to_string(legacy.join(SKILL_MANIFEST_FILE)).unwrap(),
            original
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            original
        );
    }

    #[test]
    fn exact_content_match_attaches_origin_without_replacing_local_files() {
        let (_temp, manager, repo) = fixture();
        let path = manager.linked_dir().join("alpha");
        write_skill(&path, "---\nname: Alpha\n---\nlocal content\n");
        let hash = skill_git_tree_hash(&path).unwrap();
        let remote = vec![remote(&repo, "nested/renamed", "Different name", &hash)];
        let preview = manager
            .preview_repository_matches(&repo, &remote, None)
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(preview.matches[0].exact);
        let result = manager
            .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
            .unwrap();
        assert_eq!(result, (1, vec![]));
        assert_eq!(
            std::fs::read_to_string(path.join(SKILL_MANIFEST_FILE)).unwrap(),
            "---\nname: Alpha\n---\nlocal content\n"
        );
        let state = manager.load_state();
        let item = &state.installed["alpha"];
        assert_eq!(item.repo_key, repo.key());
        assert_eq!(item.repo_path, "nested/renamed");
        assert_eq!(item.content_hash, hash);
        assert!(manager.update_local_snapshot("alpha").unwrap().is_none());
    }

    #[test]
    fn name_only_match_is_explicit_and_preserves_update_confirmation() {
        let (_temp, manager, repo) = fixture();
        write_skill(
            &manager.linked_dir().join("alpha"),
            "---\nname: Alpha\n---\ncustom version\n",
        );
        let remote = vec![remote(&repo, "skills/alpha", "Alpha", &"a".repeat(64))];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("alpha"))
            .unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(!preview.matches[0].exact);
        assert_eq!(
            manager
                .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
                .unwrap()
                .0,
            1
        );
        assert!(manager.update_local_snapshot("alpha").unwrap().is_some());
        assert!(
            manager
                .merge_entries(&remote)
                .iter()
                .any(|item| item.id == "alpha" && item.update_available)
        );
    }

    #[test]
    fn stale_local_or_remote_snapshot_and_deleted_repository_are_rejected() {
        let (_temp, manager, repo) = fixture();
        let codex = manager.linked_dir().join("alpha");
        write_skill(&codex, "---\nname: Alpha\n---\nfirst\n");
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&codex).unwrap(),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, None)
            .unwrap();
        let chosen = selection(&preview.matches[0]);
        let mut changed_remote = remote.clone();
        changed_remote[0].content_hash = "b".repeat(64);
        assert_eq!(
            manager
                .apply_repository_matches(&[chosen.clone()], &changed_remote)
                .unwrap()
                .0,
            0
        );
        std::fs::write(codex.join(SKILL_MANIFEST_FILE), "changed").unwrap();
        assert_eq!(
            manager
                .apply_repository_matches(&[chosen.clone()], &remote)
                .unwrap()
                .0,
            0
        );
        std::fs::write(
            codex.join(SKILL_MANIFEST_FILE),
            "---\nname: Alpha\n---\nfirst\n",
        )
        .unwrap();
        manager.delete_repo(&repo.key()).unwrap();
        let after_deletion = std::fs::read(&manager.state_path).unwrap();
        assert_eq!(
            manager
                .apply_repository_matches(&[chosen], &remote)
                .unwrap()
                .0,
            0
        );
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), after_deletion);
        assert!(manager.load_state().installed["alpha"].repo_key.is_empty());
    }

    #[test]
    fn third_party_agent_skill_is_adopted_only_after_selection() {
        let (temp, manager, repo) = fixture();
        let path = temp.path().join("claude/skills/alpha");
        write_skill(&path, "---\nname: Alpha\n---\n");
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&path).unwrap(),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, None)
            .unwrap();
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(
            manager
                .apply_repository_matches(&[selection(&preview.matches[0])], &remote)
                .unwrap()
                .0,
            1
        );
        assert!(manager.source_dir.join("alpha/SKILL.md").is_file());
        assert!(path.join(SKILL_MANIFEST_FILE).is_file());
    }

    #[test]
    fn duplicate_selections_and_existing_repository_origins_are_rejected() {
        let (_temp, manager, repo) = fixture();
        let codex = manager.linked_dir();
        write_skill(&codex.join("alpha"), "---\nname: Alpha\n---\n");
        write_skill(&codex.join("beta"), "---\nname: Alpha\n---\n");
        let remote = vec![remote(
            &repo,
            "alpha",
            "Alpha",
            &skill_git_tree_hash(&codex.join("alpha")).unwrap(),
        )];
        let preview = manager
            .preview_repository_matches(&repo, &remote, None)
            .unwrap();
        let first = selection(
            preview
                .matches
                .iter()
                .find(|item| item.local_id == "alpha")
                .unwrap(),
        );
        let second = selection(
            preview
                .matches
                .iter()
                .find(|item| item.local_id == "beta")
                .unwrap(),
        );
        assert!(
            manager
                .apply_repository_matches(&[first.clone(), second.clone()], &remote)
                .is_err()
        );
        assert_eq!(
            manager
                .apply_repository_matches(&[first], &remote)
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            manager
                .apply_repository_matches(&[second], &remote)
                .unwrap()
                .0,
            0
        );
    }

    #[test]
    fn repository_origin_identity_preserves_branch_and_path_case() {
        let origin = repository_match_origin("Owner/Repo@main:skills", "skills/Alpha");
        assert_eq!(
            origin,
            repository_match_origin("owner/repo@main", "skills/Alpha")
        );
        assert_eq!(
            origin,
            repository_match_origin("OWNER/REPO", "skills/Alpha")
        );
        assert_ne!(
            origin,
            repository_match_origin("owner/repo@Main", "skills/Alpha")
        );
        assert_ne!(
            origin,
            repository_match_origin("owner/repo@main", "skills/alpha")
        );
        assert_ne!(
            origin,
            repository_match_origin("owner/repo@main", "skills/Alpha/nested")
        );
        assert_ne!(
            origin,
            repository_match_origin("owner/other@main", "skills/Alpha")
        );
        assert_eq!(
            repository_match_origin("local", "."),
            ("local".into(), ".".into())
        );
    }

    #[test]
    fn overlapping_sources_reject_duplicate_batch_and_existing_origins() {
        for root_first in [true, false] {
            let (_temp, manager, root) = fixture();
            let subdir = parse_repo_key("Example/Skills@main:skills").unwrap();
            manager.upsert_repo(subdir.clone()).unwrap();
            let codex = manager.linked_dir();
            write_skill(&codex.join("alpha"), "---\nname: Shared\n---\n");
            write_skill(&codex.join("beta"), "---\nname: Shared\n---\n");
            let hash = skill_git_tree_hash(&codex.join("alpha")).unwrap();
            let remote = vec![
                remote(&root, "skills/alpha", "Shared", &hash),
                remote(&subdir, "skills/alpha", "Shared", &hash),
            ];
            let root_preview = manager
                .preview_repository_matches(&root, &remote, None)
                .unwrap();
            let subdir_preview = manager
                .preview_repository_matches(&subdir, &remote, None)
                .unwrap();
            let (first_preview, second_preview) = if root_first {
                (&root_preview, &subdir_preview)
            } else {
                (&subdir_preview, &root_preview)
            };
            let first = selection(
                first_preview
                    .matches
                    .iter()
                    .find(|item| item.local_id == "alpha")
                    .unwrap(),
            );
            let second = selection(
                second_preview
                    .matches
                    .iter()
                    .find(|item| item.local_id == "beta")
                    .unwrap(),
            );
            let before = std::fs::read(&manager.state_path).unwrap();
            assert!(
                manager
                    .apply_repository_matches(&[first.clone(), second.clone()], &remote)
                    .is_err()
            );
            assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
            assert_eq!(
                manager
                    .apply_repository_matches(&[first], &remote)
                    .unwrap()
                    .0,
                1
            );
            let after_first = std::fs::read(&manager.state_path).unwrap();
            let (count, errors) = manager
                .apply_repository_matches(&[second], &remote)
                .unwrap();
            assert_eq!(count, 0);
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("仓库路径已关联其他 Skill"))
            );
            assert_eq!(std::fs::read(&manager.state_path).unwrap(), after_first);

            let other_repo = if root_first { &subdir } else { &root };
            let rematch = manager
                .preview_repository_matches(other_repo, &remote, Some("alpha"))
                .unwrap();
            assert_eq!(
                manager
                    .apply_repository_matches(&[selection(&rematch.matches[0])], &remote)
                    .unwrap()
                    .0,
                1
            );
            assert_eq!(
                manager.load_state().installed["alpha"].repo_key,
                other_repo.key()
            );
        }
    }

    #[test]
    fn imported_repo_without_path_requires_content_evidence_and_respects_deletion() {
        let (temp, manager, _) = fixture();
        let repo = parse_repo_key("other/project@main").unwrap();
        let source = temp.path().join("cc/alpha");
        write_skill(&source, "---\nname: Alpha\n---\nlocal version\n");
        let origin = crate::ccs_import::CcsSkillOrigin {
            repo_key: repo.key(),
            repo_path: String::new(),
        };
        manager
            .import_directory_with_source(
                "alpha",
                "alpha",
                &source,
                "cc-switch",
                Some(&origin),
                None,
                None,
            )
            .unwrap();
        assert!(manager.pending_imported_repos().contains(&repo));
        let wrong = vec![remote(&repo, "alpha", "Alpha", &"a".repeat(64))];
        assert!(
            !manager
                .save_verified_imported_repo(repo.clone(), &wrong)
                .unwrap()
        );
        assert!(!manager.list_repos().contains(&repo));
        let exact = vec![remote(
            &repo,
            "skills/alpha",
            "Alpha",
            &skill_git_tree_hash(&source).unwrap(),
        )];
        assert!(
            manager
                .save_verified_imported_repo(repo.clone(), &exact)
                .unwrap()
        );
        assert!(manager.list_repos().contains(&repo));
        assert!(manager.load_state().installed["alpha"].repo_path.is_empty());
        manager.delete_repo(&repo.key()).unwrap();
        assert!(
            !manager
                .save_verified_imported_repo(repo.clone(), &exact)
                .unwrap()
        );
        assert!(!manager.pending_imported_repos().contains(&repo));
    }
}
