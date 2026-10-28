//! Codex Skills 管理。
//!
//! codex 的 skill 是文件系统约定，不是配置项：它扫描 `$CODEX_HOME/skills/<id>/SKILL.md`，
//! 从 YAML frontmatter 读 `name` / `description`，然后把清单注入 `<skills_instructions>`。
//! `config.toml` 里的 `[skills]` 是一个三字段结构体（bundled / include_instructions /
//! max_context_tokens），写 `[skills.<id>]` 会被 serde 当未知字段静默丢弃，什么都不会发生。
//!
//! 所以这里采用「SSOT + 软链」模型：
//!
//! - 安装：下载仓库 zip，只解出目标 skill 子树到 `~/.codex-session-delete/skills/<id>/`
//! - 启用：`$CODEX_HOME/skills/<id>` 软链到源目录（软链目录 codex 同样能发现）
//! - 停用：只删软链，源目录保留
//! - 卸载：源目录整体移到 `~/.codex-session-delete/skill-backups/<id>-<ts>/`
//! - 更新检测：GitHub trees API 返回每个文件的 blob sha，据此算子树哈希，不下载内容
//!
//! Windows 上建软链需要开发者模式或管理员权限，失败时自动回退成复制。

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const SKILL_MANIFEST_FILE: &str = "SKILL.md";
const BUNDLED_SKILLS_DIR: &str = ".system";
/// 单个仓库 zip 的下载上限。skill 仓库都是纯文本，64 MiB 足够宽松了。
const REPO_ZIP_DOWNLOAD_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// 一个 skill 仓库源。`subdir` 为空表示 skill 目录直接放在仓库根下。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRepo {
    pub owner: String,
    pub name: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default)]
    pub subdir: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_branch() -> String {
    "main".to_string()
}

fn default_true() -> bool {
    true
}

impl SkillRepo {
    /// 仓库的稳定标识，同时用作前端的选择值。
    pub fn key(&self) -> String {
        let base = format!("{}/{}@{}", self.owner, self.name, self.branch);
        if self.subdir.is_empty() {
            base
        } else {
            format!("{base}:{}", self.subdir)
        }
    }
}

/// 已安装 skill 的记录。`content_hash` 是安装当时的远端子树哈希，用来比对更新。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub repo_key: String,
    #[serde(default)]
    pub repo_path: String,
    #[serde(default)]
    pub content_hash: String,
    #[serde(default)]
    pub installed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsState {
    #[serde(default)]
    pub repos: Vec<SkillRepo>,
    #[serde(default)]
    pub installed: BTreeMap<String, InstalledSkill>,
}

impl Default for SkillsState {
    fn default() -> Self {
        Self {
            repos: default_skill_repos(),
            installed: BTreeMap::new(),
        }
    }
}

/// 首次启动种下的仓库源。第一个是 codex 官方 curated 列表（内置 skill-installer
/// 用的就是它），后三个与常见第三方管理工具预置的 `skill_repos` 重合。
pub fn default_skill_repos() -> Vec<SkillRepo> {
    vec![
        SkillRepo {
            owner: "openai".to_string(),
            name: "skills".to_string(),
            branch: "main".to_string(),
            subdir: "skills/.curated".to_string(),
            enabled: true,
        },
        SkillRepo {
            owner: "anthropics".to_string(),
            name: "skills".to_string(),
            branch: "main".to_string(),
            subdir: String::new(),
            enabled: true,
        },
        SkillRepo {
            owner: "ComposioHQ".to_string(),
            name: "awesome-claude-skills".to_string(),
            branch: "master".to_string(),
            subdir: String::new(),
            enabled: true,
        },
        SkillRepo {
            owner: "cexll".to_string(),
            name: "myclaude".to_string(),
            branch: "master".to_string(),
            subdir: String::new(),
            enabled: true,
        },
    ]
}

/// 远端仓库里的一个可安装 skill。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo_key: String,
    /// skill 目录在仓库里的路径，安装时据此从 zip 里挑子树。
    pub repo_path: String,
    pub content_hash: String,
}

/// 给前端的合并视图：远端清单 + 本地安装状态 + 启用状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub repo_key: String,
    pub repo_path: String,
    pub installed: bool,
    pub enabled: bool,
    /// codex 自带的 `.system` skill，只读展示，不能安装/卸载。
    pub bundled: bool,
    pub content_hash: String,
    pub remote_hash: String,
    pub update_available: bool,
    /// 已经启用到哪些 Agent（"codex" / "claude" / "grok"）：它们的 skills 目录里有这个 Skill。
    pub agents: Vec<String>,
    /// Codex++ 的受控源目录里有它（安装或导入过）；只在某个 Agent 自己目录里的 Skill 为 false。
    pub managed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillBackup {
    pub id: String,
    pub skill_id: String,
    pub name: String,
    pub backed_up_at: String,
}

#[derive(Debug, Clone)]
pub struct SkillsManager {
    source_dir: PathBuf,
    backups_dir: PathBuf,
    state_path: PathBuf,
    codex_home: PathBuf,
    /// Codex 之外也读 `<dir>/<id>/SKILL.md` 的 Agent：(Agent id, skills 目录)。
    agent_dirs: Vec<(String, PathBuf)>,
    state_lock: Arc<Mutex<()>>,
}

impl SkillsManager {
    pub fn new(
        source_dir: impl Into<PathBuf>,
        backups_dir: impl Into<PathBuf>,
        state_path: impl Into<PathBuf>,
        codex_home: impl Into<PathBuf>,
    ) -> Self {
        Self {
            source_dir: source_dir.into(),
            backups_dir: backups_dir.into(),
            state_path: state_path.into(),
            codex_home: codex_home.into(),
            agent_dirs: Vec::new(),
            state_lock: Arc::new(Mutex::new(())),
        }
    }

    /// 让 Skill 也能启用到其它 Agent（例如 Claude Code 的 `~/.claude/skills`、Grok 的 `~/.grok/skills`）。
    pub fn with_agent_dirs(mut self, dirs: Vec<(String, PathBuf)>) -> Self {
        self.agent_dirs = dirs;
        self
    }

    pub fn agent_dirs(&self) -> &[(String, PathBuf)] {
        &self.agent_dirs
    }

    fn agent_dir(&self, agent: &str) -> anyhow::Result<PathBuf> {
        self.agent_dirs
            .iter()
            .find(|(id, _)| id == agent)
            .map(|(_, dir)| dir.clone())
            .with_context(|| format!("不支持的 Agent：{agent}"))
    }

    /// 把 Skill 启用到 / 停用于某个 Agent。Codex 沿用 [`Self::set_enabled`]；其它 Agent 同样是软链到受控源目录。
    ///
    /// 启用时若还没有受控源（Codex 或其它 Agent 自建的 Skill），先从它所在的目录导入一份。
    /// 停用只删本工具建的软链；Agent 自己目录里的真实 Skill 不删，报错说明。
    pub fn set_enabled_for(&self, agent: &str, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        if enabled {
            self.ensure_managed_source(id)?;
        }
        if agent == "codex" {
            return self.set_enabled(id, enabled);
        }
        let dir = self.agent_dir(agent)?;
        let link = dir.join(id);
        let metadata = std::fs::symlink_metadata(&link);
        if !enabled {
            return match metadata {
                Ok(metadata) if metadata.file_type().is_symlink() => remove_link(&link),
                Ok(_) => anyhow::bail!("{} 是这个 Agent 自己的 Skill，不由 Codex++ 管理，未删除", link.display()),
                Err(_) => Ok(()),
            };
        }
        if link.join(SKILL_MANIFEST_FILE).is_file() {
            return Ok(());
        }
        if metadata.is_ok() {
            anyhow::bail!("启用位置已存在其他内容，未覆盖：{}", link.display());
        }
        let source = self.source_dir.join(id);
        if !source.is_dir() {
            anyhow::bail!("skill 源目录不存在：{}", source.display());
        }
        std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
        if let Err(error) = link_or_copy(&source, &link) {
            remove_link(&link).context(format!("启用失败且清理未完成：{error:#}"))?;
            return Err(error);
        }
        Ok(())
    }

    /// 没有受控源时，从 Codex 或其它 Agent 目录里找到这个 Skill 的真实目录，纳入受控源。
    ///
    /// - 在 Codex 目录里：走一遍「停用 → 启用」，停用会把真实目录复制成受控源（原目录进备份），
    ///   启用再软链回来，Codex 照常可用；
    /// - 只在其它 Agent 目录里：复制一份成受控源，那个 Agent 自己的目录保持不动。
    fn ensure_managed_source(&self, id: &str) -> anyhow::Result<()> {
        if self.source_dir.join(id).is_dir() {
            return Ok(());
        }
        let physical = |path: &Path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) && path.join(SKILL_MANIFEST_FILE).is_file()
        };
        let codex = self.linked_dir().join(id);
        if physical(&codex) {
            self.set_enabled(id, false)?;
            return self.set_enabled(id, true);
        }
        if std::fs::symlink_metadata(&codex).is_ok() {
            // Codex 目录里同名的是别的东西（例如断掉的软链），导入会把它挪进备份；交给用户处理。
            anyhow::bail!("Codex 的 skills 目录里已有同名内容，未导入：{}", codex.display());
        }
        for (_, dir) in &self.agent_dirs {
            let candidate = dir.join(id);
            if physical(&candidate) {
                self.import_from_directory(id, &candidate)?;
                return Ok(());
            }
        }
        Ok(())
    }

    pub fn source_dir(&self) -> &Path {
        &self.source_dir
    }

    /// codex 扫描的 skill 目录。启用一个 skill 就是往这里放一个指向源目录的软链。
    pub fn linked_dir(&self) -> PathBuf {
        self.codex_home.join("skills")
    }

    pub fn load_state(&self) -> SkillsState {
        let _guard = self.state_lock.lock().unwrap();
        self.load_state_unlocked()
    }

    fn load_state_unlocked(&self) -> SkillsState {
        let Ok(text) = std::fs::read_to_string(&self.state_path) else {
            return SkillsState::default();
        };
        serde_json::from_str::<SkillsState>(&text).unwrap_or_default()
    }

    fn save_state_unlocked(&self, state: &SkillsState) -> anyhow::Result<()> {
        crate::settings::atomic_write(
            &self.state_path,
            serde_json::to_string_pretty(state)?.as_bytes(),
        )
    }

    pub fn list_repos(&self) -> Vec<SkillRepo> {
        self.load_state().repos
    }

    /// 按 key 覆盖或追加一个仓库源。
    pub fn upsert_repo(&self, repo: SkillRepo) -> anyhow::Result<Vec<SkillRepo>> {
        self.save_repo(repo, None)
    }

    /// 仅启停既有源不依赖网络；身份变更必须由调用方验证后保存。
    pub fn update_repo_enabled_if_unchanged(&self, repo: &SkillRepo, previous_key: Option<&str>) -> anyhow::Result<bool> {
        let repo = normalize_repo(repo.clone())?;
        let key = repo.key();
        if previous_key.is_some_and(|previous| previous != key) { return Ok(false); }
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        let Some(existing) = state.repos.iter_mut().find(|item| item.key() == key) else { return Ok(false) };
        if existing.enabled == repo.enabled { return Ok(false); }
        existing.enabled = repo.enabled;
        self.save_state_unlocked(&state)?;
        Ok(true)
    }

    pub fn save_repo(&self, repo: SkillRepo, previous_key: Option<&str>) -> anyhow::Result<Vec<SkillRepo>> {
        let repo = normalize_repo(repo)?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        let key = repo.key();
        if let Some(previous_key) = previous_key.filter(|previous| *previous != key) {
            if !state.repos.iter().any(|item| item.key() == previous_key) {
                anyhow::bail!("原仓库已不存在，请刷新后重试");
            }
            if state.repos.iter().any(|item| item.key() == key) {
                anyhow::bail!("目标仓库已存在");
            }
            state.repos.retain(|item| item.key() != previous_key);
        }
        match state.repos.iter_mut().find(|item| item.key() == key) {
            Some(existing) => *existing = repo,
            None => state.repos.push(repo),
        }
        self.save_state_unlocked(&state)?;
        Ok(state.repos)
    }

    pub fn delete_repo(&self, key: &str) -> anyhow::Result<Vec<SkillRepo>> {
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.repos.retain(|repo| repo.key() != key);
        self.save_state_unlocked(&state)?;
        Ok(state.repos)
    }

    /// 把远端清单和本地状态合并成一份列表。`remote` 传空就是纯本地视图。
    pub fn merge_entries(&self, remote: &[RemoteSkill]) -> Vec<SkillEntry> {
        let state = self.load_state();
        let linked = self.linked_dir();
        let mut entries: BTreeMap<(String, String), SkillEntry> = BTreeMap::new();

        for skill in remote {
            let installed = state.installed.values().find(|item|
                item.repo_key == skill.repo_key && item.repo_path == skill.repo_path);
            let id = installed.map(|item| item.id.clone()).unwrap_or_else(|| skill.id.clone());
            entries.insert(
                (skill.repo_key.clone(), id.clone()),
                SkillEntry {
                    id: id.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    repo_key: skill.repo_key.clone(),
                    repo_path: skill.repo_path.clone(),
                    installed: installed.is_some(),
                    enabled: installed.is_some() && is_linked(&linked.join(&id)),
                    bundled: false,
                    content_hash: installed
                        .map(|item| item.content_hash.clone())
                        .unwrap_or_default(),
                    remote_hash: skill.content_hash.clone(),
                    update_available: installed
                        .map(|item| {
                            !item.content_hash.is_empty()
                                && !skill.content_hash.is_empty()
                                && item.content_hash != skill.content_hash
                        })
                        .unwrap_or(false),
                    agents: Vec::new(),
                    managed: false,
                },
            );
        }

        // 已安装但远端清单里没有的（仓库删了、或本地手工放的），仍要能看见和卸载。
        for (id, installed) in &state.installed {
            entries.entry((installed.repo_key.clone(), id.clone())).or_insert_with(|| SkillEntry {
                id: id.clone(),
                name: if installed.name.is_empty() {
                    id.clone()
                } else {
                    installed.name.clone()
                },
                description: installed.description.clone(),
                repo_key: installed.repo_key.clone(),
                repo_path: installed.repo_path.clone(),
                installed: true,
                enabled: is_linked(&linked.join(id)),
                bundled: false,
                content_hash: installed.content_hash.clone(),
                remote_hash: String::new(),
                update_available: false,
                agents: Vec::new(),
                managed: false,
            });
        }

        for skill in self.list_bundled_skills() {
            entries.entry((skill.repo_key.clone(), skill.id.clone())).or_insert(skill);
        }

        // 用户让 Codex 自己建的 skill（或手工放进 skills 目录的）不会进 state.installed，
        // 也不在远端清单里，此前因此完全不显示（issue #862）。这里扫描 linked_dir 补上。
        for skill in self.list_local_skills() {
            if !entries.values().any(|item| item.id == skill.id && item.installed) {
                entries.entry((skill.repo_key.clone(), skill.id.clone())).or_insert(skill);
            }
        }

        // 只放在 Claude / Grok 自己 skills 目录里的 Skill（不是本工具建的软链），也要能看见、能启用到别处。
        for (agent, dir) in &self.agent_dirs {
            for skill in scan_skill_dirs(dir) {
                if !entries.values().any(|item| item.id == skill.0) {
                    let (name, description) = skill.1;
                    entries.insert((String::new(), skill.0.clone()), SkillEntry {
                        id: skill.0.clone(),
                        name,
                        description,
                        repo_key: String::new(),
                        repo_path: String::new(),
                        installed: true,
                        enabled: false,
                        bundled: false,
                        content_hash: String::new(),
                        remote_hash: String::new(),
                        update_available: false,
                        agents: vec![agent.clone()],
                        managed: false,
                    });
                }
            }
        }

        let mut entries: Vec<SkillEntry> = entries.into_values().collect();
        for entry in &mut entries {
            if !entry.installed {
                continue;
            }
            entry.managed = self.source_dir.join(&entry.id).is_dir();
            let mut agents = Vec::new();
            if entry.enabled || entry.bundled {
                agents.push("codex".to_string());
            }
            if !entry.bundled {
                for (agent, dir) in &self.agent_dirs {
                    if dir.join(&entry.id).join(SKILL_MANIFEST_FILE).is_file() && !agents.contains(agent) {
                        agents.push(agent.clone());
                    }
                }
            }
            entry.agents = agents;
        }
        entries
    }

    /// 本地目录 inventory 不混入仅出现在远端/缓存清单中的未安装条目，也不写 TOML。
    pub fn local_inventory(&self) -> Vec<SkillEntry> {
        self.merge_entries(&[])
    }

    /// 扫描 `$CODEX_HOME/skills` 下带 `SKILL.md` 的目录，收拢前面几轮没覆盖到的条目。
    ///
    /// 软链（本工具安装的）与 `.system` 已在别的分支处理过，这里只兜底「本地自建」，
    /// 因此不做安装/卸载，只读展示。
    fn list_local_skills(&self) -> Vec<SkillEntry> {
        let root = self.linked_dir();
        let Ok(dir) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut skills = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if id == BUNDLED_SKILLS_DIR || id.starts_with('.') {
                continue;
            }
            let manifest = path.join(SKILL_MANIFEST_FILE);
            if !manifest.is_file() {
                continue;
            }
            let (name, description) = read_skill_manifest(&manifest, id);
            skills.push(SkillEntry {
                id: id.to_string(),
                name,
                description,
                repo_key: String::new(),
                repo_path: String::new(),
                installed: true,
                enabled: is_linked(&path),
                bundled: false,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                agents: Vec::new(),
                managed: false,
            });
        }
        skills
    }

    /// 从外部 Skill 目录导入到 Codex++ 的受控源目录，并默认停用。
    pub fn import_from_directory(
        &self,
        id: &str,
        external_dir: &Path,
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        let manifest = external_dir.join(SKILL_MANIFEST_FILE);
        if !manifest.is_file() {
            anyhow::bail!("{} 下没有 SKILL.md，不是有效的 Skill", external_dir.display());
        }
        std::fs::create_dir_all(&self.source_dir)?;
        if self.source_dir.canonicalize()?.starts_with(external_dir.canonicalize()?) {
            anyhow::bail!("导入目录不能包含受控源目录");
        }
        let staging = self.source_dir.join(format!(".import-{id}-{}", uuid::Uuid::new_v4()));
        if let Err(error) = copy_dir_all(external_dir, &staging) {
            let _ = remove_link(&staging);
            return Err(error);
        }
        let (name, description) = read_skill_manifest(&manifest, id);
        self.commit_staged_skill(&staging, InstalledSkill {
            id: id.to_string(), name, description, repo_key: "cc-switch".to_string(),
            repo_path: String::new(), content_hash: String::new(),
            installed_at: current_unix_timestamp_string(),
        }, false)
    }

    /// 目录、链接和安装记录一起提交；失败时恢复两处原内容。
    fn commit_staged_skill(&self, staging: &Path, installed: InstalledSkill, enabled: bool) -> anyhow::Result<SkillsState> {
        let id = installed.id.clone();
        let destination = self.source_dir.join(&id);
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.installed.insert(id.clone(), installed);
        let link = self.linked_dir().join(&id);
        let mut displaced = Vec::new();
        let mut published = false;
        let mut creating_link = false;
        let result = (|| -> anyhow::Result<()> {
            // 两处原内容都先保留，成功后仍作为可恢复备份，不覆盖同名用户目录。
            for original in [&link, &destination] {
                if std::fs::symlink_metadata(original).is_ok() {
                    let backup = self.backup_directory(&id, original)?;
                    displaced.push((original.clone(), backup));
                }
            }
            std::fs::rename(&staging, &destination)?;
            published = true;
            if enabled {
                std::fs::create_dir_all(self.linked_dir())?;
                creating_link = true;
                link_or_copy(&destination, &link)?;
            }
            self.save_state_unlocked(&state)?;
            Ok(())
        })();
        if let Err(error) = result {
            let rollback = (|| -> anyhow::Result<()> {
                if creating_link { remove_link(&link)?; }
                if published { std::fs::rename(&destination, &staging)?; }
                for (original, backup) in displaced.iter().rev() {
                    std::fs::rename(backup, original)?;
                }
                Ok(())
            })();
            if let Err(rollback_error) = rollback {
                return Err(error.context(format!("回滚失败，原内容保留在备份目录：{rollback_error:#}")));
            }
            let _ = remove_link(&staging);
            return Err(error);
        }
        // 旧安装软链不含独立内容，实体目录备份则必须保留。
        for (_, backup) in &displaced {
            if std::fs::symlink_metadata(backup).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                if let Err(error) = remove_link(backup) {
                    eprintln!("清理已提交的 Skill 软链备份失败：{error:#}");
                }
            }
        }
        Ok(state)
    }

    /// codex 随包附带的 `.system` skill，只列出来让用户知道有这些，不参与安装。
    fn list_bundled_skills(&self) -> Vec<SkillEntry> {
        let root = self.linked_dir().join(BUNDLED_SKILLS_DIR);
        let Ok(dir) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut skills = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let manifest = path.join(SKILL_MANIFEST_FILE);
            if !manifest.is_file() {
                continue;
            }
            let (name, description) = read_skill_manifest(&manifest, id);
            skills.push(SkillEntry {
                id: id.to_string(),
                name,
                description,
                repo_key: String::new(),
                repo_path: String::new(),
                installed: true,
                enabled: true,
                bundled: true,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                agents: Vec::new(),
                managed: false,
            });
        }
        skills
    }

    /// 把一份已下载的仓库 zip 里的某个 skill 子树装到 SSOT，并建立软链。
    pub fn install_from_zip(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(&skill.id)?;
        if let Some(installed) = self.load_state().installed.get(&skill.id) {
            if installed.repo_key != skill.repo_key || installed.repo_path != skill.repo_path {
                anyhow::bail!("同名 Skill 的来源路径不同或旧记录缺少路径，不能覆盖；请先备份卸载原项");
            }
        }
        let enable_after_install = !self.load_state().installed.contains_key(&skill.id)
            || is_linked(&self.linked_dir().join(&skill.id));
        let staging = self.staging_dir(&skill.id);
        if staging.exists() {
            std::fs::remove_dir_all(&staging)
                .with_context(|| format!("清理暂存目录失败：{}", staging.display()))?;
        }
        let result = extract_skill_subtree(zip_bytes, &skill.repo_path, &staging);
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result?;

        self.commit_staged_skill(&staging,
            InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                repo_path: skill.repo_path.clone(),
                content_hash: skill.content_hash.clone(),
                installed_at: current_unix_timestamp_string(),
            },
            enable_after_install,
        )
    }

    /// 启用 = 在 `$CODEX_HOME/skills/` 下建一个指向源目录的软链；停用 = 删掉它。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let source = self.source_dir.join(id);
        let link = self.linked_dir().join(id);
        if !enabled {
            let state = self.load_state();
            let physical_copy = std::fs::symlink_metadata(&link)
                .is_ok_and(|metadata| !metadata.file_type().is_symlink());
            if link.is_dir() && (physical_copy || !source.is_dir() || !state.installed.contains_key(id)) {
                // 复制回退后的安装位置可能已有用户修改；未纳管软链也需要保留可再启用的源。
                std::fs::create_dir_all(&self.source_dir)?;
                if self.source_dir.canonicalize()?.starts_with(link.canonicalize()?) {
                    anyhow::bail!("Skill 目录不能包含受控源目录，未执行停用");
                }
                let staging = self.staging_dir(id);
                if let Err(error) = copy_dir_all(&link, &staging) {
                    let _ = remove_link(&staging);
                    return Err(error);
                }
                let (name, description) = read_skill_manifest(&link.join(SKILL_MANIFEST_FILE), id);
                let installed = state.installed.get(id).cloned().unwrap_or(InstalledSkill {
                    id: id.to_string(),
                    name,
                    description,
                    repo_key: String::new(),
                    repo_path: String::new(),
                    content_hash: String::new(),
                    installed_at: current_unix_timestamp_string(),
                });
                self.commit_staged_skill(&staging, installed, false)?;
                return Ok(());
            }
            return remove_link(&link);
        }
        if link.join(SKILL_MANIFEST_FILE).is_file() {
            return Ok(());
        }
        if std::fs::symlink_metadata(&link).is_ok() {
            anyhow::bail!("启用位置已存在其他内容，未覆盖：{}", link.display());
        }
        if !source.is_dir() {
            anyhow::bail!("skill 源目录不存在：{}", source.display());
        }
        std::fs::create_dir_all(self.linked_dir())
            .with_context(|| format!("创建 {} 失败", self.linked_dir().display()))?;
        if let Err(error) = link_or_copy(&source, &link) {
            remove_link(&link).context(format!("启用失败且清理未完成：{error:#}"))?;
            return Err(error);
        }
        Ok(())
    }

    /// 卸载：删软链，源目录整体移进备份目录。不做自动轮转删除。
    pub fn uninstall(&self, id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        // 其它 Agent 里指向它的软链先摘掉，源目录移走后它们会变成断链。
        for (_, dir) in &self.agent_dirs {
            let link = dir.join(id);
            if std::fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                remove_link(&link)?;
            }
        }
        let link = self.linked_dir().join(id);
        let source = self.source_dir.join(id);
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.installed.remove(id);
        let mut displaced = Vec::new();
        let result = (|| -> anyhow::Result<()> {
            for original in [&link, &source] {
                if std::fs::symlink_metadata(original).is_ok() {
                    let backup = self.backup_directory(id, original)?;
                    displaced.push((original.clone(), backup));
                }
            }
            self.save_state_unlocked(&state)?;
            Ok(())
        })();
        if let Err(error) = result {
            for (original, backup) in displaced.iter().rev() {
                if let Err(rollback_error) = std::fs::rename(backup, original) {
                    return Err(error.context(format!("卸载回滚失败，原内容保留在备份目录：{rollback_error}")));
                }
            }
            return Err(error);
        }
        for (_, backup) in &displaced {
            if std::fs::symlink_metadata(backup).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
                if let Err(error) = remove_link(backup) {
                    eprintln!("清理已卸载的 Skill 软链备份失败：{error:#}");
                }
            }
        }
        Ok(state)
    }

    fn backup_directory(&self, id: &str, source: &Path) -> anyhow::Result<PathBuf> {
        std::fs::create_dir_all(&self.backups_dir)?;
        let mut timestamp = current_unix_timestamp_string().parse::<u128>()?;
        let backup = loop {
            let candidate = self.backups_dir.join(format!("{id}-{timestamp}"));
            if std::fs::symlink_metadata(&candidate).is_err() {
                break candidate;
            }
            timestamp += 1;
        };
        std::fs::rename(source, &backup)
            .with_context(|| format!("备份 {} 到 {} 失败", source.display(), backup.display()))?;
        Ok(backup)
    }

    pub fn list_backups(&self) -> Vec<SkillBackup> {
        let Ok(dir) = std::fs::read_dir(&self.backups_dir) else {
            return Vec::new();
        };
        let mut backups = Vec::new();
        for entry in dir.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let (skill_id, backed_up_at) = split_backup_id(id);
            let manifest = path.join(SKILL_MANIFEST_FILE);
            let (name, _) = read_skill_manifest(&manifest, &skill_id);
            backups.push(SkillBackup {
                id: id.to_string(),
                skill_id,
                name,
                backed_up_at,
            });
        }
        backups.sort_by(|a, b| b.id.cmp(&a.id));
        backups
    }

    /// 从备份恢复：目录移回 SSOT 并重新建链。
    pub fn restore_backup(&self, backup_id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(backup_id)?;
        let backup = self.backups_dir.join(backup_id);
        if !backup.is_dir() {
            anyhow::bail!("备份不存在：{}", backup.display());
        }
        let (skill_id, _) = split_backup_id(backup_id);
        validate_skill_id(&skill_id)?;
        let destination = self.source_dir.join(&skill_id);
        if std::fs::symlink_metadata(&destination).is_ok()
            || std::fs::symlink_metadata(self.linked_dir().join(&skill_id)).is_ok()
        {
            anyhow::bail!("{skill_id} 已经装着了，先卸载再从备份恢复");
        }
        std::fs::create_dir_all(&self.source_dir)
            .with_context(|| format!("创建目录失败：{}", self.source_dir.display()))?;
        let staging = self.staging_dir(&skill_id);
        if let Err(error) = copy_dir_all(&backup, &staging) {
            let _ = remove_link(&staging);
            return Err(error);
        }
        let (name, description) =
            read_skill_manifest(&staging.join(SKILL_MANIFEST_FILE), &skill_id);
        let state = self.commit_staged_skill(
            &staging,
            InstalledSkill {
                id: skill_id.clone(),
                name,
                description,
                repo_key: String::new(),
                repo_path: String::new(),
                content_hash: String::new(),
                installed_at: current_unix_timestamp_string(),
            },
            true,
        )?;
        // 提交前始终保留原备份；提交后清理失败也不撤销已恢复的安装。
        if let Err(error) = remove_link(&backup) {
            eprintln!("清理已恢复的 Skill 备份失败：{error:#}");
        }
        Ok(state)
    }

    /// 显式删除单个备份。AGENTS.md 禁止批量删除，所以只删指定的这一个。
    pub fn delete_backup(&self, backup_id: &str) -> anyhow::Result<Vec<SkillBackup>> {
        validate_skill_id(backup_id)?;
        let backup = self.backups_dir.join(backup_id);
        if backup.is_dir() {
            std::fs::remove_dir_all(&backup)
                .with_context(|| format!("删除备份失败：{}", backup.display()))?;
        }
        Ok(self.list_backups())
    }

    fn staging_dir(&self, id: &str) -> PathBuf {
        self.source_dir
            .join(format!(".staging-{id}-{}", uuid::Uuid::new_v4()))
    }
}

/// `owner/repo@branch:subdir` 形式的仓库 key 解析回结构体。
pub fn parse_repo_key(key: &str) -> Option<SkillRepo> {
    let (repo_part, subdir) = match key.split_once(':') {
        Some((head, tail)) => (head, tail.to_string()),
        None => (key, String::new()),
    };
    let (owner_repo, branch) = match repo_part.split_once('@') {
        Some((head, tail)) => (head, tail.to_string()),
        None => (repo_part, default_branch()),
    };
    let (owner, name) = owner_repo.split_once('/')?;
    if owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(SkillRepo {
        owner: owner.to_string(),
        name: name.to_string(),
        branch,
        subdir,
        enabled: true,
    })
}

pub fn normalize_repo(repo: SkillRepo) -> anyhow::Result<SkillRepo> {
    let owner = repo.owner.trim().to_string();
    let name = repo.name.trim().to_string();
    let branch = {
        let trimmed = repo.branch.trim();
        if trimmed.is_empty() {
            default_branch()
        } else {
            trimmed.to_string()
        }
    };
    let subdir = repo.subdir.trim().trim_matches('/').to_string();
    if owner.is_empty() || name.is_empty() {
        anyhow::bail!("仓库 owner 和 name 不能为空");
    }
    if !is_safe_repo_segment(&owner) || !is_safe_repo_segment(&name) {
        anyhow::bail!("仓库 owner/name 只能包含字母、数字、`.`、`_`、`-`");
    }
    Ok(SkillRepo {
        owner,
        name,
        branch,
        subdir,
        enabled: repo.enabled,
    })
}

fn is_safe_repo_segment(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// skill id 直接参与拼路径，必须挡住 `..` 和分隔符。
fn validate_skill_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() {
        anyhow::bail!("skill id 不能为空");
    }
    if id == "." || id == ".." || id.contains('/') || id.contains('\\') {
        anyhow::bail!("非法的 skill id：{id}");
    }
    Ok(())
}

/// GitHub trees API 的响应 → 该仓库里的 skill 清单。
///
/// 一个目录只要直接含 `SKILL.md` 就算一个 skill；`content_hash` 用子树里
/// 每个文件的 blob sha 算，远端内容一变哈希就变，不用下载就能判断有没有更新。
pub fn parse_skills_from_tree(repo: &SkillRepo, tree: &Value) -> Vec<RemoteSkill> {
    let Some(items) = tree.get("tree").and_then(Value::as_array) else {
        return Vec::new();
    };
    let prefix = if repo.subdir.is_empty() {
        String::new()
    } else {
        format!("{}/", repo.subdir)
    };
    let repo_key = repo.key();

    // 先定位所有清单，允许 skills/.curated、插件目录等任意层级。
    let mut directories = std::collections::BTreeSet::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let Some(path) = item.get("path").and_then(Value::as_str) else { continue };
        let Some(rest) = path.strip_prefix(&prefix) else { continue };
        if rest == SKILL_MANIFEST_FILE || rest.ends_with("/SKILL.md") {
            let directory = path.strip_suffix(SKILL_MANIFEST_FILE).unwrap().trim_end_matches('/');
            if directory.is_empty() || safe_relative_path(directory).is_ok() {
                directories.insert(directory.to_string());
            }
        }
    }
    let mut subtrees: BTreeMap<String, Vec<(String, String)>> =
        directories.iter().map(|dir| (dir.clone(), Vec::new())).collect();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let sha = item
            .get("sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        for (directory, files) in &mut subtrees {
            let directory_prefix = if directory.is_empty() { String::new() } else { format!("{directory}/") };
            if let Some(relative) = path.strip_prefix(&directory_prefix) {
                files.push((relative.to_string(), sha.clone()));
            }
        }
    }
    subtrees
        .into_iter()
        .filter_map(|(repo_path, mut files)| {
            let base = if repo_path.is_empty() { repo.name.clone() } else { repo_path.rsplit('/').next()?.to_string() };
            // 嵌套路径始终使用稳定身份，不依赖当前同名目录的数量或排序。
            let identity = format!("{repo_key}\0{repo_path}");
            let id = format!("{base}--{:x}", Sha256::digest(identity.as_bytes()));
            validate_skill_id(&id).ok()?;
            files.sort();
            let mut hasher = Sha256::new();
            for (path, sha) in &files {
                hasher.update(path.as_bytes());
                hasher.update([0]);
                hasher.update(sha.as_bytes());
                hasher.update([0]);
            }
            Some(RemoteSkill {
                id: id.clone(),
                name: base,
                description: String::new(),
                repo_key: repo_key.clone(),
                repo_path: if repo_path.is_empty() { ".".into() } else { repo_path },
                content_hash: format!("{:x}", hasher.finalize()),
            })
        })
        .collect()
}

/// GitHub API 限流时，从公开仓库的 codeload 压缩包构造同样的文件树。
/// codeload 不占 GitHub API 配额，且能同时读取 SKILL.md 的元数据。
async fn fetch_repo_skills_from_zip(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let response = with_github_auth(client.get(repo_zip_url(repo)))
        .header(reqwest::header::ACCEPT, "application/zip")
        .send()
        .await
        .with_context(|| format!("下载 {}/{} 的仓库压缩包失败", repo.owner, repo.name))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!(
            "下载 {}/{} 的仓库压缩包失败：HTTP {}{}",
            repo.owner,
            repo.name,
            status.as_u16(),
            github_status_hint(status.as_u16())
        );
    }
    let bytes = response.bytes().await.context("读取仓库压缩包内容失败")?;
    if bytes.len() > REPO_ZIP_DOWNLOAD_LIMIT_BYTES {
        anyhow::bail!("仓库压缩包太大：{} 字节", bytes.len());
    }

    parse_skills_from_zip(repo, &bytes, cached)
}

fn parse_skills_from_zip(
    repo: &SkillRepo,
    bytes: &[u8],
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).context("skill 仓库压缩包无法解析")?;
    let root = (0..archive.len())
        .find_map(|index| {
            let file = archive.by_index(index).ok()?;
            file.name().split('/').next().map(str::to_string)
        })
        .context("仓库压缩包为空")?;
    let prefix = if repo.subdir.is_empty() {
        String::new()
    } else {
        format!("{}/", repo.subdir.trim_matches('/'))
    };
    let mut tree_items = Vec::new();
    let mut manifests = BTreeMap::new();
    let mut total_size = 0u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        let name = file.name().trim_start_matches('/').to_string();
        let Some(relative) = name.strip_prefix(format!("{root}/").as_str()) else {
            continue;
        };
        let relative = relative.to_string();
        if relative.is_empty() || file.is_dir() {
            continue;
        }
        total_size = total_size.saturating_add(file.size());
        if total_size > 256 * 1024 * 1024 {
            anyhow::bail!("仓库解压内容超过 256 MiB");
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        // 与 trees API 的 Git blob ID 保持一致，避免切换传输路径后误报更新。
        let mut hasher = sha1::Sha1::new();
        hasher.update(format!("blob {}\0", contents.len()).as_bytes());
        hasher.update(&contents);
        let sha = format!("{:x}", hasher.finalize());
        tree_items.push(serde_json::json!({
            "type": "blob",
            "path": relative,
            "sha": sha,
        }));
        let is_manifest = relative == format!("{prefix}{SKILL_MANIFEST_FILE}")
            || relative
                .strip_prefix(prefix.as_str())
                .is_some_and(|path| path.ends_with(&format!("/{SKILL_MANIFEST_FILE}")));
        if is_manifest {
            manifests.insert(relative.to_string(), String::from_utf8_lossy(&contents).into_owned());
        }
    }
    let tree = serde_json::json!({ "tree": tree_items });
    let mut skills = parse_skills_from_tree(repo, &tree);
    for skill in &mut skills {
        if let Some(hit) = cached.get(&skill.id) {
            if hit.content_hash == skill.content_hash && !hit.description.is_empty()
                && !matches!(hit.description.as_str(), ">" | ">-" | ">+" | "|" | "|-" | "|+")
            {
                skill.name = hit.name.clone();
                skill.description = hit.description.clone();
                continue;
            }
        }
        if let Some(text) = manifests.get(&skill_manifest_path(&skill.repo_path)) {
            let (name, description) = parse_skill_frontmatter(text, &skill.id);
            skill.name = name;
            skill.description = description;
        }
    }
    Ok(skills)
}

fn skill_manifest_path(repo_path: &str) -> String {
    if repo_path == "." { SKILL_MANIFEST_FILE.into() }
    else { format!("{repo_path}/{SKILL_MANIFEST_FILE}") }
}

/// 解析 SKILL.md 的 YAML frontmatter。只取 `name` 和 `description` 两个标量，
/// 不引 YAML 依赖——codex 自己也只读这两个字段。
pub fn parse_skill_frontmatter(text: &str, fallback_id: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let mut in_frontmatter = false;
    let mut description_block: Option<bool> = None;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_end();
        if trimmed.trim() == "---" {
            if index == 0 {
                in_frontmatter = true;
                continue;
            }
            if in_frontmatter {
                break;
            }
            continue;
        }
        if !in_frontmatter {
            continue;
        }
        if let Some(literal) = description_block {
            if line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty() {
                if !description.is_empty() {
                    description.push(if literal { '\n' } else { ' ' });
                }
                description.push_str(line.trim());
                continue;
            }
            description_block = None;
        }
        // 嵌套字段（如 metadata 下的键）缩进了，跳过，只认顶层。
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = unquote_scalar(value);
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            let value = value.trim();
            if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
                description_block = Some(value.starts_with('|'));
                description.clear();
            } else {
                description = unquote_scalar(value);
            }
        }
    }
    if name.is_empty() {
        name = fallback_id.to_string();
    }
    (name, description)
}

fn unquote_scalar(value: &str) -> String {
    let trimmed = value.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(trimmed);
    unquoted.to_string()
}

fn read_skill_manifest(manifest: &Path, fallback_id: &str) -> (String, String) {
    match std::fs::read_to_string(manifest) {
        Ok(text) => parse_skill_frontmatter(&text, fallback_id),
        Err(_) => (fallback_id.to_string(), String::new()),
    }
}

fn zip_entry_relative_path(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut components = path.components();
    match components.next()? {
        Component::Normal(_) => {}
        _ => return None,
    }
    let mut relative = PathBuf::new();
    for component in components {
        match component {
            Component::Normal(value) => relative.push(value),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!relative.as_os_str().is_empty()).then_some(relative)
}

/// 从仓库 zip 里只解出 `repo_path` 这一棵子树。
///
/// GitHub 的 zip 统一带一层 `repo-ref/` 前缀，`zip_entry_relative_path` 会剥掉它
/// 并同时挡住路径逃逸。符号链接条目一律拒绝——与 codex 内置 skill-installer 的
/// `_validate_skill` 一致，避免装进来的 skill 指到仓库外面。
pub fn extract_skill_subtree(
    zip_bytes: &[u8],
    repo_path: &str,
    destination: &Path,
) -> anyhow::Result<()> {
    let repo_path = repo_path.trim_matches('/');
    if repo_path.is_empty() {
        anyhow::bail!("skill 在仓库中的路径不能为空");
    }
    let prefix = if repo_path == "." { String::new() } else { format!("{repo_path}/") };
    let mut archive =
        zip::ZipArchive::new(Cursor::new(zip_bytes)).context("skill 仓库压缩包无法解析")?;
    let mut wrote_manifest = false;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("读取压缩包条目 {index} 失败"))?;
        if file.is_symlink() {
            anyhow::bail!("skill 中不允许包含符号链接：{}", file.name());
        }
        let Some(relative) = zip_entry_relative_path(file.name()) else {
            continue;
        };
        // zip 内部的分隔符恒为 '/'，但上面拿回来的是 PathBuf，在 Windows 上
        // to_str() 会渲染成 '\'，跟用 '/' 拼出来的 prefix 永远匹配不上——结果就是
        // 一个文件都解不出来，报「没有 SKILL.md」。这里统一拼回 '/' 再比。
        let relative = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/");
        let Some(inner) = relative.strip_prefix(prefix.as_str()) else {
            continue;
        };
        if inner.is_empty() {
            continue;
        }
        let output_path = destination.join(safe_relative_path(inner)?);
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .with_context(|| format!("创建目录失败：{}", output_path.display()))?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .with_context(|| format!("读取压缩包条目 {} 失败", file.name()))?;
        std::fs::write(&output_path, contents)
            .with_context(|| format!("写入 {} 失败", output_path.display()))?;
        if inner == SKILL_MANIFEST_FILE {
            wrote_manifest = true;
        }
    }

    if !wrote_manifest {
        anyhow::bail!("{repo_path} 下没有 SKILL.md，不是一个有效的 skill");
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> anyhow::Result<PathBuf> {
    let mut relative = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::CurDir => {}
            _ => anyhow::bail!("压缩包条目越界：{value}"),
        }
    }
    if relative.as_os_str().is_empty() {
        anyhow::bail!("压缩包条目路径为空");
    }
    Ok(relative)
}

pub fn repo_zip_url(repo: &SkillRepo) -> String {
    format!(
        "https://codeload.github.com/{}/{}/zip/refs/heads/{}",
        repo.owner, repo.name, repo.branch
    )
}

pub fn repo_tree_url(repo: &SkillRepo) -> String {
    format!(
        "https://api.github.com/repos/{}/{}/git/trees/{}?recursive=1",
        repo.owner, repo.name, repo.branch
    )
}

fn raw_file_url(repo: &SkillRepo, path: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{}/{}/{}/{}",
        repo.owner, repo.name, repo.branch, path
    )
}

async fn github_client() -> anyhow::Result<reqwest::Client> {
    tokio::task::spawn_blocking(|| {
        let agent = format!("Codex++/{}", crate::version::VERSION);
        if ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"]
            .iter().any(|key| std::env::var(key).is_ok_and(|v| !v.trim().is_empty()))
        {
            return crate::http_client::proxied_client(&agent);
        }
        let mut proxies = BTreeMap::new();
        for host in ["api.github.com", "codeload.github.com", "raw.githubusercontent.com"] {
            if let Some(proxy) = crate::proxy::detect_system_proxy_for_url(&format!("https://{host}/")) {
                proxies.insert(host.to_string(), proxy);
            }
        }
        Ok(reqwest::Client::builder().user_agent(agent)
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(90))
            .proxy(reqwest::Proxy::custom(move |url| {
                url.host_str().and_then(|host| proxies.get(host).cloned())
            })).build()?)
    }).await?
}

pub async fn resolve_repo_skills(
    repo: SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<(SkillRepo, Vec<RemoteSkill>)> {
    let repo = normalize_repo(repo)?;
    match fetch_repo_skills(&repo, cached).await {
        Ok(skills) => Ok((repo, skills)),
        Err(error) if format!("{error:#}").contains("HTTP 404")
            && matches!(repo.branch.as_str(), "main" | "master") =>
        {
            let mut alternative = repo.clone();
            alternative.branch = if repo.branch == "main" { "master" } else { "main" }.into();
            match fetch_repo_skills(&alternative, &BTreeMap::new()).await {
                Ok(skills) => Ok((alternative, skills)),
                Err(_) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

/// 私有仓库和限流都靠这个救。与 codex 内置 skill-installer 读同样的两个变量。
fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty())
}

fn with_github_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match github_token() {
        Some(token) => request.header(reqwest::header::AUTHORIZATION, format!("token {token}")),
        None => request,
    }
}

/// 拉一个仓库的 skill 清单：trees API 一次拿到全树，再按需补 SKILL.md 的标题和描述。
///
/// `cached` 传上一次的结果，哈希没变的 skill 直接复用元数据，不重复请求。
pub async fn fetch_repo_skills(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let client = github_client().await?;
    let response = with_github_auth(client.get(repo_tree_url(repo)))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return fetch_repo_skills_from_zip(repo, cached, &client).await
            .with_context(|| format!("文件树请求失败（{error}），压缩包回退也失败")),
    };
    // 不要用 error_for_status() 一把带过：403 限流、404 仓库/分支不存在、401 令牌
    // 无效，处理方式完全不同，但用户只会看到「返回错误状态」，没法自助也没法报障
    // （#1989 里四个仓库全失败，看不出到底是限流还是网络）。
    let status = response.status();
    if !status.is_success() && (matches!(status.as_u16(), 403 | 429) || status.is_server_error()) {
        return fetch_repo_skills_from_zip(repo, cached, &client).await;
    }
    if !status.is_success() {
        anyhow::bail!(
            "{}/{} 的文件树请求失败：HTTP {}{}",
            repo.owner,
            repo.name,
            status.as_u16(),
            github_status_hint(status.as_u16())
        );
    }
    let tree = response
        .json::<Value>()
        .await
        .context("解析 GitHub 文件树响应失败")?;
    if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
        return fetch_repo_skills_from_zip(repo, cached, &client).await;
    }

    let skills = parse_skills_from_tree(repo, &tree);
    use futures_util::{StreamExt, stream};
    let skills = stream::iter(skills.into_iter().map(|mut skill| {
        let client = &client;
        async move {
            if let Some(hit) = cached.get(&skill.id) {
                if hit.content_hash == skill.content_hash && !hit.description.is_empty()
                    && !matches!(hit.description.as_str(), ">" | ">-" | ">+" | "|" | "|-" | "|+")
                {
                    skill.name = hit.name.clone();
                    skill.description = hit.description.clone();
                    return skill;
                }
            }
            let manifest_url = raw_file_url(repo, &skill_manifest_path(&skill.repo_path));
            if let Ok(response) = with_github_auth(client.get(manifest_url))
                .timeout(std::time::Duration::from_secs(15)).send().await
                && let Ok(response) = response.error_for_status()
                && let Ok(text) = response.text().await
            {
                let (name, description) = parse_skill_frontmatter(&text, &skill.id);
                skill.name = name;
                skill.description = description;
            }
            skill
        }
    })).buffer_unordered(8).collect().await;
    Ok(skills)
}

/// 把 GitHub 的 HTTP 状态翻成可操作的提示。
///
/// 之前一律 error_for_status() 带过，用户只看到「返回错误状态」——403 限流、
/// 404 不存在、401 令牌失效处理方式完全不同，既没法自助也没法准确报障（#1989）。
fn github_status_hint(status: u16) -> &'static str {
    match status {
        401 => "：GITHUB_TOKEN / GH_TOKEN 无效或已过期",
        // 未认证请求每小时只有 60 次，四个仓库一起刷很容易撞上
        403 | 429 => {
            "：GitHub API 限流。设置 GITHUB_TOKEN 环境变量可把配额从每小时 60 次提到 5000 次"
        }
        404 => "：仓库或分支不存在（私有仓库需要设置 GITHUB_TOKEN）",
        _ => "",
    }
}

pub async fn download_repo_zip(repo: &SkillRepo) -> anyhow::Result<Vec<u8>> {
    let client = github_client().await?;
    let response = with_github_auth(client.get(repo_zip_url(repo)))
        .header(reqwest::header::ACCEPT, "application/zip")
        .send()
        .await
        .with_context(|| format!("下载 {}/{} 失败", repo.owner, repo.name))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!(
            "下载 {}/{} 失败：HTTP {}{}",
            repo.owner,
            repo.name,
            status.as_u16(),
            github_status_hint(status.as_u16())
        );
    }
    let bytes = response.bytes().await.context("读取仓库压缩包内容失败")?;
    if bytes.len() > REPO_ZIP_DOWNLOAD_LIMIT_BYTES {
        anyhow::bail!("仓库压缩包太大：{} 字节", bytes.len());
    }
    Ok(bytes.to_vec())
}

/// 软链优先，失败回退复制。Windows 上建目录软链要开发者模式或管理员权限，
/// 拿不到就退化成复制——功能一样，只是更新时要重装。
fn link_or_copy(source: &Path, link: &Path) -> anyhow::Result<()> {
    if symlink_dir(source, link).is_ok() {
        return Ok(());
    }
    copy_dir_all(source, link)
        .with_context(|| format!("复制 skill 到 {} 失败（软链也没建成）", link.display()))
}

#[cfg(unix)]
fn symlink_dir(source: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, link)
}

#[cfg(windows)]
fn symlink_dir(source: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, link)
}

fn copy_dir_all(source: &Path, destination: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("创建目录失败：{}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("读取目录失败：{}", source.display()))?
    {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| format!("复制 {} 失败", from.display()))?;
        }
    }
    Ok(())
}

/// 链接可能是软链也可能是复制出来的实体目录，两种都要能删干净。
/// 停用一个 skill：删掉 `$CODEX_HOME/skills/<id>`。
///
/// 这里有两个必须分清的情况：
///
/// - **软链**（正常路径）。绝不能用 `remove_dir_all`——那会顺着链接把 SSOT 源目录里的
///   真实文件递归删掉，停用变毁数据。Unix 上软链用 `remove_file` 删；Windows 上
///   目录软链只能用 `remove_dir`，用 `remove_file` 会报 Access denied (os error 5)。
/// - **实体目录**（Windows 上建软链失败、回退成复制时）。这时才该 `remove_dir_all`。
fn remove_link(link: &Path) -> anyhow::Result<()> {
    match std::fs::symlink_metadata(link) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            let result = if file_type.is_symlink() {
                remove_symlink(link, &file_type)
            } else if file_type.is_dir() {
                std::fs::remove_dir_all(link)
            } else {
                std::fs::remove_file(link)
            };
            result.with_context(|| format!("移除 {} 失败", link.display()))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(anyhow::Error::from(error).context(format!("读取 {} 状态失败", link.display())))
        }
    }
}

#[cfg(windows)]
fn remove_symlink(link: &Path, file_type: &std::fs::FileType) -> std::io::Result<()> {
    use std::os::windows::fs::FileTypeExt;
    if file_type.is_symlink_dir() {
        std::fs::remove_dir(link)
    } else {
        std::fs::remove_file(link)
    }
}

#[cfg(not(windows))]
fn remove_symlink(link: &Path, _file_type: &std::fs::FileType) -> std::io::Result<()> {
    std::fs::remove_file(link)
}

/// 扫描一个 skills 目录里「真实目录 + SKILL.md」的 Skill（跳过软链和隐藏目录），返回 (id, (名称, 描述))。
fn scan_skill_dirs(root: &Path) -> Vec<(String, (String, String))> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        let Some(id) = path.file_name().and_then(|name| name.to_str()).map(str::to_string) else { continue };
        if id.starts_with('.') {
            continue;
        }
        let physical = std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir());
        let manifest = path.join(SKILL_MANIFEST_FILE);
        if physical && manifest.is_file() {
            let meta = read_skill_manifest(&manifest, &id);
            skills.push((id, meta));
        }
    }
    skills.sort_by(|left, right| left.0.cmp(&right.0));
    skills
}

fn is_linked(link: &Path) -> bool {
    std::fs::symlink_metadata(link).is_ok()
}

/// 备份目录名是 `<skill-id>-<timestamp>`，从右边第一个 `-` 切开。
fn split_backup_id(backup_id: &str) -> (String, String) {
    match backup_id.rsplit_once('-') {
        Some((skill_id, timestamp)) if timestamp.chars().all(|c| c.is_ascii_digit()) => {
            (skill_id.to_string(), timestamp.to_string())
        }
        _ => (backup_id.to_string(), String::new()),
    }
}

fn current_unix_timestamp_string() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn manager(temp: &tempfile::TempDir) -> SkillsManager {
        SkillsManager::new(
            temp.path().join("skills"),
            temp.path().join("skill-backups"),
            temp.path().join("skills.json"),
            temp.path().join("codex-home"),
        )
    }

    fn multi_agent_manager(temp: &tempfile::TempDir) -> SkillsManager {
        manager(temp).with_agent_dirs(vec![
            ("claude".into(), temp.path().join("claude-skills")),
            ("grok".into(), temp.path().join("grok-skills")),
        ])
    }

    fn write_skill(dir: &Path, content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), content).unwrap();
    }

    fn entry<'a>(entries: &'a [SkillEntry], id: &str) -> &'a SkillEntry {
        entries.iter().find(|item| item.id == id).unwrap()
    }

    #[test]
    fn managed_skill_enables_into_other_agents_as_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/alpha"), "---\nname: Alpha\n---\n");
        manager.import_from_directory("alpha", &temp.path().join("external/alpha")).unwrap();

        manager.set_enabled_for("claude", "alpha", true).unwrap();
        let link = temp.path().join("claude-skills/alpha");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        let entries = manager.local_inventory();
        assert_eq!(entry(&entries, "alpha").agents, vec!["claude".to_string()]);
        assert!(entry(&entries, "alpha").managed);

        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(manager.source_dir.join("alpha/SKILL.md").is_file(), "停用不动源目录");
    }

    #[test]
    fn codex_local_skill_is_adopted_and_stays_enabled_in_codex() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&manager.linked_dir().join("beta"), "codex local");

        manager.set_enabled_for("claude", "beta", true).unwrap();
        assert_eq!(std::fs::read_to_string(manager.linked_dir().join("beta/SKILL.md")).unwrap(), "codex local", "Codex 照常可用");
        assert_eq!(std::fs::read_to_string(temp.path().join("claude-skills/beta/SKILL.md")).unwrap(), "codex local");
        assert!(manager.source_dir.join("beta/SKILL.md").is_file());
        let entries = manager.local_inventory();
        let agents = &entry(&entries, "beta").agents;
        assert!(agents.contains(&"codex".to_string()) && agents.contains(&"claude".to_string()));
    }

    #[test]
    fn agent_own_skill_is_listed_never_deleted_and_can_enable_into_codex() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let own = temp.path().join("claude-skills/gamma");
        write_skill(&own, "claude own");

        let entries = manager.local_inventory();
        let gamma = entry(&entries, "gamma");
        assert_eq!(gamma.agents, vec!["claude".to_string()]);
        assert!(!gamma.managed && !gamma.enabled);

        assert!(manager.set_enabled_for("claude", "gamma", false).is_err(), "Agent 自己的 Skill 不删");
        assert!(own.join("SKILL.md").is_file());

        manager.set_enabled_for("codex", "gamma", true).unwrap();
        assert_eq!(std::fs::read_to_string(manager.linked_dir().join("gamma/SKILL.md")).unwrap(), "claude own");
        assert!(own.join("SKILL.md").is_file(), "原目录保持不动");
    }

    #[test]
    fn uninstall_unlinks_other_agents() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/delta"), "delta");
        manager.import_from_directory("delta", &temp.path().join("external/delta")).unwrap();
        manager.set_enabled_for("grok", "delta", true).unwrap();
        manager.uninstall("delta").unwrap();
        assert!(std::fs::symlink_metadata(temp.path().join("grok-skills/delta")).is_err());
    }

    #[test]
    fn ccs_import_preserves_both_conflicting_user_copies() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        for (directory, content) in [(&external, "imported"), (&manager.source_dir.join("alpha"), "old source"), (&manager.linked_dir().join("alpha"), "user local")] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        manager.import_from_directory("alpha", &external).unwrap();
        assert!(!manager.linked_dir().join("alpha").exists());
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "imported");
        let contents: std::collections::BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md")).unwrap()).collect();
        assert_eq!(contents, ["old source".to_string(), "user local".to_string()].into_iter().collect());
    }

    #[test]
    fn ccs_import_save_failure_rolls_back_content_and_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        for (directory, content) in [(&external, "imported"), (&manager.source_dir.join("alpha"), "old source"), (&manager.linked_dir().join("alpha"), "user local")] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        std::fs::create_dir_all(&manager.state_path).unwrap();
        let before = manager.load_state();
        assert!(manager.import_from_directory("alpha", &external).is_err());
        assert_eq!(manager.load_state(), before);
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "old source");
        assert_eq!(std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(), "user local");
        assert!(manager.list_backups().is_empty());
    }

    #[test]
    fn ccs_import_keeps_source_but_defaults_to_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("SKILL.md"), "---\nname: imported\n---\n").unwrap();
        manager.import_from_directory("imported", &external).unwrap();
        let entries = manager.local_inventory();
        let skill = entries.iter().find(|skill| skill.id == "imported").unwrap();
        assert!(skill.installed);
        assert!(!skill.enabled);
        assert_eq!(skill.repo_key, "cc-switch");
        assert!(manager.source_dir().join("imported/SKILL.md").is_file());
        assert!(external.join("SKILL.md").is_file());
        manager.set_enabled("imported", true).unwrap();
        assert!(manager.local_inventory().iter().find(|skill| skill.id == "imported").unwrap().enabled);
    }

    #[test]
    fn saved_repository_toggle_is_local_but_identity_changes_need_validation() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut repo = parse_repo_key("unreachable-owner/absent-repo@main").unwrap();
        manager.upsert_repo(repo.clone()).unwrap();
        repo.enabled = false;
        assert!(manager.update_repo_enabled_if_unchanged(&repo, None).unwrap());
        assert!(!manager.list_repos().iter().find(|item| item.key() == repo.key()).unwrap().enabled);
        let old = repo.key();
        for field in ["other-owner/absent-repo@main", "unreachable-owner/other-repo@main", "unreachable-owner/absent-repo@dev", "unreachable-owner/absent-repo@main:other"] {
            let changed = parse_repo_key(field).unwrap();
            assert!(!manager.update_repo_enabled_if_unchanged(&changed, Some(&old)).unwrap());
            assert!(!manager.list_repos().iter().any(|item| item.key() == changed.key()));
        }
        repo.enabled = true;
        assert!(manager.update_repo_enabled_if_unchanged(&repo, Some(&old)).unwrap());
        assert!(manager.list_repos().iter().find(|item| item.key() == old).unwrap().enabled);
    }

    #[test]
    fn editing_repository_replaces_identity_and_rejects_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = parse_repo_key("acme/old@main").unwrap();
        manager.upsert_repo(original.clone()).unwrap();
        let replacement = parse_repo_key("acme/new@dev:skills").unwrap();
        let repos = manager.save_repo(replacement.clone(), Some(&original.key())).unwrap();
        assert!(!repos.iter().any(|repo| repo.key() == original.key()));
        assert_eq!(repos.iter().filter(|repo| repo.key() == replacement.key()).count(), 1);
        assert!(manager.save_repo(parse_repo_key("openai/skills@main:skills/.curated").unwrap(), Some(&replacement.key())).is_err());
        assert!(manager.list_repos().iter().any(|repo| repo.key() == replacement.key()));
    }

    #[test]
    fn updating_skill_keeps_user_edits_in_recoverable_backups() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager.install_from_zip(&sample_skill("alpha", "alpha", "one"), &repo_zip(&[("kit-main/alpha/SKILL.md", "original")])).unwrap();
        let local = manager.linked_dir().join("alpha");
        remove_link(&local).unwrap();
        std::fs::create_dir(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "local edits").unwrap();
        manager.install_from_zip(&sample_skill("alpha", "alpha", "two"), &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])).unwrap();
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "replacement");
        let contents: std::collections::BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md")).unwrap()).collect();
        assert_eq!(contents, ["original".to_string(), "local edits".to_string()].into_iter().collect());
    }

    #[test]
    fn installation_link_failure_does_not_publish_source_or_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        std::fs::create_dir_all(&manager.codex_home).unwrap();
        std::fs::write(manager.linked_dir(), "blocked").unwrap();
        let before = manager.load_state();
        assert!(manager.install_from_zip(&sample_skill("alpha", "alpha", "one"), &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])).is_err());
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(manager.load_state(), before);
        assert_eq!(std::fs::read_to_string(manager.linked_dir()).unwrap(), "blocked");
    }

    #[test]
    fn updating_skill_rolls_back_source_when_state_cannot_be_saved() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let skill = sample_skill("alpha", "alpha", "one");
        manager.install_from_zip(&skill, &repo_zip(&[("kit-main/alpha/SKILL.md", "original")])).unwrap();
        std::fs::rename(&manager.state_path, temp.path().join("original-state.json")).unwrap();
        std::fs::create_dir(&manager.state_path).unwrap();
        assert!(manager.install_from_zip(&sample_skill("alpha", "alpha", "two"), &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])).is_err());
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(), "original");
    }

    #[test]
    fn updating_disabled_skill_preserves_disabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]);
        manager.install_from_zip(&sample_skill("alpha", "alpha", "v1"), &zip).unwrap();
        manager.set_enabled("alpha", false).unwrap();
        manager.install_from_zip(&sample_skill("alpha", "alpha", "v2"), &zip).unwrap();
        let skills = manager.local_inventory();
        let skill = skills.iter().find(|skill| skill.id == "alpha").unwrap();
        assert!(!skill.enabled);
        assert_eq!(skill.content_hash, "v2");
    }

    /// 造一个 GitHub 风格的仓库 zip：最外层带 `repo-ref/` 前缀。
    fn repo_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::<u8>::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for (path, contents) in files {
                writer.start_file(*path, options).unwrap();
                writer.write_all(contents.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    fn sample_skill(id: &str, repo_path: &str, hash: &str) -> RemoteSkill {
        RemoteSkill {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            repo_key: "openai/skills@main:skills/.curated".to_string(),
            repo_path: repo_path.to_string(),
            content_hash: hash.to_string(),
        }
    }

    #[test]
    fn parses_name_and_description_from_frontmatter() {
        let (name, description) = parse_skill_frontmatter(
            "---\nname: hello-probe\ndescription: A probe skill\nmetadata:\n  short-description: ignored\n---\n\n# body\n",
            "fallback",
        );

        assert_eq!(name, "hello-probe");
        assert_eq!(description, "A probe skill");
    }

    #[test]
    fn frontmatter_falls_back_to_directory_name() {
        let (name, description) = parse_skill_frontmatter("# 没有 frontmatter\n", "my-skill");

        assert_eq!(name, "my-skill");
        assert!(description.is_empty());
    }

    #[test]
    fn frontmatter_reads_folded_and_literal_descriptions() {
        let (_, folded) = parse_skill_frontmatter(
            "---\nname: example\ndescription: >-\n  First line.\n  Second line.\nmetadata:\n  short-description: ignored\n---\n", "example");
        assert_eq!(folded, "First line. Second line.");
        let (_, literal) = parse_skill_frontmatter("---\ndescription: |\n  First\n  Second\n---\n", "example");
        assert_eq!(literal, "First\nSecond");
    }

    #[test]
    fn tree_listing_keeps_only_directories_with_a_manifest() {
        let repo = SkillRepo {
            owner: "openai".to_string(),
            name: "skills".to_string(),
            branch: "main".to_string(),
            subdir: "skills/.curated".to_string(),
            enabled: true,
        };
        let tree = json!({
            "tree": [
                {"type": "blob", "path": "skills/.curated/alpha/SKILL.md", "sha": "aaa"},
                {"type": "blob", "path": "skills/.curated/alpha/scripts/run.py", "sha": "bbb"},
                {"type": "blob", "path": "skills/.curated/beta/README.md", "sha": "ccc"},
                {"type": "blob", "path": "other/gamma/SKILL.md", "sha": "ddd"},
                {"type": "tree", "path": "skills/.curated/alpha", "sha": "eee"},
            ]
        });

        let skills = parse_skills_from_tree(&repo, &tree);

        assert_eq!(skills.len(), 1);
        assert!(skills[0].id.starts_with("alpha--"));
        assert_eq!(skills[0].repo_path, "skills/.curated/alpha");
        assert!(!skills[0].content_hash.is_empty());
    }

    #[test]
    fn adding_same_named_directory_does_not_reassign_installed_identity() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let before = parse_skills_from_tree(&repo, &json!({"tree":[{"type":"blob","path":"z/alpha/SKILL.md","sha":"one"}]}));
        manager.install_from_zip(&before[0], &repo_zip(&[("kit-main/z/alpha/SKILL.md", "original")])).unwrap();
        let after = parse_skills_from_tree(&repo, &json!({"tree":[
            {"type":"blob","path":"a/alpha/SKILL.md","sha":"new"},
            {"type":"blob","path":"z/alpha/SKILL.md","sha":"changed"}
        ]}));
        assert_eq!(before[0].id, after.iter().find(|s| s.repo_path == "z/alpha").unwrap().id);
        let entries = manager.merge_entries(&after);
        assert!(entries.iter().any(|s| s.repo_path == "z/alpha" && s.installed && s.update_available));
        assert!(entries.iter().any(|s| s.repo_path == "a/alpha" && !s.installed && !s.update_available));
        assert_eq!(manager.load_state().installed[&before[0].id].repo_path, "z/alpha");
    }

    #[test]
    fn legacy_install_without_path_is_not_assigned_to_a_remote_skill() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = sample_skill("alpha", "z/alpha", "one");
        manager.install_from_zip(&original, &repo_zip(&[("kit-main/z/alpha/SKILL.md", "original")])).unwrap();
        let mut state = manager.load_state();
        state.installed.get_mut("alpha").unwrap().repo_path.clear();
        manager.save_state_unlocked(&state).unwrap();
        let remote = sample_skill("alpha", "a/alpha", "new");
        assert!(!manager.merge_entries(&[remote.clone()]).iter().any(|s| s.repo_path == "a/alpha" && s.update_available));
        assert!(manager.install_from_zip(&remote, &repo_zip(&[("kit-main/a/alpha/SKILL.md", "wrong")])).is_err());
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "original");
    }

    #[test]
    fn discovers_nested_skills_and_same_named_directories_without_loss() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let tree = json!({ "tree": [
            {"type":"blob","path":"skills/.curated/alpha/SKILL.md","sha":"a"},
            {"type":"blob","path":"plugins/a/skills/alpha/SKILL.md","sha":"b"},
            {"type":"blob","path":"plugins/a/skills/alpha/scripts/run.py","sha":"c"},
            {"type":"blob","path":"beta/SKILL.md","sha":"d"},
            {"type":"blob","path":"ignored/README.md","sha":"e"}
        ]});
        let skills = parse_skills_from_tree(&repo, &tree);
        assert_eq!(skills.len(), 3);
        let ids: std::collections::BTreeSet<_> = skills.iter().map(|s| &s.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(skills.iter().any(|s| s.repo_path == "skills/.curated/alpha"));
        let scoped = parse_skills_from_tree(&parse_repo_key("acme/kit@main:plugins/a").unwrap(), &tree);
        assert_eq!(scoped.len(), 1);
        assert!(scoped[0].id.starts_with("alpha--"));
    }

    #[test]
    fn zip_catalog_reads_nested_manifests_and_root_of_selected_subdirectory() {
        let repo = parse_repo_key("acme/kit@main:plugins").unwrap();
        let zip = repo_zip(&[
            ("kit-main/plugins/alpha/SKILL.md", "---\nname: Actual alpha\ndescription: Nested skill\n---\n"),
            ("kit-main/plugins/alpha/scripts/run.py", "print('ok')"),
            ("kit-main/outside/beta/SKILL.md", "---\nname: beta\n---\n"),
        ]);
        let skills = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Actual alpha");
        assert_eq!(skills[0].description, "Nested skill");
        assert_eq!(skills[0].repo_path, "plugins/alpha");
        let exact = parse_repo_key("acme/kit@main:plugins/alpha").unwrap();
        assert_eq!(parse_skills_from_zip(&exact, &zip, &BTreeMap::new()).unwrap().len(), 1);
        // Git 对 "hello\n" 的 blob ID；API 和 ZIP 的子树标识必须一致。
        let bytes = repo_zip(&[("kit-main/alpha/SKILL.md", "hello\n")]);
        let root = parse_repo_key("acme/kit@main").unwrap();
        let zipped = parse_skills_from_zip(&root, &bytes, &BTreeMap::new()).unwrap();
        let tree = json!({"tree":[{"type":"blob","path":"alpha/SKILL.md",
            "sha":"ce013625030ba8dba906f756967f9e9ca394464a"}]});
        assert_eq!(zipped[0].content_hash, parse_skills_from_tree(&root, &tree)[0].content_hash);
    }

    #[test]
    fn merging_same_ids_from_different_repositories_preserves_both() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let first = sample_skill("alpha", "skills/alpha", "one");
        let mut second = first.clone();
        second.repo_key = "another/kit@main".into();
        let entries = manager.merge_entries(&[first, second]);
        assert_eq!(entries.iter().filter(|s| s.id == "alpha").count(), 2);
    }

    #[test]
    fn root_skill_is_discovered_and_installable() {
        let repo = parse_repo_key("acme/single-skill@main").unwrap();
        let zip = repo_zip(&[
            ("single-main/SKILL.md", "---\nname: Root skill\n---\n"),
            ("single-main/scripts/run.py", "print('ok')"),
        ]);
        let skills = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].repo_path, ".");
        assert!(skills[0].id.starts_with("single-skill--"));
        let temp = tempfile::tempdir().unwrap();
        extract_skill_subtree(&zip, ".", temp.path()).unwrap();
        assert!(temp.path().join("SKILL.md").is_file());
        assert!(temp.path().join("scripts/run.py").is_file());
    }

    #[test]
    fn tree_hash_tracks_remote_blob_changes() {
        let repo = SkillRepo {
            owner: "acme".to_string(),
            name: "kit".to_string(),
            branch: "main".to_string(),
            subdir: String::new(),
            enabled: true,
        };
        let before = parse_skills_from_tree(
            &repo,
            &json!({"tree": [{"type": "blob", "path": "alpha/SKILL.md", "sha": "aaa"}]}),
        );
        let after = parse_skills_from_tree(
            &repo,
            &json!({"tree": [{"type": "blob", "path": "alpha/SKILL.md", "sha": "zzz"}]}),
        );

        assert_ne!(before[0].content_hash, after[0].content_hash);
    }

    #[test]
    fn zip_entry_relative_path_strips_archive_root_and_rejects_escape() {
        assert_eq!(
            zip_entry_relative_path("skills-main/skills/alpha/SKILL.md"),
            Some(PathBuf::from("skills").join("alpha").join("SKILL.md"))
        );
        for path in [
            "skills-main/../evil.txt",
            "../evil.txt",
            "/tmp/evil.txt",
            "skills-main/",
        ] {
            assert_eq!(zip_entry_relative_path(path), None);
        }
    }

    #[test]
    fn extracts_only_the_requested_subtree() {
        let temp = tempfile::tempdir().unwrap();
        let zip = repo_zip(&[
            (
                "skills-main/skills/.curated/alpha/SKILL.md",
                "---\nname: alpha\n---\n",
            ),
            (
                "skills-main/skills/.curated/alpha/scripts/run.py",
                "print(1)\n",
            ),
            (
                "skills-main/skills/.curated/beta/SKILL.md",
                "---\nname: beta\n---\n",
            ),
        ]);
        let destination = temp.path().join("alpha");

        extract_skill_subtree(&zip, "skills/.curated/alpha", &destination).unwrap();

        assert!(destination.join("SKILL.md").is_file());
        assert!(destination.join("scripts").join("run.py").is_file());
        assert!(!destination.join("beta").exists());
    }

    #[test]
    fn extract_rejects_a_subtree_without_a_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let zip = repo_zip(&[("kit-main/alpha/README.md", "no manifest\n")]);

        let error = extract_skill_subtree(&zip, "alpha", &temp.path().join("alpha")).unwrap_err();

        assert!(error.to_string().contains("SKILL.md"));
    }

    #[test]
    fn install_writes_source_dir_and_links_into_codex_home() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let zip = repo_zip(&[(
            "skills-main/skills/.curated/alpha/SKILL.md",
            "---\nname: alpha\ndescription: demo\n---\n",
        )]);

        let state = manager
            .install_from_zip(
                &sample_skill("alpha", "skills/.curated/alpha", "hash-1"),
                &zip,
            )
            .unwrap();

        assert!(state.installed.contains_key("alpha"));
        assert!(
            manager
                .source_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );
        // codex 靠这个路径发现 skill
        assert!(
            manager
                .linked_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );
        // 暂存目录不能留下
        let leftovers = std::fs::read_dir(manager.source_dir())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".staging-"))
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn disabling_removes_the_link_but_keeps_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let zip = repo_zip(&[("skills-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]);
        manager
            .install_from_zip(&sample_skill("alpha", "alpha", "hash-1"), &zip)
            .unwrap();

        manager.set_enabled("alpha", false).unwrap();
        assert!(!manager.linked_dir().join("alpha").exists());
        assert!(manager.source_dir().join("alpha").is_dir());
        // 停用只能删链接：如果误用 remove_dir_all 顺着软链删下去，
        // SSOT 里的真实文件会一起没掉，停用就成了毁数据。
        assert!(
            manager
                .source_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );

        manager.set_enabled("alpha", true).unwrap();
        assert!(
            manager
                .linked_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );
    }

    /// 停用 → 重新启用 → 再停用，反复切换不该残留或报错。
    /// Windows 上目录软链必须用 remove_dir 删，用 remove_file 会 Access denied。
    #[test]
    fn toggling_enabled_repeatedly_is_stable() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "hash-1"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]),
            )
            .unwrap();

        for _ in 0..3 {
            manager.set_enabled("alpha", false).unwrap();
            assert!(!manager.linked_dir().join("alpha").exists());
            manager.set_enabled("alpha", true).unwrap();
            assert!(
                manager
                    .linked_dir()
                    .join("alpha")
                    .join("SKILL.md")
                    .is_file()
            );
        }
        // 源目录自始至终完好
        assert!(
            manager
                .source_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn uninstall_local_only_skill_preserves_and_restores_original_content() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("custom");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "user original").unwrap();
        assert!(manager.local_inventory().iter().any(|skill| skill.id == "custom" && skill.installed));
        manager.uninstall("custom").unwrap();
        let backups = manager.list_backups();
        assert_eq!(backups.len(), 1);
        assert!(!local.exists());
        manager.restore_backup(&backups[0].id).unwrap();
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "user original");
    }

    #[test]
    fn uninstall_preserves_distinct_source_and_local_copies() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        for (directory, content) in [(manager.source_dir.join("custom"), "source"), (manager.linked_dir().join("custom"), "edited local")] {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        manager.uninstall("custom").unwrap();
        let contents: std::collections::BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md")).unwrap()).collect();
        assert_eq!(contents, ["source".to_string(), "edited local".to_string()].into_iter().collect());
    }

    #[test]
    fn uninstall_backup_failure_leaves_local_content_intact() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("custom");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "original").unwrap();
        std::fs::write(&manager.backups_dir, "blocked").unwrap();
        assert!(manager.uninstall("custom").is_err());
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "original");
    }

    #[test]
    fn uninstall_backs_up_the_source_and_can_restore_it() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let zip = repo_zip(&[(
            "skills-main/alpha/SKILL.md",
            "---\nname: alpha\ndescription: demo\n---\n",
        )]);
        manager
            .install_from_zip(&sample_skill("alpha", "alpha", "hash-1"), &zip)
            .unwrap();

        let physical_copy = !std::fs::symlink_metadata(manager.linked_dir().join("alpha"))
            .unwrap().file_type().is_symlink();

        let state = manager.uninstall("alpha").unwrap();
        assert!(!state.installed.contains_key("alpha"));
        assert!(!manager.source_dir().join("alpha").exists());
        assert!(!manager.linked_dir().join("alpha").exists());

        let backups = manager.list_backups();
        assert_eq!(backups.len(), if physical_copy { 2 } else { 1 });
        assert_eq!(backups[0].skill_id, "alpha");

        let restored = manager.restore_backup(&backups[0].id).unwrap();
        assert!(restored.installed.contains_key("alpha"));
        assert!(
            manager
                .source_dir()
                .join("alpha")
                .join("SKILL.md")
                .is_file()
        );
        assert_eq!(manager.list_backups().len(), usize::from(physical_copy));
    }

    #[test]
    fn reinstalling_replaces_the_source_and_refreshes_the_hash() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "hash-1"),
                &repo_zip(&[
                    ("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n"),
                    ("kit-main/alpha/old.txt", "old\n"),
                ]),
            )
            .unwrap();

        let state = manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "hash-2"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]),
            )
            .unwrap();

        assert_eq!(state.installed["alpha"].content_hash, "hash-2");
        // 旧版本残留的文件必须被清掉，不能和新版本混在一起
        assert!(!manager.source_dir().join("alpha").join("old.txt").exists());
        // 顶替成功后不能把过渡用的备份目录留在原地
        assert!(
            !manager
                .source_dir()
                .join("alpha.previous-codex-plus")
                .exists(),
            "顶替完成后不该残留 .previous-codex-plus"
        );
    }

    #[test]
    fn merged_entries_flag_available_updates() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "hash-1"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]),
            )
            .unwrap();

        let entries = manager.merge_entries(&[
            sample_skill("alpha", "alpha", "hash-2"),
            sample_skill("beta", "beta", "hash-9"),
        ]);

        let alpha = entries.iter().find(|entry| entry.id == "alpha").unwrap();
        assert!(alpha.installed);
        assert!(alpha.enabled);
        assert!(alpha.update_available);

        let beta = entries.iter().find(|entry| entry.id == "beta").unwrap();
        assert!(!beta.installed);
        assert!(!beta.update_available);
    }

    #[test]
    fn merged_entries_include_codex_bundled_skills_as_read_only() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let bundled = manager
            .linked_dir()
            .join(BUNDLED_SKILLS_DIR)
            .join("imagegen");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(
            bundled.join(SKILL_MANIFEST_FILE),
            "---\nname: imagegen\ndescription: 内置\n---\n",
        )
        .unwrap();

        let entries = manager.merge_entries(&[]);

        let imagegen = entries.iter().find(|entry| entry.id == "imagegen").unwrap();
        assert!(imagegen.bundled);
        assert_eq!(imagegen.description, "内置");
    }

    /// #862：用户让 Codex 自己建的 skill 只落在 `$CODEX_HOME/skills/<id>/`，
    /// 不进 state.installed、也不在远端清单里，此前在管理工具里完全不显示。
    #[test]
    fn merged_entries_include_locally_created_skills() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("my-own-skill");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(
            local.join(SKILL_MANIFEST_FILE),
            "---\nname: 我的技能\ndescription: 本地自建\n---\n",
        )
        .unwrap();

        let entries = manager.merge_entries(&[]);

        let entry = entries
            .iter()
            .find(|entry| entry.id == "my-own-skill")
            .unwrap();
        assert_eq!(entry.name, "我的技能");
        assert_eq!(entry.description, "本地自建");
        assert!(!entry.bundled);
        assert!(entry.installed);
        assert!(entry.enabled);
    }

    #[test]
    fn locally_created_skill_can_be_disabled_without_deleting_it() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("my-own-skill");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join(SKILL_MANIFEST_FILE), "---\nname: local\n---\n").unwrap();

        manager.set_enabled("my-own-skill", false).unwrap();

        assert!(!manager.linked_dir().join("my-own-skill").exists());
        assert!(manager.source_dir().join("my-own-skill").join(SKILL_MANIFEST_FILE).exists());
        assert!(!manager.merge_entries(&[]).iter().find(|entry| entry.id == "my-own-skill").unwrap().enabled);

        manager.set_enabled("my-own-skill", true).unwrap();
        assert!(manager.linked_dir().join("my-own-skill").exists());
    }

    /// 没有 SKILL.md 的目录、以及 `.system` 自带目录，都不应被当成「本地自建」重复收录。
    #[test]
    fn local_scan_ignores_manifest_less_and_bundled_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        std::fs::create_dir_all(manager.linked_dir().join("no-manifest")).unwrap();
        let bundled = manager.linked_dir().join(BUNDLED_SKILLS_DIR).join("imagegen");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::write(
            bundled.join(SKILL_MANIFEST_FILE),
            "---\nname: imagegen\ndescription: 内置\n---\n",
        )
        .unwrap();

        let entries = manager.merge_entries(&[]);

        assert!(entries.iter().all(|entry| entry.id != "no-manifest"));
        let imagegen: Vec<_> = entries.iter().filter(|entry| entry.id == "imagegen").collect();
        assert_eq!(imagegen.len(), 1, "自带 skill 不应被本地扫描重复收录");
        assert!(imagegen[0].bundled);
    }

    #[test]
    fn local_inventory_lists_custom_and_bundled_skills_without_writing_config() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let custom = manager.linked_dir().join("custom-skill");
        let bundled = manager.linked_dir().join(BUNDLED_SKILLS_DIR).join("builtin-skill");
        for path in [&custom, &bundled] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(path.join(SKILL_MANIFEST_FILE), "---\nname: test\ndescription: local\n---\n").unwrap();
        }
        let config = manager.codex_home.join("config.toml");
        std::fs::write(&config, "[skills]\nbundled = true\n").unwrap();
        let before = std::fs::read(&config).unwrap();
        let entries = manager.local_inventory();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.id == "custom-skill" && entry.enabled));
        assert!(entries.iter().any(|entry| entry.id == "builtin-skill" && entry.bundled));
        assert_eq!(std::fs::read(config).unwrap(), before);
    }

    /// #1989：四个仓库全部「文件树返回错误状态」，看不出是限流、网络还是仓库没了。
    /// GitHub 未认证请求每小时只有 60 次，四个仓库一起刷很容易撞上 403，
    /// 但错误信息把状态码吞掉了，用户既没法自助也没法准确报障。
    #[test]
    fn github_status_hints_are_actionable() {
        assert!(github_status_hint(403).contains("限流"));
        assert!(github_status_hint(403).contains("GITHUB_TOKEN"));
        assert!(github_status_hint(429).contains("限流"));
        assert!(github_status_hint(404).contains("不存在"));
        assert!(github_status_hint(401).contains("过期"));
        // 没有针对性提示的状态码不该硬凑一句，让原始状态码自己说话
        assert!(github_status_hint(500).is_empty());
    }

    #[test]
    fn repo_keys_round_trip() {
        let repo = parse_repo_key("openai/skills@main:skills/.curated").unwrap();

        assert_eq!(repo.owner, "openai");
        assert_eq!(repo.name, "skills");
        assert_eq!(repo.branch, "main");
        assert_eq!(repo.subdir, "skills/.curated");
        assert_eq!(repo.key(), "openai/skills@main:skills/.curated");

        let bare = parse_repo_key("anthropics/skills").unwrap();
        assert_eq!(bare.branch, "main");
        assert!(bare.subdir.is_empty());
        assert_eq!(bare.key(), "anthropics/skills@main");
    }

    #[test]
    fn repo_upsert_replaces_by_key_and_delete_removes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = SkillRepo {
            owner: "acme".to_string(),
            name: "kit".to_string(),
            branch: "main".to_string(),
            subdir: "/skills/".to_string(),
            enabled: true,
        };

        let repos = manager.upsert_repo(repo.clone()).unwrap();
        let stored = repos.iter().find(|item| item.owner == "acme").unwrap();
        assert_eq!(stored.subdir, "skills");

        let repos = manager
            .upsert_repo(SkillRepo {
                enabled: false,
                ..stored.clone()
            })
            .unwrap();
        assert_eq!(repos.iter().filter(|item| item.owner == "acme").count(), 1);
        assert!(
            !repos
                .iter()
                .find(|item| item.owner == "acme")
                .unwrap()
                .enabled
        );

        let repos = manager.delete_repo("acme/kit@main:skills").unwrap();
        assert!(!repos.iter().any(|item| item.owner == "acme"));
    }

    #[test]
    fn repo_upsert_rejects_path_like_segments() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);

        let error = manager
            .upsert_repo(SkillRepo {
                owner: "../evil".to_string(),
                name: "kit".to_string(),
                branch: "main".to_string(),
                subdir: String::new(),
                enabled: true,
            })
            .unwrap_err();

        assert!(error.to_string().contains("owner/name"));
    }

    #[test]
    fn skill_ids_that_escape_the_source_dir_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);

        assert!(manager.set_enabled("../escape", true).is_err());
        assert!(manager.uninstall("..").is_err());
        assert!(
            manager
                .install_from_zip(&sample_skill("../escape", "alpha", "h"), &repo_zip(&[]))
                .is_err()
        );
    }

    #[test]
    fn default_state_seeds_the_builtin_repositories() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);

        let repos = manager.list_repos();

        assert!(repos.iter().any(|repo| repo.owner == "openai"));
        assert!(repos.iter().any(|repo| repo.owner == "anthropics"));
    }
}
