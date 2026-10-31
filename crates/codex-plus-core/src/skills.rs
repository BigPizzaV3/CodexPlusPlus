//! Codex Skills 管理。
//!
//! codex 的 skill 是文件系统约定，不是配置项：它扫描 `$CODEX_HOME/skills/<id>/SKILL.md`，
//! 从 YAML frontmatter 读 `name` / `description`，然后把清单注入 `<skills_instructions>`。
//! `config.toml` 里的 `[skills]` 是一个三字段结构体（bundled / include_instructions /
//! max_context_tokens），写 `[skills.<id>]` 会被 serde 当未知字段静默丢弃，什么都不会发生。
//!
//! 所以这里采用独立管理库与各 Agent 副本模型：
//!
//! - 安装：下载仓库 zip，只解出目标 skill 子树到 `~/.codex-session-delete/skills/<id>/`
//! - 启用：将管理库复制到 `$CODEX_HOME/skills/<id>`，避免外部修改写穿管理库
//! - 停用：保留管理库，并检查副本是否有未解决的修改
//! - 卸载：源目录整体移到 `~/.codex-session-delete/skill-backups/<id>-<ts>/`
//! - 更新检测：GitHub trees API 返回每个文件的 blob sha，据此算子树哈希，不下载内容
//!
//! 旧版本建立的软链会逐步迁移为独立副本。

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

mod import;
mod source_match;
mod transaction;
pub use import::{SkillImportCandidate, SkillImportMatch, SkillImportSelection};
pub use source_match::{
    SkillRepositoryMatch, SkillRepositoryMatchPreview, SkillRepositoryMatchSelection,
};
use transaction::{PreparedSkillChange, SkillPlace};

const SKILL_MANIFEST_FILE: &str = "SKILL.md";
const BUNDLED_SKILLS_DIR: &str = ".system";
const UNVERIFIED_CC_SWITCH_BASELINE: &str = "unverified-cc-switch-origin";
/// 从本地目录导入的 Skill 的来源标记（#511），前端按它显示「本地导入」分组。
pub const LOCAL_IMPORT_REPO_KEY: &str = "local";
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
    /// 导入渠道与仓库来源分别记录，避免将第三方工具当成 Skill 仓库。
    #[serde(default)]
    pub import_source: String,
    /// 安装时的本地内容，用于识别已脱离远端快照的修改。
    #[serde(default)]
    pub local_baseline_hash: String,
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
    #[serde(default)]
    pub independent_library_migrated: bool,
    /// 用户删除过的导入仓库不再自动重新添加。
    #[serde(default)]
    pub suppressed_imported_repos: std::collections::BTreeSet<String>,
}

impl Default for SkillsState {
    fn default() -> Self {
        Self {
            repos: default_skill_repos(),
            installed: BTreeMap::new(),
            independent_library_migrated: false,
            suppressed_imported_repos: std::collections::BTreeSet::new(),
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
    pub import_source: String,
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
    /// Codex 副本与独立管理库内容不同，需要用户选择版本。
    pub codex_conflict: bool,
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
    legacy_source_dir: Option<PathBuf>,
    backups_dir: PathBuf,
    state_path: PathBuf,
    codex_home: PathBuf,
    /// Codex 之外也读 `<dir>/<id>/SKILL.md` 的 Agent：(Agent id, skills 目录)。
    agent_dirs: Vec<(String, PathBuf)>,
    state_lock: Arc<Mutex<()>>,
    operation_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyUpdateState {
    Clean,
    Modified,
    Unknown,
}

#[derive(Debug)]
struct LocalSkillStageError(anyhow::Error);

impl std::fmt::Display for LocalSkillStageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "无法暂存本地 Skill：{:#}", self.0)
    }
}

impl std::error::Error for LocalSkillStageError {}

fn legacy_update_state(
    installed: &InstalledSkill,
    source: &Path,
) -> anyhow::Result<LegacyUpdateState> {
    if !installed.local_baseline_hash.is_empty() {
        return Ok(
            if skill_directory_hash(source)? == installed.local_baseline_hash {
                LegacyUpdateState::Clean
            } else {
                LegacyUpdateState::Modified
            },
        );
    }
    let hash = &installed.content_hash;
    if hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(if skill_git_tree_hash(source)?.eq_ignore_ascii_case(hash) {
            LegacyUpdateState::Clean
        } else {
            LegacyUpdateState::Modified
        });
    }
    Ok(LegacyUpdateState::Unknown)
}

fn skills_state_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let path = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or_else(|| path.to_path_buf());
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .unwrap();
    locks.retain(|_, value| value.strong_count() > 0);
    if let Some(lock) = locks.get(&path).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path, Arc::downgrade(&lock));
    lock
}

impl SkillsManager {
    pub fn new(
        source_dir: impl Into<PathBuf>,
        backups_dir: impl Into<PathBuf>,
        state_path: impl Into<PathBuf>,
        codex_home: impl Into<PathBuf>,
    ) -> Self {
        let state_path = state_path.into();
        let state_lock = skills_state_lock(&state_path);
        let operation_lock = skills_state_lock(&state_path.with_extension("skills-operation-lock"));
        Self {
            source_dir: source_dir.into(),
            legacy_source_dir: None,
            backups_dir: backups_dir.into(),
            state_path,
            codex_home: codex_home.into(),
            agent_dirs: Vec::new(),
            state_lock,
            operation_lock,
        }
    }

    /// 让 Skill 也能启用到其它 Agent（例如 Claude Code 的 `~/.claude/skills`、Grok 的 `~/.grok/skills`）。
    pub fn with_agent_dirs(mut self, dirs: Vec<(String, PathBuf)>) -> Self {
        self.agent_dirs = dirs;
        self
    }

    pub fn with_legacy_source_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.legacy_source_dir = Some(path.into());
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

    /// 各 Agent 都使用独立副本；显式停用外部目录时先纳入管理库并备份原内容。
    pub fn set_enabled_for(&self, agent: &str, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let existing_agent_skill = agent != "codex"
            && self
                .agent_dir(agent)?
                .join(id)
                .join(SKILL_MANIFEST_FILE)
                .is_file();
        if enabled || existing_agent_skill {
            self.ensure_managed_source(id)?;
        }
        if agent == "codex" {
            return self.set_enabled_unlocked(id, enabled);
        }
        let dir = self.agent_dir(agent)?;
        let link = dir.join(id);
        let metadata = std::fs::symlink_metadata(&link);
        if !enabled {
            match metadata {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
            anyhow::ensure!(
                link.join(SKILL_MANIFEST_FILE).is_file(),
                "Agent 的 Skill 目录无法读取，未停用：{}",
                link.display()
            );
            let hash = skill_directory_hash(&link)?;
            let operation = uuid::Uuid::new_v4().to_string();
            return self.commit_skill_transaction(
                &operation,
                id,
                vec![PreparedSkillChange {
                    place: SkillPlace::Agent(agent.to_string()),
                    staged: false,
                    preserve_backup: true,
                    expected_backup_hash: Some(hash),
                }],
                None,
            );
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
        let operation = uuid::Uuid::new_v4().to_string();
        let place = SkillPlace::Agent(agent.to_string());
        self.stage_skill_copy(&operation, 0, &place, &source, true)?;
        self.commit_skill_transaction(
            &operation,
            id,
            vec![PreparedSkillChange {
                place,
                staged: true,
                preserve_backup: true,
                expected_backup_hash: None,
            }],
            None,
        )
    }

    /// 将已有 Skill 复制进独立管理库，原有 Agent 目录保持不动。
    fn ensure_managed_source(&self, id: &str) -> anyhow::Result<()> {
        if self.source_dir.join(id).join(SKILL_MANIFEST_FILE).is_file() {
            let _state = self.state_lock.lock().unwrap();
            let mut state = self.load_state_checked_unlocked()?;
            if !state.installed.contains_key(id) {
                let (name, description) =
                    read_skill_manifest(&self.source_dir.join(id).join(SKILL_MANIFEST_FILE), id);
                state.installed.insert(
                    id.to_string(),
                    InstalledSkill {
                        id: id.to_string(),
                        name,
                        description,
                        repo_key: String::new(),
                        repo_path: String::new(),
                        import_source: String::new(),
                        local_baseline_hash: skill_directory_hash(&self.source_dir.join(id))?,
                        content_hash: String::new(),
                        installed_at: current_unix_timestamp_string(),
                    },
                );
                self.save_state_unlocked(&state)?;
            }
            return Ok(());
        }
        let codex = self.linked_dir().join(id);
        let candidate = std::iter::once(codex.clone())
            .chain(self.agent_dirs.iter().map(|(_, dir)| dir.join(id)))
            .find(|path| path.join(SKILL_MANIFEST_FILE).is_file());
        let Some(candidate) = candidate else {
            if std::fs::symlink_metadata(&codex).is_ok() {
                anyhow::bail!(
                    "Codex 的 skills 目录里已有同名内容，未导入：{}",
                    codex.display()
                );
            }
            return Ok(());
        };
        std::fs::create_dir_all(&self.source_dir)?;
        anyhow::ensure!(
            !self
                .source_dir
                .canonicalize()?
                .starts_with(candidate.canonicalize()?),
            "Skill 目录不能包含受控源目录"
        );
        let operation = uuid::Uuid::new_v4().to_string();
        self.stage_skill_copy(&operation, 0, &SkillPlace::Library, &candidate, false)
            .map_err(LocalSkillStageError)?;
        let baseline =
            skill_directory_hash(&self.transaction_stage(&operation, 0, &SkillPlace::Library)?)?;
        let (name, description) = read_skill_manifest(&candidate.join(SKILL_MANIFEST_FILE), id);
        let _state = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        state
            .installed
            .entry(id.to_string())
            .or_insert(InstalledSkill {
                id: id.to_string(),
                name,
                description,
                repo_key: String::new(),
                repo_path: String::new(),
                import_source: String::new(),
                local_baseline_hash: baseline,
                content_hash: String::new(),
                installed_at: current_unix_timestamp_string(),
            });
        self.commit_skill_transaction(
            &operation,
            id,
            vec![PreparedSkillChange {
                place: SkillPlace::Library,
                staged: true,
                preserve_backup: true,
                expected_backup_hash: None,
            }],
            Some(&state),
        )
    }

    pub fn source_dir(&self) -> &Path {
        &self.source_dir
    }

    /// codex 扫描的 skill 目录；由本工具启用的条目是独立副本。
    pub fn linked_dir(&self) -> PathBuf {
        self.codex_home.join("skills")
    }

    /// 启动 Codex 前准备独立库，并在调用方完成启动或重启前阻止新的 Skill 写入。
    pub fn lock_recovered_transactions(&self) -> anyhow::Result<std::sync::MutexGuard<'_, ()>> {
        let operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        Ok(operation)
    }

    /// 恢复中断事务，并将旧版指向管理库的软链迁移为独立副本。
    pub fn prepare_managed_skills(&self) -> anyhow::Result<()> {
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()
    }

    fn prepare_managed_skills_unlocked(&self) -> anyhow::Result<()> {
        {
            let _state = self.state_lock.lock().unwrap();
            self.recover_skill_transactions()?;
        }
        self.load_state_checked_unlocked()?;
        let mut migration_pending = false;
        if let Some(legacy) = &self.legacy_source_dir {
            if !self.load_state().independent_library_migrated {
                migration_pending = true;
                let mut ids: std::collections::BTreeSet<String> =
                    self.load_state().installed.into_keys().collect();
                match std::fs::read_dir(legacy) {
                    Ok(entries) => {
                        for entry in entries {
                            let entry = entry?;
                            if let Some(id) =
                                entry.file_name().to_str().filter(|id| !id.starts_with('.'))
                            {
                                if entry.path().join(SKILL_MANIFEST_FILE).is_file() {
                                    ids.insert(id.to_string());
                                }
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("旧 Skill 库无法读取，未完成迁移"),
                }
                for id in ids {
                    validate_skill_id(&id)?;
                    let old = legacy.join(&id);
                    let destination = self.source_dir.join(&id);
                    if std::fs::symlink_metadata(&destination).is_ok() {
                        continue;
                    }
                    if !old.join(SKILL_MANIFEST_FILE).is_file() {
                        anyhow::ensure!(
                            std::fs::symlink_metadata(&old).is_err(),
                            "旧 Skill 管理目录无法读取，未完成迁移：{}",
                            old.display()
                        );
                        continue;
                    }
                    let operation = uuid::Uuid::new_v4().to_string();
                    self.stage_skill_copy(&operation, 0, &SkillPlace::Library, &old, false)?;
                    self.commit_skill_transaction(
                        &operation,
                        &id,
                        vec![PreparedSkillChange {
                            place: SkillPlace::Library,
                            staged: true,
                            preserve_backup: false,
                            expected_backup_hash: None,
                        }],
                        None,
                    )?;
                }
            }
        }
        // 旧版未登记的 Codex 自建 Skill 和孤立管理库目录也需要纳入独立库。
        let mut registered: std::collections::BTreeSet<_> =
            self.load_state().installed.into_keys().collect();
        std::fs::create_dir_all(&self.source_dir)?;
        for skill in self.list_local_skills() {
            if registered.insert(skill.id.clone()) {
                let local = self.linked_dir().join(&skill.id);
                if let Err(error) = self.ensure_managed_source(&skill.id) {
                    if !is_managed_copy(&local) && error.is::<LocalSkillStageError>() {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "skills.local_adoption_skipped",
                            serde_json::json!({ "id": skill.id, "message": format!("{error:#}") }),
                        );
                        continue;
                    }
                    return Err(error);
                }
            }
        }
        for (id, _) in scan_skill_dirs(&self.source_dir) {
            if registered.insert(id.clone()) {
                self.ensure_managed_source(&id)?;
            }
        }
        // 老版本可能用复制标记把其它 Agent 的副本作为唯一剩余目录。
        for (_, root) in &self.agent_dirs {
            for (id, _) in scan_skill_dirs(root) {
                if is_managed_copy(&root.join(&id)) && registered.insert(id.clone()) {
                    self.ensure_managed_source(&id)?;
                }
            }
        }
        let state = self.load_state();
        for id in state.installed.keys() {
            validate_skill_id(id)?;
            let source = self.source_dir.join(id);
            if std::fs::symlink_metadata(&source)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
                && !source.join(SKILL_MANIFEST_FILE).is_file()
            {
                anyhow::bail!("Skill 管理库链接已失效，未迁移或覆盖：{}", source.display());
            }
        }
        for id in state.installed.keys() {
            validate_skill_id(id)?;
            let source = self.source_dir.join(id);
            if !source.join(SKILL_MANIFEST_FILE).is_file() {
                continue;
            }
            let source_was_link = std::fs::symlink_metadata(&source)?.file_type().is_symlink();
            if source_was_link {
                let operation = uuid::Uuid::new_v4().to_string();
                let expected = skill_directory_hash(&source)?;
                let mut changes = Vec::new();
                let mut dependent = vec![(SkillPlace::Codex, self.linked_dir())];
                dependent.extend(
                    self.agent_dirs
                        .iter()
                        .map(|(agent, dir)| (SkillPlace::Agent(agent.clone()), dir.clone())),
                );
                // 库链接移动后，相对或绝对链接可能失效；先固定所有指向相同目标的已启用副本。
                for (place, root) in dependent {
                    let target = root.join(id);
                    if std::fs::symlink_metadata(&target)
                        .is_ok_and(|meta| meta.file_type().is_symlink())
                        && link_points_to(&target, &source)
                    {
                        self.stage_skill_copy(&operation, changes.len(), &place, &source, true)?;
                        changes.push(PreparedSkillChange {
                            place,
                            staged: true,
                            preserve_backup: false,
                            expected_backup_hash: Some(expected.clone()),
                        });
                    }
                }
                self.stage_skill_copy(
                    &operation,
                    changes.len(),
                    &SkillPlace::Library,
                    &source,
                    false,
                )?;
                changes.push(PreparedSkillChange {
                    place: SkillPlace::Library,
                    staged: true,
                    preserve_backup: false,
                    expected_backup_hash: Some(expected),
                });
                self.commit_skill_transaction(&operation, id, changes, None)?;
            }
            let mut places = vec![(SkillPlace::Codex, self.linked_dir())];
            places.extend(
                self.agent_dirs
                    .iter()
                    .map(|(agent, dir)| (SkillPlace::Agent(agent.clone()), dir.clone())),
            );
            for (place, root) in places {
                let target = root.join(id);
                if !std::fs::symlink_metadata(&target)
                    .is_ok_and(|meta| meta.file_type().is_symlink())
                {
                    continue;
                }
                let owned_link = link_points_to(&target, &source)
                    || self
                        .legacy_source_dir
                        .as_ref()
                        .is_some_and(|root| link_points_to(&target, &root.join(id)));
                if !owned_link {
                    continue;
                }
                if skill_directory_hash(&target)? != skill_directory_hash(&source)? {
                    continue;
                }
                let operation = uuid::Uuid::new_v4().to_string();
                let expected = skill_directory_hash(&source)?;
                self.stage_skill_copy(&operation, 0, &place, &source, true)?;
                self.commit_skill_transaction(
                    &operation,
                    id,
                    vec![PreparedSkillChange {
                        place,
                        staged: true,
                        preserve_backup: false,
                        expected_backup_hash: Some(expected),
                    }],
                    None,
                )?;
            }
        }
        if migration_pending {
            let _state = self.state_lock.lock().unwrap();
            let mut state = self.load_state_checked_unlocked()?;
            state.independent_library_migrated = true;
            self.save_state_unlocked(&state)?;
        }
        Ok(())
    }

    pub fn codex_copy_conflict(&self, id: &str) -> anyhow::Result<bool> {
        validate_skill_id(id)?;
        let source = self.source_dir.join(id);
        let codex = self.linked_dir().join(id);
        if !source.join(SKILL_MANIFEST_FILE).is_file() || std::fs::symlink_metadata(&codex).is_err()
        {
            return Ok(false);
        }
        if !codex.join(SKILL_MANIFEST_FILE).is_file() {
            return Ok(true);
        }
        Ok(skill_directory_hash(&source)? != skill_directory_hash(&codex)?)
    }

    /// 返回本地修改需要确认时的快照；确认后仍会在提交时重新校验。
    pub fn update_local_snapshot(&self, id: &str) -> anyhow::Result<Option<String>> {
        validate_skill_id(id)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _state = self.state_lock.lock().unwrap();
        let state = self.load_state_checked_unlocked()?;
        let Some(installed) = state.installed.get(id) else {
            return Ok(None);
        };
        let source = self.source_dir.join(id);
        anyhow::ensure!(
            source.join(SKILL_MANIFEST_FILE).is_file(),
            "Skill 管理库缺少完整内容，未覆盖"
        );
        if legacy_update_state(installed, &source)? == LegacyUpdateState::Clean {
            return Ok(None);
        }
        Ok(Some(self.import_target_snapshot_unlocked(&state, id)?))
    }

    /// 用户明确选择版本后才替换；被替换的内容进入可恢复备份。
    pub fn resolve_codex_conflict(&self, id: &str, keep_codex: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let source = self.source_dir.join(id);
        let codex = self.linked_dir().join(id);
        anyhow::ensure!(
            source.join(SKILL_MANIFEST_FILE).is_file() && codex.join(SKILL_MANIFEST_FILE).is_file(),
            "两个版本都必须包含 SKILL.md，未替换任何内容"
        );
        if !self.codex_copy_conflict(id)? {
            return Ok(());
        }
        let operation = uuid::Uuid::new_v4().to_string();
        let (place, selected, replaced) = if keep_codex {
            (SkillPlace::Library, &codex, &source)
        } else {
            (SkillPlace::Codex, &source, &codex)
        };
        let expected = skill_directory_hash(replaced)?;
        self.stage_skill_copy(&operation, 0, &place, selected, !keep_codex)?;
        let mut changes = vec![PreparedSkillChange {
            place,
            staged: true,
            preserve_backup: true,
            expected_backup_hash: Some(expected.clone()),
        }];
        if keep_codex {
            let library_stage = self.transaction_stage(&operation, 0, &SkillPlace::Library)?;
            // 外部软链不能继续暴露第三方目录；所选内容已复制进新的独立副本。
            if std::fs::symlink_metadata(&codex)?.file_type().is_symlink() {
                let codex_hash = skill_directory_hash(&codex)?;
                self.stage_skill_copy(
                    &operation,
                    changes.len(),
                    &SkillPlace::Codex,
                    &library_stage,
                    true,
                )?;
                changes.push(PreparedSkillChange {
                    place: SkillPlace::Codex,
                    staged: true,
                    preserve_backup: true,
                    expected_backup_hash: Some(codex_hash),
                });
            }
            for (agent, root) in &self.agent_dirs {
                let target = root.join(id);
                if !is_managed_copy(&target) || skill_directory_hash(&target)? != expected {
                    continue;
                }
                let place = SkillPlace::Agent(agent.clone());
                self.stage_skill_copy(&operation, changes.len(), &place, &library_stage, true)?;
                changes.push(PreparedSkillChange {
                    place,
                    staged: true,
                    preserve_backup: true,
                    expected_backup_hash: Some(expected.clone()),
                });
            }
        }
        let _state = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        if keep_codex {
            if let Some(installed) = state.installed.get_mut(id) {
                let (name, description) = read_skill_manifest(&codex.join(SKILL_MANIFEST_FILE), id);
                installed.name = name;
                installed.description = description;
            }
        }
        self.commit_skill_transaction(&operation, id, changes, Some(&state))?;
        anyhow::ensure!(
            !self.codex_copy_conflict(id)?,
            "Skill 在版本处理期间再次发生变化，请重新选择"
        );
        Ok(())
    }

    pub fn load_state(&self) -> SkillsState {
        let _guard = self.state_lock.lock().unwrap();
        self.load_state_unlocked()
    }

    fn load_state_unlocked(&self) -> SkillsState {
        self.load_state_checked_unlocked().unwrap_or_default()
    }

    fn load_state_checked_unlocked(&self) -> anyhow::Result<SkillsState> {
        match std::fs::read(&self.state_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Skill 状态文件损坏，未覆盖"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(SkillsState::default())
            }
            Err(error) => Err(error).context("Skill 状态文件无法读取，未覆盖"),
        }
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

    /// 验证仓库并解析默认分支后，仅在最终身份尚不存在时添加。
    pub async fn add_validated_repo(
        &self,
        repo: SkillRepo,
        cached: &BTreeMap<String, RemoteSkill>,
    ) -> anyhow::Result<(SkillRepo, Vec<RemoteSkill>)> {
        self.add_repo_after_validation(resolve_repo_skills(repo, cached))
            .await
    }

    async fn add_repo_after_validation(
        &self,
        validation: impl std::future::Future<Output = anyhow::Result<(SkillRepo, Vec<RemoteSkill>)>>,
    ) -> anyhow::Result<(SkillRepo, Vec<RemoteSkill>)> {
        let (repo, skills) = validation.await.context("仓库验证失败，未保存")?;
        self.add_repo_if_absent(repo.clone())?;
        Ok((repo, skills))
    }

    fn add_repo_if_absent(&self, repo: SkillRepo) -> anyhow::Result<()> {
        let repo = normalize_repo(repo)?;
        let _operation = self.operation_lock.lock().unwrap();
        let _guard = self.state_lock.lock().unwrap();
        self.recover_skill_transactions()?;
        let mut state = self.load_state_checked_unlocked()?;
        anyhow::ensure!(
            !state.repos.iter().any(|existing| {
                normalize_repo(existing.clone()).is_ok_and(|existing| {
                    existing.owner.eq_ignore_ascii_case(&repo.owner)
                        && existing.name.eq_ignore_ascii_case(&repo.name)
                        && existing.branch == repo.branch
                        && existing.subdir == repo.subdir
                })
            }),
            "仓库已存在，未修改现有配置"
        );
        state.suppressed_imported_repos.remove(&repo.key());
        state.repos.push(repo);
        self.save_state_unlocked(&state)
    }

    /// 仅启停既有源不依赖网络；身份变更必须由调用方验证后保存。
    pub fn update_repo_enabled_if_unchanged(
        &self,
        repo: &SkillRepo,
        previous_key: Option<&str>,
    ) -> anyhow::Result<bool> {
        let repo = normalize_repo(repo.clone())?;
        let key = repo.key();
        if previous_key.is_some_and(|previous| previous != key) {
            return Ok(false);
        }
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        let Some(existing) = state.repos.iter_mut().find(|item| item.key() == key) else {
            return Ok(false);
        };
        if existing.enabled == repo.enabled {
            return Ok(false);
        }
        existing.enabled = repo.enabled;
        self.save_state_unlocked(&state)?;
        Ok(true)
    }

    pub fn save_repo(
        &self,
        repo: SkillRepo,
        previous_key: Option<&str>,
    ) -> anyhow::Result<Vec<SkillRepo>> {
        let repo = normalize_repo(repo)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        let key = repo.key();
        state.suppressed_imported_repos.remove(&key);
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

    /// 后台检查完成时只替换仍未改变的仓库，避免覆盖用户刚改过的设置。
    pub fn replace_repo_if_unchanged(
        &self,
        expected: &SkillRepo,
        replacement: SkillRepo,
    ) -> anyhow::Result<bool> {
        let replacement = normalize_repo(replacement)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        let Some(index) = state.repos.iter().position(|repo| repo == expected) else {
            return Ok(false);
        };
        if replacement.key() != expected.key() {
            anyhow::ensure!(
                !state
                    .installed
                    .values()
                    .any(|skill| skill.repo_key == expected.key()),
                "原分支有已安装 Skill，不能自动更改来源，请先检查仓库配置"
            );
            anyhow::ensure!(
                !state
                    .repos
                    .iter()
                    .any(|repo| repo.key() == replacement.key()),
                "目标仓库已存在"
            );
        }
        state.repos[index] = replacement;
        self.save_state_unlocked(&state)?;
        Ok(true)
    }

    pub fn delete_repo(&self, key: &str) -> anyhow::Result<Vec<SkillRepo>> {
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        state.repos.retain(|repo| repo.key() != key);
        state.suppressed_imported_repos.insert(key.to_string());
        self.save_state_unlocked(&state)?;
        Ok(state.repos)
    }

    /// 从导入记录中列出尚未验证的仓库；手动删除的仓库不再自动检查。
    pub fn pending_imported_repos(&self) -> Vec<SkillRepo> {
        let state = self.load_state();
        let mut repos = BTreeMap::new();
        for skill in state.installed.values() {
            if skill.import_source != "cc-switch"
                || state.suppressed_imported_repos.contains(&skill.repo_key)
                || state.repos.iter().any(|repo| repo.key() == skill.repo_key)
            {
                continue;
            }
            let Some(repo) =
                parse_repo_key(&skill.repo_key).and_then(|repo| normalize_repo(repo).ok())
            else {
                continue;
            };
            if repo.key() == skill.repo_key {
                repos.insert(repo.key(), repo);
            }
        }
        repos.into_values().collect()
    }

    fn remote_install_id(&self, state: &SkillsState, skill: &RemoteSkill) -> String {
        let id_in_use = |id: &str| {
            state
                .installed
                .keys()
                .any(|key| key.eq_ignore_ascii_case(id))
                || std::fs::symlink_metadata(self.source_dir.join(id)).is_ok()
                || self
                    .legacy_source_dir
                    .as_ref()
                    .filter(|_| !state.independent_library_migrated)
                    .is_some_and(|root| std::fs::symlink_metadata(root.join(id)).is_ok())
                || std::fs::symlink_metadata(self.linked_dir().join(id)).is_ok()
                || self
                    .agent_dirs
                    .iter()
                    .any(|(_, root)| std::fs::symlink_metadata(root.join(id)).is_ok())
        };
        if !id_in_use(&skill.id) {
            return skill.id.clone();
        }
        for attempt in 0u64.. {
            let mut hash = Sha256::new();
            hash.update(b"codex-plus-skill-install\0");
            hash.update(skill.repo_key.as_bytes());
            hash.update([0]);
            hash.update(skill.repo_path.as_bytes());
            hash.update([0]);
            hash.update(attempt.to_le_bytes());
            let id = format!("cpx-skill-{:x}", hash.finalize());
            if !id_in_use(&id) {
                return id;
            }
        }
        unreachable!()
    }

    /// 把远端清单和本地状态合并成一份列表。`remote` 传空就是纯本地视图。
    pub fn merge_entries(&self, remote: &[RemoteSkill]) -> Vec<SkillEntry> {
        let state = self.load_state();
        let linked = self.linked_dir();
        let mut entries: BTreeMap<(String, String), SkillEntry> = BTreeMap::new();
        let mut installed_origins: BTreeMap<(&str, &str), &InstalledSkill> = BTreeMap::new();
        for installed in state.installed.values() {
            installed_origins
                .entry((&installed.repo_key, &installed.repo_path))
                .or_insert(installed);
        }

        for skill in remote {
            let installed = installed_origins
                .get(&(skill.repo_key.as_str(), skill.repo_path.as_str()))
                .copied();
            let id = installed
                .map(|item| item.id.clone())
                .unwrap_or_else(|| self.remote_install_id(&state, skill));
            entries.insert(
                (skill.repo_key.clone(), id.clone()),
                SkillEntry {
                    id: id.clone(),
                    name: installed
                        .map(|item| {
                            if item.name.is_empty() {
                                id.clone()
                            } else {
                                item.name.clone()
                            }
                        })
                        .unwrap_or_else(|| skill.name.clone()),
                    description: installed
                        .map(|item| item.description.clone())
                        .unwrap_or_else(|| skill.description.clone()),
                    repo_key: skill.repo_key.clone(),
                    repo_path: skill.repo_path.clone(),
                    import_source: installed
                        .map(|item| item.import_source.clone())
                        .unwrap_or_default(),
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
                    codex_conflict: false,
                },
            );
        }

        // 已安装但远端清单里没有的（仓库删了、或本地手工放的），仍要能看见和卸载。
        for (id, installed) in &state.installed {
            entries
                .entry((installed.repo_key.clone(), id.clone()))
                .or_insert_with(|| SkillEntry {
                    id: id.clone(),
                    name: if installed.name.is_empty() {
                        id.clone()
                    } else {
                        installed.name.clone()
                    },
                    description: installed.description.clone(),
                    repo_key: installed.repo_key.clone(),
                    repo_path: installed.repo_path.clone(),
                    import_source: installed.import_source.clone(),
                    installed: true,
                    enabled: is_linked(&linked.join(id)),
                    bundled: false,
                    content_hash: installed.content_hash.clone(),
                    remote_hash: String::new(),
                    update_available: false,
                    agents: Vec::new(),
                    managed: false,
                    codex_conflict: false,
                });
        }

        for skill in self.list_bundled_skills() {
            entries
                .entry((skill.repo_key.clone(), skill.id.clone()))
                .or_insert(skill);
        }

        let mut installed_ids: std::collections::BTreeSet<String> = entries
            .values()
            .filter(|entry| entry.installed)
            .map(|entry| entry.id.clone())
            .collect();
        let mut all_ids: std::collections::BTreeSet<String> =
            entries.values().map(|entry| entry.id.clone()).collect();

        // 用户让 Codex 自己建的 skill（或手工放进 skills 目录的）不会进 state.installed，
        // 也不在远端清单里，此前因此完全不显示（issue #862）。这里扫描 linked_dir 补上。
        for skill in self.list_local_skills() {
            if installed_ids.insert(skill.id.clone()) {
                all_ids.insert(skill.id.clone());
                entries
                    .entry((skill.repo_key.clone(), skill.id.clone()))
                    .or_insert(skill);
            }
        }

        // 只放在 Claude / Grok 自己 skills 目录里的 Skill（不是本工具建的软链），也要能看见、能启用到别处。
        for (agent, dir) in &self.agent_dirs {
            for skill in scan_skill_dirs(dir) {
                if all_ids.insert(skill.0.clone()) {
                    let (name, description) = skill.1;
                    entries.insert(
                        (String::new(), skill.0.clone()),
                        SkillEntry {
                            id: skill.0.clone(),
                            name,
                            description,
                            repo_key: String::new(),
                            repo_path: String::new(),
                            import_source: String::new(),
                            installed: true,
                            enabled: false,
                            bundled: false,
                            content_hash: String::new(),
                            remote_hash: String::new(),
                            update_available: false,
                            agents: vec![agent.clone()],
                            managed: false,
                            codex_conflict: false,
                        },
                    );
                }
            }
        }

        let mut entries: Vec<SkillEntry> = entries.into_values().collect();
        for entry in &mut entries {
            if !entry.installed {
                continue;
            }
            entry.managed = self.source_dir.join(&entry.id).is_dir();
            if entry.managed && !entry.bundled {
                entry.codex_conflict = self.codex_copy_conflict(&entry.id).unwrap_or(true);
            }
            let mut agents = Vec::new();
            if entry.enabled || entry.bundled {
                agents.push("codex".to_string());
            }
            if !entry.bundled {
                for (agent, dir) in &self.agent_dirs {
                    if dir.join(&entry.id).join(SKILL_MANIFEST_FILE).is_file()
                        && !agents.contains(agent)
                    {
                        agents.push(agent.clone());
                    }
                }
            }
            entry.agents = agents;
        }
        entries
    }

    /// 在最新仓库快照中核验待安装项的本地 ID 和来源路径。
    pub fn resolve_remote_install(
        &self,
        remote: &[RemoteSkill],
        repo_key: &str,
        id: &str,
        expected_repo_path: Option<&str>,
    ) -> anyhow::Result<RemoteSkill> {
        validate_skill_id(id)?;
        let entry = self
            .merge_entries(remote)
            .into_iter()
            .find(|item| {
                !item.installed
                    && item.repo_key == repo_key
                    && item.id == id
                    && expected_repo_path.is_none_or(|path| item.repo_path == path)
            })
            .context("Skill 的安装身份或仓库路径已变化，请刷新后重试")?;
        let mut skill = remote
            .iter()
            .find(|item| item.repo_key == repo_key && item.repo_path == entry.repo_path)
            .cloned()
            .context("仓库路径已不存在，请刷新后重试")?;
        anyhow::ensure!(
            expected_repo_path.is_some() || skill.id == id,
            "冲突的 Skill 需要指定仓库路径"
        );
        skill.id = id.to_string();
        Ok(skill)
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
                import_source: String::new(),
                installed: true,
                enabled: is_linked(&path),
                bundled: false,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                agents: Vec::new(),
                managed: false,
                codex_conflict: false,
            });
        }
        skills
    }

    /// 从外部 Skill 目录导入到 Codex++ 的受控源目录，并默认停用。来源记为 cc-switch。
    pub fn import_from_directory(
        &self,
        id: &str,
        external_dir: &Path,
    ) -> anyhow::Result<SkillsState> {
        self.import_directory_with_source(id, id, external_dir, "cc-switch", None, None, None)
    }

    /// 用户在管理器里选择的本地目录导入（#511）。与 cc-switch 导入同一套流程，来源记为本地导入。
    pub fn import_from_local_directory(
        &self,
        id: &str,
        external_dir: &Path,
    ) -> anyhow::Result<SkillsState> {
        self.import_directory_with_source(
            id,
            id,
            external_dir,
            LOCAL_IMPORT_REPO_KEY,
            None,
            None,
            None,
        )
    }

    fn import_directory_with_source(
        &self,
        _incoming_id: &str,
        id: &str,
        external_dir: &Path,
        import_source: &str,
        origin: Option<&crate::ccs_import::CcsSkillOrigin>,
        expected_source: Option<&str>,
        expected_target: Option<&str>,
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        if import_source == LOCAL_IMPORT_REPO_KEY {
            // Codex 的 skills 目录本来就会被扫描显示；从这里「导入」会把原目录挪进备份，反而让它从 Codex 消失。
            if let Ok(external) = external_dir.canonicalize() {
                for (root, label) in [
                    (self.linked_dir(), "Codex 的 skills 目录"),
                    (self.source_dir.clone(), "Codex++ 的 Skills 受控目录"),
                ] {
                    if root
                        .canonicalize()
                        .is_ok_and(|root| external.starts_with(root))
                    {
                        anyhow::bail!(
                            "{} 已在{label}里，Codex++ 会直接识别，无需导入",
                            external_dir.display()
                        );
                    }
                }
            }
        }
        let manifest = external_dir.join(SKILL_MANIFEST_FILE);
        if !manifest.is_file() {
            anyhow::bail!(
                "{} 下没有 SKILL.md，不是有效的 Skill",
                external_dir.display()
            );
        }
        std::fs::create_dir_all(&self.source_dir)?;
        if self
            .source_dir
            .canonicalize()?
            .starts_with(external_dir.canonicalize()?)
        {
            anyhow::bail!("导入目录不能包含受控源目录");
        }
        let staging = self
            .source_dir
            .join(format!(".import-{id}-{}", uuid::Uuid::new_v4()));
        let source_hash = skill_directory_hash(external_dir)?;
        if let Some(expected) = expected_source {
            anyhow::ensure!(source_hash == expected, "导入源已变化，请重新预览");
        }
        if let Err(error) = copy_dir_all(external_dir, &staging) {
            let _ = remove_link(&staging);
            return Err(error);
        }
        if skill_directory_hash(&staging)? != source_hash
            || skill_directory_hash(external_dir)? != source_hash
        {
            let _ = remove_link(&staging);
            anyhow::bail!("导入源在复制时发生变化，请重新预览");
        }
        let (name, description) = read_skill_manifest(&manifest, id);
        let content_hash = if origin.is_some_and(|item| !item.repo_path.is_empty()) {
            skill_git_tree_hash(&staging)?
        } else {
            String::new()
        };
        self.commit_staged_skill(
            &staging,
            InstalledSkill {
                id: id.to_string(),
                name,
                description,
                repo_key: origin
                    .map(|item| item.repo_key.clone())
                    .unwrap_or_else(|| import_source.to_string()),
                repo_path: origin
                    .map(|item| item.repo_path.clone())
                    .unwrap_or_default(),
                import_source: import_source.to_string(),
                local_baseline_hash: String::new(),
                content_hash,
                installed_at: current_unix_timestamp_string(),
            },
            false,
            expected_target.is_some(),
            expected_target.is_none(),
            false,
            expected_target,
            None,
        )
    }

    /// 目录、副本和安装记录一起提交，崩溃后可依据事务日志恢复。
    fn commit_staged_skill(
        &self,
        staging: &Path,
        mut installed: InstalledSkill,
        enabled: bool,
        preserve_enabled: bool,
        require_empty: bool,
        require_existing: bool,
        expected_target: Option<&str>,
        expected_update: Option<&str>,
    ) -> anyhow::Result<SkillsState> {
        let unverified_cc_switch_import =
            installed.import_source == "cc-switch" && !installed.repo_path.is_empty();
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let id = installed.id.clone();
        let destination = self.source_dir.join(&id);
        anyhow::ensure!(
            !self.codex_copy_conflict(&id)?,
            "Codex Skill 与管理库内容不一致，请先选择保留的版本"
        );
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        let snapshot = self.import_target_snapshot_details_unlocked(&state, &id)?;
        if let Some(expected) = expected_update {
            anyhow::ensure!(
                snapshot.token == expected,
                "Skill 在确认更新后发生变化，请重新检查"
            );
        }
        if let Some(expected) = expected_target {
            anyhow::ensure!(
                snapshot.token == expected,
                "已有 Skill 在预览后发生变化，请重新预览"
            );
        }
        if let (Some(library_hash), Some(codex_hash)) =
            (&snapshot.directory_hashes[0], &snapshot.directory_hashes[1])
        {
            anyhow::ensure!(
                library_hash == codex_hash,
                "Codex Skill 与管理库内容不一致，请先选择保留的版本"
            );
        }
        if require_empty {
            anyhow::ensure!(
                !state.installed.contains_key(&id)
                    && std::fs::symlink_metadata(&destination).is_err()
                    && std::fs::symlink_metadata(self.linked_dir().join(&id)).is_err(),
                "同名 Skill 已存在，未恢复覆盖"
            );
            if preserve_enabled {
                anyhow::ensure!(
                    state.installed.values().all(|item| {
                        !item.id.eq_ignore_ascii_case(&id)
                            && (item.repo_key != installed.repo_key
                                || item.repo_path != installed.repo_path)
                    }) && self
                        .agent_dirs
                        .iter()
                        .all(|(_, root)| { std::fs::symlink_metadata(root.join(&id)).is_err() }),
                    "Skill 的安装身份或仓库路径已被占用，请刷新后重试"
                );
            }
            if !installed.import_source.is_empty() {
                let hash = skill_directory_hash(staging)?;
                anyhow::ensure!(
                    self.import_matches_unlocked(&state, &id, &installed.name, &hash)?
                        .is_empty(),
                    "发现同名或内容重复的 Skill，请先预览并选择是否覆盖"
                );
            }
        }
        if preserve_enabled && expected_target.is_none() {
            if let Some(previous) = state.installed.get(&id) {
                anyhow::ensure!(
                    previous.repo_key == installed.repo_key
                        && previous.repo_path == installed.repo_path,
                    "同名 Skill 的来源路径已变化，不能覆盖"
                );
                if installed.import_source.is_empty() {
                    installed.import_source = previous.import_source.clone();
                }
                anyhow::ensure!(
                    destination.join(SKILL_MANIFEST_FILE).is_file(),
                    "Skill 管理库缺少完整内容，未覆盖"
                );
                anyhow::ensure!(
                    expected_update.is_some()
                        || legacy_update_state(previous, &destination)? == LegacyUpdateState::Clean,
                    "Skill 管理库包含本地修改或无法确认原始版本，请先明确选择是否更新"
                );
                anyhow::ensure!(
                    snapshot.directory_hashes[0].as_ref()
                        == Some(&skill_directory_hash(&destination)?),
                    "Skill 管理库在检查期间发生变化，请重新确认"
                );
            } else {
                anyhow::ensure!(!require_existing, "Skill 已卸载，已取消更新");
                anyhow::ensure!(
                    std::fs::symlink_metadata(&destination).is_err()
                        && std::fs::symlink_metadata(self.linked_dir().join(&id)).is_err(),
                    "已存在同名 Skill，未自动覆盖"
                );
            }
        }
        let enabled = if preserve_enabled
            && (state.installed.contains_key(&id) || expected_target.is_some())
        {
            is_linked(&self.linked_dir().join(&id))
        } else {
            enabled
        };
        let previous_hash = snapshot.directory_hashes[0].clone();
        let operation = uuid::Uuid::new_v4().to_string();
        installed.local_baseline_hash = if unverified_cc_switch_import {
            UNVERIFIED_CC_SWITCH_BASELINE.to_string()
        } else {
            skill_directory_hash(staging)?
        };
        let library_stage = self.transaction_stage(&operation, 0, &SkillPlace::Library)?;
        std::fs::rename(staging, &library_stage)?;
        let mut changes = vec![PreparedSkillChange {
            place: SkillPlace::Library,
            staged: true,
            preserve_backup: true,
            expected_backup_hash: previous_hash.clone(),
        }];
        let codex_hash = snapshot.directory_hashes[1].clone();
        if enabled {
            self.stage_skill_copy(
                &operation,
                changes.len(),
                &SkillPlace::Codex,
                &library_stage,
                true,
            )?;
            changes.push(PreparedSkillChange {
                place: SkillPlace::Codex,
                staged: true,
                preserve_backup: true,
                expected_backup_hash: codex_hash,
            });
        } else if codex_hash.is_some() {
            changes.push(PreparedSkillChange {
                place: SkillPlace::Codex,
                staged: false,
                preserve_backup: true,
                expected_backup_hash: codex_hash,
            });
        }
        // 远端更新只同步未修改的受控副本；导入覆盖经用户确认后同步所有已启用副本并备份。
        if preserve_enabled {
            for (index, (agent, root)) in self.agent_dirs.iter().enumerate() {
                let target = root.join(&id);
                if !target.join(SKILL_MANIFEST_FILE).is_file() {
                    continue;
                }
                let Some(hash) = snapshot.directory_hashes[index + 2].clone() else {
                    continue;
                };
                if expected_target.is_none()
                    && (!is_managed_copy(&target) || previous_hash.as_ref() != Some(&hash))
                {
                    continue;
                }
                let place = SkillPlace::Agent(agent.clone());
                self.stage_skill_copy(&operation, changes.len(), &place, &library_stage, true)?;
                changes.push(PreparedSkillChange {
                    place,
                    staged: true,
                    preserve_backup: true,
                    expected_backup_hash: Some(hash),
                });
            }
        }
        state.installed.insert(id.clone(), installed);
        self.commit_skill_transaction(&operation, &id, changes, Some(&state))?;
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
                import_source: String::new(),
                installed: true,
                enabled: true,
                bundled: true,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                agents: Vec::new(),
                managed: false,
                codex_conflict: false,
            });
        }
        skills
    }

    /// 使用已下载的仓库快照安装 Skill，不自动覆盖本地修改。
    pub fn install_from_zip(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
    ) -> anyhow::Result<SkillsState> {
        self.install_from_zip_with_confirmation(skill, zip_bytes, None)
    }

    /// 显式确认覆盖本地修改时，expected_update 必须与预览快照一致。
    pub fn install_from_zip_with_confirmation(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
        expected_update: Option<&str>,
    ) -> anyhow::Result<SkillsState> {
        self.install_from_zip_inner(skill, zip_bytes, expected_update, false, false)
    }

    pub fn update_existing_from_zip_with_confirmation(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
        expected_update: Option<&str>,
    ) -> anyhow::Result<SkillsState> {
        self.install_from_zip_inner(skill, zip_bytes, expected_update, false, true)
    }

    /// 新安装不能转换成覆盖已安装 Skill，即使来源路径相同。
    pub fn install_new_from_zip(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
    ) -> anyhow::Result<SkillsState> {
        self.install_from_zip_inner(skill, zip_bytes, None, true, false)
    }

    fn install_from_zip_inner(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
        expected_update: Option<&str>,
        require_empty: bool,
        require_existing: bool,
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(&skill.id)?;
        let state = self.load_state();
        let installed = state.installed.get(&skill.id);
        anyhow::ensure!(
            !require_existing || installed.is_some(),
            "Skill 已卸载，已取消更新"
        );
        if let Some(installed) = installed {
            if require_empty
                || installed.repo_key != skill.repo_key
                || installed.repo_path != skill.repo_path
            {
                anyhow::bail!(
                    "同名 Skill 的来源路径不同或旧记录缺少路径，不能覆盖；请先备份卸载原项"
                );
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

        self.commit_staged_skill(
            &staging,
            InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                repo_path: skill.repo_path.clone(),
                import_source: String::new(),
                local_baseline_hash: String::new(),
                content_hash: skill.content_hash.clone(),
                installed_at: current_unix_timestamp_string(),
            },
            enable_after_install,
            true,
            require_empty,
            require_existing,
            None,
            expected_update,
        )
    }

    /// 启用或停用 Codex 副本；内容不一致时必须先让用户选择版本。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        self.set_enabled_unlocked(id, enabled)
    }

    fn set_enabled_unlocked(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        let source = self.source_dir.join(id);
        let link = self.linked_dir().join(id);
        if !source.is_dir() && link.join(SKILL_MANIFEST_FILE).is_file() {
            self.ensure_managed_source(id)?;
        }
        anyhow::ensure!(
            !self.codex_copy_conflict(id)?,
            "Codex Skill 与管理库内容不一致，请先选择保留的版本"
        );
        if !enabled {
            match std::fs::symlink_metadata(&link) {
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            }
            anyhow::ensure!(
                link.join(SKILL_MANIFEST_FILE).is_file(),
                "Codex 目录里已有其他内容，未停用：{}",
                link.display()
            );
            let hash = skill_directory_hash(&link)?;
            anyhow::ensure!(
                skill_directory_hash(&source)? == hash,
                "Codex Skill 与管理库内容不一致，请先选择保留的版本"
            );
            let operation = uuid::Uuid::new_v4().to_string();
            return self.commit_skill_transaction(
                &operation,
                id,
                vec![PreparedSkillChange {
                    place: SkillPlace::Codex,
                    staged: false,
                    preserve_backup: true,
                    expected_backup_hash: Some(hash),
                }],
                None,
            );
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
        let operation = uuid::Uuid::new_v4().to_string();
        self.stage_skill_copy(&operation, 0, &SkillPlace::Codex, &source, true)?;
        self.commit_skill_transaction(
            &operation,
            id,
            vec![PreparedSkillChange {
                place: SkillPlace::Codex,
                staged: true,
                preserve_backup: true,
                expected_backup_hash: None,
            }],
            None,
        )
    }

    /// 卸载时保留管理库和所有受控副本的备份。
    pub fn uninstall(&self, id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        let _operation = self.operation_lock.lock().unwrap();
        self.prepare_managed_skills_unlocked()?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_checked_unlocked()?;
        state.installed.remove(id);
        let mut changes = Vec::new();
        for (place, path) in [
            (SkillPlace::Codex, self.linked_dir().join(id)),
            (SkillPlace::Library, self.source_dir.join(id)),
        ] {
            if std::fs::symlink_metadata(&path).is_ok() {
                changes.push(PreparedSkillChange {
                    place,
                    staged: false,
                    preserve_backup: true,
                    expected_backup_hash: Some(skill_directory_hash(&path)?),
                });
            }
        }
        for (agent, dir) in &self.agent_dirs {
            let path = dir.join(id);
            if is_managed_copy(&path) {
                changes.push(PreparedSkillChange {
                    place: SkillPlace::Agent(agent.clone()),
                    staged: false,
                    preserve_backup: true,
                    expected_backup_hash: Some(skill_directory_hash(&path)?),
                });
            }
        }
        let operation = uuid::Uuid::new_v4().to_string();
        self.commit_skill_transaction(&operation, id, changes, Some(&state))?;
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
                import_source: String::new(),
                local_baseline_hash: String::new(),
                content_hash: String::new(),
                installed_at: current_unix_timestamp_string(),
            },
            true,
            false,
            true,
            false,
            None,
            None,
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

/// Skill ID 必须是普通目录名，不能指向 Codex 内置目录或事务保留目录。
fn validate_skill_id(id: &str) -> anyhow::Result<()> {
    let mut components = Path::new(id).components();
    let reserved = id.to_ascii_lowercase();
    if id.is_empty()
        || reserved == ".system"
        || [".codex-plus-", ".staging-", ".import-", ".backup-"]
            .iter()
            .any(|prefix| reserved.starts_with(prefix))
        || id.contains(['/', '\\'])
        || id.chars().any(char::is_control)
        || !matches!(components.next(), Some(Component::Normal(name)) if name == std::ffi::OsStr::new(id))
        || components.next().is_some()
    {
        anyhow::bail!("非法的 skill id：{id}");
    }
    #[cfg(windows)]
    {
        if !is_safe_windows_path_component(id) {
            anyhow::bail!("非法的 Windows skill id：{id}");
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_safe_windows_path_component(value: &str) -> bool {
    let device = value
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered_device = device.len() == 4
        && (device.starts_with("COM") || device.starts_with("LPT"))
        && matches!(device.as_bytes()[3], b'1'..=b'9');
    !value.ends_with(['.', ' '])
        && !value.contains(['<', '>', ':', '"', '|', '?', '*'])
        && !value.chars().any(char::is_control)
        && !matches!(
            device.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        && !numbered_device
}

/// 把用户选中的本地目录解析成待导入的 Skill 目录（#511）。
///
/// - 目录本身含 `SKILL.md`：它就是一个 Skill，id 取目录名；
/// - 否则当作 Skills 集合目录，收直接子目录里含 `SKILL.md` 的（跳过隐藏目录）；
/// - 两者都没有时报错，明确提示缺少 `SKILL.md`。
pub fn local_skill_import_candidates(dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    if !dir.is_dir() {
        anyhow::bail!("{} 不是可读取的目录", dir.display());
    }
    if dir.join(SKILL_MANIFEST_FILE).is_file() {
        let id = dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("无法从 {} 推断 Skill 名称", dir.display()))?;
        return Ok(vec![(id, dir.to_path_buf())]);
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if id.starts_with('.') || !path.is_dir() || !path.join(SKILL_MANIFEST_FILE).is_file() {
            continue;
        }
        candidates.push((id, path));
    }
    if candidates.is_empty() {
        anyhow::bail!(
            "{} 下没有 SKILL.md，子目录里也没有，不是有效的 Skill 目录",
            dir.display()
        );
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(candidates)
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
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(rest) = path.strip_prefix(&prefix) else {
            continue;
        };
        if rest == SKILL_MANIFEST_FILE || rest.ends_with("/SKILL.md") {
            let directory = path
                .strip_suffix(SKILL_MANIFEST_FILE)
                .unwrap()
                .trim_end_matches('/');
            if directory.is_empty() || safe_relative_path(directory).is_ok() {
                directories.insert(directory.to_string());
            }
        }
    }
    let mut subtrees: BTreeMap<String, Vec<(String, String)>> = directories
        .iter()
        .map(|dir| (dir.clone(), Vec::new()))
        .collect();
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
            let directory_prefix = if directory.is_empty() {
                String::new()
            } else {
                format!("{directory}/")
            };
            if let Some(relative) = path.strip_prefix(&directory_prefix) {
                files.push((relative.to_string(), sha.clone()));
            }
        }
    }
    subtrees
        .into_iter()
        .filter_map(|(repo_path, mut files)| {
            let base = if repo_path.is_empty() {
                repo.name.clone()
            } else {
                repo_path.rsplit('/').next()?.to_string()
            };
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
                repo_path: if repo_path.is_empty() {
                    ".".into()
                } else {
                    repo_path
                },
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
    client: &GithubClient,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let response = client
        .get(repo_zip_url(repo), "application/zip", None)
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
    let bytes = read_repo_zip_response(response).await?;
    parse_skills_from_zip(repo, &bytes, cached)
}

pub fn parse_skills_from_zip(
    repo: &SkillRepo,
    bytes: &[u8],
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).context("skill 仓库压缩包无法解析")?;
    let root = zip_archive_root(&mut archive)?;
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
        let name = file.name().to_string();
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
        let declared_size = file.size();
        file.by_ref()
            .take(declared_size.saturating_add(1))
            .read_to_end(&mut contents)?;
        anyhow::ensure!(
            contents.len() as u64 == declared_size,
            "压缩包条目大小不一致：{}",
            file.name()
        );
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
            manifests.insert(
                relative.to_string(),
                String::from_utf8_lossy(&contents).into_owned(),
            );
        }
    }
    let tree = serde_json::json!({ "tree": tree_items });
    let mut skills = parse_skills_from_tree(repo, &tree);
    for skill in &mut skills {
        if let Some(hit) = cached.get(&skill.id) {
            if hit.content_hash == skill.content_hash
                && !hit.description.is_empty()
                && !matches!(
                    hit.description.as_str(),
                    ">" | ">-" | ">+" | "|" | "|-" | "|+"
                )
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
    if repo_path == "." {
        SKILL_MANIFEST_FILE.into()
    } else {
        format!("{repo_path}/{SKILL_MANIFEST_FILE}")
    }
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

/// 校验仓库压缩包只有一个顶层目录，避免目录清单和解压结果来自不同的根目录。
fn zip_archive_root(archive: &mut zip::ZipArchive<Cursor<&[u8]>>) -> anyhow::Result<String> {
    let mut root: Option<String> = None;
    let mut paths = std::collections::BTreeSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name();
        anyhow::ensure!(
            !name.contains(['\\', '\0']) && !name.starts_with('/'),
            "压缩包路径无效：{name}"
        );
        let mut parts = name.split('/');
        let current = parts
            .next()
            .filter(|part| !part.is_empty() && *part != "." && *part != "..")
            .context("压缩包根目录无效")?;
        let mut parts = parts.peekable();
        while let Some(part) = parts.next() {
            anyhow::ensure!(
                part != "." && part != ".." && (!part.is_empty() || parts.peek().is_none()),
                "压缩包路径无效：{name}"
            );
        }
        let path = name.trim_end_matches('/');
        let path = if cfg!(windows) {
            path.to_lowercase()
        } else {
            path.to_string()
        };
        anyhow::ensure!(paths.insert(path), "压缩包路径重复：{name}");
        match &root {
            Some(existing) => anyhow::ensure!(existing == current, "压缩包含有多个根目录"),
            None => root = Some(current.to_string()),
        }
    }
    root.context("仓库压缩包为空")
}

/// 从仓库 zip 里只解出 `repo_path` 这一棵子树。
///
/// GitHub 的 zip 统一带一层 `repo-ref/` 前缀，`zip_entry_relative_path` 会剥掉它
/// 并同时挡住路径逃逸。目标子树中的符号链接条目一律拒绝——与 codex 内置 skill-installer 的
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
    let prefix = if repo_path == "." {
        String::new()
    } else {
        format!("{repo_path}/")
    };
    let mut archive =
        zip::ZipArchive::new(Cursor::new(zip_bytes)).context("skill 仓库压缩包无法解析")?;
    zip_archive_root(&mut archive)?;
    let mut wrote_manifest = false;
    let mut total_size = 0u64;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .with_context(|| format!("读取压缩包条目 {index} 失败"))?;
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
        if file.is_symlink()
            && (repo_path == "." || relative == repo_path || relative.starts_with(&prefix))
        {
            anyhow::bail!("skill 中不允许包含符号链接：{}", file.name());
        }
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
        total_size = total_size.saturating_add(file.size());
        anyhow::ensure!(
            total_size <= 256 * 1024 * 1024,
            "Skill 解压内容超过 256 MiB"
        );
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        let mut contents = Vec::new();
        let declared_size = file.size();
        file.by_ref()
            .take(declared_size.saturating_add(1))
            .read_to_end(&mut contents)
            .with_context(|| format!("读取压缩包条目 {} 失败", file.name()))?;
        anyhow::ensure!(
            contents.len() as u64 == declared_size,
            "压缩包条目大小不一致：{}",
            file.name()
        );
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
            std::path::Component::Normal(part) => {
                #[cfg(windows)]
                anyhow::ensure!(
                    part.to_str().is_some_and(is_safe_windows_path_component),
                    "压缩包条目包含 Windows 保留名称：{value}"
                );
                relative.push(part)
            }
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

struct GithubClient {
    primary: reqwest::Client,
    direct: Option<reqwest::Client>,
    direct_hosts: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl GithubClient {
    async fn request(
        client: &reqwest::Client,
        url: &str,
        accept: &str,
        timeout_secs: Option<u64>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut request = with_github_auth(client.get(url)).header(reqwest::header::ACCEPT, accept);
        if let Some(seconds) = timeout_secs {
            request = request.timeout(std::time::Duration::from_secs(seconds));
        }
        Ok(
            tokio::time::timeout(std::time::Duration::from_secs(30), request.send())
                .await
                .context("等待 GitHub 响应头超过 30 秒")??,
        )
    }

    async fn get(
        &self,
        url: String,
        accept: &str,
        timeout_secs: Option<u64>,
    ) -> anyhow::Result<reqwest::Response> {
        let host = reqwest::Url::parse(&url)?
            .host_str()
            .unwrap_or_default()
            .to_owned();
        let direct = self.direct_hosts.lock().unwrap().contains(&host);
        if direct {
            return Self::request(self.direct.as_ref().unwrap(), &url, accept, timeout_secs).await;
        }
        match Self::request(&self.primary, &url, accept, timeout_secs).await {
            Ok(response) => Ok(response),
            // 仅自动系统代理的连接故障允许直连；显式代理和 HTTP 错误状态不改路由。
            Err(error)
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|error| error.is_connect() || error.is_timeout())
                        || cause.is::<tokio::time::error::Elapsed>()
                }) =>
            {
                let Some(client) = &self.direct else {
                    return Err(error.into());
                };
                let response = Self::request(client, &url, accept, timeout_secs)
                    .await
                    .with_context(|| format!("系统代理请求失败（{error:#}），直连重试也失败"))?;
                self.direct_hosts.lock().unwrap().insert(host);
                Ok(response)
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn github_https_proxy_from_env(mut get: impl FnMut(&str) -> Option<String>) -> Option<String> {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .into_iter()
        .find_map(|key| get(key).filter(|value| !value.trim().is_empty()))
}

async fn github_client() -> anyhow::Result<GithubClient> {
    tokio::task::spawn_blocking(|| {
        let agent = format!("Codex++/{}", crate::version::VERSION);
        if let Some(proxy) = github_https_proxy_from_env(|key| std::env::var(key).ok()) {
            return Ok(GithubClient {
                primary: crate::http_client::proxied_client_with_proxy(&agent, &proxy)?,
                direct: None,
                direct_hosts: Default::default(),
            });
        }
        let mut proxies = BTreeMap::new();
        for host in [
            "api.github.com",
            "codeload.github.com",
            "raw.githubusercontent.com",
        ] {
            if let Some(proxy) =
                crate::proxy::detect_system_proxy_for_url(&format!("https://{host}/"))
            {
                proxies.insert(host.to_string(), proxy);
            }
        }
        let direct = if proxies.is_empty() {
            None
        } else {
            Some(crate::http_client::direct_client_with_timeouts(&agent)?)
        };
        let primary = reqwest::Client::builder()
            .user_agent(agent)
            .connect_timeout(std::time::Duration::from_secs(10))
            .proxy(reqwest::Proxy::custom(move |url| {
                url.host_str().and_then(|host| proxies.get(host).cloned())
            }))
            .build()?;
        Ok(GithubClient {
            primary,
            direct,
            direct_hosts: Default::default(),
        })
    })
    .await?
}

pub async fn resolve_repo_skills(
    repo: SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<(SkillRepo, Vec<RemoteSkill>)> {
    resolve_repo_skills_with(repo, cached, |repo, cached| {
        Box::pin(fetch_repo_skills(repo, cached))
    })
    .await
}

async fn resolve_repo_skills_with<F>(
    repo: SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    fetch: F,
) -> anyhow::Result<(SkillRepo, Vec<RemoteSkill>)>
where
    F: for<'a> Fn(
            &'a SkillRepo,
            &'a BTreeMap<String, RemoteSkill>,
        ) -> futures_util::future::BoxFuture<'a, anyhow::Result<Vec<RemoteSkill>>>
        + Send
        + Sync,
{
    let repo = normalize_repo(repo)?;
    match fetch(&repo, cached).await {
        Ok(skills) => Ok((repo, skills)),
        Err(error)
            if format!("{error:#}").contains("HTTP 404")
                && matches!(repo.branch.as_str(), "main" | "master") =>
        {
            let mut alternative = repo.clone();
            alternative.branch = if repo.branch == "main" {
                "master"
            } else {
                "main"
            }
            .into();
            match fetch(&alternative, &BTreeMap::new()).await {
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
    let response = client
        .get(repo_tree_url(repo), "application/vnd.github+json", Some(20))
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return fetch_repo_skills_from_zip(repo, cached, &client)
                .await
                .with_context(|| format!("文件树请求失败（{error}），压缩包回退也失败"));
        }
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
                if hit.content_hash == skill.content_hash
                    && !hit.description.is_empty()
                    && !matches!(
                        hit.description.as_str(),
                        ">" | ">-" | ">+" | "|" | "|-" | "|+"
                    )
                {
                    skill.name = hit.name.clone();
                    skill.description = hit.description.clone();
                    return skill;
                }
            }
            let manifest_url = raw_file_url(repo, &skill_manifest_path(&skill.repo_path));
            if let Ok(response) = client.get(manifest_url, "text/plain", Some(15)).await
                && let Ok(response) = response.error_for_status()
                && let Ok(text) = response.text().await
            {
                let (name, description) = parse_skill_frontmatter(&text, &skill.id);
                skill.name = name;
                skill.description = description;
            }
            skill
        }
    }))
    .buffer_unordered(8)
    .collect()
    .await;
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
    let response = client
        .get(repo_zip_url(repo), "application/zip", None)
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
    read_repo_zip_response(response).await
}

async fn read_repo_zip_response(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > REPO_ZIP_DOWNLOAD_LIMIT_BYTES as u64)
    {
        anyhow::bail!("仓库压缩包超过 64 MiB，未下载");
    }
    let mut bytes = Vec::new();
    // 大仓库在慢速网络上可能持续下载数分钟；限制无数据等待，而非 90 秒截断有效下载。
    while let Some(chunk) =
        tokio::time::timeout(std::time::Duration::from_secs(30), response.chunk())
            .await
            .context("仓库压缩包连续 30 秒未收到数据")?
            .context("读取仓库压缩包内容失败")?
    {
        anyhow::ensure!(
            chunk.len() <= REPO_ZIP_DOWNLOAD_LIMIT_BYTES.saturating_sub(bytes.len()),
            "仓库压缩包超过 64 MiB，已停止下载"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// 复制回退时放进副本的标记：有它说明这份副本是 Codex++ 建的，停用时可以删。
const MANAGED_COPY_MARKER: &str = ".codex-plus-managed";

/// 复制回退建的副本（不是软链，但带标记）。
fn is_managed_copy(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
        && path.join(MANAGED_COPY_MARKER).is_file()
}

fn copy_dir_all(source: &Path, destination: &Path) -> anyhow::Result<()> {
    copy_skill_directory(source, destination, true)
}

fn copy_skill_directory(source: &Path, destination: &Path, root: bool) -> anyhow::Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("创建目录失败：{}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("读取目录失败：{}", source.display()))?
    {
        let entry = entry?;
        // 副本标记只属于那一份副本，不跟着复制（例如停用 Codex 的副本时复制回受控源）。
        if root && entry.file_name() == MANAGED_COPY_MARKER {
            continue;
        }
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        anyhow::ensure!(
            !file_type.is_symlink(),
            "Skill 内含符号链接，未复制：{}",
            from.display()
        );
        if file_type.is_dir() {
            copy_skill_directory(&from, &to, false)?;
        } else if file_type.is_file() {
            std::fs::copy(&from, &to).with_context(|| format!("复制 {} 失败", from.display()))?;
        } else {
            anyhow::bail!("Skill 内含不支持的文件类型：{}", from.display());
        }
    }
    Ok(())
}

/// 比较完整目录而不仅是 SKILL.md；标记文件是副本元数据，不参与比较。
fn link_points_to(link: &Path, target: &Path) -> bool {
    let Ok(destination) = std::fs::read_link(link) else {
        return false;
    };
    let destination = if destination.is_absolute() {
        destination
    } else {
        link.parent().unwrap_or(Path::new(".")).join(destination)
    };
    let normalize = |path: &Path| -> Option<PathBuf> {
        Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?))
    };
    normalize(&destination)
        .zip(normalize(target))
        .is_some_and(|(left, right)| left == right)
}

fn skill_directory_hash(root: &Path) -> anyhow::Result<String> {
    fn visit(
        root: &Path,
        dir: &Path,
        entries: &mut BTreeMap<String, (bool, PathBuf)>,
        total: &mut u64,
    ) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if dir == root && entry.file_name() == MANAGED_COPY_MARKER {
                continue;
            }
            let path = entry.path();
            let kind = entry.file_type()?;
            anyhow::ensure!(
                !kind.is_symlink(),
                "Skill 内含符号链接，无法安全比较：{}",
                path.display()
            );
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("Skill 路径包含无法识别的字符")?
                .replace('\\', "/");
            if kind.is_dir() {
                entries.insert(relative, (true, path.clone()));
                visit(root, &path, entries, total)?;
            } else if kind.is_file() {
                *total = total.saturating_add(entry.metadata()?.len());
                anyhow::ensure!(
                    *total <= 256 * 1024 * 1024,
                    "Skill 内容超过 256 MiB，未完成比较"
                );
                entries.insert(relative, (false, path));
            } else {
                anyhow::bail!("Skill 内含不支持的文件类型：{}", path.display());
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries, &mut 0)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 32768];
    let mut actual_size = 0u64;
    for (name, (directory, path)) in entries {
        hasher.update(if directory { b"d" } else { b"f" });
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        if !directory {
            let mut file = std::fs::File::open(path)?;
            let mut file_hash = Sha256::new();
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                actual_size = actual_size.saturating_add(count as u64);
                anyhow::ensure!(
                    actual_size <= 256 * 1024 * 1024,
                    "Skill 内容超过 256 MiB，未完成比较"
                );
                file_hash.update(&buffer[..count]);
            }
            hasher.update(file_hash.finalize());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// 与 GitHub tree/ZIP 清单使用相同的 Git blob 子树哈希，导入后仍可检查更新。
fn skill_git_tree_hash(root: &Path) -> anyhow::Result<String> {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<String, String>,
        total: &mut u64,
    ) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            if directory == root && entry.file_name() == MANAGED_COPY_MARKER {
                continue;
            }
            let kind = entry.file_type()?;
            anyhow::ensure!(!kind.is_symlink(), "Skill 内含符号链接，无法计算仓库哈希");
            let path = entry.path();
            if kind.is_dir() {
                visit(root, &path, files, total)?;
            } else if kind.is_file() {
                anyhow::ensure!(
                    total.saturating_add(entry.metadata()?.len()) <= 256 * 1024 * 1024,
                    "Skill 内容超过 256 MiB"
                );
                let contents = std::fs::read(&path)?;
                *total = total.saturating_add(contents.len() as u64);
                anyhow::ensure!(*total <= 256 * 1024 * 1024, "Skill 内容超过 256 MiB");
                let mut blob = sha1::Sha1::new();
                blob.update(format!("blob {}\0", contents.len()).as_bytes());
                blob.update(contents);
                let relative = path
                    .strip_prefix(root)?
                    .to_str()
                    .context("Skill 路径包含无法识别的字符")?
                    .replace('\\', "/");
                files.insert(relative, format!("{:x}", blob.finalize()));
            } else {
                anyhow::bail!("Skill 内含不支持的文件类型");
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files, &mut 0)?;
    let mut hash = Sha256::new();
    for (path, blob) in files {
        hash.update(path.as_bytes());
        hash.update([0]);
        hash.update(blob.as_bytes());
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
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

/// 扫描 Agent 目录中可读取的 Skill，包括第三方建立的目录链接；扫描本身不修改目标。
fn scan_skill_dirs(root: &Path) -> Vec<(String, (String, String))> {
    let Ok(dir) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in dir.flatten() {
        let path = entry.path();
        let Some(id) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        if id.starts_with('.') {
            continue;
        }
        let manifest = path.join(SKILL_MANIFEST_FILE);
        if path.is_dir() && manifest.is_file() {
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

    fn github_https_proxy_for_test(values: &[(&str, &str)]) -> Option<String> {
        github_https_proxy_from_env(|key| {
            values
                .iter()
                .find_map(|(name, value)| (*name == key).then(|| value.to_string()))
        })
    }

    #[test]
    fn github_https_proxy_ignores_http_proxy() {
        for values in [
            &[][..],
            &[("HTTP_PROXY", "http://upper-http")][..],
            &[("http_proxy", "http://lower-http")][..],
            &[
                ("HTTP_PROXY", "invalid proxy"),
                ("http_proxy", "http://lower-http"),
            ][..],
        ] {
            assert_eq!(github_https_proxy_for_test(values), None);
        }
    }

    #[test]
    fn github_https_proxy_prefers_https_then_all_with_uppercase_first() {
        let cases: &[(&[(&str, &str)], &str)] = &[
            (
                &[("HTTPS_PROXY", "http://upper-https")],
                "http://upper-https",
            ),
            (
                &[("https_proxy", "http://lower-https")],
                "http://lower-https",
            ),
            (&[("ALL_PROXY", "http://upper-all")], "http://upper-all"),
            (&[("all_proxy", "http://lower-all")], "http://lower-all"),
            (
                &[
                    ("HTTPS_PROXY", "http://upper-https"),
                    ("https_proxy", "http://lower-https"),
                    ("ALL_PROXY", "http://upper-all"),
                    ("all_proxy", "http://lower-all"),
                    ("HTTP_PROXY", "http://upper-http"),
                ],
                "http://upper-https",
            ),
            (
                &[
                    ("https_proxy", "http://lower-https"),
                    ("ALL_PROXY", "http://upper-all"),
                    ("HTTP_PROXY", "http://upper-http"),
                ],
                "http://lower-https",
            ),
            (
                &[
                    ("ALL_PROXY", "http://upper-all"),
                    ("all_proxy", "http://lower-all"),
                    ("http_proxy", "http://lower-http"),
                ],
                "http://upper-all",
            ),
        ];
        for (values, expected) in cases {
            assert_eq!(
                github_https_proxy_for_test(values).as_deref(),
                Some(*expected)
            );
        }
    }

    #[test]
    fn github_https_proxy_skips_empty_values() {
        let cases: &[(&[(&str, &str)], Option<&str>)] = &[
            (
                &[("HTTPS_PROXY", ""), ("https_proxy", "http://lower-https")],
                Some("http://lower-https"),
            ),
            (
                &[
                    ("HTTPS_PROXY", "  \t"),
                    ("https_proxy", ""),
                    ("ALL_PROXY", "\n"),
                    ("all_proxy", "http://lower-all"),
                ],
                Some("http://lower-all"),
            ),
            (
                &[
                    ("HTTPS_PROXY", ""),
                    ("https_proxy", " "),
                    ("ALL_PROXY", ""),
                    ("all_proxy", "\t"),
                    ("HTTP_PROXY", "http://upper-http"),
                ],
                None,
            ),
        ];
        for (values, expected) in cases {
            assert_eq!(github_https_proxy_for_test(values).as_deref(), *expected);
        }
    }

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
    fn managed_skill_enables_into_other_agents_as_independent_copy() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(
            &temp.path().join("external/alpha"),
            "---\nname: Alpha\n---\n",
        );
        manager
            .import_from_directory("alpha", &temp.path().join("external/alpha"))
            .unwrap();

        manager.set_enabled_for("claude", "alpha", true).unwrap();
        let link = temp.path().join("claude-skills/alpha");
        assert!(
            !std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(link.join("SKILL.md"), "agent edit").unwrap();
        assert!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md"))
                .unwrap()
                .contains("Alpha")
        );
        let entries = manager.local_inventory();
        assert_eq!(entry(&entries, "alpha").agents, vec!["claude".to_string()]);
        assert!(entry(&entries, "alpha").managed);

        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(
            manager.source_dir.join("alpha/SKILL.md").is_file(),
            "停用不动源目录"
        );
    }

    /// Windows 没有建软链的权限时退化成复制：带标记的副本停用时照样删掉，复制回受控源时不带标记。
    #[test]
    fn copied_fallback_can_be_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(
            &temp.path().join("external/alpha"),
            "---\nname: Alpha\n---\n",
        );
        manager
            .import_from_directory("alpha", &temp.path().join("external/alpha"))
            .unwrap();
        let copy = temp.path().join("claude-skills/alpha");
        copy_dir_all(&manager.source_dir.join("alpha"), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();

        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(std::fs::symlink_metadata(&copy).is_err(), "副本被删掉");
        assert!(
            manager.source_dir.join("alpha/SKILL.md").is_file(),
            "停用不动源目录"
        );

        let again = temp.path().join("copy-again");
        std::fs::write(
            manager.source_dir.join("alpha").join(MANAGED_COPY_MARKER),
            "",
        )
        .unwrap();
        copy_dir_all(&manager.source_dir.join("alpha"), &again).unwrap();
        assert!(!again.join(MANAGED_COPY_MARKER).exists(), "标记不跟着复制");
    }

    #[test]
    fn nested_files_named_like_the_managed_marker_are_preserved_and_compared() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let copy = temp.path().join("copy");
        write_skill(&source, "skill");
        std::fs::write(source.join(MANAGED_COPY_MARKER), "root marker").unwrap();
        std::fs::create_dir_all(source.join("nested")).unwrap();
        std::fs::write(
            source.join("nested").join(MANAGED_COPY_MARKER),
            "real content",
        )
        .unwrap();
        copy_dir_all(&source, &copy).unwrap();
        assert!(!copy.join(MANAGED_COPY_MARKER).exists());
        assert_eq!(
            std::fs::read_to_string(copy.join("nested").join(MANAGED_COPY_MARKER)).unwrap(),
            "real content"
        );
        assert_eq!(
            skill_directory_hash(&source).unwrap(),
            skill_directory_hash(&copy).unwrap()
        );
        std::fs::write(copy.join("nested").join(MANAGED_COPY_MARKER), "changed").unwrap();
        assert_ne!(
            skill_directory_hash(&source).unwrap(),
            skill_directory_hash(&copy).unwrap()
        );
        assert_ne!(
            skill_git_tree_hash(&source).unwrap(),
            skill_git_tree_hash(&copy).unwrap()
        );
    }

    #[test]
    fn codex_local_skill_is_adopted_and_stays_enabled_in_codex() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&manager.linked_dir().join("beta"), "codex local");

        manager.set_enabled_for("claude", "beta", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("beta/SKILL.md")).unwrap(),
            "codex local",
            "Codex 照常可用"
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("claude-skills/beta/SKILL.md")).unwrap(),
            "codex local"
        );
        assert!(manager.source_dir.join("beta/SKILL.md").is_file());
        let entries = manager.local_inventory();
        let agents = &entry(&entries, "beta").agents;
        assert!(agents.contains(&"codex".to_string()) && agents.contains(&"claude".to_string()));
    }

    #[test]
    fn agent_own_skill_can_be_disabled_without_losing_its_contents() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let own = temp.path().join("claude-skills/gamma");
        write_skill(&own, "claude own");

        let entries = manager.local_inventory();
        let gamma = entry(&entries, "gamma");
        assert_eq!(gamma.agents, vec!["claude".to_string()]);
        assert!(!gamma.managed && !gamma.enabled);

        manager.set_enabled_for("claude", "gamma", false).unwrap();
        assert!(!own.exists());
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("gamma/SKILL.md")).unwrap(),
            "claude own"
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|content| content == "claude own")
        }));

        manager.set_enabled_for("codex", "gamma", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("gamma/SKILL.md")).unwrap(),
            "claude own"
        );
        manager.set_enabled_for("claude", "gamma", true).unwrap();
        assert_eq!(
            std::fs::read_to_string(own.join("SKILL.md")).unwrap(),
            "claude own"
        );
    }

    #[test]
    fn uninstall_unlinks_other_agents() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/delta"), "delta");
        manager
            .import_from_directory("delta", &temp.path().join("external/delta"))
            .unwrap();
        manager.set_enabled_for("grok", "delta", true).unwrap();
        manager.uninstall("delta").unwrap();
        assert!(std::fs::symlink_metadata(temp.path().join("grok-skills/delta")).is_err());
    }

    #[test]
    fn cc_switch_origin_requires_confirmation_until_a_verified_update() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let incoming = temp.path().join("cc-switch/alpha");
        write_skill(&incoming, "customized import");
        let repo = parse_repo_key("example/skills@main").unwrap();
        let mut origins = BTreeMap::new();
        origins.insert(
            "alpha".to_string(),
            crate::ccs_import::CcsSkillOrigin {
                repo_key: repo.key(),
                repo_path: "skills/alpha".to_string(),
            },
        );
        let candidates = vec![("alpha".to_string(), incoming.clone())];
        let preview = manager.preview_skill_import(&candidates, &origins).unwrap();
        let selection = SkillImportSelection {
            id: "alpha".to_string(),
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
        manager.set_enabled("alpha", true).unwrap();
        let imported = manager.load_state().installed["alpha"].clone();
        assert_eq!(imported.local_baseline_hash, UNVERIFIED_CC_SWITCH_BASELINE);
        assert_eq!(
            imported.content_hash,
            skill_git_tree_hash(&incoming).unwrap()
        );

        let zip = repo_zip(&[("skills-main/skills/alpha/SKILL.md", "remote update")]);
        let mut remote = parse_skills_from_zip(&repo, &zip, &BTreeMap::new())
            .unwrap()
            .remove(0);
        remote.id = "alpha".to_string();
        assert!(entry(&manager.merge_entries(&[remote.clone()]), "alpha").update_available);
        let snapshot = manager.update_local_snapshot("alpha").unwrap().unwrap();
        assert!(
            manager
                .update_existing_from_zip_with_confirmation(&remote, &zip, None)
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "customized import"
        );
        manager
            .update_existing_from_zip_with_confirmation(&remote, &zip, Some(&snapshot))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "remote update"
        );
        assert_eq!(
            std::fs::read_to_string(incoming.join("SKILL.md")).unwrap(),
            "customized import"
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|content| content == "customized import")
        }));
        assert_eq!(
            manager.load_state().installed["alpha"].import_source,
            "cc-switch"
        );
        assert!(manager.update_local_snapshot("alpha").unwrap().is_none());

        let next_zip = repo_zip(&[("skills-main/skills/alpha/SKILL.md", "next update")]);
        let mut next = parse_skills_from_zip(&repo, &next_zip, &BTreeMap::new())
            .unwrap()
            .remove(0);
        next.id = "alpha".to_string();
        manager
            .update_existing_from_zip_with_confirmation(&next, &next_zip, None)
            .unwrap();
        assert!(manager.update_local_snapshot("alpha").unwrap().is_none());
    }

    #[test]
    fn ccs_import_preserves_both_conflicting_user_copies() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        for (directory, content) in [
            (&external, "imported"),
            (&manager.source_dir.join("alpha"), "old source"),
            (&manager.linked_dir().join("alpha"), "user local"),
        ] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        assert!(manager.import_from_directory("alpha", &external).is_err());
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "user local"
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "old source"
        );
        assert_eq!(
            std::fs::read_to_string(external.join("SKILL.md")).unwrap(),
            "imported"
        );
    }

    #[test]
    fn ccs_import_save_failure_rolls_back_content_and_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        for (directory, content) in [
            (&external, "imported"),
            (&manager.source_dir.join("alpha"), "old source"),
            (&manager.linked_dir().join("alpha"), "user local"),
        ] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        std::fs::create_dir_all(&manager.state_path).unwrap();
        let before = manager.load_state();
        assert!(manager.import_from_directory("alpha", &external).is_err());
        assert_eq!(manager.load_state(), before);
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "old source"
        );
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "user local"
        );
        assert!(manager.list_backups().is_empty());
    }

    #[test]
    fn ccs_import_keeps_source_but_defaults_to_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("SKILL.md"), "---\nname: imported\n---\n").unwrap();
        manager
            .import_from_directory("imported", &external)
            .unwrap();
        let entries = manager.local_inventory();
        let skill = entries.iter().find(|skill| skill.id == "imported").unwrap();
        assert!(skill.installed);
        assert!(!skill.enabled);
        assert_eq!(skill.repo_key, "cc-switch");
        assert!(manager.source_dir().join("imported/SKILL.md").is_file());
        assert!(external.join("SKILL.md").is_file());
        manager.set_enabled("imported", true).unwrap();
        assert!(
            manager
                .local_inventory()
                .iter()
                .find(|skill| skill.id == "imported")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn local_import_accepts_a_skill_or_a_folder_of_skills_and_explains_missing_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);

        // 选中的目录本身就是一个 Skill。
        let single = temp.path().join("picked/my-skill");
        write_skill(&single, "---\nname: My Skill\ndescription: demo\n---\n");
        let candidates = local_skill_import_candidates(&single).unwrap();
        assert_eq!(candidates, vec![("my-skill".to_string(), single.clone())]);
        manager
            .import_from_local_directory("my-skill", &single)
            .unwrap();
        let entries = manager.local_inventory();
        let skill = entry(&entries, "my-skill");
        assert!(skill.installed);
        assert!(!skill.enabled, "本地导入与 cc-switch 导入一样默认停用");
        assert_eq!(skill.repo_key, LOCAL_IMPORT_REPO_KEY);
        assert_eq!(skill.name, "My Skill");
        assert!(manager.source_dir().join("my-skill/SKILL.md").is_file());
        assert!(
            single.join("SKILL.md").is_file(),
            "导入是复制，原目录保持不动"
        );

        // 选中的是一组 Skill 的父目录：只收含 SKILL.md 的子目录，跳过隐藏目录和普通文件夹。
        let bundle = temp.path().join("bundle");
        write_skill(&bundle.join("beta"), "---\nname: beta\n---\n");
        write_skill(&bundle.join("alpha"), "---\nname: alpha\n---\n");
        write_skill(&bundle.join(".hidden"), "---\nname: hidden\n---\n");
        std::fs::create_dir_all(bundle.join("notes")).unwrap();
        let ids: Vec<_> = local_skill_import_candidates(&bundle)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);

        // 没有 SKILL.md 时给出明确提示。
        let empty = temp.path().join("empty");
        std::fs::create_dir_all(empty.join("notes")).unwrap();
        let error = local_skill_import_candidates(&empty)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SKILL.md"), "{error}");
        assert!(local_skill_import_candidates(&temp.path().join("missing")).is_err());

        // Codex 自己的 skills 目录里的 Skill 已会被扫描显示，不允许再「导入」把原目录挪走。
        let codex_local = manager.linked_dir().join("codex-local");
        write_skill(&codex_local, "---\nname: codex-local\n---\n");
        let error = manager
            .import_from_local_directory("codex-local", &codex_local)
            .unwrap_err()
            .to_string();
        assert!(error.contains("无需导入"), "{error}");
        assert!(codex_local.join("SKILL.md").is_file());
    }

    #[test]
    fn saved_repository_toggle_is_local_but_identity_changes_need_validation() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut repo = parse_repo_key("unreachable-owner/absent-repo@main").unwrap();
        manager.upsert_repo(repo.clone()).unwrap();
        repo.enabled = false;
        assert!(
            manager
                .update_repo_enabled_if_unchanged(&repo, None)
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
        let old = repo.key();
        for field in [
            "other-owner/absent-repo@main",
            "unreachable-owner/other-repo@main",
            "unreachable-owner/absent-repo@dev",
            "unreachable-owner/absent-repo@main:other",
        ] {
            let changed = parse_repo_key(field).unwrap();
            assert!(
                !manager
                    .update_repo_enabled_if_unchanged(&changed, Some(&old))
                    .unwrap()
            );
            assert!(
                !manager
                    .list_repos()
                    .iter()
                    .any(|item| item.key() == changed.key())
            );
        }
        repo.enabled = true;
        assert!(
            manager
                .update_repo_enabled_if_unchanged(&repo, Some(&old))
                .unwrap()
        );
        assert!(
            manager
                .list_repos()
                .iter()
                .find(|item| item.key() == old)
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn editing_repository_replaces_identity_and_rejects_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = parse_repo_key("acme/old@main").unwrap();
        manager.upsert_repo(original.clone()).unwrap();
        let replacement = parse_repo_key("acme/new@dev:skills").unwrap();
        let repos = manager
            .save_repo(replacement.clone(), Some(&original.key()))
            .unwrap();
        assert!(!repos.iter().any(|repo| repo.key() == original.key()));
        assert_eq!(
            repos
                .iter()
                .filter(|repo| repo.key() == replacement.key())
                .count(),
            1
        );
        assert!(
            manager
                .save_repo(
                    parse_repo_key("openai/skills@main:skills/.curated").unwrap(),
                    Some(&replacement.key())
                )
                .is_err()
        );
        assert!(
            manager
                .list_repos()
                .iter()
                .any(|repo| repo.key() == replacement.key())
        );
    }

    #[test]
    fn updating_skill_refuses_to_overwrite_resolved_user_edits() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "one"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        let local = manager.linked_dir().join("alpha");
        remove_link(&local).unwrap();
        std::fs::create_dir(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "local edits").unwrap();
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "local edits"
        );
        manager.resolve_codex_conflict("alpha", true).unwrap();
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "local edits"
        );
        let contents: std::collections::BTreeSet<_> = manager
            .list_backups()
            .iter()
            .map(|backup| {
                std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                    .unwrap()
            })
            .collect();
        assert!(contents.contains("original"));
    }

    #[test]
    fn remote_update_does_not_overwrite_modified_or_shared_local_library_content() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = sample_skill("alpha", "alpha", "one");
        manager
            .install_from_zip(
                &original,
                &repo_zip(&[("kit-main/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        for dir in [
            manager.source_dir.join("alpha"),
            manager.linked_dir().join("alpha"),
        ] {
            std::fs::write(dir.join("SKILL.md"), "local edit").unwrap();
        }
        assert!(!manager.codex_copy_conflict("alpha").unwrap());
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "remote")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "local edit"
        );
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "local edit"
        );
        assert_eq!(manager.load_state().installed["alpha"].content_hash, "one");
    }

    #[test]
    fn choosing_codex_edit_does_not_silently_authorize_a_later_remote_update() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager
            .install_from_zip(
                &sample_skill("alpha", "alpha", "one"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        std::fs::write(manager.linked_dir().join("alpha/SKILL.md"), "codex edit").unwrap();
        manager.resolve_codex_conflict("alpha", true).unwrap();
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "remote update")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "codex edit"
        );
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "codex edit"
        );
        let snapshot = manager.update_local_snapshot("alpha").unwrap().unwrap();
        std::fs::write(
            manager.linked_dir().join("alpha/SKILL.md"),
            "changed after confirmation",
        )
        .unwrap();
        assert!(
            manager
                .install_from_zip_with_confirmation(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "remote update")]),
                    Some(&snapshot)
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "codex edit"
        );
        manager.resolve_codex_conflict("alpha", true).unwrap();
        let snapshot = manager.update_local_snapshot("alpha").unwrap().unwrap();
        manager
            .install_from_zip_with_confirmation(
                &sample_skill("alpha", "alpha", "two"),
                &repo_zip(&[("kit-main/alpha/SKILL.md", "remote update")]),
                Some(&snapshot),
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "remote update"
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|text| text == "changed after confirmation")
        }));
    }

    #[test]
    fn legacy_updates_use_stored_remote_hash_and_require_confirmation_when_unknown_or_modified() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let original = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let old = parse_skills_from_zip(&repo, &original, &BTreeMap::new())
            .unwrap()
            .remove(0);
        manager.install_from_zip(&old, &original).unwrap();
        let id = old.id.clone();
        let _guard = manager.state_lock.lock().unwrap();
        let mut state = manager.load_state_unlocked();
        state
            .installed
            .get_mut(&id)
            .unwrap()
            .local_baseline_hash
            .clear();
        manager.save_state_unlocked(&state).unwrap();
        drop(_guard);
        assert!(manager.update_local_snapshot(&id).unwrap().is_none());
        std::fs::write(
            manager.source_dir.join(&id).join("SKILL.md"),
            "modified legacy",
        )
        .unwrap();
        std::fs::write(
            manager.linked_dir().join(&id).join("SKILL.md"),
            "modified legacy",
        )
        .unwrap();
        assert!(manager.update_local_snapshot(&id).unwrap().is_some());
        let _guard = manager.state_lock.lock().unwrap();
        let mut state = manager.load_state_unlocked();
        state.installed.get_mut(&id).unwrap().content_hash.clear();
        manager.save_state_unlocked(&state).unwrap();
        drop(_guard);
        assert!(manager.update_local_snapshot(&id).unwrap().is_some());
        assert!(manager.install_from_zip(&old, &original).is_err());
    }

    #[test]
    fn transaction_uses_confirmed_directory_hash_after_the_target_changes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        write_skill(&manager.source_dir.join("alpha"), "confirmed");
        manager.prepare_managed_skills().unwrap();
        let _operation = manager.operation_lock.lock().unwrap();
        let state = manager.load_state();
        let snapshot = manager
            .import_target_snapshot_details_unlocked(&state, "alpha")
            .unwrap();
        let operation = uuid::Uuid::new_v4().to_string();
        manager
            .stage_skill_copy(
                &operation,
                0,
                &SkillPlace::Library,
                &manager.source_dir.join("alpha"),
                false,
            )
            .unwrap();
        std::fs::write(
            manager.source_dir.join("alpha/SKILL.md"),
            "new edit after confirmation",
        )
        .unwrap();
        assert!(
            manager
                .commit_skill_transaction(
                    &operation,
                    "alpha",
                    vec![PreparedSkillChange {
                        place: SkillPlace::Library,
                        staged: true,
                        preserve_backup: true,
                        expected_backup_hash: snapshot.directory_hashes[0].clone(),
                    }],
                    None
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "new edit after confirmation"
        );
    }

    #[test]
    fn staged_zip_update_rejects_origin_changes_before_commit() {
        for cross_repo in [false, true] {
            for confirmed in [false, true] {
                for enabled in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let manager = manager(&temp);
                    let old = sample_skill("alpha", "z/alpha", "old-hash");
                    let original = "---\nname: Original\n---\nlocal content\n";
                    manager
                        .install_from_zip(
                            &old,
                            &repo_zip(&[("kit-main/z/alpha/SKILL.md", original)]),
                        )
                        .unwrap();
                    manager.set_enabled("alpha", enabled).unwrap();
                    let mut changed = manager.load_state();
                    let installed = changed.installed.get_mut("alpha").unwrap();
                    installed.repo_path = "a/alpha".into();
                    if cross_repo {
                        installed.repo_key = "other/kit@main".into();
                    }
                    let expected = if confirmed {
                        Some(
                            manager
                                .import_target_snapshot_unlocked(&changed, "alpha")
                                .unwrap(),
                        )
                    } else {
                        None
                    };
                    let guard = manager.operation_lock.lock().unwrap();
                    let worker_manager = manager.clone();
                    let updated_zip =
                        repo_zip(&[("kit-main/z/alpha/SKILL.md", "downloaded update")]);
                    let worker = std::thread::spawn(move || {
                        worker_manager.install_from_zip_with_confirmation(
                            &old,
                            &updated_zip,
                            expected.as_deref(),
                        )
                    });
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    let staged = loop {
                        let staged = std::fs::read_dir(&manager.source_dir)
                            .unwrap()
                            .flatten()
                            .any(|entry| {
                                entry
                                    .file_name()
                                    .to_string_lossy()
                                    .starts_with(".staging-alpha-")
                                    && entry.path().join(SKILL_MANIFEST_FILE).is_file()
                            });
                        if staged || worker.is_finished() || std::time::Instant::now() >= deadline {
                            break staged;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    };
                    if staged {
                        let _state = manager.state_lock.lock().unwrap();
                        manager.save_state_unlocked(&changed).unwrap();
                    }
                    drop(guard);
                    let result = worker.join().unwrap();
                    assert!(staged, "update did not reach staging");
                    let error = result.unwrap_err().to_string();
                    assert!(
                        error.contains("来源路径已变化"),
                        "unexpected error: {error}"
                    );
                    assert_eq!(manager.load_state(), changed);
                    assert_eq!(
                        std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
                        original
                    );
                    if enabled {
                        assert_eq!(
                            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md"))
                                .unwrap(),
                            original
                        );
                    } else {
                        assert!(!manager.linked_dir().join("alpha").exists());
                    }
                }
            }
        }
    }

    #[test]
    fn staged_zip_update_does_not_reinstall_a_removed_skill() {
        for enabled in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let manager = manager(&temp);
            let skill = sample_skill("alpha", "z/alpha", "old-hash");
            let original = "---\nname: Original\n---\nlocal content\n";
            manager
                .install_from_zip(
                    &skill,
                    &repo_zip(&[("kit-main/z/alpha/SKILL.md", original)]),
                )
                .unwrap();
            manager.set_enabled("alpha", enabled).unwrap();
            let guard = manager.operation_lock.lock().unwrap();
            let worker_manager = manager.clone();
            let worker_skill = skill.clone();
            let zip = repo_zip(&[("kit-main/z/alpha/SKILL.md", "downloaded update")]);
            let worker = std::thread::spawn(move || {
                worker_manager.update_existing_from_zip_with_confirmation(&worker_skill, &zip, None)
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let staged = loop {
                let staged = std::fs::read_dir(&manager.source_dir)
                    .unwrap()
                    .flatten()
                    .any(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".staging-alpha-")
                            && entry.path().join(SKILL_MANIFEST_FILE).is_file()
                    });
                if staged || worker.is_finished() || std::time::Instant::now() >= deadline {
                    break staged;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            if staged {
                let _state = manager.state_lock.lock().unwrap();
                let mut state = manager.load_state_checked_unlocked().unwrap();
                state.installed.remove("alpha");
                std::fs::rename(
                    manager.source_dir.join("alpha"),
                    temp.path().join("removed-source"),
                )
                .unwrap();
                if enabled {
                    std::fs::rename(
                        manager.linked_dir().join("alpha"),
                        temp.path().join("removed-codex"),
                    )
                    .unwrap();
                }
                manager.save_state_unlocked(&state).unwrap();
            }
            drop(guard);
            let result = worker.join().unwrap();
            assert!(staged, "update did not reach staging");
            assert!(result.unwrap_err().to_string().contains("已卸载"));
            assert!(!manager.load_state().installed.contains_key("alpha"));
            assert!(!manager.source_dir.join("alpha").exists());
            assert!(!manager.linked_dir().join("alpha").exists());
            assert_eq!(
                std::fs::read_to_string(temp.path().join("removed-source/SKILL.md")).unwrap(),
                original
            );
        }
    }

    #[test]
    fn staged_new_zip_install_does_not_replace_concurrent_install() {
        for occupied in ["id", "origin", "agent"] {
            let temp = tempfile::tempdir().unwrap();
            let agent_dir = temp.path().join("agent-skills");
            let manager =
                manager(&temp).with_agent_dirs(vec![("claude".into(), agent_dir.clone())]);
            std::fs::create_dir_all(&manager.source_dir).unwrap();
            let candidate = sample_skill("alpha", "z/alpha", "remote-hash");
            let original = "---\nname: Concurrent\n---\nlocal content\n";
            let guard = manager.operation_lock.lock().unwrap();
            let worker_manager = manager.clone();
            let worker_skill = candidate.clone();
            let zip = repo_zip(&[("kit-main/z/alpha/SKILL.md", "downloaded content")]);
            let worker = std::thread::spawn(move || {
                worker_manager.install_new_from_zip(&worker_skill, &zip)
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let staged = loop {
                let staged = std::fs::read_dir(&manager.source_dir)
                    .unwrap()
                    .flatten()
                    .any(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".staging-alpha-")
                            && entry.path().join(SKILL_MANIFEST_FILE).is_file()
                    });
                if staged || worker.is_finished() || std::time::Instant::now() >= deadline {
                    break staged;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            let existing_id = if occupied == "origin" {
                "other-alpha"
            } else {
                "alpha"
            };
            let existing = if occupied == "agent" {
                agent_dir.join(existing_id)
            } else {
                manager.source_dir.join(existing_id)
            };
            if staged {
                write_skill(&existing, original);
                if occupied != "agent" {
                    let _state = manager.state_lock.lock().unwrap();
                    let mut state = manager.load_state_checked_unlocked().unwrap();
                    state.installed.insert(
                        existing_id.into(),
                        InstalledSkill {
                            id: existing_id.into(),
                            name: "Concurrent".into(),
                            description: String::new(),
                            repo_key: candidate.repo_key.clone(),
                            repo_path: candidate.repo_path.clone(),
                            import_source: String::new(),
                            local_baseline_hash: skill_directory_hash(&existing).unwrap(),
                            content_hash: candidate.content_hash.clone(),
                            installed_at: "concurrent".into(),
                        },
                    );
                    manager.save_state_unlocked(&state).unwrap();
                }
            }
            drop(guard);
            let result = worker.join().unwrap();
            assert!(staged, "{occupied}: install did not reach staging");
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("同名 Skill 已存在") || error.contains("仓库路径已被占用"),
                "{occupied}: unexpected error {error}"
            );
            assert_eq!(
                std::fs::read_to_string(existing.join(SKILL_MANIFEST_FILE)).unwrap(),
                original
            );
            if occupied == "id" {
                assert_eq!(
                    manager.load_state().installed["alpha"].installed_at,
                    "concurrent"
                );
            } else {
                assert!(!manager.source_dir.join("alpha").exists());
                assert!(!manager.load_state().installed.contains_key("alpha"));
            }
        }
    }

    #[test]
    fn repeated_codex_toggle_preserves_library_and_recoverable_disabled_versions() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        write_skill(&manager.linked_dir().join("alpha"), "user content");
        manager.prepare_managed_skills().unwrap();
        for _ in 0..3 {
            manager.set_enabled("alpha", false).unwrap();
            assert!(!manager.linked_dir().join("alpha").exists());
            manager.set_enabled("alpha", true).unwrap();
            assert!(!manager.codex_copy_conflict("alpha").unwrap());
        }
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "user content"
        );
        assert!(manager.list_backups().iter().any(|backup| {
            std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|value| value == "user content")
        }));
    }

    #[test]
    fn installation_link_failure_does_not_publish_source_or_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        std::fs::create_dir_all(&manager.codex_home).unwrap();
        std::fs::write(manager.linked_dir(), "blocked").unwrap();
        let before = manager.load_state();
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "one"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])
                )
                .is_err()
        );
        assert!(!manager.source_dir.join("alpha").exists());
        assert_eq!(manager.load_state(), before);
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir()).unwrap(),
            "blocked"
        );
    }

    #[test]
    fn updating_skill_rolls_back_source_when_state_cannot_be_saved() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let skill = sample_skill("alpha", "alpha", "one");
        manager
            .install_from_zip(
                &skill,
                &repo_zip(&[("kit-main/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        std::fs::rename(&manager.state_path, temp.path().join("original-state.json")).unwrap();
        std::fs::create_dir(&manager.state_path).unwrap();
        assert!(
            manager
                .install_from_zip(
                    &sample_skill("alpha", "alpha", "two"),
                    &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "original"
        );
        assert_eq!(
            std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(),
            "original"
        );
    }

    #[test]
    fn updating_disabled_skill_preserves_disabled_state() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\n---\n")]);
        manager
            .install_from_zip(&sample_skill("alpha", "alpha", "v1"), &zip)
            .unwrap();
        manager.set_enabled("alpha", false).unwrap();
        manager
            .install_from_zip(&sample_skill("alpha", "alpha", "v2"), &zip)
            .unwrap();
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
            "---\nname: example\ndescription: >-\n  First line.\n  Second line.\nmetadata:\n  short-description: ignored\n---\n",
            "example",
        );
        assert_eq!(folded, "First line. Second line.");
        let (_, literal) =
            parse_skill_frontmatter("---\ndescription: |\n  First\n  Second\n---\n", "example");
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
        let before = parse_skills_from_tree(
            &repo,
            &json!({"tree":[{"type":"blob","path":"z/alpha/SKILL.md","sha":"one"}]}),
        );
        manager
            .install_from_zip(
                &before[0],
                &repo_zip(&[("kit-main/z/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        let after = parse_skills_from_tree(
            &repo,
            &json!({"tree":[
                {"type":"blob","path":"a/alpha/SKILL.md","sha":"new"},
                {"type":"blob","path":"z/alpha/SKILL.md","sha":"changed"}
            ]}),
        );
        assert_eq!(
            before[0].id,
            after.iter().find(|s| s.repo_path == "z/alpha").unwrap().id
        );
        let entries = manager.merge_entries(&after);
        assert!(
            entries
                .iter()
                .any(|s| s.repo_path == "z/alpha" && s.installed && s.update_available)
        );
        assert!(
            entries
                .iter()
                .any(|s| s.repo_path == "a/alpha" && !s.installed && !s.update_available)
        );
        assert_eq!(
            manager.load_state().installed[&before[0].id].repo_path,
            "z/alpha"
        );
    }

    #[test]
    fn legacy_install_without_path_is_not_assigned_to_a_remote_skill() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = sample_skill("alpha", "z/alpha", "one");
        manager
            .install_from_zip(
                &original,
                &repo_zip(&[("kit-main/z/alpha/SKILL.md", "original")]),
            )
            .unwrap();
        let mut state = manager.load_state();
        state.installed.get_mut("alpha").unwrap().repo_path.clear();
        manager.save_state_unlocked(&state).unwrap();
        let remote = sample_skill("alpha", "a/alpha", "new");
        assert!(
            !manager
                .merge_entries(&[remote.clone()])
                .iter()
                .any(|s| s.repo_path == "a/alpha" && s.update_available)
        );
        assert!(
            manager
                .install_from_zip(
                    &remote,
                    &repo_zip(&[("kit-main/a/alpha/SKILL.md", "wrong")])
                )
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "original"
        );
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
        assert!(
            skills
                .iter()
                .any(|s| s.repo_path == "skills/.curated/alpha")
        );
        let scoped =
            parse_skills_from_tree(&parse_repo_key("acme/kit@main:plugins/a").unwrap(), &tree);
        assert_eq!(scoped.len(), 1);
        assert!(scoped[0].id.starts_with("alpha--"));
    }

    #[test]
    fn zip_catalog_reads_nested_manifests_and_root_of_selected_subdirectory() {
        let repo = parse_repo_key("acme/kit@main:plugins").unwrap();
        let zip = repo_zip(&[
            (
                "kit-main/plugins/alpha/SKILL.md",
                "---\nname: Actual alpha\ndescription: Nested skill\n---\n",
            ),
            ("kit-main/plugins/alpha/scripts/run.py", "print('ok')"),
            ("kit-main/outside/beta/SKILL.md", "---\nname: beta\n---\n"),
        ]);
        let skills = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Actual alpha");
        assert_eq!(skills[0].description, "Nested skill");
        assert_eq!(skills[0].repo_path, "plugins/alpha");
        let exact = parse_repo_key("acme/kit@main:plugins/alpha").unwrap();
        assert_eq!(
            parse_skills_from_zip(&exact, &zip, &BTreeMap::new())
                .unwrap()
                .len(),
            1
        );
        // Git 对 "hello\n" 的 blob ID；API 和 ZIP 的子树标识必须一致。
        let bytes = repo_zip(&[("kit-main/alpha/SKILL.md", "hello\n")]);
        let root = parse_repo_key("acme/kit@main").unwrap();
        let zipped = parse_skills_from_zip(&root, &bytes, &BTreeMap::new()).unwrap();
        let tree = json!({"tree":[{"type":"blob","path":"alpha/SKILL.md",
            "sha":"ce013625030ba8dba906f756967f9e9ca394464a"}]});
        assert_eq!(
            zipped[0].content_hash,
            parse_skills_from_tree(&root, &tree)[0].content_hash
        );
    }

    #[test]
    fn installing_a_snapshot_records_the_hash_and_metadata_of_its_actual_contents() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let old = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: Old\n---\n")]);
        let current = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: Current\n---\n")]);
        let old_skill = parse_skills_from_zip(&repo, &old, &BTreeMap::new())
            .unwrap()
            .remove(0);
        let skill = parse_skills_from_zip(&repo, &current, &BTreeMap::new())
            .unwrap()
            .remove(0);

        assert_eq!(old_skill.id, skill.id);
        assert_ne!(old_skill.content_hash, skill.content_hash);
        manager.install_from_zip(&skill, &current).unwrap();
        let installed = &manager.load_state().installed[&skill.id];
        assert_eq!(installed.content_hash, skill.content_hash);
        assert_eq!(installed.name, "Current");
        assert!(
            std::fs::read_to_string(manager.source_dir().join(&skill.id).join("SKILL.md"))
                .unwrap()
                .contains("Current")
        );
    }

    #[test]
    fn snapshot_rejects_inconsistent_roots_and_paths() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        for zip in [
            repo_zip(&[
                ("first/alpha/SKILL.md", "first"),
                ("second/alpha/SKILL.md", "second"),
            ]),
            repo_zip(&[
                ("first/alpha/SKILL.md", "first"),
                ("first/alpha/../evil", "escape"),
            ]),
            repo_zip(&[
                ("first/alpha/SKILL.md", "first"),
                ("first/alpha//SKILL.md", "duplicate"),
            ]),
        ] {
            assert!(parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).is_err());
            let temp = tempfile::tempdir().unwrap();
            assert!(extract_skill_subtree(&zip, "alpha", temp.path()).is_err());
        }
    }

    #[test]
    #[cfg(windows)]
    fn extraction_rejects_windows_aliases_and_alternate_stream_paths() {
        let temp = tempfile::tempdir().unwrap();
        for invalid in [
            "bad.",
            "bad ",
            "CON",
            "nested/file:stream",
            "nested/NUL.txt",
        ] {
            let zip = repo_zip(&[
                ("kit-main/alpha/SKILL.md", "skill"),
                (&format!("kit-main/alpha/{invalid}"), "unexpected"),
            ]);
            let destination = temp.path().join(format!("case-{}", uuid::Uuid::new_v4()));
            assert!(
                extract_skill_subtree(&zip, "alpha", &destination).is_err(),
                "{invalid}"
            );
        }
    }

    #[tokio::test]
    async fn repository_zip_download_stops_before_oversized_body_is_buffered() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 1024];
                let read = socket.read(&mut request).await.unwrap();
                let request = String::from_utf8_lossy(&request[..read]);
                if request.starts_with("GET /declared ") {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        REPO_ZIP_DOWNLOAD_LIMIT_BYTES + 1
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                } else if request.starts_with("GET /chunked ") {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                    let payload = vec![b'x'; 65536];
                    for _ in 0..=REPO_ZIP_DOWNLOAD_LIMIT_BYTES / payload.len() {
                        if socket.write_all(b"10000\r\n").await.is_err()
                            || socket.write_all(&payload).await.is_err()
                            || socket.write_all(b"\r\n").await.is_err()
                        {
                            break;
                        }
                    }
                } else {
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc",
                        )
                        .await
                        .unwrap();
                }
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for path in ["declared", "chunked"] {
            let response = client
                .get(format!("http://{address}/{path}"))
                .send()
                .await
                .unwrap();
            assert!(
                read_repo_zip_response(response)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("64 MiB")
            );
        }
        let response = client
            .get(format!("http://{address}/small"))
            .send()
            .await
            .unwrap();
        assert_eq!(read_repo_zip_response(response).await.unwrap(), b"abc");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn github_system_proxy_failure_retries_direct_and_remembers_host() {
        use std::io::{Read, Write};
        let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_address = dead.local_addr().unwrap();
        drop(dead);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            for _ in 0..2 {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < deadline);
                            std::thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = [0; 4096];
                stream.read(&mut bytes).unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .unwrap();
            }
        });
        let primary = || {
            reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(format!("http://{dead_address}")).unwrap())
                .connect_timeout(std::time::Duration::from_secs(1))
                .build()
                .unwrap()
        };
        let client = GithubClient {
            primary: primary(),
            direct: Some(crate::http_client::direct_client_with_timeouts("test").unwrap()),
            direct_hosts: Default::default(),
        };
        for _ in 0..2 {
            assert_eq!(
                client
                    .get(format!("http://{address}/"), "text/plain", Some(3))
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap(),
                "ok"
            );
        }
        assert!(client.direct_hosts.lock().unwrap().contains("127.0.0.1"));
        let explicit = GithubClient {
            primary: primary(),
            direct: None,
            direct_hosts: Default::default(),
        };
        assert!(
            explicit
                .get(format!("http://{address}/"), "text/plain", Some(3))
                .await
                .is_err()
        );
        server.join().unwrap();
    }

    #[tokio::test]
    async fn github_zip_body_keeps_streaming_without_metadata_total_timeout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                stream.read(&mut bytes).await.unwrap();
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\no",
                    )
                    .await
                    .unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
                let _ = stream.write_all(b"k").await;
            }
        });
        let client = GithubClient {
            primary: crate::http_client::direct_client_with_timeouts("test").unwrap(),
            direct: None,
            direct_hosts: Default::default(),
        };
        let metadata = client
            .get(format!("http://{address}/"), "text/plain", Some(1))
            .await
            .unwrap();
        assert!(metadata.text().await.unwrap_err().is_timeout());
        let zip = client
            .get(format!("http://{address}/"), "application/zip", None)
            .await
            .unwrap();
        assert_eq!(read_repo_zip_response(zip).await.unwrap(), b"ok");
        server.await.unwrap();
    }

    #[tokio::test]
    #[ignore = "需要访问 GitHub，用于验证真实仓库快照"]
    async fn planning_with_files_snapshot_can_be_installed() {
        let repo = parse_repo_key("OthmanAdi/planning-with-files@master").unwrap();
        let zip = download_repo_zip(&repo).await.unwrap();
        let skills = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        let skill = skills
            .iter()
            .find(|skill| skill.repo_path == "skills/planning-with-files")
            .expect("未在仓库快照中找到目标 Skill");
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager.install_from_zip(skill, &zip).unwrap();
        assert!(
            manager
                .source_dir()
                .join(&skill.id)
                .join("SKILL.md")
                .is_file()
        );
        assert_eq!(
            manager.load_state().installed[&skill.id].content_hash,
            skill.content_hash
        );
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
    fn extraction_rejects_only_symlinks_in_the_selected_subtree() {
        let mut buffer = Cursor::new(Vec::<u8>::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            writer
                .start_file("kit-main/alpha/SKILL.md", options)
                .unwrap();
            writer.write_all(b"alpha").unwrap();
            writer
                .start_file("kit-main/beta/SKILL.md", options)
                .unwrap();
            writer.write_all(b"beta").unwrap();
            writer
                .add_symlink("kit-main/beta/link", "../../outside", options)
                .unwrap();
            writer
                .add_symlink("kit-main/alpha-other", "../../outside", options)
                .unwrap();
            writer.finish().unwrap();
        }
        let zip = buffer.into_inner();
        let temp = tempfile::tempdir().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        assert_eq!(
            parse_skills_from_zip(&repo, &zip, &BTreeMap::new())
                .unwrap()
                .len(),
            2
        );

        extract_skill_subtree(&zip, "alpha", &temp.path().join("alpha")).unwrap();
        assert!(temp.path().join("alpha/SKILL.md").is_file());
        assert!(!temp.path().join("alpha/alpha-other").exists());
        for path in ["beta", ".", "alpha-other"] {
            let error =
                extract_skill_subtree(&zip, path, &temp.path().join("rejected")).unwrap_err();
            assert!(error.to_string().contains("符号链接"));
        }
    }

    #[test]
    fn extract_rejects_a_subtree_without_a_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let zip = repo_zip(&[("kit-main/alpha/README.md", "no manifest\n")]);

        let error = extract_skill_subtree(&zip, "alpha", &temp.path().join("alpha")).unwrap_err();

        assert!(error.to_string().contains("SKILL.md"));
    }

    #[cfg(windows)]
    pub(super) fn make_junction(link: &Path, target: &Path) {
        use std::ffi::c_void;
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn CreateFileW(
                name: *const u16,
                access: u32,
                share: u32,
                security: *const c_void,
                creation: u32,
                flags: u32,
                template: *mut c_void,
            ) -> *mut c_void;
            fn DeviceIoControl(
                handle: *mut c_void,
                code: u32,
                input: *const c_void,
                input_len: u32,
                output: *mut c_void,
                output_len: u32,
                returned: *mut u32,
                overlapped: *mut c_void,
            ) -> i32;
            fn CloseHandle(handle: *mut c_void) -> i32;
        }
        const GENERIC_WRITE: u32 = 0x40000000;
        const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
        std::fs::create_dir_all(link).unwrap();
        let absolute = target
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join(target.file_name().unwrap());
        let target = absolute
            .to_str()
            .unwrap()
            .strip_prefix(r"\\?\")
            .unwrap_or(absolute.to_str().unwrap());
        let substitute: Vec<u16> = format!(r"\??\{target}").encode_utf16().collect();
        let print: Vec<u16> = target.encode_utf16().collect();
        let mut data = Vec::new();
        data.extend_from_slice(&0xA000_0003u32.to_le_bytes());
        data.extend_from_slice(
            &((8 + (substitute.len() + print.len() + 2) * 2) as u16).to_le_bytes(),
        );
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&((substitute.len() * 2) as u16).to_le_bytes());
        data.extend_from_slice(&(((substitute.len() + 1) * 2) as u16).to_le_bytes());
        data.extend_from_slice(&((print.len() * 2) as u16).to_le_bytes());
        for unit in substitute.into_iter().chain([0]).chain(print).chain([0]) {
            data.extend_from_slice(&unit.to_le_bytes());
        }
        let wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                7,
                std::ptr::null(),
                3,
                0x0220_0000,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(handle as isize, -1);
        let mut returned = 0;
        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                data.as_ptr().cast(),
                data.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        unsafe {
            CloseHandle(handle);
        }
        assert_ne!(result, 0, "{}", std::io::Error::last_os_error());
    }

    #[test]
    #[cfg(windows)]
    fn launch_gate_migrates_legacy_junction_before_action() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy/alpha");
        write_skill(&legacy, "original");
        let manager = manager(&temp).with_legacy_source_dir(temp.path().join("legacy"));
        let codex = manager.linked_dir().join("alpha");
        make_junction(&codex, &legacy);

        let guard = manager.lock_recovered_transactions().unwrap();
        assert!(manager.load_state().independent_library_migrated);
        assert!(
            !std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "original"
        );
        assert!(manager.operation_lock.try_lock().is_err());
        std::fs::write(codex.join("SKILL.md"), "codex edit").unwrap();
        assert_eq!(
            std::fs::read_to_string(legacy.join("SKILL.md")).unwrap(),
            "original"
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(),
            "original"
        );
        drop(guard);
        assert!(manager.operation_lock.try_lock().is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn legacy_junctions_detach_codex_and_other_agents_from_library() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let source = manager.source_dir.join("alpha");
        write_skill(&source, "original");
        let codex = manager.linked_dir().join("alpha");
        let claude = temp.path().join("claude-skills/alpha");
        make_junction(&codex, &source);
        make_junction(&claude, &source);
        assert!(
            std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(
            &manager.state_path,
            r#"{"repos":[],"installed":{"alpha":{"id":"alpha"}}}"#,
        )
        .unwrap();
        manager.prepare_managed_skills().unwrap();
        assert!(
            !std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::symlink_metadata(&claude)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(codex.join("SKILL.md"), "edited").unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
            "original"
        );
        assert!(manager.codex_copy_conflict("alpha").unwrap());

        let third_party = temp.path().join("third-party/beta");
        write_skill(&third_party, "outside original");
        std::fs::create_dir_all(&manager.source_dir).unwrap();
        let library_link = manager.source_dir.join("beta");
        let codex_link = manager.linked_dir().join("beta");
        let claude_link = temp.path().join("claude-skills/beta");
        make_junction(&library_link, &third_party);
        make_junction(&codex_link, &library_link);
        make_junction(&claude_link, &third_party);
        std::fs::write(
            &manager.state_path,
            r#"{"repos":[],"installed":{"beta":{"id":"beta"}}}"#,
        )
        .unwrap();
        manager.prepare_managed_skills().unwrap();
        for path in [&library_link, &codex_link] {
            assert!(
                !std::fs::symlink_metadata(path)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(
                std::fs::read_to_string(path.join("SKILL.md")).unwrap(),
                "outside original"
            );
        }
        assert!(
            std::fs::symlink_metadata(&claude_link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(third_party.join("SKILL.md"), "outside edit").unwrap();
        assert_eq!(
            std::fs::read_to_string(library_link.join("SKILL.md")).unwrap(),
            "outside original"
        );

        let legacy = temp.path().join("legacy/gamma");
        write_skill(&legacy, "old private content");
        let codex = manager.linked_dir().join("gamma");
        let third_party_link = temp.path().join("third-party-link/gamma");
        make_junction(&codex, &legacy);
        make_junction(&third_party_link, &legacy);
        std::fs::write(
            &manager.state_path,
            r#"{"repos":[],"installed":{"gamma":{"id":"gamma"}}}"#,
        )
        .unwrap();
        let migrated = manager
            .clone()
            .with_legacy_source_dir(temp.path().join("legacy"));
        migrated.prepare_managed_skills().unwrap();
        assert!(
            !std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(third_party_link.join("SKILL.md"), "external mutation").unwrap();
        assert_eq!(
            std::fs::read_to_string(migrated.source_dir.join("gamma/SKILL.md")).unwrap(),
            "old private content"
        );
        assert_eq!(
            std::fs::read_to_string(codex.join("SKILL.md")).unwrap(),
            "old private content"
        );

        let external = temp.path().join("external/delta");
        write_skill(&external, "external skill");
        let grok = temp.path().join("grok-skills/delta");
        make_junction(&grok, &external);
        migrated.set_enabled_for("grok", "delta", false).unwrap();
        assert!(!grok.exists());
        assert_eq!(
            std::fs::read_to_string(external.join("SKILL.md")).unwrap(),
            "external skill"
        );
        assert_eq!(
            std::fs::read_to_string(migrated.source_dir.join("delta/SKILL.md")).unwrap(),
            "external skill"
        );
        assert!(migrated.list_backups().iter().any(|backup| {
            std::fs::read_to_string(migrated.backups_dir.join(&backup.id).join("SKILL.md"))
                .is_ok_and(|text| text == "external skill")
        }));

        let external = temp.path().join("external/override");
        let incoming = temp.path().join("incoming/override");
        write_skill(
            &external,
            "---\nname: Shared Override\n---\nexternal original",
        );
        write_skill(&incoming, "---\nname: Shared Override\n---\nreplacement");
        let agent = temp.path().join("claude-skills/external-override");
        make_junction(&agent, &external);
        let candidates = vec![("incoming-override".to_string(), incoming)];
        let preview = migrated
            .preview_skill_import(&candidates, &BTreeMap::new())
            .unwrap();
        let matched = preview[0]
            .matches
            .iter()
            .find(|item| item.id == "external-override")
            .unwrap();
        let selection = SkillImportSelection {
            id: "incoming-override".into(),
            source_hash: preview[0].source_hash.clone(),
            repo_key: String::new(),
            repo_path: String::new(),
            target_id: Some(matched.id.clone()),
            target_snapshot: Some(matched.snapshot.clone()),
        };
        let (imported, errors) = migrated
            .apply_skill_import(&candidates, &BTreeMap::new(), "local", &[selection])
            .unwrap();
        assert_eq!(imported, 1);
        assert!(errors.is_empty());
        assert!(
            std::fs::read_to_string(agent.join("SKILL.md"))
                .unwrap()
                .contains("replacement")
        );
        assert!(
            std::fs::read_to_string(external.join("SKILL.md"))
                .unwrap()
                .contains("external original")
        );
        assert!(
            !std::fs::symlink_metadata(&agent)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    #[cfg(windows)]
    fn unmanaged_codex_junction_stays_linked_after_adoption() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let external = temp.path().join("external/custom");
        let codex = manager.linked_dir().join("custom");
        write_skill(&external, "original");
        make_junction(&codex, &external);

        manager.prepare_managed_skills().unwrap();
        manager.prepare_managed_skills().unwrap();

        assert!(manager.load_state().installed.contains_key("custom"));
        assert!(
            std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            codex.canonicalize().unwrap(),
            external.canonicalize().unwrap()
        );
        std::fs::write(external.join("SKILL.md"), "edited externally").unwrap();
        assert_eq!(
            std::fs::read_to_string(codex.join("SKILL.md")).unwrap(),
            "edited externally"
        );
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("custom/SKILL.md")).unwrap(),
            "original"
        );
    }

    #[test]
    #[cfg(windows)]
    fn external_codex_junction_sharing_library_link_target_is_not_detached() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let external = temp.path().join("external/shared");
        let source = manager.source_dir.join("shared");
        let codex = manager.linked_dir().join("shared");
        let agent = temp.path().join("claude-skills/shared");
        write_skill(&external, "original");
        make_junction(&source, &external);
        make_junction(&codex, &external);
        make_junction(&agent, &source);
        std::fs::write(
            &manager.state_path,
            r#"{"repos":[],"installed":{"shared":{"id":"shared"}}}"#,
        )
        .unwrap();

        manager.prepare_managed_skills().unwrap();

        assert!(
            !std::fs::symlink_metadata(&source)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::symlink_metadata(&codex)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(
            !std::fs::symlink_metadata(&agent)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::write(external.join("SKILL.md"), "external edit").unwrap();
        assert_eq!(
            std::fs::read_to_string(codex.join("SKILL.md")).unwrap(),
            "external edit"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
            "original"
        );
        assert_eq!(
            std::fs::read_to_string(agent.join("SKILL.md")).unwrap(),
            "original"
        );
    }

    #[test]
    #[cfg(windows)]
    fn startup_skips_unmanaged_codex_skills_with_internal_links_or_oversized_content() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let linked = manager.linked_dir().join("linked");
        let oversized = manager.linked_dir().join("oversized");
        let healthy = manager.linked_dir().join("healthy");
        let external = temp.path().join("external/nested");
        write_skill(&linked, "linked original");
        write_skill(&oversized, "oversized original");
        write_skill(&healthy, "healthy original");
        write_skill(&external, "nested original");
        make_junction(&linked.join("nested"), &external);
        std::fs::File::create(oversized.join("large.bin"))
            .unwrap()
            .set_len(256 * 1024 * 1024 + 1)
            .unwrap();

        manager.prepare_managed_skills().unwrap();

        let state = manager.load_state();
        assert!(!state.installed.contains_key("linked"));
        assert!(!state.installed.contains_key("oversized"));
        assert!(state.installed.contains_key("healthy"));
        assert!(
            std::fs::symlink_metadata(linked.join("nested"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(external.join("SKILL.md")).unwrap(),
            "nested original"
        );
        assert_eq!(
            std::fs::metadata(oversized.join("large.bin"))
                .unwrap()
                .len(),
            256 * 1024 * 1024 + 1
        );
        assert!(!manager.source_dir.join("linked").exists());
        assert!(!manager.source_dir.join("oversized").exists());
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join("healthy/SKILL.md")).unwrap(),
            "healthy original"
        );
        assert!(
            manager
                .local_inventory()
                .iter()
                .any(|entry| entry.id == "linked" && !entry.managed)
        );
        assert!(
            manager
                .local_inventory()
                .iter()
                .any(|entry| entry.id == "oversized" && !entry.managed)
        );
    }

    #[test]
    #[cfg(windows)]
    fn managed_codex_copy_with_internal_link_still_fails_preparation() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let codex = manager.linked_dir().join("managed");
        let external = temp.path().join("external/nested");
        write_skill(&codex, "managed original");
        write_skill(&external, "external original");
        std::fs::write(codex.join(MANAGED_COPY_MARKER), "").unwrap();
        make_junction(&codex.join("nested"), &external);

        let error = manager.prepare_managed_skills().unwrap_err();

        assert!(format!("{error:#}").contains("符号链接"));
        assert!(!manager.source_dir.join("managed").exists());
        assert!(codex.join("nested/SKILL.md").is_file());
    }

    #[test]
    fn corrupted_recovery_journal_is_not_skipped_during_adoption() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("custom");
        write_skill(&local, "original");
        let journal_dir = temp.path().join(".skill-transactions");
        std::fs::create_dir_all(&journal_dir).unwrap();
        let journal = journal_dir.join(format!("{}.json", uuid::Uuid::new_v4()));
        std::fs::write(&journal, "{broken").unwrap();

        assert!(manager.prepare_managed_skills().is_err());
        assert_eq!(std::fs::read_to_string(&journal).unwrap(), "{broken");
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "original"
        );
        assert!(!manager.source_dir.join("custom").exists());
    }

    #[test]
    fn unavailable_library_is_not_skipped_during_unmanaged_adoption() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("custom");
        write_skill(&local, "original");
        std::fs::write(&manager.source_dir, "not a directory").unwrap();

        assert!(manager.prepare_managed_skills().is_err());
        assert_eq!(
            std::fs::read_to_string(&manager.source_dir).unwrap(),
            "not a directory"
        );
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "original"
        );
        assert!(!manager.load_state().installed.contains_key("custom"));
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
        assert!(
            manager
                .local_inventory()
                .iter()
                .any(|skill| skill.id == "custom" && skill.installed)
        );
        manager.uninstall("custom").unwrap();
        let backups = manager.list_backups();
        assert!(!backups.is_empty());
        assert!(!local.exists());
        manager.restore_backup(&backups[0].id).unwrap();
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "user original"
        );
    }

    #[test]
    fn uninstall_preserves_distinct_source_and_local_copies() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        for (directory, content) in [
            (manager.source_dir.join("custom"), "source"),
            (manager.linked_dir().join("custom"), "edited local"),
        ] {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), content).unwrap();
        }
        manager.uninstall("custom").unwrap();
        let contents: std::collections::BTreeSet<_> = manager
            .list_backups()
            .iter()
            .map(|backup| {
                std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md"))
                    .unwrap()
            })
            .collect();
        assert_eq!(
            contents,
            ["source".to_string(), "edited local".to_string()]
                .into_iter()
                .collect()
        );
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
        assert_eq!(
            std::fs::read_to_string(local.join("SKILL.md")).unwrap(),
            "original"
        );
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
            .unwrap()
            .file_type()
            .is_symlink();

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
        assert!(
            manager
                .source_dir()
                .join("my-own-skill")
                .join(SKILL_MANIFEST_FILE)
                .exists()
        );
        assert!(
            !manager
                .merge_entries(&[])
                .iter()
                .find(|entry| entry.id == "my-own-skill")
                .unwrap()
                .enabled
        );

        manager.set_enabled("my-own-skill", true).unwrap();
        assert!(manager.linked_dir().join("my-own-skill").exists());
    }

    /// 没有 SKILL.md 的目录、以及 `.system` 自带目录，都不应被当成「本地自建」重复收录。
    #[test]
    fn local_scan_ignores_manifest_less_and_bundled_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        std::fs::create_dir_all(manager.linked_dir().join("no-manifest")).unwrap();
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

        assert!(entries.iter().all(|entry| entry.id != "no-manifest"));
        let imagegen: Vec<_> = entries
            .iter()
            .filter(|entry| entry.id == "imagegen")
            .collect();
        assert_eq!(imagegen.len(), 1, "自带 skill 不应被本地扫描重复收录");
        assert!(imagegen[0].bundled);
    }

    #[test]
    fn local_inventory_lists_custom_and_bundled_skills_without_writing_config() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let custom = manager.linked_dir().join("custom-skill");
        let bundled = manager
            .linked_dir()
            .join(BUNDLED_SKILLS_DIR)
            .join("builtin-skill");
        for path in [&custom, &bundled] {
            std::fs::create_dir_all(path).unwrap();
            std::fs::write(
                path.join(SKILL_MANIFEST_FILE),
                "---\nname: test\ndescription: local\n---\n",
            )
            .unwrap();
        }
        let config = manager.codex_home.join("config.toml");
        std::fs::write(&config, "[skills]\nbundled = true\n").unwrap();
        let before = std::fs::read(&config).unwrap();
        let entries = manager.local_inventory();
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "custom-skill" && entry.enabled)
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "builtin-skill" && entry.bundled)
        );
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

    fn fallback_repo_fixture<'a>(
        repo: &'a SkillRepo,
        _cached: &'a BTreeMap<String, RemoteSkill>,
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<Vec<RemoteSkill>>> {
        Box::pin(async move {
            if repo.branch == "main" || repo.name == "missing" {
                anyhow::bail!("GitHub HTTP 404");
            }
            Ok(vec![RemoteSkill {
                repo_key: repo.key(),
                ..sample_skill("alpha", "skills/alpha", "hash")
            }])
        })
    }

    #[tokio::test]
    async fn validated_repo_add_rejects_existing_disabled_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut existing = parse_repo_key("Example/Kit@master").unwrap();
        existing.enabled = false;
        manager.upsert_repo(existing.clone()).unwrap();
        let before = std::fs::read(&manager.state_path).unwrap();
        let requested = parse_repo_key("example/kit@main").unwrap();
        let cached = BTreeMap::new();

        let error = manager
            .add_repo_after_validation(resolve_repo_skills_with(
                requested,
                &cached,
                fallback_repo_fixture,
            ))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("仓库已存在"));
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(manager.list_repos().contains(&existing));
        assert!(
            !manager.list_repos().iter().any(|repo| {
                repo.owner.eq_ignore_ascii_case("example") && repo.branch == "main"
            })
        );
    }

    #[tokio::test]
    async fn validated_repo_add_rejects_invalid_input_without_saving() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let existing = parse_repo_key("example/kit@main").unwrap();
        manager.upsert_repo(existing).unwrap();
        let before = std::fs::read(&manager.state_path).unwrap();
        let mut invalid = parse_repo_key("example/other@main").unwrap();
        invalid.owner = "../invalid".into();
        let error = manager
            .add_validated_repo(invalid, &BTreeMap::new())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("仓库验证失败"));
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
    }

    #[tokio::test]
    async fn validated_repo_add_persists_resolved_branch_only_after_success() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let requested = parse_repo_key("example/kit@main").unwrap();
        let cached = BTreeMap::new();
        let (resolved, skills) = manager
            .add_repo_after_validation(resolve_repo_skills_with(
                requested,
                &cached,
                fallback_repo_fixture,
            ))
            .await
            .unwrap();
        assert_eq!(resolved.branch, "master");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].repo_key, resolved.key());
        assert!(manager.list_repos().contains(&resolved));
        assert!(!manager.list_repos().iter().any(|repo| {
            repo.owner == "example" && repo.name == "kit" && repo.branch == "main"
        }));

        let before = std::fs::read(&manager.state_path).unwrap();
        let missing = parse_repo_key("example/missing@main").unwrap();
        let error = manager
            .add_repo_after_validation(resolve_repo_skills_with(
                missing,
                &cached,
                fallback_repo_fixture,
            ))
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("仓库验证失败"));
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
    }

    #[tokio::test]
    async fn validated_repo_add_rechecks_after_concurrent_validation() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let worker = manager.clone();
        let candidate = parse_repo_key("example/kit@master").unwrap();
        let mut existing = candidate.clone();
        existing.enabled = false;
        let (started, waiting) = tokio::sync::oneshot::channel();
        let (resume, wait_resume) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            worker
                .add_repo_after_validation(async move {
                    started.send(()).unwrap();
                    wait_resume.await.unwrap();
                    Ok((candidate, Vec::new()))
                })
                .await
        });
        waiting.await.unwrap();
        manager.add_repo_if_absent(existing.clone()).unwrap();
        let before = std::fs::read(&manager.state_path).unwrap();
        resume.send(()).unwrap();
        assert!(
            task.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("仓库已存在")
        );
        assert_eq!(std::fs::read(&manager.state_path).unwrap(), before);
        assert!(manager.list_repos().contains(&existing));
    }

    #[test]
    fn validated_repo_add_deduplicates_concurrent_managers() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        assert!(Arc::ptr_eq(&first.operation_lock, &second.operation_lock));
        let barrier = Arc::new(std::sync::Barrier::new(12));
        let workers: Vec<_> = (0..12)
            .map(|index| {
                let manager = if index % 2 == 0 {
                    first.clone()
                } else {
                    second.clone()
                };
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let mut repo = parse_repo_key(if index % 2 == 0 {
                        "Example/Kit@master:skills"
                    } else {
                        "example/kit@master:/skills/"
                    })
                    .unwrap();
                    repo.enabled = index % 2 == 0;
                    barrier.wait();
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .build()
                        .unwrap();
                    runtime.block_on(
                        manager.add_repo_after_validation(async move { Ok((repo, Vec::new())) }),
                    )
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        let successes: Vec<_> = results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .collect();
        assert_eq!(successes.len(), 1);
        assert!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .all(|error| error.to_string().contains("仓库已存在"))
        );
        let added: Vec<_> = first
            .list_repos()
            .into_iter()
            .filter(|repo| {
                repo.owner.eq_ignore_ascii_case("example") && repo.name.eq_ignore_ascii_case("kit")
            })
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].enabled, successes[0].0.enabled);
        assert_eq!(added[0].subdir, "skills");
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
    fn separate_managers_share_a_state_lock_during_parallel_repository_updates() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        assert!(Arc::ptr_eq(&first.state_lock, &second.state_lock));
        let workers: Vec<_> = (0..12)
            .map(|index| {
                let manager = if index % 2 == 0 {
                    first.clone()
                } else {
                    second.clone()
                };
                std::thread::spawn(move || {
                    manager
                        .upsert_repo(SkillRepo {
                            owner: "example".into(),
                            name: format!("repo-{index}"),
                            branch: "main".into(),
                            subdir: String::new(),
                            enabled: true,
                        })
                        .unwrap()
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        let repos = first.list_repos();
        for index in 0..12 {
            assert!(
                repos
                    .iter()
                    .any(|repo| repo.name == format!("repo-{index}"))
            );
        }
    }

    #[test]
    fn background_branch_fallback_does_not_override_changed_or_installed_sources() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let original = parse_repo_key("example/kit@main").unwrap();
        manager.upsert_repo(original.clone()).unwrap();
        let replacement = parse_repo_key("example/kit@master").unwrap();
        let mut disabled = original.clone();
        disabled.enabled = false;
        manager.upsert_repo(disabled.clone()).unwrap();
        assert!(
            !manager
                .replace_repo_if_unchanged(&original, replacement.clone())
                .unwrap()
        );
        manager.upsert_repo(original.clone()).unwrap();
        manager
            .install_from_zip(
                &RemoteSkill {
                    repo_key: original.key(),
                    ..sample_skill("alpha", "alpha", "hash")
                },
                &repo_zip(&[("kit-main/alpha/SKILL.md", "content")]),
            )
            .unwrap();
        assert!(
            manager
                .replace_repo_if_unchanged(&original, replacement)
                .is_err()
        );
        assert!(manager.list_repos().iter().any(|repo| repo == &original));
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
    fn hidden_and_builtin_directories_cannot_be_managed_through_commands() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let builtin = manager.linked_dir().join(".system");
        write_skill(&builtin, "builtin manifest");
        write_skill(&builtin.join("imagegen"), "builtin content");
        let external = temp.path().join("external");
        write_skill(&external, "incoming");
        for id in [
            ".system",
            ".SYSTEM",
            ".codex-plus-hidden",
            ".staging-work",
            ".",
            "..",
            "bad\nname",
        ] {
            assert!(manager.set_enabled(id, false).is_err(), "{id}");
            assert!(manager.set_enabled_for("claude", id, true).is_err(), "{id}");
            assert!(manager.uninstall(id).is_err(), "{id}");
            assert!(
                manager.import_from_local_directory(id, &external).is_err(),
                "{id}"
            );
            assert!(
                manager
                    .install_from_zip(
                        &sample_skill(id, "alpha", "hash"),
                        &repo_zip(&[("kit-main/alpha/SKILL.md", "incoming")])
                    )
                    .is_err(),
                "{id}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(builtin.join("imagegen/SKILL.md")).unwrap(),
            "builtin content"
        );
        assert!(manager.list_backups().is_empty());
        assert!(validate_skill_id(".custom-skill").is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn windows_reserved_or_aliased_skill_names_are_rejected() {
        for id in [
            ".system ",
            ".system.",
            "alpha:stream",
            "CON",
            "nul.txt",
            "COM1",
            "LPT9.log",
            "alpha.",
            "alpha ",
            "bad?name",
            "C:relative",
        ] {
            assert!(validate_skill_id(id).is_err(), "{id}");
        }
        for id in ["regular-skill", "CONSOLE", "COM10", "研究技能"] {
            assert!(validate_skill_id(id).is_ok(), "{id}");
        }
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
