use super::*;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSkillVersion {
    pub root: String,
    path: PathBuf,
    modified_at_ms: u128,
    pub revision: String,
    linked: bool,
    managed: bool,
}

impl SkillsManager {
    fn version_root(&self, kind: &str) -> anyhow::Result<PathBuf> {
        match kind {
            "source" => Ok(self.source_dir.clone()),
            "codex" => Ok(self.linked_dir()),
            "shared" => self.shared_skills_dir().map(Path::to_path_buf).context("未配置共享目录"),
            agent => self.agent_dir(agent),
        }
    }

    fn local_version(&self, id: &str, root: &str) -> anyhow::Result<LocalSkillVersion> {
        let path = self.version_root(root)?.join(id);
        let actual = path.canonicalize()?;
        anyhow::ensure!(!id.starts_with('.') && actual.join(SKILL_MANIFEST_FILE).is_file(), "不是可管理的本地 Skill");
        let identity = skill_path_identity(&path)?;
        let digest = skill_directory_digest(&actual)?.context("Skill 版本已不存在")?;
        let files = self.skill_file_times(id, root, None)?;
        let modified_at_ms = files.iter().filter(|f| f.path != MANAGED_COPY_MARKER)
            .map(|f| f.modified_at_ms).max().context("Skill 目录为空")?.max(0) as u128;
        Ok(LocalSkillVersion { root: root.into(), revision: format!("{identity:?}:{digest:x?}"),
            linked: metadata_is_link(&std::fs::symlink_metadata(&path)?), managed: is_managed_copy(&path), path, modified_at_ms })
    }

    pub fn local_versions(&self, id: &str) -> anyhow::Result<Vec<LocalSkillVersion>> {
        validate_skill_id(id)?;
        let mut roots = vec!["source", "codex"];
        if self.shared_skills_dir().is_some() { roots.push("shared"); }
        roots.extend(self.agent_dirs.iter().map(|(agent, _)| agent.as_str()));
        let mut versions = Vec::new();
        for root in roots {
            if self.version_root(root)?.join(id).join(SKILL_MANIFEST_FILE).try_exists()? {
                versions.push(self.local_version(id, root)?);
            }
        }
        Ok(versions)
    }

    pub fn archive_local_version(&self, id: &str, root: &str, revision: &str) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        anyhow::ensure!(root != "source", "受控工作源请通过卸载或受控源归档管理");
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(id, &state)?;
        let version = self.local_version(id, root)?;
        anyhow::ensure!(!version.linked && version.revision == revision, "版本已改变或是目录链接，请重新选择");
        self.ensure_duplicate_has_no_discovered_references(&version.path)?;
        let identity = skill_path_identity(&version.path)?;
        let backup = self.backup_directory_checked(id, &version.path, identity)?;
        let saved = (|| -> anyhow::Result<()> {
            state.backup_origins.insert(self.backup_id_for_path(&backup)?, SkillBackupOrigin {
                path: version.path.clone(), purpose: "local-version".into(),
            });
            self.save_state_unlocked(&state)
        })();
        if let Err(error) = saved {
            restore_skill_backup(&backup, &version.path, identity)
                .with_context(|| format!("{error:#}；原版本保留于 {}", backup.display()))?;
            return Err(error);
        }
        coordination.changed_nested_skill(self.coordination_scope(), id);
        Ok(())
    }

    pub fn reconcile_managed_versions(&self, id: &str, root: &str, revisions: BTreeMap<String, String>) -> anyhow::Result<()> {
        self.reconcile_versions(id, Some(root), revisions)
    }

    pub fn sync_installed_copies(&self, id: &str, revisions: BTreeMap<String, String>) -> anyhow::Result<()> {
        self.reconcile_versions(id, None, revisions)
    }

    fn reconcile_versions(&self, id: &str, choice: Option<&str>, revisions: BTreeMap<String, String>) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(id, &state)?;
        let versions = self.local_versions(id)?;
        anyhow::ensure!(versions.len() == revisions.len() && versions.iter().all(|v| revisions.get(&v.root) == Some(&v.revision)), "版本列表已改变，请重新选择");
        let source = self.source_dir.join(id);
        let managed = self.unclaimed_managed_source(id, &state).is_some() && !state.instance_records.contains_key(id) && versions.iter().all(|v| v.root == "source" || v.root != "shared" && (is_managed_copy(&v.path) || is_managed_link(&v.path, &source)));
        let selected = if let Some(root) = choice {
            anyhow::ensure!(root != "source", "请选择实际安装的版本");
            versions.iter().find(|v| v.root == root).context("所选版本不存在")?
        } else {
            anyhow::ensure!(managed, "来源已改变，请重新查看安装位置");
            let mut copies = versions.iter().filter(|v| v.root != "source");
            let first = copies.next().context("没有本地安装")?;
            let expected = skill_directory_digest(&first.path.canonicalize()?)?.context("安装位置已消失")?;
            for copy in copies { anyhow::ensure!(skill_directory_digest(&copy.path.canonicalize()?)? == Some(expected), "各 Agent 安装不一致，请选择要保留的版本"); }
            first
        };
        if !managed {
            let keep = selected.path.canonicalize()?;
            let mut targets = BTreeMap::new();
            for v in versions.iter().filter(|v| v.root != "source") {
                if v.path.canonicalize()? == keep { continue; }
                anyhow::ensure!(!v.linked, "其他位置仍为目录链接，请先停用该位置");
                self.ensure_duplicate_has_no_discovered_references(&v.path)?;
                targets.insert(skill_path_identity(&v.path)?, v.path.clone());
            }
            let mut moved = Vec::new();
            let result = (|| -> anyhow::Result<()> {
                for (identity, path) in targets {
                    let backup = self.backup_directory_checked(id, &path, identity)?;
                    moved.push((path.clone(), backup.clone(), identity));
                    state.backup_origins.insert(self.backup_id_for_path(&backup)?, SkillBackupOrigin { path, purpose: "local-version".into() });
                }
                self.save_state_unlocked(&state)
            })();
            if let Err(error) = result {
                let mut failures = Vec::new();
                for (p, b, ident) in moved.iter().rev() { if let Err(e) = restore_skill_backup(b, p, *ident) { failures.push(format!("{}: {e:#}", b.display())); } }
                anyhow::bail!("{error:#}{}", failures.join("；"));
            }
            coordination.changed_nested_skill(self.coordination_scope(), id);
            return Ok(());
        }
        let mut installed = state.installed[id].clone();
        installed.content_hash = format!("local:{}", selected.revision);
        let staging = self.staging_dir(id);
        std::fs::create_dir(&staging)?;
        let result = (|| -> anyhow::Result<()> {
            copy_dir_all_inner(&selected.path, &staging, true, true)?;
            anyhow::ensure!(skill_directory_digest(&staging)? == skill_directory_digest(&selected.path.canonicalize()?)?, "所选内容已改变");
            let scope = self.coordination_scope();
            let snapshot = AgentCopiesSnapshot { generation: coordination.mutation_generation(&scope, id), scope,
                paths: self.agent_copy_paths(id)?, source_identity: Some(skill_path_identity(&source)?) };
            self.commit_staged_skill_unlocked(&staging, installed, is_linked(&self.linked_dir().join(id)), true, Some(&snapshot), false, None, &mut coordination)?;
            Ok(())
        })();
        result.map_err(|error| retain_staging_context(error, &staging))
    }

    pub fn keep_local_versions(&self, id: &str, revisions: BTreeMap<String, String>) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(id, &state)?;
        let versions = self.local_versions(id)?;
        let mut markers = Vec::new();
        for (root, revision) in revisions {
            let v = versions.iter().find(|v| v.root == root).context("安装位置已消失")?;
            anyhow::ensure!(root != "source" && v.revision == revision && !v.linked, "版本已改变或为共享链接，请重新选择");
            let path = v.path.canonicalize()?;
            let identity = skill_path_identity(&path)?;
            if v.managed {
                let marker = v.path.join(MANAGED_COPY_MARKER);
                let metadata = std::fs::symlink_metadata(&marker)?;
                anyhow::ensure!(metadata.is_file() && !metadata_is_link(&metadata) && metadata.len() == 0, "受控副本标记已改变");
                if !markers.contains(&marker) { markers.push(marker); }
            }
            let (name, description) = read_skill_manifest(&path.join(SKILL_MANIFEST_FILE), id);
            let installed = InstalledSkill { id: id.into(), name, description, repo_key: LOCAL_IMPORT_REPO_KEY.into(), repo_path: String::new(), content_hash: String::new(), installed_at: current_unix_timestamp_string(), verified_cc_switch: false, install_channel: "local".into(), original_dir: Some(path.clone()), update_owner: String::new() };
            let records = state.instance_records.entry(id.into()).or_default();
            for record in records.values_mut() { record.managed_copies.retain(|copy| copy.identity != identity); }
            records.insert(path.clone(), SkillInstanceRecord { installed: installed.clone(), target: SkillInstanceTarget { path, identity }, managed_copies: vec![] });
            if state.instance_targets.get(id).is_some_and(|v| v.identity == identity) { state.installed.insert(id.into(), installed); }
        }
        let mut removed = Vec::new();
        let saved = (|| -> anyhow::Result<()> {
            for marker in markers { std::fs::remove_file(&marker)?; removed.push(marker); }
            self.save_state_unlocked(&state)
        })();
        if let Err(error) = saved {
            for marker in removed { std::fs::File::create_new(&marker).with_context(|| format!("{error:#}；未能恢复标记 {}", marker.display()))?; }
            return Err(error);
        }
        coordination.changed_nested_skill(self.coordination_scope(), id);
        Ok(())
    }

    pub fn export_skill_backup(&self, backup_id: &str, directory: &Path) -> anyhow::Result<()> {
        let _guard = skills_coordination();
        let (backup, name) = self.resolve_backup(backup_id)?;
        let (id, _) = split_backup_id(&name);
        validate_skill_id(&id)?;
        anyhow::ensure!(!metadata_is_link(&std::fs::symlink_metadata(&backup)?) && backup.join(SKILL_MANIFEST_FILE).is_file(), "不是可导出的 Skill 备份");
        if directory == self.linked_dir() || self.agent_dirs.iter().any(|(_, root)| root == directory) {
            std::fs::create_dir_all(directory)?;
        }
        publish_local_version(&backup, directory, &id)
    }

    pub fn enable_local_version(&self, id: &str, root: &str, revision: &str, agent: &str) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let mut coordination = skills_coordination();
        self.ensure_skill_name_not_aliased(id, &self.load_state_checked_unlocked()?)?;
        let chosen = self.local_version(id, root)?;
        anyhow::ensure!(chosen.revision == revision, "版本已改变，请重新选择");
        let target = if agent == "codex" { self.linked_dir() } else { self.agent_dir(agent)? };
        if agent == "codex" {
            anyhow::ensure!(!self.shared_skills_dir().is_some_and(|dir| dir.join(id).join(SKILL_MANIFEST_FILE).is_file()), "Codex 已有共享安装，请先处理共享位置");
            let manifest = target.join(id).join(SKILL_MANIFEST_FILE);
            let (name, _) = read_skill_manifest(&chosen.path.join(SKILL_MANIFEST_FILE), id);
            let config = self.read_native_skill_config()?;
            let matched = config.name_for_matching(&manifest, &name)?;
            anyhow::ensure!(config.enabled(&manifest, &matched)?, "此位置被 Codex 原生规则停用，请先调整启用状态");
        }
        std::fs::create_dir_all(&target)?;
        publish_local_version(&chosen.path, &target, id)?;
        coordination.changed_nested_skill(self.coordination_scope(), id);
        Ok(())
    }
}

fn publish_local_version(source: &Path, directory: &Path, id: &str) -> anyhow::Result<()> {
    let directory = directory.canonicalize()?;
    let source = source.canonicalize()?;
    anyhow::ensure!(!directory.starts_with(&source), "目标不能位于原版本内");
    let target = directory.join(id);
    anyhow::ensure!(!target.try_exists()?, "目标已存在，未覆盖：{}", target.display());
    let staging = directory.join(format!(".staging-{id}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    let result = (|| -> anyhow::Result<()> {
        copy_dir_all_inner(&source, &staging, true, true)?;
        anyhow::ensure!(skill_directory_digest(&staging)? == skill_directory_digest(&source)?, "复制期间原版本发生变化");
        move_path_no_replace(&staging, &target)?;
        Ok(())
    })();
    result.map_err(|error| retain_staging_context(error, &staging))
}
