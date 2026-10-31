//! 第三方 Skill 的导入预览、重复匹配和显式覆盖保护。
use super::*;

pub(super) struct SkillTargetSnapshot {
    pub token: String,
    pub directory_hashes: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportMatch {
    pub id: String,
    pub name: String,
    pub reasons: Vec<String>,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportCandidate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub source_hash: String,
    pub repo_key: String,
    pub repo_path: String,
    pub matches: Vec<SkillImportMatch>,
    pub duplicate_candidates: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillImportSelection {
    pub id: String,
    pub source_hash: String,
    pub repo_key: String,
    pub repo_path: String,
    pub target_id: Option<String>,
    pub target_snapshot: Option<String>,
}

impl SkillsManager {
    pub fn preview_skill_import(
        &self,
        candidates: &[(String, PathBuf)],
        origins: &BTreeMap<String, crate::ccs_import::CcsSkillOrigin>,
    ) -> anyhow::Result<Vec<SkillImportCandidate>> {
        self.prepare_managed_skills()?;
        let _operation = self.operation_lock.lock().unwrap();
        let _guard = self.state_lock.lock().unwrap();
        let state = self.load_state_checked_unlocked()?;
        let mut result = Vec::new();
        for (id, path) in candidates {
            let origin = origins.get(id);
            let mut candidate = SkillImportCandidate {
                id: id.clone(),
                name: id.clone(),
                description: String::new(),
                source_hash: String::new(),
                repo_key: origin.map(|item| item.repo_key.clone()).unwrap_or_default(),
                repo_path: origin
                    .map(|item| item.repo_path.clone())
                    .unwrap_or_default(),
                matches: Vec::new(),
                duplicate_candidates: Vec::new(),
                error: None,
            };
            let checked = (|| -> anyhow::Result<()> {
                validate_skill_id(id).context("导入项标识无效")?;
                anyhow::ensure!(
                    path.join(SKILL_MANIFEST_FILE).is_file(),
                    "{id} 没有 SKILL.md"
                );
                candidate.source_hash = skill_directory_hash(path).context("检查导入源失败")?;
                (candidate.name, candidate.description) =
                    read_skill_manifest(&path.join(SKILL_MANIFEST_FILE), id);
                candidate.matches = self
                    .import_matches_unlocked(&state, id, &candidate.name, &candidate.source_hash)
                    .context("检查已有 Skill 冲突失败")?;
                if let Some(origin) =
                    origin.and_then(|item| known_repository_origin(&item.repo_key, &item.repo_path))
                {
                    for (target, installed) in &state.installed {
                        if known_repository_origin(&installed.repo_key, &installed.repo_path)
                            .as_ref()
                            != Some(&origin)
                        {
                            continue;
                        }
                        if let Some(existing) =
                            candidate.matches.iter_mut().find(|item| item.id == *target)
                        {
                            existing.reasons.push("origin".into());
                        } else {
                            candidate.matches.push(SkillImportMatch {
                                id: target.clone(),
                                name: if installed.name.is_empty() {
                                    target.clone()
                                } else {
                                    installed.name.clone()
                                },
                                reasons: vec!["origin".into()],
                                snapshot: self
                                    .import_target_snapshot_unlocked(&state, target)
                                    .with_context(|| format!("无法比较已有 Skill：{target}"))?,
                            });
                        }
                    }
                    candidate
                        .matches
                        .sort_by(|left, right| left.id.cmp(&right.id));
                }
                Ok(())
            })();
            if let Err(error) = checked {
                candidate.error = Some(format!("{error:#}"));
            }
            result.push(candidate);
        }
        let candidate_origins: Vec<_> = result
            .iter()
            .map(|item| known_repository_origin(&item.repo_key, &item.repo_path))
            .collect();
        for index in 0..result.len() {
            if result[index].error.is_some() {
                continue;
            }
            result[index].duplicate_candidates = result
                .iter()
                .enumerate()
                .filter(|(other, candidate)| {
                    *other != index
                        && candidate.error.is_none()
                        && (same_name(&candidate.name, &result[index].name)
                            || candidate.source_hash == result[index].source_hash
                            || (candidate_origins[index].is_some()
                                && candidate_origins[index] == candidate_origins[*other]))
                })
                .map(|(_, candidate)| candidate.id.clone())
                .collect();
        }
        Ok(result)
    }

    pub fn apply_skill_import(
        &self,
        candidates: &[(String, PathBuf)],
        origins: &BTreeMap<String, crate::ccs_import::CcsSkillOrigin>,
        import_source: &str,
        selections: &[SkillImportSelection],
    ) -> anyhow::Result<(usize, Vec<String>)> {
        anyhow::ensure!(
            matches!(import_source, LOCAL_IMPORT_REPO_KEY | "cc-switch"),
            "不支持的导入渠道"
        );
        let _operation = self.operation_lock.lock().unwrap();
        // 外层持有共享操作锁；临时克隆只在本次导入内使用，避免现有方法重复获取同一把锁。
        let mut manager = self.clone();
        manager.operation_lock = Arc::new(Mutex::new(()));
        let preview = manager.preview_skill_import(candidates, origins)?;
        let mut selected_ids = std::collections::BTreeSet::new();
        let mut target_ids = std::collections::BTreeSet::new();
        let mut protected_targets = std::collections::BTreeSet::new();
        for selection in selections {
            anyhow::ensure!(
                selected_ids.insert(selection.id.clone()),
                "同一个导入项不能重复选择"
            );
            let target = selection.target_id.as_deref().unwrap_or(&selection.id);
            anyhow::ensure!(
                target_ids.insert(target.to_lowercase()),
                "多个导入项不能同时写入同一个 Skill"
            );
            if selection.target_id.is_some() {
                protected_targets.insert(target.to_lowercase());
            }
        }
        let mut imported = 0;
        let mut errors = Vec::new();
        for selection in selections {
            let candidate = match preview.iter().find(|item| item.id == selection.id) {
                Some(candidate) => candidate,
                None => {
                    errors.push(format!("{}：导入项已不存在", selection.id));
                    continue;
                }
            };
            if let Some(error) = &candidate.error {
                errors.push(format!("{}：{}", selection.id, error));
                continue;
            }
            if candidate.source_hash != selection.source_hash {
                errors.push(format!("{}：导入源已变化，请重新预览", selection.id));
                continue;
            }
            if candidate.repo_key != selection.repo_key
                || candidate.repo_path != selection.repo_path
            {
                errors.push(format!("{}：仓库来源已变化，请重新预览", selection.id));
                continue;
            }
            if candidate
                .duplicate_candidates
                .iter()
                .any(|id| selected_ids.contains(id))
            {
                errors.push(format!(
                    "{}：所选导入项之间存在同名、相同内容或相同仓库来源，请只选择一个",
                    selection.id
                ));
                continue;
            }
            let (target, snapshot) = match (&selection.target_id, &selection.target_snapshot) {
                (Some(target), Some(snapshot))
                    if candidate
                        .matches
                        .iter()
                        .any(|item| item.id == *target && item.snapshot == *snapshot) =>
                {
                    (target.as_str(), Some(snapshot.as_str()))
                }
                (None, None) if candidate.matches.is_empty() => (candidate.id.as_str(), None),
                _ => {
                    errors.push(format!(
                        "{}：重复项或已有内容已变化，请重新预览并选择",
                        selection.id
                    ));
                    continue;
                }
            };
            let origin_matches: Vec<_> = candidate
                .matches
                .iter()
                .filter(|item| item.reasons.iter().any(|reason| reason == "origin"))
                .collect();
            if !origin_matches.is_empty()
                && (origin_matches.len() != 1 || origin_matches[0].id != target)
            {
                errors.push(format!(
                    "{}：仓库路径已关联其他 Skill，请选择该已有项",
                    selection.id
                ));
                continue;
            }
            let path = &candidates
                .iter()
                .find(|(id, _)| id == &selection.id)
                .expect("preview candidate")
                .1;
            let current = match manager.preview_skill_import(candidates, origins) {
                Ok(current) => current,
                Err(error) => {
                    errors.push(format!("{}：重新检查失败：{error:#}", selection.id));
                    continue;
                }
            };
            let current = current
                .iter()
                .find(|item| item.id == selection.id)
                .expect("unchanged candidate list");
            if let Some(error) = &current.error {
                errors.push(format!("{}：重新检查失败：{}", selection.id, error));
                continue;
            }
            if current.source_hash != selection.source_hash
                || current.repo_key != selection.repo_key
                || current.repo_path != selection.repo_path
            {
                errors.push(format!(
                    "{}：导入源或仓库来源已变化，请重新预览",
                    selection.id
                ));
                continue;
            }
            if current
                .duplicate_candidates
                .iter()
                .any(|id| selected_ids.contains(id))
            {
                errors.push(format!(
                    "{}：所选导入项的重复关系已变化，请重新预览",
                    selection.id
                ));
                continue;
            }
            let current_ids: std::collections::BTreeSet<_> = current
                .matches
                .iter()
                .map(|item| item.id.to_lowercase())
                .collect();
            let shown_ids: std::collections::BTreeSet<_> = candidate
                .matches
                .iter()
                .map(|item| item.id.to_lowercase())
                .collect();
            if current_ids != shown_ids
                || current_ids
                    .iter()
                    .any(|id| target_ids.contains(id) && !protected_targets.contains(id))
            {
                errors.push(format!("{}：重复项已变化，请重新预览", selection.id));
                continue;
            }
            match manager.import_directory_with_source(
                &candidate.id,
                target,
                path,
                import_source,
                origins.get(&candidate.id),
                Some(&selection.source_hash),
                snapshot,
            ) {
                Ok(_) => imported += 1,
                Err(error) => errors.push(format!("{}：{error:#}", selection.id)),
            }
        }
        Ok((imported, errors))
    }

    pub(super) fn import_target_snapshot_unlocked(
        &self,
        state: &SkillsState,
        id: &str,
    ) -> anyhow::Result<String> {
        Ok(self
            .import_target_snapshot_details_unlocked(state, id)?
            .token)
    }

    pub(super) fn import_target_snapshot_details_unlocked(
        &self,
        state: &SkillsState,
        id: &str,
    ) -> anyhow::Result<SkillTargetSnapshot> {
        validate_skill_id(id)?;
        let mut hash = Sha256::new();
        hash.update(serde_json::to_vec(&state.installed.get(id))?);
        let mut roots = vec![self.source_dir.clone(), self.linked_dir()];
        roots.extend(self.agent_dirs.iter().map(|(_, root)| root.clone()));
        let mut directory_hashes = Vec::with_capacity(roots.len());
        for root in roots {
            let path = root.join(id);
            match std::fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    hash.update(b"present");
                    hash.update(if metadata.file_type().is_symlink() {
                        b"link"
                    } else {
                        b"copy"
                    });
                    hash.update(if is_managed_copy(&path) {
                        &b"managed"[..]
                    } else {
                        &b"external"[..]
                    });
                    let directory_hash = skill_directory_hash(&path)?;
                    hash.update(directory_hash.as_bytes());
                    directory_hashes.push(Some(directory_hash));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    hash.update(b"absent");
                    directory_hashes.push(None);
                }
                Err(error) => return Err(error.into()),
            }
            hash.update([0]);
        }
        Ok(SkillTargetSnapshot {
            token: format!("{:x}", hash.finalize()),
            directory_hashes,
        })
    }

    pub(super) fn import_matches_unlocked(
        &self,
        state: &SkillsState,
        id: &str,
        name: &str,
        source_hash: &str,
    ) -> anyhow::Result<Vec<SkillImportMatch>> {
        let mut existing: BTreeMap<String, String> = state
            .installed
            .iter()
            .map(|(id, item)| {
                (
                    id.clone(),
                    if item.name.is_empty() {
                        id.clone()
                    } else {
                        item.name.clone()
                    },
                )
            })
            .collect();
        for skill in self.list_bundled_skills() {
            if same_name(id, &skill.id) || same_name(name, &skill.name) {
                anyhow::bail!("导入项与 Codex 内置 Skill 重名：{}", skill.id);
            }
        }
        for skill in self.list_local_skills() {
            existing.entry(skill.id).or_insert(skill.name);
        }
        for (_, root) in &self.agent_dirs {
            for (id, (name, _)) in scan_skill_dirs(root) {
                existing.entry(id).or_insert(name);
            }
        }
        let mut matches = Vec::new();
        for (target, existing_name) in existing {
            let mut reasons = Vec::new();
            if same_name(id, &target) {
                reasons.push("id".into());
            }
            if same_name(name, &existing_name) {
                reasons.push("name".into());
            }
            let snapshot = self
                .import_target_snapshot_details_unlocked(state, &target)
                .with_context(|| format!("无法比较已有 Skill：{target}"))?;
            if snapshot
                .directory_hashes
                .iter()
                .flatten()
                .any(|hash| hash == source_hash)
            {
                reasons.push("content".into());
            }
            if !reasons.is_empty() {
                matches.push(SkillImportMatch {
                    snapshot: snapshot.token,
                    id: target,
                    name: existing_name,
                    reasons,
                });
            }
        }
        Ok(matches)
    }
}

fn known_repository_origin(repo_key: &str, repo_path: &str) -> Option<(String, String)> {
    if repo_path.is_empty() {
        return None;
    }
    normalize_repo(parse_repo_key(repo_key)?).ok()?;
    Some(super::source_match::repository_match_origin(
        repo_key, repo_path,
    ))
}

fn same_name(left: &str, right: &str) -> bool {
    !left.trim().is_empty() && left.trim().to_lowercase() == right.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, SkillsManager) {
        let temp = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("home"),
        );
        (temp, manager)
    }

    fn write_skill(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\n---\n{body}"),
        )
        .unwrap();
    }

    fn selection(candidate: &SkillImportCandidate) -> SkillImportSelection {
        SkillImportSelection {
            id: candidate.id.clone(),
            source_hash: candidate.source_hash.clone(),
            repo_key: candidate.repo_key.clone(),
            repo_path: candidate.repo_path.clone(),
            target_id: None,
            target_snapshot: None,
        }
    }

    fn add_oversized_file(dir: &Path) {
        std::fs::File::create(dir.join("oversized.bin"))
            .unwrap()
            .set_len(256 * 1024 * 1024 + 1)
            .unwrap();
    }

    #[test]
    fn invalid_source_isolated_from_other_candidates_for_both_import_channels() {
        for import_source in ["local", "cc-switch"] {
            let (temp, manager) = fixture();
            let bundle = temp.path().join("bundle");
            let bad = bundle.join("bad");
            let good = bundle.join("good");
            write_skill(&bad, "Bad", "unsafe source");
            write_skill(&good, "Good", "safe source");
            add_oversized_file(&bad);
            let candidates = local_skill_import_candidates(&bundle).unwrap();
            let mut origins = BTreeMap::new();
            if import_source == "cc-switch" {
                let origin = crate::ccs_import::CcsSkillOrigin {
                    repo_key: "owner/repo@main".into(),
                    repo_path: "skills/shared".into(),
                };
                origins.insert("bad".into(), origin.clone());
                origins.insert("good".into(), origin);
            }
            let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
            assert!(preview[0].error.as_ref().unwrap().contains("256 MiB"));
            assert!(preview[1].error.is_none());
            assert!(preview[1].duplicate_candidates.is_empty());
            let payload = serde_json::to_value(&preview).unwrap();
            assert!(payload[0]["error"].as_str().unwrap().contains("256 MiB"));
            assert!(payload[1]["error"].is_null());
            let selections: Vec<_> = preview.iter().map(selection).collect();
            let (count, errors) = manager
                .apply_skill_import(&candidates, &origins, import_source, &selections)
                .unwrap();
            assert_eq!(count, 1);
            assert_eq!(errors.len(), 1);
            assert!(errors[0].contains("bad") && errors[0].contains("256 MiB"));
            assert!(!manager.source_dir.join("bad").exists());
            assert!(manager.source_dir.join("good/SKILL.md").is_file());
            assert!(bad.join("oversized.bin").exists());
            assert_eq!(
                manager.load_state().installed["good"].import_source,
                import_source
            );
        }
    }

    #[test]
    fn source_becoming_unsafe_after_preview_does_not_block_other_selection() {
        let (temp, manager) = fixture();
        let bundle = temp.path().join("bundle");
        let bad = bundle.join("bad");
        let good = bundle.join("good");
        write_skill(&bad, "Bad", "original");
        write_skill(&good, "Good", "safe");
        let candidates = local_skill_import_candidates(&bundle).unwrap();
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let selections: Vec<_> = preview.iter().map(selection).collect();
        add_oversized_file(&bad);
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &selections)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad") && errors[0].contains("256 MiB"));
        assert!(!manager.source_dir.join("bad").exists());
        assert!(manager.source_dir.join("good/SKILL.md").is_file());
    }

    #[test]
    fn failed_target_copy_comparison_never_becomes_a_new_import() {
        let temp = tempfile::tempdir().unwrap();
        let agent_root = temp.path().join("agent");
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("home"),
        )
        .with_agent_dirs(vec![("claude".into(), agent_root.clone())]);
        let original = temp.path().join("original");
        let incoming = temp.path().join("incoming");
        write_skill(&original, "Original", "original");
        write_skill(&incoming, "Unrelated", "incoming");
        manager
            .import_from_local_directory("existing", &original)
            .unwrap();
        manager.set_enabled_for("claude", "existing", true).unwrap();
        let candidates = vec![("new".into(), incoming)];
        let before = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert!(before[0].error.is_none());
        assert!(before[0].matches.is_empty());
        add_oversized_file(&agent_root.join("existing"));
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let error = preview[0].error.as_ref().unwrap();
        assert!(
            error.contains("已有 Skill") && error.contains("existing") && error.contains("256 MiB")
        );
        for candidate in [&before[0], &preview[0]] {
            let (count, errors) = manager
                .apply_skill_import(
                    &candidates,
                    &BTreeMap::new(),
                    "local",
                    &[selection(candidate)],
                )
                .unwrap();
            assert_eq!(count, 0);
            assert_eq!(errors.len(), 1);
            assert!(errors[0].contains("existing") && errors[0].contains("256 MiB"));
        }
        assert!(!manager.source_dir.join("new").exists());
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("original")
        );
    }

    #[test]
    fn mixed_bad_source_and_target_allow_good_import_after_target_repair() {
        let temp = tempfile::tempdir().unwrap();
        let agent_root = temp.path().join("agent");
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("home"),
        )
        .with_agent_dirs(vec![("claude".into(), agent_root.clone())]);
        let original = temp.path().join("original");
        let bundle = temp.path().join("bundle");
        let bad = bundle.join("bad");
        let good = bundle.join("good");
        write_skill(&original, "Existing", "original");
        write_skill(&bad, "Bad", "unsafe");
        write_skill(&good, "Good", "safe");
        manager
            .import_from_local_directory("existing", &original)
            .unwrap();
        manager.set_enabled_for("claude", "existing", true).unwrap();
        add_oversized_file(&bad);
        let agent_copy = agent_root.join("existing");
        add_oversized_file(&agent_copy);
        let candidates = local_skill_import_candidates(&bundle).unwrap();
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let source_error = preview[0].error.as_ref().unwrap();
        let target_error = preview[1].error.as_ref().unwrap();
        assert!(source_error.contains("导入源") && source_error.contains("256 MiB"));
        assert!(target_error.contains("已有 Skill") && target_error.contains("existing"));
        let selections: Vec<_> = preview.iter().map(selection).collect();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &selections)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 2);
        assert!(!manager.source_dir.join("bad").exists());
        assert!(!manager.source_dir.join("good").exists());

        std::fs::OpenOptions::new()
            .write(true)
            .open(agent_copy.join("oversized.bin"))
            .unwrap()
            .set_len(0)
            .unwrap();
        let repaired = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert!(repaired[0].error.as_ref().unwrap().contains("导入源"));
        assert!(repaired[1].error.is_none());
        assert!(repaired[1].matches.is_empty());
        let selections: Vec<_> = repaired.iter().map(selection).collect();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &selections)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad") && errors[0].contains("256 MiB"));
        assert!(!manager.source_dir.join("bad").exists());
        assert!(manager.source_dir.join("good/SKILL.md").is_file());
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("original")
        );
    }

    #[test]
    fn duplicate_content_checks_independent_agent_copies() {
        let temp = tempfile::tempdir().unwrap();
        let agent_root = temp.path().join("agent");
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("home"),
        )
        .with_agent_dirs(vec![("claude".into(), agent_root.clone())]);
        let original = temp.path().join("original");
        let incoming = temp.path().join("incoming");
        write_skill(&original, "Original", "original");
        write_skill(&incoming, "Different", "agent content");
        manager
            .import_from_local_directory("existing", &original)
            .unwrap();
        manager.set_enabled_for("claude", "existing", true).unwrap();
        write_skill(&agent_root.join("existing"), "Different", "agent content");
        let preview = manager
            .preview_skill_import(&[("new".into(), incoming)], &BTreeMap::new())
            .unwrap();
        assert!(preview[0].error.is_none());
        assert_eq!(preview[0].matches.len(), 1);
        assert_eq!(preview[0].matches[0].id, "existing");
        assert!(
            preview[0].matches[0]
                .reasons
                .iter()
                .any(|reason| reason == "content")
        );
    }

    #[test]
    #[cfg(any(unix, windows))]
    fn internal_symlink_is_reported_per_candidate() {
        let (temp, manager) = fixture();
        let bad = temp.path().join("bad");
        let good = temp.path().join("good");
        write_skill(&bad, "Bad", "source");
        write_skill(&good, "Good", "source");
        #[cfg(unix)]
        let link = std::os::unix::fs::symlink(bad.join("SKILL.md"), bad.join("link"));
        #[cfg(windows)]
        let link = std::os::windows::fs::symlink_file(bad.join("SKILL.md"), bad.join("link"));
        if let Err(error) = link {
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.raw_os_error() == Some(1314)
            {
                eprintln!("符号链接测试跳过：{error}");
                return;
            }
            panic!("创建符号链接失败：{error}");
        }
        let candidates = vec![("bad".into(), bad), ("good".into(), good)];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert!(preview[0].error.as_ref().unwrap().contains("符号链接"));
        assert!(preview[1].error.is_none());
        let (count, errors) = manager
            .apply_skill_import(
                &candidates,
                &BTreeMap::new(),
                "local",
                &[selection(&preview[1])],
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn internal_junction_is_reported_per_candidate() {
        let (temp, manager) = fixture();
        let bad = temp.path().join("bad");
        let good = temp.path().join("good");
        let external = temp.path().join("external");
        write_skill(&bad, "Bad", "source");
        write_skill(&good, "Good", "source");
        write_skill(&external, "External", "untouched");
        super::super::tests::make_junction(&bad.join("link"), &external);
        let candidates = vec![("bad".into(), bad), ("good".into(), good)];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert!(preview[0].error.as_ref().unwrap().contains("符号链接"));
        assert!(preview[1].error.is_none());
        let selections: Vec<_> = preview.iter().map(selection).collect();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &selections)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad") && errors[0].contains("符号链接"));
        assert!(!manager.source_dir.join("bad").exists());
        assert!(manager.source_dir.join("good/SKILL.md").is_file());
        assert!(
            std::fs::read_to_string(external.join("SKILL.md"))
                .unwrap()
                .contains("untouched")
        );
    }

    #[test]
    #[cfg(unix)]
    fn special_file_is_reported_per_candidate() {
        let (temp, manager) = fixture();
        let bad = temp.path().join("bad");
        let good = temp.path().join("good");
        write_skill(&bad, "Bad", "source");
        write_skill(&good, "Good", "source");
        let _socket = std::os::unix::net::UnixListener::bind(bad.join("socket")).unwrap();
        let candidates = vec![("bad".into(), bad), ("good".into(), good)];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert!(
            preview[0]
                .error
                .as_ref()
                .unwrap()
                .contains("不支持的文件类型")
        );
        assert!(preview[1].error.is_none());
    }

    #[test]
    fn duplicate_requires_explicit_target_and_preserves_enabled_state() {
        let (temp, manager) = fixture();
        let original = temp.path().join("old");
        let incoming = temp.path().join("new");
        write_skill(&original, "Shared", "old");
        write_skill(&incoming, "Shared", "new");
        manager
            .import_from_local_directory("existing", &original)
            .unwrap();
        manager.set_enabled("existing", true).unwrap();
        assert!(
            manager
                .import_from_local_directory("new", &incoming)
                .is_err()
        );
        let candidates = vec![("new".into(), incoming.clone())];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert_eq!(preview[0].matches.len(), 1);
        assert_eq!(preview[0].matches[0].id, "existing");
        let without_choice = SkillImportSelection {
            id: "new".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: String::new(),
            repo_path: String::new(),
            target_id: None,
            target_snapshot: None,
        };
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[without_choice])
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 1);
        let selection = SkillImportSelection {
            id: "new".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: String::new(),
            repo_path: String::new(),
            target_id: Some("existing".into()),
            target_snapshot: Some(preview[0].matches[0].snapshot.clone()),
        };
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[selection])
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("new")
        );
        assert!(
            std::fs::read_to_string(manager.linked_dir().join("existing/SKILL.md"))
                .unwrap()
                .contains("new")
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|text| text.contains("old"))
        }));
    }

    #[test]
    fn explicit_import_overwrite_updates_existing_agent_copy_and_keeps_backup() {
        let temp = tempfile::tempdir().unwrap();
        let agent_root = temp.path().join("claude");
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("home"),
        )
        .with_agent_dirs(vec![("claude".into(), agent_root.clone())]);
        let existing = agent_root.join("existing");
        let incoming = temp.path().join("incoming");
        write_skill(&existing, "Shared", "original Claude version");
        write_skill(&incoming, "Shared", "imported replacement");
        let candidates = vec![("new".into(), incoming.clone())];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let target = preview[0]
            .matches
            .iter()
            .find(|item| item.id == "existing")
            .unwrap();
        let selection = SkillImportSelection {
            id: "new".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: String::new(),
            repo_path: String::new(),
            target_id: Some(target.id.clone()),
            target_snapshot: Some(target.snapshot.clone()),
        };
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[selection])
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
        assert!(
            std::fs::read_to_string(existing.join("SKILL.md"))
                .unwrap()
                .contains("imported replacement")
        );
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("imported replacement")
        );
        assert!(
            std::fs::read_to_string(incoming.join("SKILL.md"))
                .unwrap()
                .contains("imported replacement")
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|text| text.contains("original Claude version"))
        }));
    }

    #[test]
    fn stale_source_or_target_requires_a_new_preview() {
        let (temp, manager) = fixture();
        let old = temp.path().join("old");
        let incoming = temp.path().join("incoming");
        write_skill(&old, "Shared", "old");
        write_skill(&incoming, "Shared", "incoming");
        manager
            .import_from_local_directory("existing", &old)
            .unwrap();
        let candidates = vec![("new".into(), incoming.clone())];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let selection = || SkillImportSelection {
            id: "new".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: String::new(),
            repo_path: String::new(),
            target_id: Some("existing".into()),
            target_snapshot: Some(preview[0].matches[0].snapshot.clone()),
        };
        write_skill(&incoming, "Shared", "changed source");
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[selection()])
            .unwrap();
        assert_eq!(count, 0);
        assert!(!errors.is_empty());
        write_skill(&incoming, "Shared", "incoming");
        write_skill(
            &manager.source_dir.join("existing"),
            "Shared",
            "changed target",
        );
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[selection()])
            .unwrap();
        assert_eq!(count, 0);
        assert!(!errors.is_empty());
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("changed target")
        );
    }

    #[test]
    fn duplicate_content_and_duplicate_incoming_items_are_not_silently_imported() {
        let (temp, manager) = fixture();
        let old = temp.path().join("old");
        let one = temp.path().join("one");
        let two = temp.path().join("two");
        write_skill(&old, "same", "old");
        write_skill(&one, "other", "new");
        write_skill(&two, "other", "new");
        manager
            .import_from_local_directory("existing", &old)
            .unwrap();
        let candidates = vec![("one".into(), one), ("two".into(), two)];
        let preview = manager
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        assert_eq!(preview[0].duplicate_candidates, vec!["two"]);
        let selections: Vec<_> = preview
            .iter()
            .map(|item| SkillImportSelection {
                id: item.id.clone(),
                source_hash: item.source_hash.clone(),
                repo_key: String::new(),
                repo_path: String::new(),
                target_id: None,
                target_snapshot: None,
            })
            .collect();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &selections)
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(errors.len(), 2);
        let equal = temp.path().join("equal");
        write_skill(&equal, "same", "old");
        let preview = manager
            .preview_skill_import(&[("different".into(), equal)], &BTreeMap::new())
            .unwrap();
        assert!(
            preview[0].matches[0]
                .reasons
                .iter()
                .any(|reason| reason == "content")
        );
    }

    #[test]
    fn repository_origin_and_import_channel_remain_separate_and_hash_matches_remote() {
        let (temp, manager) = fixture();
        let incoming = temp.path().join("incoming");
        write_skill(&incoming, "alpha", "content");
        let repo = parse_repo_key("owner/repo@main").unwrap();
        let mut origins = BTreeMap::new();
        origins.insert(
            "alpha".into(),
            crate::ccs_import::CcsSkillOrigin {
                repo_key: repo.key(),
                repo_path: "skills/alpha".into(),
            },
        );
        let candidates = vec![("alpha".into(), incoming.clone())];
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        let selection = SkillImportSelection {
            id: "alpha".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: preview[0].repo_key.clone(),
            repo_path: preview[0].repo_path.clone(),
            target_id: None,
            target_snapshot: None,
        };
        let (count, errors) = manager
            .apply_skill_import(&candidates, &origins, "cc-switch", &[selection])
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
        let installed = &manager.load_state().installed["alpha"];
        assert_eq!(installed.repo_key, repo.key());
        assert_eq!(installed.repo_path, "skills/alpha");
        assert_eq!(installed.import_source, "cc-switch");
        let content = std::fs::read(incoming.join("SKILL.md")).unwrap();
        let mut blob = sha1::Sha1::new();
        blob.update(format!("blob {}\0", content.len()).as_bytes());
        blob.update(content);
        let tree = serde_json::json!({"tree":[{"type":"blob","path":"skills/alpha/SKILL.md","sha":format!("{:x}", blob.finalize())}]});
        assert_eq!(
            installed.content_hash,
            parse_skills_from_tree(&repo, &tree)[0].content_hash
        );
    }

    #[test]
    fn cc_switch_same_origin_requires_the_existing_target_and_preserves_updates() {
        let (temp, manager) = fixture();
        let original = temp.path().join("original");
        let incoming = temp.path().join("incoming");
        let unrelated = temp.path().join("unrelated");
        write_skill(&original, "Old name", "old content");
        write_skill(&incoming, "New name", "new content");
        write_skill(&unrelated, "New name", "unrelated content");
        let old_origin = crate::ccs_import::CcsSkillOrigin {
            repo_key: "Owner/Repo@main:skills".into(),
            repo_path: "skills/alpha".into(),
        };
        manager
            .import_directory_with_source(
                "old",
                "existing",
                &original,
                "cc-switch",
                Some(&old_origin),
                None,
                None,
            )
            .unwrap();
        manager
            .import_from_local_directory("unrelated", &unrelated)
            .unwrap();
        let candidates = vec![("new".into(), incoming.clone())];
        let origin = crate::ccs_import::CcsSkillOrigin {
            repo_key: "owner/repo@main".into(),
            repo_path: "skills/alpha".into(),
        };
        let origins = BTreeMap::from([("new".into(), origin.clone())]);
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        let existing = preview[0]
            .matches
            .iter()
            .find(|item| item.id == "existing")
            .unwrap();
        assert!(existing.reasons.iter().any(|reason| reason == "origin"));
        let unrelated = preview[0]
            .matches
            .iter()
            .find(|item| item.id == "unrelated")
            .unwrap();
        let mut chosen = SkillImportSelection {
            id: "new".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: origin.repo_key.clone(),
            repo_path: origin.repo_path.clone(),
            target_id: None,
            target_snapshot: None,
        };
        let before = std::fs::read(&manager.state_path).unwrap();
        assert_eq!(
            manager
                .apply_skill_import(&candidates, &origins, "cc-switch", &[chosen.clone()])
                .unwrap()
                .0,
            0
        );
        chosen.target_id = Some(unrelated.id.clone());
        chosen.target_snapshot = Some(unrelated.snapshot.clone());
        assert_eq!(
            manager
                .apply_skill_import(&candidates, &origins, "cc-switch", &[chosen.clone()])
                .unwrap()
                .0,
            0
        );
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        chosen.target_id = Some(existing.id.clone());
        chosen.target_snapshot = Some(existing.snapshot.clone());
        let (count, errors) = manager
            .apply_skill_import(&candidates, &origins, "cc-switch", &[chosen])
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
        let state = manager.load_state();
        assert!(!state.installed.contains_key("new"));
        assert_eq!(state.installed["existing"].repo_key, origin.repo_key);
        assert!(
            std::fs::read_to_string(manager.source_dir.join("existing/SKILL.md"))
                .unwrap()
                .contains("new content")
        );
        let remote = RemoteSkill {
            id: "remote".into(),
            name: "Remote".into(),
            description: String::new(),
            repo_key: origin.repo_key,
            repo_path: origin.repo_path,
            content_hash: "updated upstream".into(),
        };
        assert!(
            manager
                .merge_entries(&[remote])
                .iter()
                .any(|item| item.id == "existing" && item.update_available)
        );
    }

    #[test]
    fn cc_switch_batch_rejects_same_origin_but_not_other_branches_or_paths() {
        let (temp, manager) = fixture();
        let candidates: Vec<_> = ["one", "two", "three", "four"]
            .iter()
            .map(|id| {
                let path = temp.path().join(id);
                write_skill(&path, id, id);
                (id.to_string(), path)
            })
            .collect();
        let origins = BTreeMap::from([
            (
                "one".into(),
                crate::ccs_import::CcsSkillOrigin {
                    repo_key: "Owner/Repo@main".into(),
                    repo_path: "skills/alpha".into(),
                },
            ),
            (
                "two".into(),
                crate::ccs_import::CcsSkillOrigin {
                    repo_key: "owner/repo@main:skills".into(),
                    repo_path: "skills/alpha".into(),
                },
            ),
            (
                "three".into(),
                crate::ccs_import::CcsSkillOrigin {
                    repo_key: "owner/repo@Main:skills".into(),
                    repo_path: "skills/alpha".into(),
                },
            ),
            (
                "four".into(),
                crate::ccs_import::CcsSkillOrigin {
                    repo_key: "owner/repo@main:skills".into(),
                    repo_path: "skills/Alpha".into(),
                },
            ),
        ]);
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        assert_eq!(preview[0].duplicate_candidates, vec!["two"]);
        assert_eq!(preview[1].duplicate_candidates, vec!["one"]);
        assert!(preview[2].duplicate_candidates.is_empty());
        assert!(preview[3].duplicate_candidates.is_empty());
        let selections: Vec<_> = preview
            .iter()
            .map(|item| SkillImportSelection {
                id: item.id.clone(),
                source_hash: item.source_hash.clone(),
                repo_key: item.repo_key.clone(),
                repo_path: item.repo_path.clone(),
                target_id: None,
                target_snapshot: None,
            })
            .collect();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &origins, "cc-switch", &selections)
            .unwrap();
        assert_eq!(count, 2);
        assert_eq!(errors.len(), 2);
        let state = manager.load_state();
        assert!(!state.installed.contains_key("one"));
        assert!(!state.installed.contains_key("two"));
        assert!(state.installed.contains_key("three"));
        assert!(state.installed.contains_key("four"));
    }

    #[test]
    fn concurrent_cc_switch_imports_cannot_create_duplicate_origins() {
        let (temp, manager) = fixture();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut threads = Vec::new();
        for id in ["one", "two"] {
            let path = temp.path().join(id);
            write_skill(&path, id, id);
            let candidates = vec![(id.to_string(), path)];
            let origins = BTreeMap::from([(
                id.to_string(),
                crate::ccs_import::CcsSkillOrigin {
                    repo_key: "owner/repo@main".into(),
                    repo_path: "skills/alpha".into(),
                },
            )]);
            let item = manager
                .preview_skill_import(&candidates, &origins)
                .unwrap()
                .remove(0);
            let chosen = SkillImportSelection {
                id: item.id,
                source_hash: item.source_hash,
                repo_key: item.repo_key,
                repo_path: item.repo_path,
                target_id: None,
                target_snapshot: None,
            };
            let manager = manager.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                manager
                    .apply_skill_import(&candidates, &origins, "cc-switch", &[chosen])
                    .unwrap()
            }));
        }
        let results: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(results.iter().map(|(count, _)| count).sum::<usize>(), 1);
        assert_eq!(manager.load_state().installed.len(), 1);
    }

    #[test]
    fn concurrent_cc_switch_import_and_source_match_cannot_claim_the_same_origin() {
        let (temp, manager) = fixture();
        let incoming = temp.path().join("incoming");
        let local = manager.linked_dir().join("local");
        write_skill(&incoming, "Incoming", "new content");
        write_skill(&local, "Local", "existing content");
        let repo = parse_repo_key("owner/repo@main").unwrap();
        manager.upsert_repo(repo.clone()).unwrap();
        let remote = vec![RemoteSkill {
            id: "remote".into(),
            name: "Local".into(),
            description: String::new(),
            repo_key: repo.key(),
            repo_path: "skills/alpha".into(),
            content_hash: skill_git_tree_hash(&local).unwrap(),
        }];
        let preview = manager
            .preview_repository_matches(&repo, &remote, Some("local"))
            .unwrap();
        let matched = &preview.matches[0];
        let match_selection = SkillRepositoryMatchSelection {
            local_id: matched.local_id.clone(),
            local_hash: matched.local_hash.clone(),
            snapshot: matched.snapshot.clone(),
            repo_key: matched.repo_key.clone(),
            repo_path: matched.repo_path.clone(),
            remote_hash: matched.remote_hash.clone(),
            manual: false,
        };
        let candidates = vec![("incoming".into(), incoming)];
        let origins = BTreeMap::from([(
            "incoming".into(),
            crate::ccs_import::CcsSkillOrigin {
                repo_key: repo.key(),
                repo_path: "skills/alpha".into(),
            },
        )]);
        let candidate = manager
            .preview_skill_import(&candidates, &origins)
            .unwrap()
            .remove(0);
        let import_selection = SkillImportSelection {
            id: candidate.id,
            source_hash: candidate.source_hash,
            repo_key: candidate.repo_key,
            repo_path: candidate.repo_path,
            target_id: None,
            target_snapshot: None,
        };
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let import_manager = manager.clone();
        let import_barrier = barrier.clone();
        let import = std::thread::spawn(move || {
            import_barrier.wait();
            import_manager
                .apply_skill_import(&candidates, &origins, "cc-switch", &[import_selection])
                .unwrap()
                .0
        });
        let match_manager = manager.clone();
        let matching = std::thread::spawn(move || {
            barrier.wait();
            match_manager
                .apply_repository_matches(&[match_selection], &remote)
                .unwrap()
                .0
        });
        assert_eq!(import.join().unwrap() + matching.join().unwrap(), 1);
        assert_eq!(
            manager
                .load_state()
                .installed
                .values()
                .filter(|item| item.repo_path == "skills/alpha")
                .count(),
            1
        );
    }

    #[test]
    fn imported_repository_requires_verification_and_respects_explicit_removal() {
        let (temp, manager) = fixture();
        let incoming = temp.path().join("incoming");
        write_skill(&incoming, "alpha", "content");
        let repo = parse_repo_key("owner/repo@main").unwrap();
        let mut origins = BTreeMap::new();
        origins.insert(
            "alpha".into(),
            crate::ccs_import::CcsSkillOrigin {
                repo_key: repo.key(),
                repo_path: "skills/alpha".into(),
            },
        );
        let candidates = vec![("alpha".into(), incoming)];
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        let selection = SkillImportSelection {
            id: "alpha".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: repo.key(),
            repo_path: "skills/alpha".into(),
            target_id: None,
            target_snapshot: None,
        };
        let (count, errors) = manager
            .apply_skill_import(&candidates, &origins, "cc-switch", &[selection])
            .unwrap();
        assert_eq!(count, 1);
        assert!(errors.is_empty());
        assert!(
            !manager
                .list_repos()
                .iter()
                .any(|item| item.key() == repo.key())
        );
        assert_eq!(manager.pending_imported_repos(), vec![repo.clone()]);
        let remote = vec![RemoteSkill {
            id: "alpha".into(),
            name: "alpha".into(),
            description: String::new(),
            repo_key: repo.key(),
            repo_path: "skills/alpha".into(),
            content_hash: "verified".into(),
        }];
        assert!(
            manager
                .save_verified_imported_repo(repo.clone(), &remote)
                .unwrap()
        );
        assert!(manager.pending_imported_repos().is_empty());
        manager.delete_repo(&repo.key()).unwrap();
        assert!(manager.pending_imported_repos().is_empty());
        assert!(
            !manager
                .save_verified_imported_repo(repo.clone(), &remote)
                .unwrap()
        );
        manager.save_repo(repo.clone(), None).unwrap();
        let mut disabled = repo.clone();
        disabled.enabled = false;
        assert!(
            manager
                .update_repo_enabled_if_unchanged(&disabled, None)
                .unwrap()
        );
        assert!(
            !manager
                .save_verified_imported_repo(repo.clone(), &remote)
                .unwrap()
        );
        assert!(
            !manager
                .list_repos()
                .iter()
                .find(|item| item.key() == repo.key())
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn changed_repository_metadata_is_not_silently_accepted() {
        let (temp, manager) = fixture();
        let incoming = temp.path().join("incoming");
        write_skill(&incoming, "alpha", "content");
        let candidates = vec![("alpha".into(), incoming)];
        let mut origins = BTreeMap::new();
        origins.insert(
            "alpha".into(),
            crate::ccs_import::CcsSkillOrigin {
                repo_key: "owner/old@main".into(),
                repo_path: "alpha".into(),
            },
        );
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        let selection = SkillImportSelection {
            id: "alpha".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: preview[0].repo_key.clone(),
            repo_path: preview[0].repo_path.clone(),
            target_id: None,
            target_snapshot: None,
        };
        origins.get_mut("alpha").unwrap().repo_key = "owner/new@main".into();
        let (count, errors) = manager
            .apply_skill_import(&candidates, &origins, "cc-switch", &[selection])
            .unwrap();
        assert_eq!(count, 0);
        assert!(!errors.is_empty());
        assert!(!manager.source_dir.join("alpha").exists());
    }

    #[test]
    fn bundled_name_cannot_be_overwritten_or_imported_as_a_duplicate() {
        let (temp, manager) = fixture();
        let builtin = manager.linked_dir().join(".system/builtin");
        let incoming = temp.path().join("incoming");
        write_skill(&builtin, "Built-in", "original");
        write_skill(&incoming, "Built-in", "replacement");
        let preview = manager
            .preview_skill_import(&[("new".into(), incoming.clone())], &BTreeMap::new())
            .unwrap();
        assert!(preview[0].error.as_ref().unwrap().contains("内置 Skill"));
        assert!(
            manager
                .import_from_local_directory("builtin", &incoming)
                .is_err()
        );
        assert!(
            std::fs::read_to_string(builtin.join("SKILL.md"))
                .unwrap()
                .contains("original")
        );
    }
}
