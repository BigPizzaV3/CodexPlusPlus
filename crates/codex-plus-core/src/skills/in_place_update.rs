use super::*;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInPlaceUpdatePreview {
    pub preview_id: String,
    pub id: String,
    pub repo_key: String,
    pub repo_path: String,
    pub target_path: PathBuf,
    pub current_hash: String,
    pub next_hash: String,
    pub conflicts: Vec<String>,
    pub managed_paths: Vec<PathBuf>,
    pub followed_paths: Vec<PathBuf>,
    pub skipped_paths: Vec<PathBuf>,
}

#[derive(Clone)]
struct UpdatePath {
    path: PathBuf,
    identity: SkillPathIdentity,
    digest: [u8; 32],
    marked: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct FollowingPath {
    path: PathBuf,
    identity: SkillPathIdentity,
    target: PathBuf,
}

struct InPlaceUpdate {
    scope: CoordinationScope,
    generation: u64,
    repo_generation: u64,
    installed: InstalledSkill,
    repo: SkillRepo,
    remote: RemoteSkill,
    paths: Vec<UpdatePath>,
    zip: Vec<u8>,
    conflicts: Vec<String>,
    instance_target: Option<SkillInstanceTarget>,
    instance_record: Option<SkillInstanceRecord>,
    followed_paths: Vec<FollowingPath>,
    skipped_paths: Vec<PathBuf>,
}

fn classify_following_paths(
    paths: &[UpdatePath],
    candidates: Vec<FollowingPath>,
    skipped: &mut Vec<PathBuf>,
) -> anyhow::Result<Vec<FollowingPath>> {
    let targets = paths.iter().map(|path| path.path.canonicalize())
        .collect::<std::io::Result<BTreeSet<_>>>()?;
    let mut followed = Vec::new();
    for candidate in candidates {
        if targets.contains(&candidate.target) {
            followed.push(candidate);
        } else {
            skipped.push(candidate.path);
        }
    }
    Ok(followed)
}

fn update_previews() -> MutexGuard<'static, BTreeMap<String, InPlaceUpdate>> {
    static PREVIEWS: OnceLock<Mutex<BTreeMap<String, InPlaceUpdate>>> = OnceLock::new();
    PREVIEWS.get_or_init(Mutex::default).lock()
        .expect("Skill 更新预览锁中毒，拒绝继续修改")
}

#[cfg(test)]
thread_local! {
    static BEFORE_UPDATE_BACKUP: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

impl SkillsManager {
    fn in_place_update_paths(
        &self,
        id: &str,
        original: &Path,
        instance: Option<&SkillInstanceTarget>,
        record: Option<&SkillInstanceRecord>,
    ) -> anyhow::Result<(Vec<UpdatePath>, Vec<String>, Vec<FollowingPath>, Vec<PathBuf>)> {
        let target = original.canonicalize().context("无法解析 Skill 原目录")?;
        let mut roots = vec![self.linked_dir()];
        roots.extend(self.shared_skills_dir().map(Path::to_path_buf));
        roots.extend(self.agent_dirs.iter().map(|(_, path)| path.clone()));
        if let Some(instance) = instance {
            anyhow::ensure!(instance.path == original
                && roots.iter().any(|root| root.join(id).canonicalize().is_ok_and(|path| path == target)),
                "关联的原目录不属于当前发现范围，请重新关联");
            anyhow::ensure!(std::fs::symlink_metadata(original)?.file_type().is_dir()
                && skill_path_identity(original)? == instance.identity,
                "关联的原目录已被替换，请重新关联");
            let digest = skill_directory_digest(original)?.context("Skill 原目录不存在")?;
            let mut skipped = Vec::new();
            let mut conflicts = Vec::new();
            let mut following = Vec::new();
            let mut seen_paths = BTreeSet::new();
            let mut paths = vec![UpdatePath {
                path: original.to_path_buf(), identity: instance.identity, digest,
                marked: is_managed_copy(original),
            }];
            for path in std::iter::once(self.source_dir.join(id))
                .chain(roots.iter().map(|root| root.join(id)))
            {
                if seen_paths.insert(path.clone()) && path != original && path.join(SKILL_MANIFEST_FILE).is_file() {
                    let resolved = path.canonicalize()?;
                    if std::fs::symlink_metadata(&path)?.file_type().is_symlink() || resolved == target {
                        following.push(FollowingPath {
                            identity: skill_path_identity(&path)?, path, target: resolved,
                        });
                        continue;
                    }
                    let tracked = record.and_then(|record| record.managed_copies.iter().find(|item| item.path == path));
                    let marked = is_managed_copy(&path);
                    if tracked.is_some_and(|item| skill_path_identity(&path).ok() == Some(item.identity))
                        && (path == self.source_dir.join(id) || marked) {
                        if skill_directory_digest(&path)? == Some(digest) {
                            paths.push(UpdatePath { identity: skill_path_identity(&path)?, digest, path, marked });
                        } else {
                            conflicts.push(format!("受控副本内容已修改：{}", path.display()));
                        }
                    } else {
                        following.push(FollowingPath {
                            identity: skill_path_identity(&path)?, path, target: resolved,
                        });
                    }
                }
            }
            if std::fs::symlink_metadata(original.join(".git")).is_ok() {
                conflicts.push("Skill 原目录是 Git 工作树，不能直接替换其仓库信息".into());
            }
            let followed = classify_following_paths(&paths, following, &mut skipped)?;
            return Ok((paths, conflicts, followed, skipped));
        }
        let discovered = roots.iter().map(|root| root.join(id))
            .find(|path| path.canonicalize().is_ok_and(|path| path == target))
            .context("原目录不属于当前发现范围，请重新关联来源")?;
        let target_path = if std::fs::symlink_metadata(&discovered)?.file_type().is_dir() {
            discovered
        } else {
            target.clone()
        };
        let target_digest = skill_directory_digest(&target_path)?
            .context("Skill 原目录不存在")?;
        let mut paths = vec![UpdatePath {
            identity: skill_path_identity(&target_path)?,
            digest: target_digest,
            path: target_path.clone(),
            marked: false,
        }];
        let mut seen = BTreeSet::from([target]);
        let mut conflicts = Vec::new();
        let mut following = Vec::new();
        let mut skipped = Vec::new();
        let mut seen_paths = BTreeSet::new();
        if std::fs::symlink_metadata(paths[0].path.join(".git")).is_ok() {
            conflicts.push("Skill 原目录是 Git 工作树，不能直接替换其仓库信息".into());
        }
        let mut candidates = vec![(self.source_dir.join(id), true)];
        candidates.extend(roots.into_iter().map(|root| (root.join(id), false)));
        for (path, source) in candidates {
            if !seen_paths.insert(path.clone()) || !path.join(SKILL_MANIFEST_FILE).is_file() {
                continue;
            }
            let resolved = path.canonicalize().with_context(|| format!("无法解析 {}", path.display()))?;
            if path == target_path {
                continue;
            }
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() || seen.contains(&resolved) {
                following.push(FollowingPath {
                    identity: skill_path_identity(&path)?, path, target: resolved,
                });
                continue;
            }
            if !seen.insert(resolved.clone()) {
                continue;
            }
            let marked = is_managed_copy(&path);
            if source && !std::fs::symlink_metadata(&path)?.file_type().is_dir() {
                conflicts.push(format!("受控源是指向其他位置的链接，不能自动替换：{}", path.display()));
                continue;
            }
            if !source && !marked {
                conflicts.push(format!("独立 Skill 目录需要单独确认：{}", path.display()));
                continue;
            }
            let digest = skill_directory_digest(&path)?
                .context("受控 Skill 副本不存在")?;
            if digest != target_digest {
                conflicts.push(format!("受控副本内容已修改：{}", path.display()));
            }
            paths.push(UpdatePath {
                identity: skill_path_identity(&path)?,
                digest,
                path,
                marked,
            });
        }
        let followed = classify_following_paths(&paths, following, &mut skipped)?;
        Ok((paths, conflicts, followed, skipped))
    }

    /// 下载完成后固定目录和远端版本；关联来源不改变更新所有权。
    pub fn preview_skill_in_place_update(
        &self,
        id: &str,
        remote: RemoteSkill,
        zip: Vec<u8>,
    ) -> anyhow::Result<SkillInPlaceUpdatePreview> {
        self.preview_skill_in_place_update_inner(id, None, remote, zip)
    }

    pub fn preview_skill_in_place_update_for_instance(
        &self,
        id: &str,
        local_path: &Path,
        remote: RemoteSkill,
        zip: Vec<u8>,
    ) -> anyhow::Result<SkillInPlaceUpdatePreview> {
        self.preview_skill_in_place_update_inner(id, Some(local_path), remote, zip)
    }

    fn preview_skill_in_place_update_inner(
        &self,
        id: &str,
        local_path: Option<&Path>,
        remote: RemoteSkill,
        zip: Vec<u8>,
    ) -> anyhow::Result<SkillInPlaceUpdatePreview> {
        validate_skill_id(id)?;
        anyhow::ensure!(zip.len() <= REPO_ZIP_DOWNLOAD_LIMIT_BYTES, "仓库压缩包超过 64 MiB");
        let (scope, generation, repo_generation, installed, repo, instance_target, instance_record) = {
            let coordination = skills_coordination();
            let state = self.load_state_checked_unlocked()?;
            let instance_record = if let Some(path) = local_path {
                let path = path.canonicalize().context("所选 Skill 原件不存在")?;
                let record = state.instance_record(id, &path).cloned();
                if record.is_none() {
                    let legacy = state.installed.get(id).and_then(|item| item.original_dir.as_ref())
                        .and_then(|original| original.canonicalize().ok()).as_ref() == Some(&path);
                    let target_valid = state.instance_targets.get(id).is_none_or(|target|
                        target.path.canonicalize().ok().as_ref() == Some(&path)
                            && skill_path_identity(&target.path).ok() == Some(target.identity));
                    anyhow::ensure!(legacy && target_valid, "所选 Skill 尚未关联来源或原件已被替换");
                }
                record
            } else {
                state.instance_targets.get(id).and_then(|target|
                    target.path.canonicalize().ok().and_then(|path| state.instance_record(id, &path).cloned()))
            };
            let installed = instance_record.as_ref().map(|record| record.installed.clone())
                .or_else(|| state.installed.get(id).cloned()).context("Skill 尚未关联来源")?;
            anyhow::ensure!(installed.repo_key == remote.repo_key
                && installed.repo_path == remote.repo_path
                && !remote.content_hash.is_empty(), "仓库路径或版本与来源记录不匹配");
            anyhow::ensure!(installed.update_owner.is_empty() || installed.update_owner == "in_place",
                "此 Skill 由其他更新方式管理，请使用原有更新入口");
            let repo = state.repos.iter().find(|repo| repo.key() == installed.repo_key)
                .cloned().context("Skill 仓库已不存在")?;
            let scope = self.coordination_scope();
            let generation = coordination.mutation_generation(&scope, id);
            let repo_generation = coordination.repo_generation(&scope, &repo.key());
            let instance_target = instance_record.as_ref().map(|record| record.target.clone())
                .or_else(|| state.instance_targets.get(id).cloned());
            (scope, generation, repo_generation, installed, repo, instance_target, instance_record)
        };
        let original = installed.original_dir.as_ref()
            .context("无法确定唯一原目录，请先处理独立副本")?;
        let (paths, mut conflicts, followed_paths, skipped_paths) = self.in_place_update_paths(id, original, instance_target.as_ref(), instance_record.as_ref())?;
        if !installed.content_hash.is_empty()
            && source_match::local_repository_hash(&paths[0].path)? != installed.content_hash
        {
            conflicts.push("Skill 原件自上次关联或更新后已修改，请先处理本地更改".into());
        }
        let update = InPlaceUpdate {
            scope,
            generation,
            repo_generation,
            installed,
            repo,
            remote,
            paths,
            zip,
            conflicts,
            instance_target,
            instance_record,
            followed_paths,
            skipped_paths,
        };
        {
            let coordination = skills_coordination();
            self.check_in_place_state(&update, &coordination)?;
        }
        let preview_id = uuid::Uuid::new_v4().to_string();
        let preview = SkillInPlaceUpdatePreview {
            preview_id: preview_id.clone(),
            id: id.to_string(),
            repo_key: update.remote.repo_key.clone(),
            repo_path: update.remote.repo_path.clone(),
            target_path: update.paths[0].path.clone(),
            current_hash: update.installed.content_hash.clone(),
            next_hash: update.remote.content_hash.clone(),
            conflicts: update.conflicts.clone(),
            managed_paths: update.paths.iter().skip(1).map(|path| path.path.clone()).collect(),
            followed_paths: update.followed_paths.iter().map(|path| path.path.clone()).collect(),
            skipped_paths: update.skipped_paths.clone(),
        };
        let mut previews = update_previews();
        while previews.len() >= 4 {
            previews.pop_first();
        }
        previews.insert(preview_id, update);
        Ok(preview)
    }

    fn check_in_place_state(
        &self,
        update: &InPlaceUpdate,
        coordination: &SkillsCoordination,
    ) -> anyhow::Result<()> {
        let state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(&update.installed.id, &state)?;
        anyhow::ensure!(coordination_scope_is_current(&update.scope, &self.coordination_scope())
            && coordination.mutation_generation(&update.scope, &update.installed.id) == update.generation
            && coordination.repo_generation(&update.scope, &update.repo.key()) == update.repo_generation
            && if let Some(record) = &update.instance_record {
                state.instance_records.get(&update.installed.id)
                    .and_then(|records| records.get(&record.target.path)) == Some(record)
            } else {
                state.installed.get(&update.installed.id) == Some(&update.installed)
                    && state.instance_targets.get(&update.installed.id) == update.instance_target.as_ref()
            }
            && state.repos.contains(&update.repo),
            "预览后 Skill 或仓库已改变，请重新预览");
        for path in &update.paths {
            anyhow::ensure!(skill_path_identity(&path.path)? == path.identity,
                "预览后 Skill 路径已改变，请重新预览");
        }
        for path in &update.followed_paths {
            anyhow::ensure!(skill_path_identity(&path.path)? == path.identity
                && path.path.canonicalize()? == path.target,
                "预览后 Skill 链接已改变，请重新预览");
        }
        Ok(())
    }

    fn check_in_place_contents(&self, update: &InPlaceUpdate) -> anyhow::Result<()> {
        let original = update.installed.original_dir.as_ref().context("Skill 原目录缺失")?;
        let (paths, conflicts, followed_paths, skipped_paths) = self.in_place_update_paths(&update.installed.id, original, update.instance_target.as_ref(), update.instance_record.as_ref())?;
        let expected_conflicts: Vec<_> = update.conflicts.iter()
            .filter(|item| !item.starts_with("Skill 原件自上次关联或更新后已修改"))
            .cloned().collect();
        anyhow::ensure!(paths.len() == update.paths.len()
            && paths.iter().zip(&update.paths).all(|(current, previous)|
                current.path == previous.path && current.identity == previous.identity
                    && current.digest == previous.digest && current.marked == previous.marked)
            && conflicts == expected_conflicts && followed_paths == update.followed_paths
            && skipped_paths == update.skipped_paths,
            "预览后 Skill 文件或副本已改变，请重新预览");
        Ok(())
    }

    /// 获取后端保存的远端身份，供确认时重新检查仓库版本。
    pub fn in_place_update_preview_remote(&self, preview_id: &str) -> anyhow::Result<RemoteSkill> {
        let previews = update_previews();
        let update = previews.get(preview_id).context("Skill 更新预览已失效，请重新预览")?;
        anyhow::ensure!(coordination_scope_is_current(&update.scope, &self.coordination_scope()),
            "Skill 管理范围已改变，请重新预览");
        Ok(update.remote.clone())
    }

    pub fn discard_in_place_update_preview(preview_id: &str) {
        update_previews().remove(preview_id);
    }

    /// 只执行后端保存的预览，仓库 ZIP 和目标路径均不由调用方在确认时提供。
    pub fn execute_skill_in_place_update(
        &self,
        preview_id: &str,
        observed_remote: &RemoteSkill,
    ) -> anyhow::Result<SkillsState> {
        let update = update_previews().remove(preview_id)
            .context("Skill 更新预览已失效，请重新预览")?;
        anyhow::ensure!(observed_remote.repo_key == update.remote.repo_key
            && observed_remote.repo_path == update.remote.repo_path
            && observed_remote.content_hash == update.remote.content_hash,
            "预览后仓库版本已改变，请重新预览");
        anyhow::ensure!(update.conflicts.is_empty(), "存在 Skill 副本冲突，请先处理");
        self.check_in_place_contents(&update)?;
        {
            let coordination = skills_coordination();
            self.check_in_place_state(&update, &coordination)?;
        }
        let id = &update.installed.id;
        let mut staged: Vec<(PathBuf, SkillPathIdentity)> = Vec::new();
        let cleanup = |error: anyhow::Error, paths: &[(PathBuf, SkillPathIdentity)]| {
            paths.iter().fold(error, |error, (path, identity)| {
                match recover_published_path(path, *identity) {
                    Ok(Some(recovery)) => match std::fs::remove_dir_all(&recovery) {
                        Ok(()) => error,
                        Err(cleanup) => error.context(format!("清理 Skill 暂存目录失败，保留在 {}：{cleanup}", recovery.display())),
                    },
                    Ok(None) => error,
                    Err(cleanup) => error.context(format!("清理 Skill 暂存目录失败：{cleanup:#}")),
                }
            })
        };
        for path in &update.paths {
            let parent = path.path.parent().context("Skill 原目录没有父目录")?;
            let staging = parent.join(format!(".codex-plus-update-{id}-{}", uuid::Uuid::new_v4()));
            if let Err(error) = std::fs::create_dir(&staging) {
                let error = anyhow::Error::new(error)
                    .context(format!("创建 Skill 暂存目录失败：{}", staging.display()));
                return Err(cleanup(error, &staged));
            }
            let identity = match skill_path_identity(&staging) {
                Ok(identity) => identity,
                Err(error) => return Err(cleanup(retain_staging_context(error, &staging), &staged)),
            };
            staged.push((staging, identity));
        }
        let result = (|| -> anyhow::Result<SkillsState> {
            let remote = archive::stage_downloaded_zip(
                &update.repo, &update.zip, id, Some(&update.installed.repo_path), &staged[0].0,
            )?;
            anyhow::ensure!(remote.repo_path == update.remote.repo_path
                && remote.content_hash == update.remote.content_hash,
                "仓库压缩包与预览版本不一致，请重新预览");
            if update.paths[0].marked {
                std::fs::write(staged[0].0.join(MANAGED_COPY_MARKER), "")?;
            }
            for (index, path) in update.paths.iter().enumerate().skip(1) {
                copy_dir_all(&staged[0].0, &staged[index].0)?;
                if path.marked {
                    std::fs::write(staged[index].0.join(MANAGED_COPY_MARKER), "")?;
                }
            }
            self.check_in_place_contents(&update)?;
            let mut coordination = skills_coordination();
            let mut state = self.load_state_checked_unlocked()?;
            let mut displaced = Vec::new();
            let mut published = Vec::new();
            let commit = (|| -> anyhow::Result<()> {
                #[cfg(test)]
                BEFORE_UPDATE_BACKUP.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().take() { hook(); }
                });
                self.check_in_place_state(&update, &coordination)?;
                for path in &update.paths {
                    let backup = self.backup_directory_checked(id, &path.path, path.identity)?;
                    displaced.push((path.path.clone(), backup.clone(), path.identity));
                    anyhow::ensure!(skill_directory_digest(&backup)? == Some(path.digest),
                        "更新期间 Skill 内容已改变，请重新预览：{}", path.path.display());
                }
                for (path, (staging, identity)) in update.paths.iter().zip(&staged) {
                    anyhow::ensure!(skill_path_identity(staging)? == *identity,
                        "Skill 暂存目录已被替换：{}", staging.display());
                    move_path_no_replace(staging, &path.path)?;
                    published.push((path.path.clone(), *identity));
                    anyhow::ensure!(skill_path_identity(&path.path)? == *identity,
                        "Skill 更新期间目标路径发生变化");
                }
                let mut backup_ids = Vec::new();
                for (original, backup, _) in &displaced {
                    let backup_id = self.backup_id_for_path(backup)?;
                    state.backup_origins.insert(backup_id.clone(), SkillBackupOrigin {
                        path: original.clone(),
                        purpose: "update".into(),
                    });
                    backup_ids.push(backup_id);
                }
                state.update_backup_groups.insert(uuid::Uuid::new_v4().to_string(),
                    SkillUpdateBackupGroup {
                        installed: update.installed.clone(),
                        backup_ids,
                        partially_deleted: false,
                        instance_path: update.instance_record.as_ref().map(|record| record.target.path.clone()),
                        published: update.paths.iter().map(|path| {
                            Ok(SkillPublishedSnapshot {
                                path: path.path.clone(),
                                identity: skill_path_identity(&path.path)?,
                                digest: skill_directory_digest(&path.path)?.context("更新后的 Skill 目录不存在")?,
                            })
                        }).collect::<anyhow::Result<_>>()?,
                    });
                let mirror = update.instance_record.as_ref().is_none_or(|record|
                    state.instance_targets.get(id) == Some(&record.target));
                let installed = if let Some(record) = &update.instance_record {
                    &mut state.instance_records.get_mut(id).context("Skill 原件记录已消失")?
                        .get_mut(&record.target.path).context("Skill 原件记录已消失")?.installed
                } else {
                    state.installed.get_mut(id).context("Skill 安装记录已消失")?
                };
                installed.name = remote.name;
                installed.description = remote.description;
                installed.content_hash = remote.content_hash;
                installed.original_dir = Some(update.paths[0].path.clone());
                installed.update_owner = "in_place".into();
                let updated_installed = installed.clone();
                if let Some(record) = &update.instance_record {
                    let record = state.instance_records.get_mut(id).unwrap().get_mut(&record.target.path).unwrap();
                    record.target.identity = skill_path_identity(&update.paths[0].path)?;
                    for tracked in &mut record.managed_copies {
                        if let Some(path) = update.paths.iter().skip(1).find(|path| path.path == tracked.path) {
                            tracked.identity = skill_path_identity(&path.path)?;
                        }
                    }
                }
                if mirror {
                    state.installed.insert(id.clone(), updated_installed);
                }
                if mirror && update.instance_target.is_some() {
                    state.instance_targets.insert(id.clone(), SkillInstanceTarget {
                        path: update.paths[0].path.clone(),
                        identity: skill_path_identity(&update.paths[0].path)?,
                    });
                }
                self.save_state_unlocked(&state)
            })();
            if let Err(error) = commit {
                let mut failures = Vec::new();
                let mut recoveries = Vec::new();
                for (path, identity) in published.iter().rev() {
                    match recover_published_path(path, *identity) {
                        Ok(Some(path)) => recoveries.push(path.display().to_string()),
                        Ok(None) => {}
                        Err(error) => failures.push(format!("{error:#}")),
                    }
                }
                for (path, backup, identity) in displaced.iter().rev() {
                    if let Err(error) = restore_skill_backup(backup, path, *identity) {
                        failures.push(format!("原内容保留在 {}：{error:#}", backup.display()));
                    }
                }
                if !failures.is_empty() || !recoveries.is_empty() {
                    return Err(error.context(format!("回滚：{}；新内容保留在 {}",
                        failures.join("；"), recoveries.join("、"))));
                }
                return Err(error);
            }
            coordination.changed_skill(self.coordination_scope(), id);
            Ok(state)
        })();
        result.map_err(|error| cleanup(error, &staged))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> (tempfile::TempDir, SkillsManager, SkillRepo, RemoteSkill, Vec<u8>) {
        let temp = tempfile::tempdir().unwrap();
        let shared = temp.path().join("shared");
        let agent = temp.path().join("agent");
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("codex"),
        ).with_shared_skills_dir(&shared)
            .with_agent_dirs(vec![("claude".into(), agent.clone())]);
        let original = shared.join("alpha");
        std::fs::create_dir_all(&original).unwrap();
        std::fs::write(original.join(SKILL_MANIFEST_FILE), "---\nname: alpha\n---\nold").unwrap();
        std::fs::write(original.join("details.txt"), "old details").unwrap();
        copy_dir_all(&original, &manager.source_dir.join("alpha")).unwrap();
        copy_dir_all(&original, &agent.join("alpha")).unwrap();
        std::fs::write(agent.join("alpha").join(MANAGED_COPY_MARKER), "").unwrap();
        let repo = SkillRepo {
            owner: "example".into(), name: "skills".into(), branch: "main".into(),
            subdir: String::new(), enabled: true,
        };
        let mut state = manager.load_state();
        state.repos.push(repo.clone());
        state.installed.insert("alpha".into(), InstalledSkill {
            id: "alpha".into(), name: "alpha".into(), description: String::new(),
            repo_key: repo.key(), repo_path: "skills/alpha".into(),
            content_hash: source_match::local_repository_hash(&original).unwrap(),
            installed_at: String::new(), verified_cc_switch: false,
            install_channel: "local".into(), original_dir: Some(original), update_owner: String::new(),
        });
        manager.save_state_unlocked(&state).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("skills-main/skills/alpha/SKILL.md", options).unwrap();
        writer.write_all(b"---\nname: alpha\n---\nnew").unwrap();
        writer.start_file("skills-main/skills/alpha/details.txt", options).unwrap();
        writer.write_all(b"new details").unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap()
            .into_iter().find(|item| item.repo_path == "skills/alpha").unwrap();
        (temp, manager, repo, remote, zip)
    }

    #[test]
    fn discarded_preview_cannot_be_executed() {
        let (_temp, manager, _repo, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        SkillsManager::discard_in_place_update_preview(&preview.preview_id);
        assert!(manager.in_place_update_preview_remote(&preview.preview_id).is_err());
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        SkillsManager::discard_in_place_update_preview(&preview.preview_id);
    }

    #[test]
    fn discarding_an_older_preview_preserves_another_preview_for_the_same_skill() {
        let (_temp, manager, _repo, remote, zip) = fixture();
        let current = manager.preview_skill_in_place_update("alpha", remote.clone(), zip.clone()).unwrap();
        let late = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        SkillsManager::discard_in_place_update_preview(&late.preview_id);
        assert_eq!(manager.in_place_update_preview_remote(&current.preview_id).unwrap().content_hash, remote.content_hash);
        SkillsManager::discard_in_place_update_preview(&current.preview_id);
    }

    #[test]
    fn update_without_a_baseline_still_checks_files_after_preview() {
        let (temp, manager, _repo, remote, zip) = fixture();
        let mut state = manager.load_state();
        state.installed.get_mut("alpha").unwrap().content_hash.clear();
        manager.save_state_unlocked(&state).unwrap();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        assert!(preview.current_hash.is_empty());
        assert!(preview.conflicts.is_empty());
        std::fs::write(temp.path().join("shared/alpha/details.txt"), "changed after preview").unwrap();
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "changed after preview");
    }

    #[cfg(unix)]
    fn test_directory_link(source: &Path, target: &Path) {
        std::os::unix::fs::symlink(source, target).unwrap();
    }

    #[cfg(windows)]
    fn test_directory_link(source: &Path, target: &Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(target.to_string_lossy().replace('/', "\\"))
            .arg(source.to_string_lossy().replace('/', "\\")).output().unwrap();
        assert!(output.status.success(), "无法创建测试 junction：{}",
            String::from_utf8_lossy(&output.stderr));
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn following_links_are_reported_and_rechecked_without_replacement() {
        let (temp, manager, repo, remote, zip) = fixture();
        let manager = manager.with_agent_dirs(vec![
            ("claude".into(), temp.path().join("agent")),
            ("independent".into(), temp.path().join("independent")),
            ("unknown".into(), temp.path().join("unknown")),
        ]);
        let original = temp.path().join("shared/alpha");
        let source = manager.source_dir.join("alpha");
        let copy = temp.path().join("agent/alpha");
        let linked = manager.linked_dir().join("alpha");
        let independent = temp.path().join("independent/alpha");
        let unknown = temp.path().join("unknown/alpha");
        copy_dir_all(&original, &independent).unwrap();
        std::fs::create_dir_all(manager.linked_dir()).unwrap();
        std::fs::create_dir_all(unknown.parent().unwrap()).unwrap();
        test_directory_link(&source, &linked);
        test_directory_link(&independent, &unknown);
        let matches = manager.preview_repository_matches_for_instance(
            &repo, &[remote.clone()], "alpha", &original,
        ).unwrap();
        let selection = SkillRepositoryMatchSelection {
            preview_id: matches.preview_id, local_id: "alpha".into(),
            repo_key: repo.key(), repo_path: remote.repo_path.clone(),
            local_path: Some(original.clone()), manual: false,
        };
        let result = manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);

        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &original, remote.clone(), zip.clone(),
        ).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        assert!(preview.managed_paths.contains(&source));
        assert!(preview.managed_paths.contains(&copy));
        assert!(preview.followed_paths.contains(&linked));
        assert!(!preview.managed_paths.contains(&linked));
        assert!(preview.skipped_paths.contains(&independent));
        assert!(preview.skipped_paths.contains(&unknown));

        let held_link = temp.path().join("original-link");
        std::fs::rename(&linked, &held_link).unwrap();
        test_directory_link(&source, &linked);
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "old details");
        let held_replacement = temp.path().join("replacement-link");
        std::fs::rename(&linked, &held_replacement).unwrap();
        std::fs::rename(&held_link, &linked).unwrap();

        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &original, remote.clone(), zip.clone(),
        ).unwrap();
        let linked_during_commit = linked.clone();
        let source_during_commit = source.clone();
        let held_during_commit = temp.path().join("link-during-commit");
        let held_for_hook = held_during_commit.clone();
        BEFORE_UPDATE_BACKUP.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            std::fs::rename(&linked_during_commit, &held_for_hook).unwrap();
            test_directory_link(&source_during_commit, &linked_during_commit);
        })));
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "old details");
        let replaced_during_commit = temp.path().join("replacement-during-commit");
        std::fs::rename(&linked, &replaced_during_commit).unwrap();
        std::fs::rename(&held_during_commit, &linked).unwrap();
        let link_identity = skill_path_identity(&linked).unwrap();

        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &original, remote.clone(), zip,
        ).unwrap();
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        assert_eq!(skill_path_identity(&linked).unwrap(), link_identity);
        assert_eq!(std::fs::read_to_string(linked.join("details.txt")).unwrap(), "new details");
        assert_eq!(std::fs::read_to_string(unknown.join("details.txt")).unwrap(), "old details");
        assert!(!state.backup_origins.values().any(|origin| origin.path == linked || origin.path == unknown));
    }

    #[test]
    fn independently_associated_instances_keep_sources_updates_and_backups_separate() {
        let (temp, manager, repo, remote, zip) = fixture();
        let first = temp.path().join("shared/alpha").canonicalize().unwrap();
        let second = temp.path().join("agent/alpha").canonicalize().unwrap();
        std::fs::remove_file(second.join(MANAGED_COPY_MARKER)).unwrap();
        let mut other_repo = repo.clone();
        other_repo.owner = "another".into();
        let mut state = manager.load_state();
        state.repos.push(other_repo.clone());
        manager.save_state_unlocked(&state).unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("skills-main/SKILL.md", options).unwrap();
        writer.write_all(b"---\nname: alpha\n---\nnew").unwrap();
        writer.start_file("skills-main/details.txt", options).unwrap();
        writer.write_all(b"new details").unwrap();
        let other_zip = writer.finish().unwrap().into_inner();
        let other_remote = parse_skills_from_zip(&other_repo, &other_zip, &BTreeMap::new()).unwrap()
            .into_iter().find(|item| item.repo_path == ".").unwrap();
        for (path, source, candidate) in [(&first, &repo, &remote), (&second, &other_repo, &other_remote)] {
            let preview = manager.preview_repository_matches_for_instance(
                source, &[candidate.clone()], "alpha", path,
            ).unwrap();
            let selection = SkillRepositoryMatchSelection {
                preview_id: preview.preview_id, local_id: "alpha".into(), repo_key: source.key(),
                repo_path: candidate.repo_path.clone(), local_path: Some(path.clone()), manual: false,
            };
            let result = manager.apply_repository_matches(&[selection], &[candidate.clone()]).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert_eq!(result.applied[0].local_path.as_ref(), Some(path));
        }
        let inventory = manager.try_merge_entries(&[remote.clone(), other_remote.clone()]).unwrap();
        let first_row = inventory.iter().find(|item| item.id == "alpha"
            && item.original_dir.as_ref() == Some(&first)).unwrap();
        let second_row = inventory.iter().find(|item| item.id == "alpha"
            && item.original_dir.as_ref() == Some(&second)).unwrap();
        assert_eq!(first_row.repo_key, repo.key());
        assert_eq!(second_row.repo_key, other_repo.key());
        assert_eq!(second_row.repo_path, ".");
        assert_eq!(first_row.locations[0].repo_key, repo.key());
        assert_eq!(second_row.locations[0].repo_key, other_repo.key());

        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &first, remote.clone(), zip.clone(),
        ).unwrap();
        assert!(preview.skipped_paths.iter().any(|path| path.canonicalize().ok().as_ref() == Some(&second)));
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        let first_backup = state.update_backup_groups.values().next().unwrap().backup_ids[0].clone();
        assert_eq!(std::fs::read_to_string(second.join("details.txt")).unwrap(), "old details");
        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &second, other_remote.clone(), other_zip,
        ).unwrap();
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &other_remote).unwrap();
        let second_backup = state.update_backup_groups.values().find(|group|
            group.instance_path.as_ref() == Some(&second)).unwrap().backup_ids[0].clone();
        manager.restore_backup(&second_backup).unwrap();
        assert_eq!(std::fs::read_to_string(first.join("details.txt")).unwrap(), "new details");
        assert_eq!(std::fs::read_to_string(second.join("details.txt")).unwrap(), "old details");
        manager.restore_backup(&first_backup).unwrap();
        let inventory = manager.try_local_inventory().unwrap();
        assert_eq!(inventory.iter().find(|item| item.original_dir.as_ref() == Some(&first)).unwrap().repo_key, repo.key());
        assert_eq!(inventory.iter().find(|item| item.original_dir.as_ref() == Some(&second)).unwrap().repo_key, other_repo.key());
    }

    #[test]
    fn same_repository_agent_paths_update_only_the_selected_installation() {
        let (temp, manager, repo, _, _) = fixture();
        let first = temp.path().join("shared/alpha").canonicalize().unwrap();
        let second = temp.path().join("agent/alpha").canonicalize().unwrap();
        std::fs::remove_file(second.join(MANAGED_COPY_MARKER)).unwrap();
        std::fs::write(second.join("details.txt"), "claude old").unwrap();
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (path, details) in [(".agents/skills/alpha", "codex new"), (".codex/skills/alpha", "claude new")] {
            writer.start_file(format!("skills-main/{path}/SKILL.md"), options).unwrap();
            writer.write_all(b"---\nname: alpha\n---\nnew").unwrap();
            writer.start_file(format!("skills-main/{path}/details.txt"), options).unwrap();
            writer.write_all(details.as_bytes()).unwrap();
        }
        let zip = writer.finish().unwrap().into_inner();
        let candidates = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        let codex_remote = candidates.iter().find(|item| item.repo_path == ".agents/skills/alpha").unwrap();
        let claude_remote = candidates.iter().find(|item| item.repo_path == ".codex/skills/alpha").unwrap();
        for (path, remote) in [(&first, codex_remote), (&second, claude_remote)] {
            let preview = manager.preview_repository_matches_for_instance(
                &repo, &[remote.clone()], "alpha", path,
            ).unwrap();
            let selection = SkillRepositoryMatchSelection {
                preview_id: preview.preview_id, local_id: "alpha".into(), repo_key: repo.key(),
                repo_path: remote.repo_path.clone(), local_path: Some(path.clone()), manual: false,
            };
            let result = manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
        }

        let inventory = manager.try_merge_entries(&candidates).unwrap();
        let instances = inventory.iter().filter(|item| item.id == "alpha" && item.installed).collect::<Vec<_>>();
        assert_eq!(instances.len(), 2);
        assert!(instances.iter().all(|item| item.conflict.is_none()));
        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &first, codex_remote.clone(), zip.clone(),
        ).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        assert!(preview.skipped_paths.iter().any(|path| path.canonicalize().ok().as_ref() == Some(&second)));
        manager.execute_skill_in_place_update(&preview.preview_id, codex_remote).unwrap();
        assert_eq!(std::fs::read_to_string(first.join("details.txt")).unwrap(), "codex new");
        assert_eq!(std::fs::read_to_string(second.join("details.txt")).unwrap(), "claude old");

        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &second, claude_remote.clone(), zip,
        ).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        assert!(preview.skipped_paths.iter().any(|path| path.canonicalize().ok().as_ref() == Some(&first)));
        manager.execute_skill_in_place_update(&preview.preview_id, claude_remote).unwrap();
        assert_eq!(std::fs::read_to_string(first.join("details.txt")).unwrap(), "codex new");
        assert_eq!(std::fs::read_to_string(second.join("details.txt")).unwrap(), "claude new");
    }

    #[test]
    fn equal_agent_installations_with_the_same_repository_are_not_conflicts() {
        let (temp, manager, repo, remote, _) = fixture();
        let first = temp.path().join("shared/alpha").canonicalize().unwrap();
        let second = temp.path().join("agent/alpha").canonicalize().unwrap();
        std::fs::remove_file(second.join(MANAGED_COPY_MARKER)).unwrap();
        for path in [&first, &second] {
            let preview = manager.preview_repository_matches_for_instance(
                &repo, &[remote.clone()], "alpha", path,
            ).unwrap();
            let selection = SkillRepositoryMatchSelection {
                preview_id: preview.preview_id, local_id: "alpha".into(), repo_key: repo.key(),
                repo_path: remote.repo_path.clone(), local_path: Some(path.clone()), manual: false,
            };
            let result = manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
        }
        let inventory = manager.try_local_inventory().unwrap();
        let instances = inventory.iter().filter(|item| item.id == "alpha").collect::<Vec<_>>();
        assert_eq!(instances.len(), 2);
        assert!(instances.iter().all(|item| item.conflict.is_none()));
        assert_eq!(instances.iter().filter_map(|item| item.original_dir.clone()).collect::<BTreeSet<_>>(),
            BTreeSet::from([first, second]));
    }

    #[test]
    fn confirmed_managed_copies_update_and_restore_with_original() {
        let (temp, manager, repo, remote, zip) = fixture();
        let original = temp.path().join("shared/alpha");
        let source = manager.source_dir.join("alpha");
        let copy = temp.path().join("agent/alpha");
        let mut state = manager.load_state();
        state.installed.get_mut("alpha").unwrap().update_owner = "manager".into();
        manager.save_state_unlocked(&state).unwrap();
        let preview = manager.preview_repository_matches_for_instance(&repo, &[remote.clone()], "alpha", &original).unwrap();
        let selection = SkillRepositoryMatchSelection {
            preview_id: preview.preview_id, local_id: "alpha".into(), repo_key: repo.key(),
            repo_path: remote.repo_path.clone(), local_path: Some(original.clone()), manual: false,
        };
        assert!(manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap().errors.is_empty());
        std::fs::write(copy.join("details.txt"), "modified").unwrap();
        let preview = manager.preview_skill_in_place_update_for_instance("alpha", &original, remote.clone(), zip.clone()).unwrap();
        assert!(preview.conflicts.iter().any(|item| item.contains("受控副本内容已修改")));
        std::fs::write(copy.join("details.txt"), "old details").unwrap();
        let preview = manager.preview_skill_in_place_update_for_instance("alpha", &original, remote.clone(), zip).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        assert!(preview.managed_paths.contains(&source));
        assert!(preview.managed_paths.contains(&copy));
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        for path in [&original, &source, &copy] {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "new details");
        }
        let backup = state.update_backup_groups.values().next().unwrap().backup_ids[0].clone();
        manager.restore_backup(&backup).unwrap();
        for path in [&original, &source, &copy] {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "old details");
        }
    }

    #[test]
    fn archived_source_preserves_repository_association_and_updates_the_codex_original() {
        let (temp, manager, repo, remote, zip) = fixture();
        let shared = temp.path().join("shared/alpha");
        let codex = manager.linked_dir().join("alpha");
        std::fs::create_dir_all(manager.linked_dir()).unwrap();
        std::fs::rename(&shared, &codex).unwrap();
        std::fs::write(codex.join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::rename(temp.path().join("agent/alpha"), temp.path().join("held-agent")).unwrap();
        let mut state = manager.load_state();
        state.installed.get_mut("alpha").unwrap().original_dir = None;
        manager.save_state_unlocked(&state).unwrap();

        let duplicates = manager.preview_skill_duplicates().unwrap();
        assert_eq!(duplicates.candidates.iter().find(|item| item.kind == "source").unwrap().status, "identical-source");
        manager.archive_skill_managed_source(&duplicates.preview_id, "alpha").unwrap();
        let original = codex.canonicalize().unwrap();
        let unselected = manager.preview_repository_matches(&repo, &[remote.clone()], Some("alpha")).unwrap();
        assert!(unselected.matches.iter().any(|item| item.conflict.is_none()));
        let matches = manager.preview_repository_matches_for_instance(
            &repo, &[remote.clone()], "alpha", &original,
        ).unwrap();
        let selected = matches.matches.iter().find(|item| item.repo_path == remote.repo_path).unwrap();
        let selection = SkillRepositoryMatchSelection {
            preview_id: matches.preview_id.clone(), local_id: "alpha".into(),
            repo_key: repo.key(), repo_path: selected.repo_path.clone(),
            local_path: Some(original.clone()), manual: false,
        };
        let result = manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &original, remote.clone(), zip,
        ).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        assert!(codex.join(MANAGED_COPY_MARKER).is_file());
        assert_eq!(std::fs::read_to_string(codex.join("details.txt")).unwrap(), "new details");
        let group = state.update_backup_groups.values().next().unwrap();
        manager.restore_backup(&group.backup_ids[0]).unwrap();
        assert!(codex.join(MANAGED_COPY_MARKER).is_file());
        assert_eq!(std::fs::read_to_string(codex.join("details.txt")).unwrap(), "old details");
    }

    #[test]
    fn legacy_single_instance_source_remains_available_at_its_location() {
        let (temp, manager, repo, remote, zip) = fixture();
        let original = temp.path().join("shared/alpha");
        let inventory = manager.try_local_inventory().unwrap();
        let location = inventory.iter().find(|item| item.id == "alpha").unwrap().locations.iter()
            .find(|location| location.root_kind == "shared").unwrap();
        assert_eq!(location.repo_key, repo.key());
        assert_eq!(location.repo_path, remote.repo_path);
        let preview = manager.preview_skill_in_place_update_for_instance(
            "alpha", &original, remote, zip,
        ).unwrap();
        assert_eq!(preview.target_path, original);
    }

    #[test]
    fn selected_independent_original_updates_and_restores_without_touching_other_installs() {
        let (temp, manager, repo, remote, zip) = fixture();
        let original = temp.path().join("shared/alpha");
        let agent = temp.path().join("agent/alpha");
        std::fs::remove_file(agent.join(MANAGED_COPY_MARKER)).unwrap();
        std::fs::write(agent.join(SKILL_MANIFEST_FILE), "---\nname: alpha\n---\nindependent").unwrap();
        let agent_identity = skill_path_identity(&agent).unwrap();
        let source = manager.source_dir.join("alpha");
        let source_identity = skill_path_identity(&source).unwrap();
        let matches = manager.preview_repository_matches_for_instance(
            &repo, &[remote.clone()], "alpha", &original,
        ).unwrap();
        let found = matches.matches.iter().find(|item| item.repo_path == "skills/alpha").unwrap();
        let mut selected = SkillRepositoryMatchSelection {
            preview_id: matches.preview_id.clone(), local_id: "alpha".into(),
            repo_key: repo.key(), repo_path: found.repo_path.clone(),
            local_path: Some(agent.clone()), manual: false,
        };
        assert!(!manager.apply_repository_matches(&[selected.clone()], &[remote.clone()]).unwrap().errors.is_empty());
        selected.local_path = Some(original.clone());
        assert!(manager.apply_repository_matches(&[selected], &[remote.clone()]).unwrap().errors.is_empty());
        let inventory = manager.try_local_inventory().unwrap();
        let associated = inventory.iter().find(|item| item.original_dir.as_ref()
            .is_some_and(|path| path == &original.canonicalize().unwrap())).unwrap();
        assert_eq!(associated.repo_key, repo.key());
        let other = inventory.iter().find(|item| item.original_dir.as_ref()
            .is_some_and(|path| path == &agent.canonicalize().unwrap())).unwrap();
        assert!(other.repo_key.is_empty());

        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip.clone()).unwrap();
        assert!(preview.conflicts.is_empty());
        assert!(preview.skipped_paths.contains(&agent));
        assert!(preview.managed_paths.contains(&source));
        std::fs::write(original.join("details.txt"), "edited after preview").unwrap();
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        std::fs::write(original.join("details.txt"), "old details").unwrap();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip.clone()).unwrap();
        FAIL_SKILL_STATE_SAVE.with(|fail| fail.set(true));
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "old details");
        assert_eq!(skill_path_identity(&agent).unwrap(), agent_identity);
        assert_eq!(skill_path_identity(&source).unwrap(), source_identity);
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip.clone()).unwrap();
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "new details");
        assert_eq!(std::fs::read_to_string(agent.join(SKILL_MANIFEST_FILE)).unwrap(), "---\nname: alpha\n---\nindependent");
        assert_eq!(skill_path_identity(&agent).unwrap(), agent_identity);
        assert_eq!(std::fs::read_to_string(source.join("details.txt")).unwrap(), "new details");
        assert_ne!(skill_path_identity(&source).unwrap(), source_identity);
        let backup_id = state.update_backup_groups.values().next().unwrap().backup_ids[0].clone();
        manager.restore_backup(&backup_id).unwrap();
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "old details");
        assert_eq!(skill_path_identity(&agent).unwrap(), agent_identity);
        assert_eq!(skill_path_identity(&source).unwrap(), source_identity);
        let state = manager.load_state();
        assert_eq!(state.instance_targets["alpha"].identity, skill_path_identity(&original).unwrap());
        let replaced = temp.path().join("preserved-original");
        std::fs::rename(&original, &replaced).unwrap();
        copy_dir_all(&replaced, &original).unwrap();
        assert!(manager.preview_skill_in_place_update("alpha", remote, zip).is_err());
    }

    #[test]
    fn equal_independent_installs_can_associate_and_update_one_original() {
        let (temp, manager, repo, remote, zip) = fixture();
        let original = temp.path().join("shared/alpha");
        let agent = temp.path().join("agent/alpha");
        std::fs::remove_file(agent.join(MANAGED_COPY_MARKER)).unwrap();
        let agent_identity = skill_path_identity(&agent).unwrap();
        let source_identity = skill_path_identity(&manager.source_dir.join("alpha")).unwrap();
        let entries = manager.try_local_inventory().unwrap();
        let instances = entries.iter().filter(|item| item.id == "alpha").collect::<Vec<_>>();
        assert_eq!(instances.len(), 2);
        assert!(instances.iter().all(|item| item.conflict.is_none()));
        let shared = instances.iter().flat_map(|item| &item.locations)
            .find(|location| location.root_kind == "shared").unwrap();
        let claude = instances.iter().flat_map(|item| &item.locations)
            .find(|location| location.root_kind == "claude").unwrap();
        assert!(shared.selectable && claude.selectable);
        assert_eq!(shared.original_path, Some(original.canonicalize().unwrap()));
        assert_eq!(claude.original_path, Some(agent.canonicalize().unwrap()));
        let selected_path = shared.original_path.clone().unwrap();
        let matches = manager.preview_repository_matches_for_instance(
            &repo, &[remote.clone()], "alpha", &selected_path,
        ).unwrap();
        let found = matches.matches.iter().find(|item| item.repo_path == "skills/alpha").unwrap();
        let selection = SkillRepositoryMatchSelection {
            preview_id: matches.preview_id.clone(), local_id: "alpha".into(),
            repo_key: repo.key(), repo_path: found.repo_path.clone(),
            local_path: Some(selected_path.clone()), manual: false,
        };
        assert!(manager.apply_repository_matches(&[selection], &[remote.clone()]).unwrap().errors.is_empty());
        let entries = manager.try_local_inventory().unwrap();
        let selected = entries.iter().find(|item| item.id == "alpha"
            && item.locations.iter().any(|location| location.original_path.as_ref() == Some(&selected_path))).unwrap();
        assert!(selected.conflict.is_none());
        assert!(selected.locations.iter().any(|location| location.original_path.as_ref() == Some(&selected_path)
            && location.repo_key == repo.key() && location.repo_path == "skills/alpha"));

        let saved = temp.path().join("original-before-replacement");
        let replacement = temp.path().join("unconfirmed-replacement");
        std::fs::rename(&original, &saved).unwrap();
        copy_dir_all(&saved, &original).unwrap();
        let entries = manager.try_local_inventory().unwrap();
        assert!(entries.iter().find(|item| item.id == "alpha"
            && item.locations.iter().any(|location| location.root_kind == "shared")).unwrap().repo_key.is_empty());
        std::fs::rename(&original, &replacement).unwrap();
        std::fs::rename(&saved, &original).unwrap();

        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        assert_eq!(preview.target_path, selected_path);
        assert!(preview.skipped_paths.contains(&agent));
        assert!(preview.managed_paths.contains(&manager.source_dir.join("alpha")));
        manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        assert_eq!(std::fs::read_to_string(original.join("details.txt")).unwrap(), "new details");
        assert_eq!(std::fs::read_to_string(agent.join("details.txt")).unwrap(), "old details");
        assert_eq!(skill_path_identity(&agent).unwrap(), agent_identity);
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/details.txt")).unwrap(), "new details");
        assert_ne!(skill_path_identity(&manager.source_dir.join("alpha")).unwrap(), source_identity);
    }

    #[test]
    fn update_synchronizes_managed_copies_and_restores_the_whole_group() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        assert!(preview.conflicts.is_empty());
        let state = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
        let paths = [temp.path().join("shared/alpha"), manager.source_dir.join("alpha"),
            temp.path().join("agent/alpha")];
        for path in &paths {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "new details");
        }
        assert!(is_managed_copy(&paths[2]));
        let group = state.update_backup_groups.values().next().unwrap();
        assert_eq!(group.backup_ids.len(), 3);
        assert_eq!(group.published.len(), 3);
        manager.restore_backup(&group.backup_ids[0]).unwrap();
        for path in &paths {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "old details");
        }
    }

    #[test]
    fn adopted_external_skill_updates_its_original_after_source_association() {
        for shared_original in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let shared = temp.path().join("shared");
            let agent = temp.path().join("agent");
            let other = temp.path().join("other");
            let manager = SkillsManager::new(
                temp.path().join("library"), temp.path().join("backups"),
                temp.path().join("state.json"), temp.path().join("codex"),
            ).with_agent_dirs(vec![("claude".into(), agent.clone()), ("grok".into(), other.clone())]);
            let original = if shared_original { shared.join("alpha") } else { agent.join("alpha") };
            std::fs::create_dir_all(&original).unwrap();
            std::fs::write(original.join(SKILL_MANIFEST_FILE), "---\nname: alpha\n---\nold").unwrap();
            let repo = SkillRepo {
                owner: "example".into(), name: "skills".into(), branch: "main".into(),
                subdir: String::new(), enabled: true,
            };
            manager.save_repo(repo.clone(), None).unwrap();
            if shared_original {
                manager.import_from_local_directory("alpha", &original).unwrap();
                let mut state = manager.load_state();
                state.installed.get_mut("alpha").unwrap().update_owner.clear();
                manager.save_state_unlocked(&state).unwrap();
            }
            manager.set_enabled_for("grok", "alpha", true).unwrap();
            let manager = manager.with_shared_skills_dir(&shared);
            assert!(manager.load_state().installed["alpha"].update_owner.is_empty());
            // 模拟旧版本采纳时错误写入的管理归属。
            let mut state = manager.load_state();
            state.installed.get_mut("alpha").unwrap().update_owner = "manager".into();
            state.installed.get_mut("alpha").unwrap().repo_key = repo.key();
            state.installed.get_mut("alpha").unwrap().repo_path = "skills/alpha".into();
            manager.save_state_unlocked(&state).unwrap();
            assert!(manager.capture_download_guard("alpha", &repo.key(), false).is_err());

            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            writer.start_file("skills-main/skills/alpha/SKILL.md",
                zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(b"---\nname: alpha\n---\nnew").unwrap();
            let zip = writer.finish().unwrap().into_inner();
            let remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap()
                .into_iter().find(|item| item.repo_path == "skills/alpha").unwrap();
            let matches = manager.preview_repository_matches(&repo, &[remote.clone()], Some("alpha")).unwrap();
            let found = matches.matches.iter().find(|item| item.repo_path == "skills/alpha").unwrap();
            let result = manager.apply_repository_matches(&[SkillRepositoryMatchSelection {
                preview_id: matches.preview_id, local_id: "alpha".into(),
                repo_key: repo.key(), repo_path: found.repo_path.clone(), manual: false, local_path: None,
            }], &[remote.clone()]).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            assert!(manager.load_state().installed["alpha"].update_owner.is_empty());
            assert!(manager.capture_download_guard("alpha", &repo.key(), false).is_err());
            let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
            assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
            manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap();
            for path in [&original, &manager.source_dir.join("alpha"), &other.join("alpha")] {
                assert!(std::fs::read_to_string(path.join(SKILL_MANIFEST_FILE)).unwrap().ends_with("new"));
            }
        }
    }

    #[test]
    fn changed_contents_after_preview_are_not_overwritten() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        std::fs::write(temp.path().join("agent/alpha/details.txt"), "user edit").unwrap();
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(temp.path().join("agent/alpha/details.txt")).unwrap(), "user edit");
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "old details");
    }

    #[test]
    fn nested_managed_marker_change_after_association_blocks_update() {
        let (temp, manager, _, remote, zip) = fixture();
        for path in [temp.path().join("shared/alpha"), manager.source_dir.join("alpha"),
            temp.path().join("agent/alpha")] {
            let nested = path.join("nested");
            std::fs::create_dir(&nested).unwrap();
            std::fs::write(nested.join(MANAGED_COPY_MARKER), "user content").unwrap();
        }
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        assert!(preview.conflicts.iter().any(|item| item.contains("原件自上次关联或更新后已修改")));
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/nested")
            .join(MANAGED_COPY_MARKER)).unwrap(), "user content");
    }

    #[test]
    fn changed_contents_during_commit_are_restored_without_publication() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        let edited = manager.source_dir.join("alpha/details.txt");
        BEFORE_UPDATE_BACKUP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                std::fs::write(edited, "late user edit").unwrap();
            }));
        });
        let error = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap_err();
        assert!(format!("{error:#}").contains("更新期间 Skill 内容已改变"));
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/details.txt")).unwrap(), "late user edit");
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "old details");
        assert_eq!(std::fs::read_to_string(temp.path().join("agent/alpha/details.txt")).unwrap(), "old details");
        assert!(manager.load_state().update_backup_groups.is_empty());
    }

    #[test]
    fn state_save_failure_restores_original_and_all_managed_copies() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        FAIL_SKILL_STATE_SAVE.with(|fail| fail.set(true));
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        FAIL_SKILL_STATE_SAVE.with(|fail| fail.set(false));
        for path in [temp.path().join("shared/alpha"), manager.source_dir.join("alpha"),
            temp.path().join("agent/alpha")] {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "old details");
        }
        assert!(manager.load_state().update_backup_groups.is_empty());
    }

    #[test]
    fn changed_remote_after_preview_is_not_installed() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        let mut changed = remote;
        changed.content_hash = "different".into();
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &changed).is_err());
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "old details");
    }

    #[test]
    fn failed_archive_staging_cleans_only_created_directories() {
        let (temp, manager, _, remote, zip) = fixture();
        let unrelated = temp.path().join("shared/.codex-plus-update-alpha-unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        std::fs::write(unrelated.join("user.txt"), "preserve").unwrap();
        for invalid_zip in [true, false] {
            let mut selected = remote.clone();
            let bytes = if invalid_zip {
                b"invalid archive".to_vec()
            } else {
                selected.content_hash = "different".into();
                zip.clone()
            };
            let preview = manager.preview_skill_in_place_update("alpha", selected.clone(), bytes).unwrap();
            assert!(manager.execute_skill_in_place_update(&preview.preview_id, &selected).is_err());
            for parent in [temp.path().join("shared"), manager.source_dir.clone(), temp.path().join("agent")] {
                let staging = std::fs::read_dir(parent).unwrap().map(|entry| entry.unwrap().path())
                    .filter(|path| path.file_name().unwrap().to_string_lossy().starts_with(".codex-plus-update-"))
                    .collect::<Vec<_>>();
                assert!(staging.iter().all(|path| path == &unrelated));
            }
            assert_eq!(std::fs::read_to_string(unrelated.join("user.txt")).unwrap(), "preserve");
            assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "old details");
        }
    }

    #[test]
    fn replaced_staging_directory_is_not_published_or_deleted() {
        let (temp, manager, _, remote, zip) = fixture();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        let parent = temp.path().join("shared");
        let held = temp.path().join("moved-staging");
        BEFORE_UPDATE_BACKUP.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                let staging = std::fs::read_dir(&parent).unwrap().map(|entry| entry.unwrap().path())
                    .find(|path| path.file_name().unwrap().to_string_lossy().starts_with(".codex-plus-update-"))
                    .unwrap();
                std::fs::rename(&staging, &held).unwrap();
                std::fs::create_dir(&staging).unwrap();
                std::fs::write(staging.join("user.txt"), "preserve").unwrap();
            }));
        });
        let error = manager.execute_skill_in_place_update(&preview.preview_id, &remote).unwrap_err();
        assert!(format!("{error:#}").contains("暂存目录已被替换"));
        let replacement = std::fs::read_dir(temp.path().join("shared")).unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.file_name().unwrap().to_string_lossy().starts_with(".codex-plus-update-"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(replacement.join("user.txt")).unwrap(), "preserve");
        assert_eq!(std::fs::read_to_string(temp.path().join("shared/alpha/details.txt")).unwrap(), "old details");
    }

    #[test]
    fn git_worktree_at_original_path_cannot_be_replaced() {
        let (temp, manager, _, remote, zip) = fixture();
        let git = temp.path().join("shared/alpha/.git");
        std::fs::write(&git, "gitdir: ../gitdir").unwrap();
        let preview = manager.preview_skill_in_place_update("alpha", remote.clone(), zip).unwrap();
        assert!(preview.conflicts.iter().any(|message| message.contains("Git")));
        assert!(manager.execute_skill_in_place_update(&preview.preview_id, &remote).is_err());
        assert_eq!(std::fs::read_to_string(git).unwrap(), "gitdir: ../gitdir");
    }
}
