//! 将本地 Skill 与仓库路径关联；关联只更新记录，不接管技能目录。
use super::*;

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
    pub conflict: Option<String>,
    pub local_paths: Vec<PathBuf>,
    pub local_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillSourceEvidence {
    pub repo: String,
    pub kind: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchPreview {
    pub preview_id: String,
    pub matches: Vec<SkillRepositoryMatch>,
    pub manual_candidates: Vec<SkillRepositoryMatch>,
    pub examined: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchSelection {
    pub preview_id: String,
    pub local_id: String,
    pub repo_key: String,
    pub repo_path: String,
    #[serde(default)]
    pub local_path: Option<PathBuf>,
    #[serde(default)]
    pub manual: bool,
}

#[derive(Clone)]
struct RepositoryMatchSnapshot {
    state_path: PathBuf,
    scope: CoordinationScope,
    matches: Vec<SkillRepositoryMatch>,
    manual_candidates: Vec<SkillRepositoryMatch>,
}

fn repository_match_previews() -> MutexGuard<'static, std::collections::VecDeque<(String, RepositoryMatchSnapshot)>> {
    static PREVIEWS: OnceLock<Mutex<std::collections::VecDeque<(String, RepositoryMatchSnapshot)>>> = OnceLock::new();
    PREVIEWS.get_or_init(Mutex::default).lock()
        .expect("Skill 来源匹配预览锁中毒，拒绝继续修改")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchApplied {
    pub local_id: String,
    pub from_repo_key: String,
    pub to_repo_key: String,
    pub local_path: Option<PathBuf>,
    pub instance_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepositoryMatchApply {
    pub applied: Vec<SkillRepositoryMatchApplied>,
    pub errors: Vec<String>,
}

struct MatchSnapshot {
    value: String,
    local_hash: String,
    name: String,
    description: String,
    installed: Option<InstalledSkill>,
    scope: CoordinationScope,
    skill_generation: u64,
    repo_generation: u64,
    original_dir: Option<PathBuf>,
    instance_target: Option<SkillInstanceTarget>,
    instance_records: Option<BTreeMap<PathBuf, SkillInstanceRecord>>,
    selected_identity: Option<SkillPathIdentity>,
    selected_digest: Option<[u8; 32]>,
    selected_managed_source: bool,
    local_paths: Vec<PathBuf>,
    conflict: Option<String>,
}

fn comparable_name(value: &str) -> String {
    value.trim().to_lowercase().chars().filter(|ch| ch.is_alphanumeric()).collect()
}

fn github_repositories(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (offset, _) in text.match_indices("github.com") {
        let prefix = text[..offset].chars().next_back();
        if !matches!(prefix, Some('/') | Some('@')) {
            continue;
        }
        let rest = &text[offset + "github.com".len()..];
        let Some(rest) = rest.strip_prefix('/').or_else(|| rest.strip_prefix(':')) else {
            continue;
        };
        let mut parts = rest.split('/');
        let Some(owner) = parts.next() else { continue };
        let Some(repo) = parts.next() else { continue };
        let repo = repo.trim_end_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' && ch != '.')
            .trim_end_matches(".git");
        let valid = |part: &str| !part.is_empty()
            && part.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'));
        if valid(owner) && valid(repo) && owner != "." && owner != ".." && repo != "." && repo != ".." {
            found.insert(format!("{owner}/{repo}"));
        }
    }
    found
}

fn json_repository_values(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::String(value) => {
            out.extend(github_repositories(value));
            let trimmed = value.trim().trim_start_matches("github:");
            if !trimmed.contains("://") && !trimmed.contains('@') && trimmed.matches('/').count() == 1 {
                out.extend(github_repositories(&format!("https://github.com/{trimmed}")));
            }
        }
        Value::Array(values) => values.iter().for_each(|value| json_repository_values(value, out)),
        Value::Object(values) => values.values().for_each(|value| json_repository_values(value, out)),
        _ => {}
    }
}

fn plausible_match(id: &str, name: &str, remote: &RemoteSkill, local_hash: &str) -> (bool, bool) {
    let exact = !remote.content_hash.is_empty() && remote.content_hash == local_hash;
    let remote_dir = if remote.repo_path == "." {
        remote.repo_key.split('/').nth(1).unwrap_or("").split('@').next().unwrap_or("")
    } else {
        remote.repo_path.rsplit('/').next().unwrap_or("")
    };
    let names = [comparable_name(id), comparable_name(name)];
    let targets = [comparable_name(remote_dir), comparable_name(&remote.name)];
    (exact || names.iter().any(|name| !name.is_empty() && targets.contains(name)), exact)
}

/// 复用远端清单的哈希格式：相对路径与 Git blob ID 的排序列表。
pub(super) fn local_repository_hash(root: &Path) -> anyhow::Result<String> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<(String, String)>) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if directory == root && entry.file_name() == MANAGED_COPY_MARKER {
                continue;
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!("Skill 包含无法比较的符号链接：{}", path.display());
            }
            if metadata.is_dir() {
                visit(root, &path, files)?;
            } else if metadata.is_file() {
                let relative = path.strip_prefix(root)?.components()
                    .map(|part| part.as_os_str().to_str().context("Skill 文件名无法比较"))
                    .collect::<anyhow::Result<Vec<_>>>()?.join("/");
                let mut file = std::fs::File::open(&path)?;
                let mut blob = sha1::Sha1::new();
                blob.update(format!("blob {}\0", metadata.len()).as_bytes());
                let mut bytes_read = 0u64;
                let mut buffer = [0u8; 64 * 1024];
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    bytes_read += count as u64;
                    blob.update(&buffer[..count]);
                }
                let after = std::fs::metadata(&path)?;
                anyhow::ensure!(
                    bytes_read == metadata.len() && after.len() == metadata.len()
                        && after.modified().ok() == metadata.modified().ok(),
                    "读取期间 Skill 文件发生变化：{}", path.display()
                );
                files.push((relative, format!("{:x}", blob.finalize())));
            } else {
                anyhow::bail!("Skill 包含无法比较的特殊文件：{}", path.display());
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    files.sort();
    let mut hash = Sha256::new();
    for (path, blob) in files {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(blob.as_bytes());
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl SkillsManager {
    /// 线索仅供用户核对，不表示来源已验证，也不会修改安装状态。
    pub fn discover_skill_source_evidence(&self, id: &str) -> anyhow::Result<Vec<SkillSourceEvidence>> {
        self.discover_skill_source_evidence_inner(id, None)
    }

    pub fn discover_skill_source_evidence_for_instance(
        &self,
        id: &str,
        local_path: &Path,
    ) -> anyhow::Result<Vec<SkillSourceEvidence>> {
        self.discover_skill_source_evidence_inner(id, Some(local_path))
    }

    fn discover_skill_source_evidence_inner(
        &self,
        id: &str,
        local_path: Option<&Path>,
    ) -> anyhow::Result<Vec<SkillSourceEvidence>> {
        validate_skill_id(id)?;
        let mut evidence = BTreeSet::<(String, String, PathBuf)>::new();
        let mut targets = BTreeSet::new();
        let managed_root = self.source_dir.canonicalize().ok();
        for root in self.repository_match_roots() {
            let path = root.join(id);
            if path.join(SKILL_MANIFEST_FILE).is_file() {
                let target = path.canonicalize()?;
                if local_path.is_none_or(|selected| (selected == path || selected == target)
                    && (root == self.source_dir
                        || !managed_root.as_ref().is_some_and(|managed| target.starts_with(managed))))
                {
                    targets.insert(target);
                }
            }
        }
        if let Some(selected) = local_path {
            anyhow::ensure!(!targets.is_empty() && std::fs::symlink_metadata(selected)?.file_type().is_dir()
                && !is_managed_copy(selected), "所选路径不是已发现的实体 Skill 原件");
        }
        for target in targets {
            let mut add_text = |path: &Path, kind: &str| -> anyhow::Result<()> {
                let bytes = match std::fs::read(path) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                    Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
                };
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    for repo in github_repositories(text) {
                        evidence.insert((repo, kind.to_string(), path.to_path_buf()));
                    }
                }
                Ok(())
            };
            add_text(&target.join(SKILL_MANIFEST_FILE), "manifest-link")?;
            let scripts = target.join("scripts");
            match std::fs::read_dir(&scripts) {
                Ok(entries) => for entry in entries {
                    let entry = entry?;
                    let path = entry.path();
                    if entry.file_type()?.is_file()
                        && path.extension().and_then(|ext| ext.to_str()).map_or(true, |ext|
                            matches!(ext.to_ascii_lowercase().as_str(), "sh" | "cmd" | "bat" | "ps1"))
                        && entry.metadata()?.len() <= 1024 * 1024
                    {
                        add_text(&path, "script-link")?;
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            drop(add_text);
            for (path, kind) in [
                (target.join("package.json"), "package"),
                (target.join(".skill-receipt.json"), "receipt"),
                (target.join(".install-receipt.json"), "receipt"),
            ] {
                let bytes = match std::fs::read(&path) {
                    Ok(bytes) => bytes,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                let value: Value = serde_json::from_slice(&bytes)
                    .with_context(|| format!("解析 {} 失败", path.display()))?;
                let mut repos = BTreeSet::new();
                if kind == "package" {
                    for key in ["repository", "homepage"] {
                        if let Some(value) = value.get(key) { json_repository_values(value, &mut repos); }
                    }
                } else {
                    json_repository_values(&value, &mut repos);
                }
                for repo in repos { evidence.insert((repo, kind.into(), path.clone())); }
            }
            for directory in target.ancestors().take(4) {
                for name in ["skills-lock.json", ".skills-lock.json", ".skill-lock.json"] {
                    let path = directory.join(name);
                    let bytes = match std::fs::read(&path) {
                        Ok(bytes) => bytes,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error.into()),
                    };
                    let value: Value = serde_json::from_slice(&bytes)
                        .with_context(|| format!("解析 {} 失败", path.display()))?;
                    if let Some(record) = value.get("skills").and_then(|items| items.get(id)) {
                        let mut repos = BTreeSet::new();
                        json_repository_values(record, &mut repos);
                        for repo in repos { evidence.insert((repo, "lock".into(), path.clone())); }
                    }
                }
            }
            let git = std::process::Command::new("git")
                .args(["-C"]).arg(&target)
                .args(["remote", "get-url", "origin"]).output();
            match git {
                Ok(output) if output.status.success() => {
                    for repo in github_repositories(&String::from_utf8_lossy(&output.stdout)) {
                        evidence.insert((repo, "git-remote".into(), target.clone()));
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(evidence.into_iter().map(|(repo, kind, path)| SkillSourceEvidence { repo, kind, path }).collect())
    }

    fn repository_match_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.source_dir.clone(), self.linked_dir()];
        if let Some(root) = self.shared_skills_dir() {
            roots.push(root.to_path_buf());
        }
        roots.extend(self.agent_dirs.iter().map(|(_, dir)| dir.clone()));
        roots
    }

    fn repository_match_ids(&self) -> anyhow::Result<BTreeSet<String>> {
        let state = self.load_state_checked_unlocked()?;
        let mut ids: BTreeSet<_> = state.installed.keys().cloned().collect();
        for root in self.repository_match_roots() {
            let entries = match std::fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", root.display())),
            };
            for entry in entries {
                let entry = entry?;
                let id = entry.file_name().into_string()
                    .map_err(|_| anyhow::anyhow!("Skill 目录名无法识别"))?;
                if !id.starts_with('.') && entry.path().join(SKILL_MANIFEST_FILE).is_file() {
                    ids.insert(id);
                }
            }
        }
        Ok(ids)
    }

    fn repository_match_snapshot(&self, id: &str, repo: &SkillRepo, selected: Option<&Path>) -> anyhow::Result<MatchSnapshot> {
        validate_skill_id(id)?;
        let (scope, skill_generation, repo_generation, installed, instance_target, instance_records, managed_source) = {
            let coordination = skills_coordination();
            let state = self.load_state_checked_unlocked()?;
            anyhow::ensure!(state.repos.contains(repo), "仓库配置已变化，请重新匹配");
            let scope = self.coordination_scope();
            let skill_generation = coordination.mutation_generation(&scope, id);
            let repo_generation = coordination.repo_change_generation(&scope, &repo.key());
            (scope, skill_generation, repo_generation, state.installed.get(id).cloned(), state.instance_targets.get(id).cloned(),
                state.instance_records.get(id).cloned(), self.unclaimed_managed_source(id, &state))
        };

        let mut snapshot = Sha256::new();
        snapshot.update(serde_json::to_vec(&installed)?);
        snapshot.update(serde_json::to_vec(&instance_target)?);
        snapshot.update(serde_json::to_vec(&instance_records)?);
        snapshot.update(serde_json::to_vec(&selected)?);
        snapshot.update(serde_json::to_vec(repo)?);
        snapshot.update(skill_generation.to_le_bytes());
        snapshot.update(repo_generation.to_le_bytes());
        let mut first: Option<(String, String, String, [u8; 32])> = None;
        let mut originals = BTreeMap::<PathBuf, PathBuf>::new();
        let mut divergent = false;
        let mut selected_identity = None;
        let mut selected_digest = None;
        let mut selected_managed_source = false;
        let managed_root = self.source_dir.canonicalize().ok();
        for root in self.repository_match_roots() {
            let path = root.join(id);
            snapshot.update(path.to_string_lossy().as_bytes());
            match std::fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    snapshot.update(b"missing");
                }
                Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
                Ok(_) => {
                    anyhow::ensure!(path.join(SKILL_MANIFEST_FILE).is_file(),
                        "同名路径不是有效 Skill：{}", path.display());
                    let identity = skill_path_identity(&path)?;
                    let target = path.canonicalize()?;
                    let digest = skill_directory_digest(&target)?.context("Skill 内容已不存在")?;
                    snapshot.update(identity.device.to_le_bytes());
                    snapshot.update(identity.file.to_le_bytes());
                    snapshot.update(target.to_string_lossy().as_bytes());
                    snapshot.update(digest);
                    if root != self.source_dir
                        && !managed_root.as_ref().is_some_and(|managed| target.starts_with(managed))
                        && (!target.join(MANAGED_COPY_MARKER).exists()
                            || instance_target.as_ref().is_some_and(|recorded|
                                recorded.path.canonicalize().ok().as_ref() == Some(&target)
                                    && skill_path_identity(&recorded.path).ok() == Some(recorded.identity)))
                    {
                        originals.entry(target.clone()).or_insert_with(|| path.clone());
                    }
                    if selected.is_some_and(|selected| selected == path || selected == target)
                        && (root == self.source_dir
                            || !managed_root.as_ref().is_some_and(|managed| target.starts_with(managed)))
                    {
                        let is_managed_source = root == self.source_dir;
                        if is_managed_source {
                            anyhow::ensure!(managed_source.as_ref() == Some(&path),
                                "受控源属于其他安装位置，请选择该安装位置关联");
                        }
                        let selected_path = selected.unwrap();
                        let recorded_original = instance_target.as_ref().is_some_and(|recorded|
                            recorded.path.canonicalize().ok().as_ref() == Some(&target)
                                && skill_path_identity(&recorded.path).ok() == Some(recorded.identity));
                        anyhow::ensure!(std::fs::symlink_metadata(selected_path)?.file_type().is_dir()
                            && (!is_managed_copy(selected_path) || recorded_original),
                            "所选路径不是可单独管理的实体原件");
                        let (name, description) = read_skill_manifest(&selected_path.join(SKILL_MANIFEST_FILE), id);
                        selected_identity = Some(skill_path_identity(selected_path)?);
                        selected_digest = Some(digest);
                        selected_managed_source = is_managed_source;
                        let local_hash = local_repository_hash(selected_path)?;
                        anyhow::ensure!(skill_directory_digest(selected_path)? == Some(digest),
                            "读取期间 Skill 内容已变化，请重新匹配");
                        first = Some((name, description, local_hash, digest));
                    } else if selected.is_some() {
                        continue;
                    } else if let Some((_, _, _, previous)) = &first {
                        divergent |= *previous != digest;
                    } else {
                        let (name, description) = read_skill_manifest(&target.join(SKILL_MANIFEST_FILE), id);
                        let local_hash = local_repository_hash(&target)?;
                        anyhow::ensure!(skill_directory_digest(&target)? == Some(digest),
                            "读取期间 Skill 内容已变化，请重新匹配");
                        first = Some((name, description, local_hash, digest));
                    }
                }
            }
            snapshot.update([0]);
        }
        let (name, description, local_hash, _) = first.context("本地 Skill 已不存在")?;
        let recorded = installed.as_ref().and_then(|item| item.original_dir.as_ref())
            .and_then(|path| path.canonicalize().ok())
            .filter(|path| originals.contains_key(path));
        let original_dir = if selected_managed_source {
            None
        } else {
            selected.map(Path::to_path_buf).or_else(|| recorded.and_then(|path| originals.get(&path).cloned())
                .or_else(|| (originals.len() == 1).then(|| originals.values().next().cloned()).flatten()))
        };
        let conflict = if selected.is_some() {
            None
        } else if divergent {
            Some("同名 Skill 副本内容不同，请先确定原件".to_string())
        } else if originals.len() > 1 {
            Some("存在多个独立的本地原件，请先确定原件".to_string())
        } else {
            None
        };
        let coordination = skills_coordination();
        let state = self.load_state_checked_unlocked()?;
        anyhow::ensure!(
            coordination_scope_is_current(&scope, &self.coordination_scope())
                && coordination.mutation_generation(&scope, id) == skill_generation
                && coordination.repo_change_generation(&scope, &repo.key()) == repo_generation
                && state.repos.contains(repo)
                && state.installed.get(id) == installed.as_ref()
                && state.instance_targets.get(id) == instance_target.as_ref()
                && state.instance_records.get(id) == instance_records.as_ref(),
            "Skill 或仓库在匹配期间发生变化，请重试"
        );
        Ok(MatchSnapshot {
            value: format!("{:x}", snapshot.finalize()),
            local_hash,
            name,
            description,
            installed,
            scope,
            skill_generation,
            repo_generation,
            original_dir,
            instance_target,
            instance_records,
            selected_identity,
            selected_digest,
            selected_managed_source,
            local_paths: originals.into_values().collect(),
            conflict,
        })
    }

    /// 批量匹配缺少来源的技能；指定 ID 时也提供其他路径供手动关联。
    pub fn preview_repository_matches(
        &self,
        repo: &SkillRepo,
        remote: &[RemoteSkill],
        only_id: Option<&str>,
    ) -> anyhow::Result<SkillRepositoryMatchPreview> {
        self.preview_repository_matches_inner(repo, remote, only_id, None)
    }

    pub fn preview_repository_matches_for_instance(
        &self,
        repo: &SkillRepo,
        remote: &[RemoteSkill],
        id: &str,
        local_path: &Path,
    ) -> anyhow::Result<SkillRepositoryMatchPreview> {
        self.preview_repository_matches_inner(repo, remote, Some(id), Some(local_path))
    }

    fn preview_repository_matches_inner(
        &self,
        repo: &SkillRepo,
        remote: &[RemoteSkill],
        only_id: Option<&str>,
        local_path: Option<&Path>,
    ) -> anyhow::Result<SkillRepositoryMatchPreview> {
        if let Some(id) = only_id {
            validate_skill_id(id)?;
        }
        let ids = {
            let _coordination = skills_coordination();
            let state = self.load_state_checked_unlocked()?;
            anyhow::ensure!(state.repos.contains(repo), "仓库配置已变化，请重新匹配");
            self.repository_match_ids()?
        };
        let mut preview = SkillRepositoryMatchPreview::default();
        for id in ids {
            if only_id.is_some_and(|wanted| wanted != id) {
                continue;
            }
            let snapshot = match self.repository_match_snapshot(&id, repo, local_path) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    preview.warnings.push(format!("{id}：{error:#}"));
                    continue;
                }
            };
            if only_id.is_none() && snapshot.local_paths.len() <= 1 && snapshot.installed.as_ref().is_some_and(|item|
                !item.repo_path.is_empty() && parse_repo_key(&item.repo_key).is_some())
            {
                continue;
            }
            preview.examined += 1;
            for candidate in remote.iter().filter(|item| item.repo_key == repo.key()) {
                let (matches, exact) = plausible_match(&id, &snapshot.name, candidate, &snapshot.local_hash);
                if matches || only_id.is_some() {
                    let found = SkillRepositoryMatch {
                        local_id: id.clone(),
                        local_name: snapshot.name.clone(),
                        local_hash: snapshot.local_hash.clone(),
                        snapshot: snapshot.value.clone(),
                        repo_key: candidate.repo_key.clone(),
                        repo_path: candidate.repo_path.clone(),
                        remote_name: candidate.name.clone(),
                        remote_hash: candidate.content_hash.clone(),
                        exact,
                        conflict: snapshot.conflict.clone(),
                        local_paths: snapshot.local_paths.clone(),
                        local_path: local_path.map(Path::to_path_buf),
                    };
                    if matches {
                        preview.matches.push(found);
                    } else {
                        preview.manual_candidates.push(found);
                    }
                }
            }
        }
        preview.matches.sort_by(|a, b| a.local_id.cmp(&b.local_id)
            .then(b.exact.cmp(&a.exact)).then(a.repo_path.cmp(&b.repo_path)));
        preview.manual_candidates.sort_by(|a, b| a.repo_path.cmp(&b.repo_path));
        preview.preview_id = uuid::Uuid::new_v4().to_string();
        let mut previews = repository_match_previews();
        while previews.len() >= 256 {
            previews.pop_front();
        }
        previews.push_back((preview.preview_id.clone(), RepositoryMatchSnapshot {
            state_path: self.state_path.clone(),
            scope: self.coordination_scope(),
            matches: preview.matches.clone(),
            manual_candidates: preview.manual_candidates.clone(),
        }));
        Ok(preview)
    }

    /// `remote` 由调用方在确认时重新拉取；每项独立提交并返回实际成功项。
    pub fn apply_repository_matches(
        &self,
        selections: &[SkillRepositoryMatchSelection],
        remote: &[RemoteSkill],
    ) -> anyhow::Result<SkillRepositoryMatchApply> {
        let mut ids = BTreeSet::new();
        let mut origins = BTreeSet::new();
        for selection in selections {
            validate_skill_id(&selection.local_id)?;
            anyhow::ensure!(ids.insert(selection.local_id.to_lowercase()), "同一个 Skill 只能选择一个来源");
            anyhow::ensure!(origins.insert((selection.repo_key.clone(), selection.repo_path.clone())),
                "同一个仓库路径不能关联多个 Skill");
            anyhow::ensure!(!selection.manual || selections.len() == 1, "手动选择只能单项确认");
        }
        let mut outcome = SkillRepositoryMatchApply::default();
        for selection in selections {
            let result = (|| -> anyhow::Result<SkillRepositoryMatchApplied> {
                let preview = repository_match_previews().iter()
                    .find(|(id, _)| id == &selection.preview_id)
                    .map(|(_, preview)| preview.clone())
                    .context("来源匹配预览已失效，请重新匹配")?;
                anyhow::ensure!(preview.state_path == self.state_path
                    && coordination_scope_is_current(&preview.scope, &self.coordination_scope()),
                    "Skill 管理范围已变化，请重新匹配");
                let candidates = if selection.manual { &preview.manual_candidates } else { &preview.matches };
                let confirmed = candidates.iter().find(|item|
                    item.local_id == selection.local_id && item.repo_key == selection.repo_key
                        && item.repo_path == selection.repo_path && item.local_path == selection.local_path)
                    .context("所选来源不在已确认的预览中，请重新匹配")?;
                let repo = self.list_repos().into_iter()
                    .find(|repo| repo.key() == selection.repo_key).context("仓库已删除")?;
                let candidate = remote.iter().find(|item|
                    item.repo_key == selection.repo_key && item.repo_path == selection.repo_path
                        && item.content_hash == confirmed.remote_hash && !item.content_hash.is_empty())
                    .context("仓库内容已变化或来源不可用，请重新匹配")?;
                let snapshot = self.repository_match_snapshot(&selection.local_id, &repo, selection.local_path.as_deref())?;
                anyhow::ensure!(snapshot.conflict.is_none(), "{}", snapshot.conflict.as_deref().unwrap_or(""));
                let (matches, _) = plausible_match(&selection.local_id, &snapshot.name, candidate, &snapshot.local_hash);
                anyhow::ensure!(snapshot.value == confirmed.snapshot && snapshot.local_hash == confirmed.local_hash
                    && (matches || selection.manual), "Skill 已变化或来源不匹配，请重新匹配");
                let mut coordination = skills_coordination();
                let mut state = self.load_state_checked_unlocked()?;
                if let Some(path) = &selection.local_path {
                    self.ensure_skill_name_aliases_only(&selection.local_id, &state)?;
                    anyhow::ensure!(Some(skill_path_identity(path)?) == snapshot.selected_identity
                        && skill_directory_digest(path)? == snapshot.selected_digest,
                        "所选 Skill 已改变，请重新匹配");
                } else {
                    self.ensure_skill_name_not_aliased(&selection.local_id, &state)?;
                }
                anyhow::ensure!(
                    coordination_scope_is_current(&snapshot.scope, &self.coordination_scope())
                        && coordination.mutation_generation(&snapshot.scope, &selection.local_id) == snapshot.skill_generation
                        && coordination.repo_change_generation(&snapshot.scope, &repo.key()) == snapshot.repo_generation
                        && state.repos.contains(&repo)
                        && state.installed.get(&selection.local_id) == snapshot.installed.as_ref()
                        && state.instance_targets.get(&selection.local_id) == snapshot.instance_target.as_ref()
                        && state.instance_records.get(&selection.local_id) == snapshot.instance_records.as_ref(),
                    "Skill 或仓库在确认期间发生变化，请重新匹配"
                );
                anyhow::ensure!(!state.installed.values().any(|item|
                    item.id != selection.local_id && item.repo_key == selection.repo_key
                        && item.repo_path == selection.repo_path), "仓库路径已关联其他 Skill");
                let previous = if snapshot.selected_managed_source {
                    snapshot.installed.clone()
                } else if let Some(path) = &selection.local_path {
                    let canonical = path.canonicalize()?;
                    state.instance_record(&selection.local_id, &canonical).map(|record| record.installed.clone())
                        .or_else(|| snapshot.instance_target.as_ref().filter(|target| {
                            target.path.canonicalize().ok().as_ref() == Some(&canonical)
                                && skill_path_identity(&target.path).ok() == Some(target.identity)
                        }).and(snapshot.installed.clone()))
                } else {
                    snapshot.installed.clone()
                };
                let from_repo_key = previous.as_ref().map(|item| item.repo_key.clone()).unwrap_or_default();
                let same_original = previous.as_ref().is_some_and(|item| {
                    match (&item.original_dir, &snapshot.original_dir) {
                        (Some(old), Some(new)) => old.canonicalize().ok() == new.canonicalize().ok(),
                        (None, None) => true,
                        _ => false,
                    }
                });
                let same_source = same_original && previous.as_ref().is_some_and(|item|
                    item.repo_key == selection.repo_key && item.repo_path == selection.repo_path);
                let mut item = previous.unwrap_or_else(|| InstalledSkill {
                    id: selection.local_id.clone(),
                    name: snapshot.name.clone(),
                    description: snapshot.description.clone(),
                    repo_key: String::new(),
                    repo_path: String::new(),
                    content_hash: String::new(),
                    installed_at: current_unix_timestamp_string(),
                    verified_cc_switch: false,
                    install_channel: "local".into(),
                    original_dir: snapshot.original_dir.clone(),
                    update_owner: String::new(),
                });
                item.name = snapshot.name;
                item.description = snapshot.description;
                item.repo_key = selection.repo_key.clone();
                item.repo_path = selection.repo_path.clone();
                if !same_source {
                    item.content_hash = snapshot.local_hash;
                    item.update_owner.clear();
                }
                if snapshot.original_dir.is_some() && item.update_owner == "manager" {
                    item.update_owner.clear();
                }
                if !same_original {
                    item.verified_cc_switch = false;
                    item.install_channel = "local".into();
                }
                if let Some(original) = snapshot.original_dir {
                    if selection.local_path.is_some() || !same_original {
                        item.original_dir = Some(original);
                    }
                }
                if snapshot.selected_managed_source {
                    state.installed.insert(selection.local_id.clone(), item);
                } else if let Some(path) = &selection.local_path {
                    if let (Some(installed), Some(target)) = (&snapshot.installed, &snapshot.instance_target) {
                        state.instance_records.entry(selection.local_id.clone()).or_default()
                            .entry(target.path.clone()).or_insert_with(|| SkillInstanceRecord {
                                installed: installed.clone(), target: target.clone(), managed_copies: Vec::new(),
                            });
                    }
                    let canonical = path.canonicalize()?;
                    let mut managed_copies = state.instance_record(&selection.local_id, &canonical)
                        .map(|record| record.managed_copies.clone()).unwrap_or_default();
                    let recorded_original = snapshot.installed.as_ref().is_some_and(|installed| {
                        installed.original_dir.as_ref().and_then(|original| original.canonicalize().ok()).as_ref() == Some(&canonical)
                    }) && snapshot.instance_target.as_ref().is_none_or(|target| {
                        target.path.canonicalize().ok().as_ref() == Some(&canonical)
                            && skill_path_identity(&target.path).ok() == Some(target.identity)
                    }) && snapshot.instance_records.as_ref().is_none_or(|records| {
                        records.values().filter(|record| {
                            record.target.path.canonicalize().ok().as_ref() == Some(&canonical)
                        }).all(|record| skill_path_identity(&record.target.path).ok() == Some(record.target.identity))
                    });
                    let tracked_by_other = |candidate: &Path| snapshot.instance_records.as_ref()
                        .into_iter().flat_map(|records| records.values())
                        .filter(|record| record.target.path.canonicalize().ok().as_ref() != Some(&canonical))
                        .any(|record| record.managed_copies.iter().any(|copy| copy.path == candidate));
                    if recorded_original {
                        let source = self.source_dir.join(&selection.local_id);
                        if !tracked_by_other(&source) && source.is_dir() && !source.is_symlink()
                            && skill_directory_digest(&source)? == snapshot.selected_digest
                        {
                            let source_identity = skill_path_identity(&source)?;
                            let existing_source = managed_copies.iter().find(|copy| copy.path == source);
                            if existing_source.is_none_or(|copy| copy.identity == source_identity) {
                                if existing_source.is_none() {
                                    managed_copies.push(SkillInstanceTarget { identity: source_identity, path: source });
                                }
                                for root in std::iter::once(self.linked_dir()).chain(self.agent_dirs.iter().map(|(_, root)| root.clone())) {
                                    let copy = root.join(&selection.local_id);
                                    if !tracked_by_other(&copy) && !managed_copies.iter().any(|item| item.path == copy)
                                        && is_managed_copy(&copy) && skill_directory_digest(&copy)? == snapshot.selected_digest
                                    {
                                        managed_copies.push(SkillInstanceTarget {
                                            identity: skill_path_identity(&copy)?, path: copy,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    let target = SkillInstanceTarget {
                        path: path.clone(),
                        identity: skill_path_identity(path)?,
                    };
                    state.instance_records.entry(selection.local_id.clone()).or_default()
                        .insert(path.clone(), SkillInstanceRecord { installed: item.clone(), target: target.clone(), managed_copies });
                    let mirror = snapshot.installed.is_none()
                        || snapshot.installed.as_ref().is_some_and(|installed| installed.repo_path.is_empty())
                        || snapshot.installed.as_ref().and_then(|installed| installed.original_dir.as_ref())
                            .and_then(|original| original.canonicalize().ok()) == path.canonicalize().ok()
                        || snapshot.instance_target.as_ref().is_some_and(|old| old.path == *path);
                    if mirror {
                        state.installed.insert(selection.local_id.clone(), item);
                        state.instance_targets.insert(selection.local_id.clone(), target);
                    }
                } else {
                    state.installed.insert(selection.local_id.clone(), item);
                }
                self.save_state_unlocked(&state)?;
                coordination.changed_skill(self.coordination_scope(), &selection.local_id);
                Ok(SkillRepositoryMatchApplied {
                    local_id: selection.local_id.clone(),
                    from_repo_key,
                    to_repo_key: selection.repo_key.clone(),
                    local_path: selection.local_path.clone(),
                    instance_key: selection.local_path.as_ref().map(|path| format!("{}:{}", selection.local_id, path.display())),
                })
            })();
            match result {
                Ok(applied) => outcome.applied.push(applied),
                Err(error) => outcome.errors.push(format!("{}：{error:#}", selection.local_id)),
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture() -> (tempfile::TempDir, SkillsManager, SkillRepo) {
        let temp = tempfile::tempdir().unwrap();
        let manager = SkillsManager::new(
            temp.path().join("library"),
            temp.path().join("backups"),
            temp.path().join("state.json"),
            temp.path().join("codex"),
        ).with_shared_skills_dir(temp.path().join("shared"))
            .with_agent_dirs(vec![("claude".into(), temp.path().join("claude"))]);
        let repo = SkillRepo {
            owner: "example".into(), name: "repo".into(), branch: "main".into(),
            subdir: String::new(), enabled: true,
        };
        manager.save_repo(repo.clone(), None).unwrap();
        (temp, manager, repo)
    }

    fn write_skill(root: &Path, id: &str, description: &str) {
        let path = root.join(id);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("SKILL.md"), format!("---\nname: {id}\ndescription: {description}\n---\n")).unwrap();
        std::fs::write(path.join("details.txt"), "whole tree").unwrap();
    }

    fn candidate(repo: &SkillRepo, path: &str, hash: String) -> RemoteSkill {
        RemoteSkill {
            id: "remote".into(), name: path.rsplit('/').next().unwrap().into(),
            description: String::new(), repo_key: repo.key(), repo_path: path.into(), content_hash: hash,
        }
    }

    #[test]
    fn selected_instance_evidence_does_not_include_other_installs() {
        let (temp, manager, _) = fixture();
        let shared = temp.path().join("shared/alpha");
        let claude = temp.path().join("claude/alpha");
        write_skill(&temp.path().join("shared"), "alpha", "shared");
        write_skill(&temp.path().join("claude"), "alpha", "claude");
        std::fs::write(shared.join(SKILL_MANIFEST_FILE), "https://github.com/example/shared").unwrap();
        std::fs::write(claude.join(SKILL_MANIFEST_FILE), "https://github.com/example/claude").unwrap();
        let evidence = manager.discover_skill_source_evidence_for_instance("alpha", &shared).unwrap();
        assert!(evidence.iter().any(|item| item.repo == "example/shared"));
        assert!(!evidence.iter().any(|item| item.repo == "example/claude"));
        let unrelated = temp.path().join("unrelated");
        std::fs::create_dir(&unrelated).unwrap();
        assert!(manager.discover_skill_source_evidence_for_instance("alpha", &unrelated).is_err());
    }

    #[test]
    fn selected_managed_source_can_be_associated_without_claiming_an_independent_install() {
        let (temp, manager, repo) = fixture();
        let source = manager.source_dir.join("alpha");
        let independent = temp.path().join("shared/alpha");
        write_skill(&manager.source_dir, "alpha", "managed");
        write_skill(&temp.path().join("shared"), "alpha", "independent");
        write_skill(&temp.path().join("claude"), "alpha", "managed");
        std::fs::write(temp.path().join("claude/alpha").join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::write(source.join("scripts.txt"), "managed content").unwrap();
        let mut state = manager.load_state();
        state.installed.insert("alpha".into(), InstalledSkill {
            id: "alpha".into(), name: "alpha".into(), description: String::new(),
            repo_key: LOCAL_IMPORT_REPO_KEY.into(), repo_path: String::new(), content_hash: String::new(),
            installed_at: String::new(), verified_cc_switch: false, install_channel: "local".into(),
            original_dir: None, update_owner: String::new(),
        });
        manager.save_state_unlocked(&state).unwrap();
        let inventory = manager.try_local_inventory().unwrap();
        assert!(inventory.iter().flat_map(|item| &item.locations)
            .any(|location| location.managed && location.association_path.as_ref() == Some(&source)));
        let remote = vec![candidate(&repo, "skills/alpha", local_repository_hash(&source).unwrap())];
        let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &source).unwrap();
        assert!(preview.matches[0].exact);
        assert!(preview.matches[0].conflict.is_none());
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let state = manager.load_state();
        assert_eq!(state.installed["alpha"].repo_key, repo.key());
        assert_eq!(state.installed["alpha"].repo_path, "skills/alpha");
        assert!(state.installed["alpha"].original_dir.is_none());
        assert!(state.instance_records.get("alpha").is_none());
        assert_eq!(std::fs::read_to_string(source.join("scripts.txt")).unwrap(), "managed content");
        assert!(std::fs::read_to_string(independent.join("SKILL.md")).unwrap().contains("independent"));
    }

    #[test]
    fn agent_installations_associate_their_selected_repository_paths_independently() {
        for multiple_paths in [false, true] {
            let (temp, manager, repo) = fixture();
            let source = manager.source_dir.join("alpha");
            let codex = manager.linked_dir().join("alpha");
            let claude = temp.path().join("claude/alpha");
            write_skill(&manager.source_dir, "alpha", "managed");
            copy_dir_all(&source, &codex).unwrap();
            std::fs::write(codex.join(MANAGED_COPY_MARKER), "").unwrap();
            write_skill(&temp.path().join("claude"), "alpha", "independent");
            let mut state = manager.load_state();
            state.installed.insert("alpha".into(), InstalledSkill {
                id: "alpha".into(), name: "alpha".into(), description: String::new(),
                repo_key: LOCAL_IMPORT_REPO_KEY.into(), repo_path: String::new(), content_hash: String::new(),
                installed_at: String::new(), verified_cc_switch: false, install_channel: "local".into(),
                original_dir: None, update_owner: String::new(),
            });
            manager.save_state_unlocked(&state).unwrap();
            let first = ".agents/skills/alpha";
            let second = if multiple_paths { ".codex/skills/alpha" } else { first };
            let mut remote = vec![candidate(&repo, first, local_repository_hash(&source).unwrap())];
            if multiple_paths {
                remote.push(candidate(&repo, second, local_repository_hash(&claude).unwrap()));
            }
            for (path, repo_path) in [(&source, first), (&claude, second)] {
                let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", path).unwrap();
                assert_eq!(preview.matches.len(), remote.len());
                let found = preview.matches.iter().find(|item| item.repo_path == repo_path).unwrap();
                let result = manager.apply_repository_matches(&[selection(&preview, found, false)], &remote).unwrap();
                assert!(result.errors.is_empty(), "{:?}", result.errors);
            }
            let state = manager.load_state();
            assert_eq!(state.installed["alpha"].repo_path, first);
            assert_eq!(state.instance_record("alpha", &claude.canonicalize().unwrap()).unwrap().installed.repo_path, second);
            assert!(state.instance_record("alpha", &claude.canonicalize().unwrap()).unwrap().installed.update_owner.is_empty());
            let entries = manager.try_local_inventory().unwrap();
            let instances = entries.iter().filter(|item| item.id == "alpha").collect::<Vec<_>>();
            assert_eq!(instances.len(), 2);
            assert!(instances.iter().all(|item| item.conflict.is_none()));
            assert_eq!(std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
                std::fs::read_to_string(codex.join("SKILL.md")).unwrap());
            assert!(std::fs::read_to_string(claude.join("SKILL.md")).unwrap().contains("independent"));
            if multiple_paths {
                let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &claude).unwrap();
                let found = preview.matches.iter().find(|item| item.repo_path == first).unwrap();
                let result = manager.apply_repository_matches(&[selection(&preview, found, false)], &remote).unwrap();
                assert!(result.errors.is_empty(), "{:?}", result.errors);
                let state = manager.load_state();
                assert_eq!(state.installed["alpha"].repo_path, first);
                assert_eq!(state.instance_record("alpha", &claude.canonicalize().unwrap()).unwrap().installed.repo_path, first);
            }
        }
    }

    #[test]
    fn tracked_agent_copy_uses_its_originals_association_without_hiding_changes() {
        for scenario in ["matching", "modified", "replaced"] {
            let (temp, manager, repo) = fixture();
            let original = temp.path().join("shared/alpha");
            let source = manager.source_dir.join("alpha");
            let copy = manager.linked_dir().join("alpha");
            let independent = temp.path().join("claude/alpha");
            write_skill(&temp.path().join("shared"), "alpha", "shared");
            copy_dir_all(&original, &source).unwrap();
            copy_dir_all(&original, &copy).unwrap();
            std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
            write_skill(&temp.path().join("claude"), "alpha", "independent");
            let record = |path: &Path, repo_path: &str| {
                let installed = InstalledSkill {
                    id: "alpha".into(), name: "alpha".into(), description: String::new(),
                    repo_key: repo.key(), repo_path: repo_path.into(), content_hash: String::new(),
                    installed_at: String::new(), verified_cc_switch: false, install_channel: "local".into(),
                    original_dir: Some(path.to_path_buf()), update_owner: String::new(),
                };
                SkillInstanceRecord {
                    installed, target: SkillInstanceTarget { path: path.to_path_buf(), identity: skill_path_identity(path).unwrap() },
                    managed_copies: Vec::new(),
                }
            };
            let mut shared = record(&original, ".agents/skills/alpha");
            shared.managed_copies = [&source, &copy].into_iter().map(|path| SkillInstanceTarget {
                path: path.clone(), identity: skill_path_identity(path).unwrap(),
            }).collect();
            let other = record(&independent, ".claude/skills/alpha");
            let mut state = manager.load_state();
            state.installed.insert("alpha".into(), shared.installed.clone());
            state.instance_targets.insert("alpha".into(), shared.target.clone());
            state.instance_records.insert("alpha".into(), BTreeMap::from([
                (original.clone(), shared), (independent.clone(), other),
            ]));
            manager.save_state_unlocked(&state).unwrap();
            if scenario == "modified" {
                std::fs::write(copy.join("details.txt"), "changed").unwrap();
            } else if scenario == "replaced" {
                std::fs::rename(&copy, manager.linked_dir().join(".old-alpha")).unwrap();
                copy_dir_all(&original, &copy).unwrap();
                std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
            }

            let entries = manager.try_local_inventory().unwrap();
            let instances = entries.iter().filter(|item| item.id == "alpha").collect::<Vec<_>>();
            let codex = instances.iter().flat_map(|item| &item.locations)
                .find(|location| location.root_kind == "codex").unwrap();
            if scenario == "matching" {
                assert_eq!(instances.len(), 2);
                assert!(instances.iter().all(|item| item.conflict.is_none()));
                assert_eq!(codex.association_path.as_ref(), Some(&original.canonicalize().unwrap()));
                assert_eq!(codex.repo_path, ".agents/skills/alpha");
                assert!(instances.iter().any(|item| item.locations.iter().any(|location| location.root_kind == "shared")
                    && item.locations.iter().any(|location| location.root_kind == "codex")));
            } else {
                assert!(instances.iter().any(|item| item.conflict.is_some()));
                if scenario == "replaced" {
                    assert!(codex.association_path.is_none());
                }
            }
        }
    }

    #[test]
    fn managed_source_association_rejects_claimed_sources_and_changed_files() {
        let (temp, manager, repo) = fixture();
        let source = manager.source_dir.join("alpha");
        let original = temp.path().join("shared/alpha");
        write_skill(&manager.source_dir, "alpha", "managed");
        write_skill(&temp.path().join("shared"), "alpha", "original");
        let mut state = manager.load_state();
        let installed = InstalledSkill {
            id: "alpha".into(), name: "alpha".into(), description: String::new(),
            repo_key: LOCAL_IMPORT_REPO_KEY.into(), repo_path: String::new(), content_hash: String::new(),
            installed_at: String::new(), verified_cc_switch: false, install_channel: "local".into(),
            original_dir: None, update_owner: String::new(),
        };
        state.installed.insert("alpha".into(), installed.clone());
        state.instance_records.entry("alpha".into()).or_default().insert(original.clone(), SkillInstanceRecord {
            installed, target: SkillInstanceTarget { path: original.clone(), identity: skill_path_identity(&original).unwrap() },
            managed_copies: vec![SkillInstanceTarget { path: source.clone(), identity: skill_path_identity(&source).unwrap() }],
        });
        manager.save_state_unlocked(&state).unwrap();
        let inventory = manager.try_local_inventory().unwrap();
        assert!(inventory.iter().flat_map(|item| &item.locations)
            .all(|location| location.association_path.is_none()));
        let remote = vec![candidate(&repo, "skills/alpha", local_repository_hash(&source).unwrap())];
        let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &source).unwrap();
        assert!(preview.matches.is_empty());
        assert!(preview.warnings.iter().any(|warning| warning.contains("其他安装位置")));

        state.instance_records.clear();
        manager.save_state_unlocked(&state).unwrap();
        let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &source).unwrap();
        std::fs::write(source.join("details.txt"), "changed").unwrap();
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert!(!result.errors.is_empty());
        assert_eq!(manager.load_state().installed["alpha"].repo_key, LOCAL_IMPORT_REPO_KEY);
    }

    fn selection(preview: &SkillRepositoryMatchPreview, found: &SkillRepositoryMatch, manual: bool) -> SkillRepositoryMatchSelection {
        SkillRepositoryMatchSelection {
            preview_id: preview.preview_id.clone(), local_id: found.local_id.clone(),
            repo_key: found.repo_key.clone(), repo_path: found.repo_path.clone(), manual,
            local_path: found.local_path.clone(),
        }
    }

    #[test]
    fn shared_skill_matches_whole_tree_and_association_preserves_files() {
        let (temp, manager, repo) = fixture();
        let shared = temp.path().join("shared");
        write_skill(&shared, "alpha", "shared");
        let hash = local_repository_hash(&shared.join("alpha")).unwrap();
        let remote = vec![candidate(&repo, "skills/alpha", hash)];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(preview.matches[0].exact);
        assert!(!manager.source_dir.join("alpha").exists());
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert_eq!(result.applied.len(), 1);
        assert!(result.errors.is_empty());
        assert_eq!(manager.load_state().installed["alpha"].repo_path, "skills/alpha");
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(std::fs::read_to_string(shared.join("alpha/details.txt")).unwrap(), "whole tree");
    }

    #[test]
    fn adopted_shared_skill_tracks_matching_managed_copies_without_taking_update_ownership() {
        let (temp, manager, repo) = fixture();
        let manager = manager.with_agent_dirs(vec![
            ("claude".into(), temp.path().join("claude")),
            ("gemini".into(), temp.path().join("gemini")),
            ("other".into(), temp.path().join("other")),
        ]);
        let original = temp.path().join("shared/alpha");
        write_skill(&temp.path().join("shared"), "alpha", "shared");
        manager.set_enabled_for("claude", "alpha", true).unwrap();
        let source = manager.source_dir.join("alpha");
        let copy = temp.path().join("gemini/alpha");
        copy_dir_all(&source, &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        let independent = manager.linked_dir().join("alpha");
        copy_dir_all(&original, &independent).unwrap();

        let installed = manager.load_state().installed["alpha"].clone();
        assert_eq!(installed.original_dir.as_ref(), Some(&original));
        assert!(installed.update_owner.is_empty());
        let remote = vec![candidate(&repo, "skills/alpha", local_repository_hash(&original).unwrap())];
        let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &original).unwrap();
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let state = manager.load_state();
        let record = state.instance_record("alpha", &original.canonicalize().unwrap()).unwrap();
        assert!(record.installed.update_owner.is_empty());
        assert!(record.managed_copies.iter().any(|item| item.path == source));
        let copy_identity = skill_path_identity(&copy).unwrap();
        assert!(record.managed_copies.iter().any(|item| item.path == copy && item.identity == copy_identity));
        assert!(!record.managed_copies.iter().any(|item| item.path == independent));

        // Reassociation must keep a modified tracked copy and add an equal, previously untracked copy.
        std::fs::write(copy.join("details.txt"), "modified").unwrap();
        let later = temp.path().join("other/alpha");
        copy_dir_all(&source, &later).unwrap();
        std::fs::write(later.join(MANAGED_COPY_MARKER), "").unwrap();
        let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &original).unwrap();
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        let state = manager.load_state();
        let record = state.instance_record("alpha", &original.canonicalize().unwrap()).unwrap();
        assert!(record.managed_copies.iter().any(|item| item.path == copy && item.identity == copy_identity));
        assert!(record.managed_copies.iter().any(|item| item.path == later));
        assert!(!record.managed_copies.iter().any(|item| item.path == independent));

        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("repo-main/skills/alpha/SKILL.md", options).unwrap();
        writer.write_all(b"---\nname: alpha\n---\nnew").unwrap();
        writer.start_file("repo-main/skills/alpha/details.txt", options).unwrap();
        writer.write_all(b"new details").unwrap();
        let zip = writer.finish().unwrap().into_inner();
        let next = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap()
            .into_iter().find(|item| item.repo_path == "skills/alpha").unwrap();
        let preview = manager.preview_skill_in_place_update_for_instance("alpha", &original, next.clone(), zip.clone()).unwrap();
        assert!(preview.conflicts.iter().any(|item| item.contains("受控副本内容已修改")));
        std::fs::write(copy.join("details.txt"), "whole tree").unwrap();
        let preview = manager.preview_skill_in_place_update_for_instance("alpha", &original, next.clone(), zip).unwrap();
        assert!(preview.conflicts.is_empty(), "{:?}", preview.conflicts);
        assert!(preview.managed_paths.contains(&source));
        assert!(preview.managed_paths.contains(&copy));
        assert!(preview.managed_paths.contains(&later));
        assert!(preview.skipped_paths.contains(&independent));
        manager.execute_skill_in_place_update(&preview.preview_id, &next).unwrap();
        for path in [&original, &source, &copy, &later] {
            assert_eq!(std::fs::read_to_string(path.join("details.txt")).unwrap(), "new details");
        }
        assert_eq!(std::fs::read_to_string(independent.join("details.txt")).unwrap(), "whole tree");
    }

    #[test]
    fn selected_instance_does_not_claim_unrelated_or_replaced_managed_copies() {
        for scenario in ["unrelated", "replaced", "replaced-record", "other-owner"] {
            let (temp, manager, repo) = fixture();
            let manager = manager.with_agent_dirs(vec![
                ("claude".into(), temp.path().join("claude")),
                ("gemini".into(), temp.path().join("gemini")),
            ]);
            let original = temp.path().join("shared/alpha");
            let independent = temp.path().join("claude/alpha");
            let source = manager.source_dir.join("alpha");
            let copy = temp.path().join("gemini/alpha");
            write_skill(&temp.path().join("shared"), "alpha", "same");
            copy_dir_all(&original, &independent).unwrap();
            copy_dir_all(&original, &source).unwrap();
            copy_dir_all(&original, &copy).unwrap();
            std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
            let mut state = manager.load_state();
            let installed = InstalledSkill {
                id: "alpha".into(), name: "alpha".into(), description: String::new(),
                repo_key: LOCAL_IMPORT_REPO_KEY.into(), repo_path: String::new(), content_hash: String::new(),
                installed_at: String::new(), verified_cc_switch: false, install_channel: "local".into(),
                original_dir: Some(original.clone()), update_owner: String::new(),
            };
            state.installed.insert("alpha".into(), installed.clone());
            if scenario == "replaced" {
                state.instance_targets.insert("alpha".into(), SkillInstanceTarget {
                    path: original.clone(), identity: skill_path_identity(&source).unwrap(),
                });
            }
            if scenario == "replaced-record" {
                state.instance_records.entry("alpha".into()).or_default().insert(
                    original.clone(), SkillInstanceRecord {
                        installed: installed.clone(),
                        target: SkillInstanceTarget {
                            path: original.clone(), identity: skill_path_identity(&source).unwrap(),
                        },
                        managed_copies: Vec::new(),
                    },
                );
            }
            if scenario == "other-owner" {
                let mut other = installed;
                other.original_dir = Some(independent.clone());
                state.instance_records.entry("alpha".into()).or_default().insert(
                    independent.clone(), SkillInstanceRecord {
                        installed: other,
                        target: SkillInstanceTarget {
                            path: independent.clone(), identity: skill_path_identity(&independent).unwrap(),
                        },
                        managed_copies: vec![SkillInstanceTarget {
                            path: source.clone(), identity: skill_path_identity(&source).unwrap(),
                        }],
                    },
                );
            }
            manager.save_state_unlocked(&state).unwrap();
            let selected = if scenario == "unrelated" { &independent } else { &original };
            let remote = vec![candidate(&repo, "skills/alpha", local_repository_hash(selected).unwrap())];
            let preview = manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", selected).unwrap();
            let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
            assert!(result.errors.is_empty(), "{scenario}: {:?}", result.errors);
            let state = manager.load_state();
            let record = state.instance_record("alpha", &selected.canonicalize().unwrap()).unwrap();
            assert!(record.managed_copies.is_empty(), "{scenario}: {:?}", record.managed_copies);
        }
    }

    #[test]
    fn reassociation_preserves_the_maintenance_baseline_and_install_channel() {
        for selected_instance in [false, true] {
            let (temp, manager, repo) = fixture();
            let shared = temp.path().join("shared");
            write_skill(&shared, "alpha", "shared");
            let original = shared.join("alpha").canonicalize().unwrap();
            let baseline = local_repository_hash(&original).unwrap();
            let remote = vec![candidate(&repo, "skills/alpha", baseline.clone())];
            let preview = if selected_instance {
                manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &original).unwrap()
            } else {
                manager.preview_repository_matches(&repo, &remote, Some("alpha")).unwrap()
            };
            let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);

            let mut state = manager.load_state();
            let configure = |item: &mut InstalledSkill| {
                item.update_owner = "in_place".into();
                item.verified_cc_switch = true;
                item.install_channel = "cc-switch".into();
            };
            configure(state.installed.get_mut("alpha").unwrap());
            if selected_instance {
                configure(&mut state.instance_records.get_mut("alpha").unwrap().values_mut().next().unwrap().installed);
            }
            manager.save_state_unlocked(&state).unwrap();
            std::fs::write(original.join("details.txt"), "local edit").unwrap();
            let changed = local_repository_hash(&original).unwrap();
            assert_ne!(changed, baseline);

            let preview = if selected_instance {
                manager.preview_repository_matches_for_instance(&repo, &remote, "alpha", &original).unwrap()
            } else {
                manager.preview_repository_matches(&repo, &remote, Some("alpha")).unwrap()
            };
            let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            let state = manager.load_state();
            let installed = if selected_instance {
                &state.instance_records["alpha"].values().next().unwrap().installed
            } else {
                &state.installed["alpha"]
            };
            assert_eq!(installed.content_hash, baseline);
            assert_eq!(installed.update_owner, "in_place");
            assert!(installed.verified_cc_switch);
            assert_eq!(installed.install_channel, "cc-switch");

            let other = SkillRepo { owner: "other".into(), ..repo.clone() };
            manager.save_repo(other.clone(), None).unwrap();
            let other_remote = vec![candidate(&other, "skills/alpha", changed.clone())];
            let preview = if selected_instance {
                manager.preview_repository_matches_for_instance(&other, &other_remote, "alpha", &original).unwrap()
            } else {
                manager.preview_repository_matches(&other, &other_remote, Some("alpha")).unwrap()
            };
            let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &other_remote).unwrap();
            assert!(result.errors.is_empty(), "{:?}", result.errors);
            let state = manager.load_state();
            let installed = if selected_instance {
                &state.instance_records["alpha"].values().next().unwrap().installed
            } else {
                &state.installed["alpha"]
            };
            assert_eq!(installed.content_hash, changed);
            assert!(installed.update_owner.is_empty());
            assert!(installed.verified_cc_switch);
            assert_eq!(installed.install_channel, "cc-switch");
        }
    }

    #[test]
    fn only_root_managed_marker_is_excluded_from_source_matching() {
        let (temp, manager, repo) = fixture();
        let shared = temp.path().join("shared");
        write_skill(&shared, "alpha", "shared");
        let skill = shared.join("alpha");
        let original_hash = local_repository_hash(&skill).unwrap();
        std::fs::write(skill.join(MANAGED_COPY_MARKER), "").unwrap();
        assert_eq!(local_repository_hash(&skill).unwrap(), original_hash);
        std::fs::create_dir(skill.join("nested")).unwrap();
        std::fs::write(skill.join("nested").join(MANAGED_COPY_MARKER), "user content").unwrap();
        let remote = vec![candidate(&repo, "skills/alpha", original_hash)];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(!preview.matches[0].exact);
    }

    #[test]
    fn divergent_copies_are_reported_and_never_associated() {
        let (temp, manager, repo) = fixture();
        write_skill(&temp.path().join("shared"), "alpha", "shared");
        write_skill(&temp.path().join("claude"), "alpha", "different");
        let remote = vec![candidate(&repo, "skills/alpha", "remote".into())];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(preview.matches[0].conflict.as_ref().unwrap().contains("副本内容不同"));
        assert_eq!(preview.matches[0].local_paths.len(), 2);
        let result = manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert!(!manager.load_state().installed.contains_key("alpha"));
    }

    #[test]
    fn multiple_equal_originals_are_reported_without_guessing() {
        let (temp, manager, repo) = fixture();
        write_skill(&temp.path().join("shared"), "alpha", "same");
        write_skill(&temp.path().join("claude"), "alpha", "same");
        let hash = local_repository_hash(&temp.path().join("shared/alpha")).unwrap();
        let remote = vec![candidate(&repo, "skills/alpha", hash)];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        assert!(preview.matches[0].exact);
        assert!(preview.matches[0].conflict.as_ref().unwrap().contains("多个独立"));
        assert!(manager.apply_repository_matches(&[selection(&preview, &preview.matches[0], false)], &remote)
            .unwrap().applied.is_empty());
    }

    #[test]
    fn source_evidence_reports_links_and_lock_without_claiming_ownership() {
        let (temp, manager, _) = fixture();
        let shared = temp.path().join("shared");
        write_skill(&shared, "alpha", "source");
        std::fs::write(shared.join("alpha/SKILL.md"), "See https://github.com/example/alpha/blob/main/README.md").unwrap();
        std::fs::create_dir_all(shared.join("alpha/scripts")).unwrap();
        std::fs::write(shared.join("alpha/scripts/start.cmd"), "https://github.com/example/engine/releases").unwrap();
        std::fs::write(shared.join("skills-lock.json"), r#"{"skills":{"alpha":{"source":"example/locked"}}}"#).unwrap();
        let evidence = manager.discover_skill_source_evidence("alpha").unwrap();
        assert!(evidence.iter().any(|item| item.repo == "example/alpha" && item.kind == "manifest-link"));
        assert!(evidence.iter().any(|item| item.repo == "example/engine" && item.kind == "script-link"));
        assert!(evidence.iter().any(|item| item.repo == "example/locked" && item.kind == "lock"));
        assert!(!manager.load_state().installed.contains_key("alpha"));
    }

    #[test]
    fn changed_local_or_remote_contents_reject_stale_confirmation() {
        let (temp, manager, repo) = fixture();
        let shared = temp.path().join("shared");
        write_skill(&shared, "alpha", "shared");
        let hash = local_repository_hash(&shared.join("alpha")).unwrap();
        let remote = vec![candidate(&repo, "skills/alpha", hash)];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        let selected = selection(&preview, &preview.matches[0], false);
        std::fs::write(shared.join("alpha/details.txt"), "changed").unwrap();
        let result = manager.apply_repository_matches(&[selected.clone()], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert_eq!(result.errors.len(), 1);
        let mut changed_remote = remote;
        changed_remote[0].content_hash = "changed".into();
        let result = manager.apply_repository_matches(&[selected], &changed_remote).unwrap();
        assert!(result.applied.is_empty());
        assert_eq!(result.errors.len(), 1);
        assert!(!manager.load_state().installed.contains_key("alpha"));
    }

    #[test]
    fn name_only_and_manual_matches_do_not_claim_content_equality() {
        let (temp, manager, repo) = fixture();
        write_skill(&temp.path().join("shared"), "alpha", "shared");
        let remote = vec![candidate(&repo, "skills/alpha", "different".into()),
            candidate(&repo, "skills/beta", "other".into())];
        let preview = manager.preview_repository_matches(&repo, &remote, Some("alpha")).unwrap();
        assert_eq!(preview.matches.len(), 1);
        assert!(!preview.matches[0].exact);
        assert_eq!(preview.manual_candidates.len(), 1);
        let denied = manager.apply_repository_matches(&[selection(&preview, &preview.manual_candidates[0], false)], &remote).unwrap();
        assert!(denied.applied.is_empty());
        let accepted = manager.apply_repository_matches(&[selection(&preview, &preview.manual_candidates[0], true)], &remote).unwrap();
        assert_eq!(accepted.applied.len(), 1);
    }

    #[test]
    fn confirmation_rejects_missing_expired_and_unpreviewed_selections() {
        let (temp, manager, repo) = fixture();
        write_skill(&temp.path().join("shared"), "alpha", "shared");
        let remote = vec![candidate(&repo, "skills/alpha", "different".into()),
            candidate(&repo, "skills/beta", "other".into())];
        let preview = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        let selected = selection(&preview, &preview.matches[0], false);

        let mut wrong_id = selected.clone();
        wrong_id.preview_id = uuid::Uuid::new_v4().to_string();
        let result = manager.apply_repository_matches(&[wrong_id], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert!(result.errors[0].contains("预览已失效"));

        let mut changed = selected.clone();
        changed.repo_path = "skills/beta".into();
        changed.manual = true;
        let result = manager.apply_repository_matches(&[changed], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert!(result.errors[0].contains("不在已确认的预览中"));

        repository_match_previews().retain(|(id, _)| id != &preview.preview_id);
        let result = manager.apply_repository_matches(&[selected], &remote).unwrap();
        assert!(result.applied.is_empty());
        assert!(result.errors[0].contains("预览已失效"));
        assert!(!manager.load_state().installed.contains_key("alpha"));
    }

    #[test]
    fn batch_confirmation_uses_each_repository_preview_and_reports_partial_failure() {
        let (temp, manager, repo) = fixture();
        let other = SkillRepo { owner: "other".into(), name: "skills".into(), ..repo.clone() };
        manager.save_repo(other.clone(), None).unwrap();
        let shared = temp.path().join("shared");
        write_skill(&shared, "alpha", "first");
        write_skill(&shared, "beta", "second");
        let remote = vec![
            candidate(&repo, "skills/alpha", local_repository_hash(&shared.join("alpha")).unwrap()),
            candidate(&other, "skills/beta", local_repository_hash(&shared.join("beta")).unwrap()),
        ];
        let first = manager.preview_repository_matches(&repo, &remote, None).unwrap();
        let second = manager.preview_repository_matches(&other, &remote, None).unwrap();
        let selections = [
            selection(&first, first.matches.iter().find(|item| item.local_id == "alpha").unwrap(), false),
            selection(&second, second.matches.iter().find(|item| item.local_id == "beta").unwrap(), false),
        ];
        std::fs::write(shared.join("beta/details.txt"), "changed after preview").unwrap();
        let result = manager.apply_repository_matches(&selections, &remote).unwrap();
        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.applied[0].local_id, "alpha");
        assert_eq!(result.errors.len(), 1);
        let state = manager.load_state();
        assert_eq!(state.installed["alpha"].repo_key, repo.key());
        assert!(!state.installed.contains_key("beta"));
    }
}
