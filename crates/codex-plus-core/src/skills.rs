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

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

mod archive;

/// 在锁外检查过的 Agent 副本；提交只核验路径身份与代次，不再重读内容。
struct AgentCopiesSnapshot {
    scope: CoordinationScope,
    generation: u64,
    paths: Vec<(PathBuf, SkillPathIdentity)>,
    source_identity: Option<SkillPathIdentity>,
}

const SKILL_MANIFEST_FILE: &str = "SKILL.md";
const BUNDLED_SKILLS_DIR: &str = ".system";
/// 本地清单只需要 frontmatter，避免将整个 Skill 正文读入内存。
const SKILL_FRONTMATTER_LIMIT_BYTES: usize = 256 * 1024;
/// 从本地目录导入的 Skill 的来源标记（#511），前端按它显示「本地导入」分组。
pub const LOCAL_IMPORT_REPO_KEY: &str = "local";
/// 单个仓库 zip 的下载上限。skill 仓库都是纯文本，64 MiB 足够宽松了。
const REPO_ZIP_DOWNLOAD_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const REPO_ZIP_UNPACKED_LIMIT_BYTES: u64 = 256 * 1024 * 1024;
const REPO_ZIP_HEADER_TIMEOUT: Duration = Duration::from_secs(30);
const REPO_ZIP_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const REPO_ZIP_BODY_TIMEOUT: Duration = Duration::from_secs(15 * 60);

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

/// 下载前的状态快照。公开字段供调用方定位仓库和原安装路径，代次只在本进程有效。
#[derive(Debug, Clone)]
pub struct SkillDownloadGuard {
    pub installed: Option<InstalledSkill>,
    pub repo: SkillRepo,
    id: String,
    install_only: bool,
    state_path: PathBuf,
    source_dir: PathBuf,
    backups_dir: PathBuf,
    codex_home: PathBuf,
    agent_dirs: Vec<(String, PathBuf)>,
    installed_snapshot: Option<InstalledSkill>,
    repo_snapshot: SkillRepo,
    contents: SkillContentSnapshot,
    coordination_scope: CoordinationScope,
    skill_generation: u64,
    repo_generation: u64,
}

#[derive(Debug, Clone, Default)]
struct SkillContentSnapshot {
    source: Option<[u8; 32]>,
    codex_copy: Option<[u8; 32]>,
}

/// 刷新仓库前捕获的快照；仓库启停也会使它失效。
#[derive(Debug, Clone)]
pub struct SkillRepoGuard {
    pub repo: SkillRepo,
    state_path: PathBuf,
    repo_snapshot: SkillRepo,
    coordination_scope: CoordinationScope,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CoordinationPath {
    normalized: PathBuf,
    ancestors: Vec<(SkillPathIdentity, PathBuf)>,
}

type CoordinationScope = Option<Vec<CoordinationPath>>;

#[derive(Default)]
struct SkillsCoordination {
    generation: u64,
    skills: BTreeMap<(CoordinationScope, String), u64>,
    nested_skills: BTreeMap<(CoordinationScope, String), u64>,
    repos: BTreeMap<(CoordinationScope, String), u64>,
    repo_changes: BTreeMap<(CoordinationScope, String), u64>,
}

impl SkillsCoordination {
    fn skill_generation(&self, scope: &CoordinationScope, id: &str) -> u64 {
        let changed = self.skills.iter()
            .filter(|((other_scope, other), _)| {
                if skill_names_may_alias(other, id) {
                    coordination_scopes_overlap(scope, other_scope)
                } else {
                    coordination_scopes_nested(scope, other_scope)
                }
            })
            .map(|(_, generation)| *generation)
            .max()
            .unwrap_or_default();
        self.nested_skills.iter()
            .filter(|((other_scope, other), _)| {
                coordination_scopes_nested(scope, other_scope)
                    || (!coordination_scopes_same_paths(scope, other_scope)
                        && skill_names_may_alias(other, id)
                        && coordination_scopes_overlap(scope, other_scope))
            })
            .map(|(_, generation)| *generation)
            .max()
            .unwrap_or_default()
            .max(changed)
    }

    // 提交前的短时副本快照需要捕获启停变化，下载快照则允许启停并采用最新状态。
    fn mutation_generation(&self, scope: &CoordinationScope, id: &str) -> u64 {
        self.skill_generation(scope, id).max(self.nested_skills.iter()
            .filter(|((other_scope, other), _)| skill_names_may_alias(other, id)
                && coordination_scopes_overlap(scope, other_scope))
            .map(|(_, generation)| *generation).max().unwrap_or_default())
    }

    fn repo_generation(&self, scope: &CoordinationScope, key: &str) -> u64 {
        Self::scoped_repo_generation(&self.repos, scope, key)
    }

    fn repo_change_generation(&self, scope: &CoordinationScope, key: &str) -> u64 {
        Self::scoped_repo_generation(&self.repo_changes, scope, key)
    }

    fn scoped_repo_generation(
        entries: &BTreeMap<(CoordinationScope, String), u64>,
        scope: &CoordinationScope,
        key: &str,
    ) -> u64 {
        entries.iter()
            .filter(|((other_scope, other_key), _)| {
                other_key == key && coordination_scopes_overlap(scope, other_scope)
            })
            .map(|(_, generation)| *generation)
            .max()
            .unwrap_or_default()
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.checked_add(1).expect("Skill 代次耗尽");
        self.generation
    }

    fn changed_skill(&mut self, scope: CoordinationScope, id: &str) {
        let generation = self.next_generation();
        Self::record_generation(&mut self.skills, scope, id, generation);
    }

    fn changed_nested_skill(&mut self, scope: CoordinationScope, id: &str) {
        let generation = self.next_generation();
        Self::record_generation(&mut self.nested_skills, scope, id, generation);
    }

    fn changed_repo(&mut self, scope: CoordinationScope, key: &str) {
        let generation = self.next_generation();
        Self::record_generation(&mut self.repos, scope.clone(), key, generation);
        Self::record_generation(&mut self.repo_changes, scope, key, generation);
    }

    fn changed_repo_settings(&mut self, scope: CoordinationScope, key: &str) {
        let generation = self.next_generation();
        Self::record_generation(&mut self.repo_changes, scope, key, generation);
    }

    fn record_generation(
        entries: &mut BTreeMap<(CoordinationScope, String), u64>,
        mut scope: CoordinationScope,
        key: &str,
        generation: u64,
    ) {
        entries.retain(|(other_scope, other), _| {
            if other != key || !coordination_scopes_same_paths(&scope, other_scope) {
                return true;
            }
            // 旧路径身份仍可能被其他挂载路径引用。
            if let (Some(current), Some(previous)) = (&mut scope, other_scope) {
                for (current, previous) in current.iter_mut().zip(previous) {
                    for ancestor in &previous.ancestors {
                        if !current.ancestors.contains(ancestor) {
                            current.ancestors.push(ancestor.clone());
                        }
                    }
                }
            }
            false
        });
        entries.insert((scope, key.to_string()), generation);
    }
}

// 锁覆盖整个进程；代次只隔离能确认不相交的目录，未知路径仍按全局冲突处理。
// 下载、ZIP 校验及提取不持锁；不协调其它进程或绕过 SkillsManager 的直接写入。
fn skills_coordination() -> MutexGuard<'static, SkillsCoordination> {
    static COORDINATION: OnceLock<Mutex<SkillsCoordination>> = OnceLock::new();
    COORDINATION.get_or_init(Mutex::default).lock()
        .expect("Skill 协调锁中毒，拒绝继续修改")
}

#[cfg(test)]
thread_local! {
    static COORDINATION_MISSING_PATH_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&Path)>>> =
        std::cell::RefCell::new(None);
}

fn coordination_path(path: &Path) -> Option<CoordinationPath> {
    if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return None;
    }
    // 暂存过程可能在解析期间创建目录；多次仍无法确认时保持保守。
    (0..3).find_map(|_| coordination_path_once(path))
}

fn coordination_path_once(path: &Path) -> Option<CoordinationPath> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                let mut ancestors = Vec::new();
                let mut current = resolved.as_path();
                loop {
                    match skill_path_identity(current) {
                        Ok(identity) => ancestors.push((identity, resolved.strip_prefix(current).ok()?.to_path_buf())),
                        Err(_) => match std::fs::symlink_metadata(current) {
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            _ => return None,
                        },
                    }
                    match current.parent() {
                        Some(parent) if parent != current => current = parent,
                        _ => break,
                    }
                }
                return Some(CoordinationPath { normalized: resolved, ancestors });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(test)]
                COORDINATION_MISSING_PATH_HOOK.with(|hook| {
                    if let Some(hook) = hook.borrow_mut().take() {
                        hook(ancestor);
                    }
                });
                match std::fs::symlink_metadata(ancestor) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    _ => return None,
                }
                missing.push(ancestor.file_name()?.to_os_string());
                ancestor = ancestor.parent()?;
            }
            Err(_) => return None,
        }
    }
}

fn coordination_path_starts_with(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let mut parts = path.components();
        root.components().all(|root_part| parts.next().is_some_and(|part| {
            windows_names_equal(
                &part.as_os_str().to_string_lossy(),
                &root_part.as_os_str().to_string_lossy(),
            )
        }))
    }
    #[cfg(target_os = "macos")]
    {
        let mut parts = path.components();
        root.components().all(|root_part| parts.next().is_some_and(|part| {
            match (part.as_os_str().to_str(), root_part.as_os_str().to_str()) {
                (Some(part), Some(root_part)) => macos_names_may_alias(part, root_part),
                _ => true,
            }
        }))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        path.starts_with(root)
    }
}

fn coordination_scopes_overlap(left: &CoordinationScope, right: &CoordinationScope) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.iter().any(|left| right.iter().any(|right| {
            coordination_path_starts_with(&left.normalized, &right.normalized)
                || coordination_path_starts_with(&right.normalized, &left.normalized)
                || left.ancestors.iter().any(|(left_id, left_suffix)| {
                    right.ancestors.iter().any(|(right_id, right_suffix)| {
                        left_id == right_id && (
                            coordination_path_starts_with(left_suffix, right_suffix)
                                || coordination_path_starts_with(right_suffix, left_suffix)
                        )
                    })
                })
        })),
        _ => true,
    }
}

fn coordination_scopes_nested(left: &CoordinationScope, right: &CoordinationScope) -> bool {
    fn strictly_nested(left: &Path, right: &Path) -> bool {
        left.components().count() > right.components().count()
            && coordination_path_starts_with(left, right)
    }
    match (left, right) {
        (Some(left), Some(right)) => left.iter().any(|left| right.iter().any(|right| {
            strictly_nested(&left.normalized, &right.normalized)
                || strictly_nested(&right.normalized, &left.normalized)
                || left.ancestors.iter().any(|(left_id, left_suffix)| {
                    right.ancestors.iter().any(|(right_id, right_suffix)| {
                        left_id == right_id && (
                            strictly_nested(left_suffix, right_suffix) || strictly_nested(right_suffix, left_suffix)
                        )
                    })
                })
        })),
        _ => true,
    }
}

fn coordination_scopes_same_paths(left: &CoordinationScope, right: &CoordinationScope) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.len() == right.len()
            && left.iter().zip(right).all(|(left, right)| left.normalized == right.normalized),
        (None, None) => true,
        _ => false,
    }
}

fn coordination_scope_is_current(captured: &CoordinationScope, current: &CoordinationScope) -> bool {
    match (captured, current) {
        (None, _) => true,
        (Some(_), Some(_)) => coordination_scopes_same_paths(captured, current),
        _ => false,
    }
}

#[cfg(windows)]
fn skill_names_may_alias(left: &str, right: &str) -> bool {
    windows_names_equal(left, right)
}

#[cfg(windows)]
fn windows_names_equal(left: &str, right: &str) -> bool {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CompareStringOrdinal(
            left: *const u16, left_len: i32, right: *const u16, right_len: i32, ignore_case: i32,
        ) -> i32;
    }
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    let (Ok(left_len), Ok(right_len)) = (i32::try_from(left.len()), i32::try_from(right.len())) else {
        return true;
    };
    // 返回 0 表示比较失败，此时也按可能冲突处理。
    let result = unsafe { CompareStringOrdinal(left.as_ptr(), left_len, right.as_ptr(), right_len, 1) };
    result == 0 || result == 2
}

#[cfg(target_os = "macos")]
fn macos_names_may_alias(left: &str, right: &str) -> bool {
    use objc2_foundation::{NSComparisonResult, NSString};
    let left = NSString::from_str(left).decomposedStringWithCanonicalMapping();
    let right = NSString::from_str(right).decomposedStringWithCanonicalMapping();
    left.caseInsensitiveCompare(&right) == NSComparisonResult::Same
}

#[cfg(target_os = "macos")]
fn skill_names_may_alias(left: &str, right: &str) -> bool {
    macos_names_may_alias(left, right)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn skill_names_may_alias(left: &str, right: &str) -> bool {
    left == right
}

#[derive(Debug, Clone)]
pub struct SkillsManager {
    source_dir: PathBuf,
    backups_dir: PathBuf,
    state_path: PathBuf,
    codex_home: PathBuf,
    /// Codex 之外也读 `<dir>/<id>/SKILL.md` 的 Agent：(Agent id, skills 目录)。
    agent_dirs: Vec<(String, PathBuf)>,
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
        }
    }

    /// 让 Skill 也能启用到其它 Agent（例如 Claude Code 的 `~/.claude/skills`、Grok 的 `~/.grok/skills`）。
    pub fn with_agent_dirs(mut self, dirs: Vec<(String, PathBuf)>) -> Self {
        self.agent_dirs = dirs;
        self
    }

    /// 路径不完整时返回全局范围；任意受管路径相交都不能互相隔离。
    fn coordination_scope(&self) -> CoordinationScope {
        let paths = [
            self.source_dir.clone(),
            self.backups_dir.clone(),
            self.state_path.clone(),
            self.linked_dir(),
        ];
        let mut scope: Vec<_> = paths.iter().chain(self.agent_dirs.iter().map(|(_, dir)| dir))
            .map(|path| coordination_path(path))
            .collect::<Option<_>>()?;
        // 状态文件通过原子替换写入；父目录身份和文件名足以识别共享路径。
        scope[2].ancestors.retain(|(_, suffix)| !suffix.as_os_str().is_empty());
        Some(scope)
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
        let mut coordination = skills_coordination();
        self.ensure_skill_name_not_aliased(id, &self.load_state_checked_unlocked()?)?;
        let result = (|| {
            if enabled {
                self.ensure_managed_source_unlocked(id, &mut coordination)?;
            }
            if agent == "codex" {
                return self.set_enabled_unlocked(id, enabled, &mut coordination);
            }
            let dir = self.agent_dir(agent)?;
            let link = dir.join(id);
            let metadata = std::fs::symlink_metadata(&link);
            if !enabled {
                return match metadata {
                    Ok(metadata) if metadata.file_type().is_symlink() => remove_link(&link),
                    Ok(_) if is_managed_copy(&link) => {
                        self.backup_directory(id, &link)?;
                        Ok(())
                    }
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
            link_or_copy(&source, &link).map(|_| ())
        })();
        coordination.changed_nested_skill(self.coordination_scope(), id);
        result
    }

    /// 没有受控源时，从 Codex 或其它 Agent 目录里找到这个 Skill 的真实目录，纳入受控源。
    ///
    /// - 在 Codex 目录里：走一遍「停用 → 启用」，停用会把真实目录复制成受控源（原目录进备份），
    ///   启用再软链回来，Codex 照常可用；
    /// - 只在其它 Agent 目录里：复制一份成受控源，那个 Agent 自己的目录保持不动。
    fn ensure_managed_source_unlocked(
        &self,
        id: &str,
        coordination: &mut SkillsCoordination,
    ) -> anyhow::Result<()> {
        if self.source_dir.join(id).is_dir() {
            return Ok(());
        }
        let physical = |path: &Path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir()) && path.join(SKILL_MANIFEST_FILE).is_file()
        };
        let codex = self.linked_dir().join(id);
        if physical(&codex) {
            self.set_enabled_unlocked(id, false, coordination)?;
            return self.set_enabled_unlocked(id, true, coordination);
        }
        if std::fs::symlink_metadata(&codex).is_ok() {
            // Codex 目录里同名的是别的东西（例如断掉的软链），导入会把它挪进备份；交给用户处理。
            anyhow::bail!("Codex 的 skills 目录里已有同名内容，未导入：{}", codex.display());
        }
        for (_, dir) in &self.agent_dirs {
            let candidate = dir.join(id);
            if physical(&candidate) {
                let (staging, installed) = self.stage_directory_import(id, &candidate, "cc-switch")?;
                let result = self.commit_staged_skill_unlocked(&staging, installed, false, true, None, false, coordination);
                result.map_err(|error| retain_staging_context(error, &staging))?;
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
        let _guard = skills_coordination();
        self.load_state_unlocked()
    }

    fn load_state_unlocked(&self) -> SkillsState {
        let Ok(text) = std::fs::read_to_string(&self.state_path) else {
            return SkillsState::default();
        };
        serde_json::from_str::<SkillsState>(&text).unwrap_or_default()
    }

    fn load_state_checked_unlocked(&self) -> anyhow::Result<SkillsState> {
        let text = match std::fs::read_to_string(&self.state_path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(SkillsState::default()),
            Err(error) => return Err(error).context("读取 Skill 状态失败"),
        };
        serde_json::from_str::<SkillsState>(&text).context("Skill 状态无法解析")
    }

    fn save_state_unlocked(&self, state: &SkillsState) -> anyhow::Result<()> {
        crate::settings::atomic_write(
            &self.state_path,
            serde_json::to_string_pretty(state)?.as_bytes(),
        )
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn ensure_skill_name_not_aliased(&self, id: &str, state: &SkillsState) -> anyhow::Result<()> {
        for existing in state.installed.keys() {
            if existing != id && skill_names_may_alias(existing, id) {
                anyhow::bail!("Skill 名称可能与现有安装记录冲突：{existing}");
            }
        }
        let mut roots = vec![self.source_dir.clone(), self.linked_dir()];
        roots.extend(self.agent_dirs.iter().map(|(_, dir)| dir.clone()));
        for root in roots {
            let entries = match std::fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", root.display())),
            };
            let mut exact = false;
            for entry in entries {
                let name = entry?.file_name().into_string()
                    .map_err(|_| anyhow::anyhow!("无法确认 {} 下的 Skill 名称", root.display()))?;
                if name == id {
                    exact = true;
                } else if skill_names_may_alias(&name, id) {
                    anyhow::bail!("Skill 名称与现有目录冲突：{name}");
                }
            }
            // 还要检测 read_dir 不会单独列出的文件系统别名。
            if !exact && std::fs::symlink_metadata(root.join(id)).is_ok() {
                anyhow::bail!("Skill 名称是现有目录的别名：{id}");
            }
        }
        Ok(())
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn ensure_skill_name_not_aliased(&self, _id: &str, _state: &SkillsState) -> anyhow::Result<()> {
        Ok(())
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
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        let Some(existing) = state.repos.iter_mut().find(|item| item.key() == key) else { return Ok(false) };
        if existing.enabled == repo.enabled { return Ok(false); }
        existing.enabled = repo.enabled;
        self.save_state_unlocked(&state)?;
        coordination.changed_repo_settings(self.coordination_scope(), &key);
        Ok(true)
    }

    pub fn save_repo(&self, repo: SkillRepo, previous_key: Option<&str>) -> anyhow::Result<Vec<SkillRepo>> {
        let repo = normalize_repo(repo)?;
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        let key = repo.key();
        let existed = state.repos.iter().any(|item| item.key() == key);
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
        if let Some(previous_key) = previous_key.filter(|previous| *previous != key) {
            coordination.changed_repo(self.coordination_scope(), previous_key);
        }
        if !existed {
            coordination.changed_repo(self.coordination_scope(), &key);
        } else {
            coordination.changed_repo_settings(self.coordination_scope(), &key);
        }
        Ok(state.repos)
    }

    pub fn delete_repo(&self, key: &str) -> anyhow::Result<Vec<SkillRepo>> {
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        let existed = state.repos.iter().any(|repo| repo.key() == key);
        state.repos.retain(|repo| repo.key() != key);
        self.save_state_unlocked(&state)?;
        if existed {
            coordination.changed_repo(self.coordination_scope(), key);
        }
        Ok(state.repos)
    }

    /// 仓库刷新与后续回退必须使用同一份快照，不能只比较 key。
    pub fn capture_repo_guard(&self, key: &str) -> anyhow::Result<SkillRepoGuard> {
        let coordination = skills_coordination();
        let repo = self.load_state_checked_unlocked()?.repos.into_iter()
            .find(|repo| repo.key() == key)
            .context("仓库已不存在，请刷新后重试")?;
        let scope = self.coordination_scope();
        Ok(SkillRepoGuard {
            repo: repo.clone(),
            state_path: self.state_path.clone(),
            repo_snapshot: repo,
            generation: coordination.repo_change_generation(&scope, key),
            coordination_scope: scope,
        })
    }

    /// 只读检查刷新快照；锁只覆盖本地状态读取，不包含网络等待。
    pub fn repo_guard_is_current(&self, guard: &SkillRepoGuard) -> anyhow::Result<bool> {
        let coordination = skills_coordination();
        if guard.repo != guard.repo_snapshot || self.state_path != guard.state_path
            || !coordination_scope_is_current(&guard.coordination_scope, &self.coordination_scope())
        {
            return Ok(false);
        }
        let state = self.load_state_checked_unlocked()?;
        Ok(state.repos.iter().any(|repo| repo == &guard.repo_snapshot)
            && coordination.repo_change_generation(&guard.coordination_scope, &guard.repo_snapshot.key())
                == guard.generation)
    }

    /// 仅在仓库自抓取以来完全未变时保存 main/master 回退；返回 false 表示已过期或目标已存在。
    pub fn save_resolved_repo_if_unchanged(
        &self,
        guard: &SkillRepoGuard,
        resolved: SkillRepo,
    ) -> anyhow::Result<bool> {
        let resolved = normalize_repo(resolved)?;
        if guard.repo != guard.repo_snapshot || self.state_path != guard.state_path {
            anyhow::bail!("仓库快照与当前管理器不匹配");
        }
        let original = &guard.repo_snapshot;
        if resolved.owner != original.owner || resolved.name != original.name
            || resolved.subdir != original.subdir || resolved.enabled != original.enabled
            || !matches!(
                (original.branch.as_str(), resolved.branch.as_str()),
                ("main", "master") | ("master", "main")
            )
        {
            anyhow::bail!("仓库回退只能切换 main 与 master");
        }
        let old_key = original.key();
        let new_key = resolved.key();
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        if !coordination_scope_is_current(&guard.coordination_scope, &self.coordination_scope())
            || coordination.repo_change_generation(&guard.coordination_scope, &old_key) != guard.generation
            || !state.repos.iter().any(|repo| repo == original)
            || state.repos.iter().any(|repo| repo.key() == new_key)
        {
            return Ok(false);
        }
        let repo = state.repos.iter_mut().find(|repo| repo.key() == old_key).unwrap();
        *repo = resolved;
        self.save_state_unlocked(&state)?;
        coordination.changed_repo(self.coordination_scope(), &old_key);
        coordination.changed_repo(self.coordination_scope(), &new_key);
        Ok(true)
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

    /// 从外部 Skill 目录导入到 Codex++ 的受控源目录，并默认停用。来源记为 cc-switch。
    pub fn import_from_directory(
        &self,
        id: &str,
        external_dir: &Path,
    ) -> anyhow::Result<SkillsState> {
        self.import_directory_with_source(id, external_dir, "cc-switch")
    }

    /// 用户在管理器里选择的本地目录导入（#511）。与 cc-switch 导入同一套流程，来源记为本地导入。
    pub fn import_from_local_directory(
        &self,
        id: &str,
        external_dir: &Path,
    ) -> anyhow::Result<SkillsState> {
        self.import_directory_with_source(id, external_dir, LOCAL_IMPORT_REPO_KEY)
    }

    fn import_directory_with_source(
        &self,
        id: &str,
        external_dir: &Path,
        repo_key: &str,
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        let generation = {
            let coordination = skills_coordination();
            self.ensure_skill_name_not_aliased(id, &self.load_state_checked_unlocked()?)?;
            let scope = self.coordination_scope();
            let generation = coordination.skill_generation(&scope, id);
            (scope, generation)
        };
        let (staging, installed) = self.stage_directory_import(id, external_dir, repo_key)?;
        let result = self.commit_staged_import_if_unchanged(&staging, installed, generation);
        result.map_err(|error| retain_staging_context(error, &staging))
    }

    fn commit_staged_import_if_unchanged(
        &self,
        staging: &Path,
        installed: InstalledSkill,
        generation: (CoordinationScope, u64),
    ) -> anyhow::Result<SkillsState> {
        let mut coordination = skills_coordination();
        if !coordination_scope_is_current(&generation.0, &self.coordination_scope())
            || coordination.skill_generation(&generation.0, &installed.id) != generation.1
        {
            anyhow::bail!("导入期间 Skill 已改变，请重新导入");
        }
        self.commit_staged_skill_unlocked(staging, installed, false, true, None, false, &mut coordination)
    }

    fn stage_directory_import(
        &self,
        id: &str,
        external_dir: &Path,
        repo_key: &str,
    ) -> anyhow::Result<(PathBuf, InstalledSkill)> {
        validate_skill_id(id)?;
        if repo_key == LOCAL_IMPORT_REPO_KEY {
            // Codex 的 skills 目录本来就会被扫描显示；从这里「导入」会把原目录挪进备份，反而让它从 Codex 消失。
            if let Ok(external) = external_dir.canonicalize() {
                for (root, label) in [(self.linked_dir(), "Codex 的 skills 目录"), (self.source_dir.clone(), "Codex++ 的 Skills 受控目录")] {
                    if root.canonicalize().is_ok_and(|root| external.starts_with(root)) {
                        anyhow::bail!("{} 已在{label}里，Codex++ 会直接识别，无需导入", external_dir.display());
                    }
                }
            }
        }
        let manifest = external_dir.join(SKILL_MANIFEST_FILE);
        if !manifest.is_file() {
            anyhow::bail!("{} 下没有 SKILL.md，不是有效的 Skill", external_dir.display());
        }
        std::fs::create_dir_all(&self.source_dir)?;
        if self.source_dir.canonicalize()?.starts_with(external_dir.canonicalize()?) {
            anyhow::bail!("导入目录不能包含受控源目录");
        }
        let staging = self.source_dir.join(format!(".import-{id}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&staging)
            .with_context(|| format!("创建 Skill 暂存目录失败：{}", staging.display()))?;
        if let Err(error) = copy_dir_all(external_dir, &staging) {
            return Err(retain_staging_context(error, &staging));
        }
        let (name, description) = read_skill_manifest(&manifest, id);
        Ok((staging, InstalledSkill {
            id: id.to_string(), name, description, repo_key: repo_key.to_string(),
            repo_path: String::new(), content_hash: String::new(),
            installed_at: current_unix_timestamp_string(),
        }))
    }

    fn agent_copy_paths(&self, id: &str) -> anyhow::Result<Vec<(PathBuf, SkillPathIdentity)>> {
        let mut seen = BTreeSet::new();
        // 共享 Codex 目录或受控源的 Agent 由主路径处理，不能重复备份、发布。
        for path in [self.source_dir.join(id), self.linked_dir().join(id)] {
            if let Ok(identity) = skill_path_identity(&path) { seen.insert(identity); }
        }
        let mut copies = Vec::new();
        for (_, dir) in &self.agent_dirs {
            let path = dir.join(id);
            if is_managed_copy(&path) {
                let identity = skill_path_identity(&path)?;
                if seen.insert(identity) { copies.push((path, identity)); }
            }
        }
        Ok(copies)
    }

    fn capture_agent_copies(&self, id: &str) -> anyhow::Result<AgentCopiesSnapshot> {
        let snapshot = {
            let coordination = skills_coordination();
            let scope = self.coordination_scope();
            AgentCopiesSnapshot {
                generation: coordination.mutation_generation(&scope, id),
                scope,
                paths: self.agent_copy_paths(id)?,
                source_identity: skill_path_identity(&self.source_dir.join(id)).ok(),
            }
        };
        // 没有实体副本时不扫描源；有副本也不占用全局操作锁。
        if !snapshot.paths.is_empty() {
            let source_digest = skill_directory_digest(&self.source_dir.join(id))?;
            for (path, identity) in &snapshot.paths {
                if skill_directory_digest(path)? != source_digest || skill_path_identity(path)? != *identity {
                    anyhow::bail!("Agent 的 Skill 副本与受控源内容不一致，请先停用并备份副本，未更新");
                }
            }
        }
        Ok(snapshot)
    }

    /// 目录、链接和安装记录一起提交；失败时恢复两处原内容。
    fn commit_staged_skill_unlocked(
        &self,
        staging: &Path,
        installed: InstalledSkill,
        enabled: bool,
        replace_identity: bool,
        agent_snapshot: Option<&AgentCopiesSnapshot>,
        require_empty: bool,
        coordination: &mut SkillsCoordination,
    ) -> anyhow::Result<SkillsState> {
        let id = installed.id.clone();
        let destination = self.source_dir.join(&id);
        let mut state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(&id, &state)?;
        let identity_changed = replace_identity || state.installed.get(&id) != Some(&installed);
        state.installed.insert(id.clone(), installed);
        let link = self.linked_dir().join(&id);
        let agent_copies = match agent_snapshot {
            Some(snapshot) => {
                if !coordination_scope_is_current(&snapshot.scope, &self.coordination_scope())
                    || coordination.mutation_generation(&snapshot.scope, &id) != snapshot.generation
                    || self.agent_copy_paths(&id)? != snapshot.paths
                    || skill_path_identity(&destination).ok() != snapshot.source_identity
                {
                    anyhow::bail!("检查副本期间 Skill 已改变，请重试");
                }
                snapshot.paths.clone()
            }
            None => Vec::new(),
        };
        if require_empty && !self.agent_copy_paths(&id)?.is_empty() {
            anyhow::bail!("Agent 已有同名托管 Skill 副本，请先处理");
        }
        let mut displaced = Vec::new();
        let mut published = None;
        let mut created_links = Vec::new();
        let result = (|| -> anyhow::Result<()> {
            // 两处原内容都先保留，成功后仍作为可恢复备份，不覆盖同名用户目录。
            if !require_empty {
                for original in [&link, &destination] {
                    if std::fs::symlink_metadata(original).is_ok() {
                        let identity = skill_path_identity(original)?;
                        let backup = self.backup_directory_checked(&id, original, identity)?;
                        displaced.push((original.clone(), backup, identity));
                    }
                }
            }
            for (original, identity) in &agent_copies {
                let backup = self.backup_directory_checked(&id, original, *identity)?;
                displaced.push((original.clone(), backup, *identity));
            }
            let identity = skill_path_identity(staging)?;
            move_path_no_replace(staging, &destination)?;
            published = Some(identity);
            #[cfg(test)]
            run_source_publication_hook(staging)?;
            if skill_path_identity(&destination)? != identity {
                anyhow::bail!("Skill 发布期间源目录发生变化：{}", destination.display());
            }
            if enabled {
                std::fs::create_dir_all(self.linked_dir())?;
                let mut published_link = None;
                let result = link_or_copy_tracked(&destination, &link, &mut published_link);
                if let Some(identity) = published_link {
                    created_links.push((link.clone(), identity));
                }
                result?;
            }
            for (copy, _) in &agent_copies {
                let mut published_copy = None;
                let result = link_or_copy_tracked(&destination, copy, &mut published_copy);
                if let Some(identity) = published_copy {
                    created_links.push((copy.clone(), identity));
                }
                result?;
            }
            self.save_state_unlocked(&state)?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            let mut recoveries = Vec::new();
            for (created, identity) in created_links.iter().rev()
                .map(|(path, identity)| (path, identity))
                .chain(published.iter().map(|identity| (&destination, identity)))
            {
                match recover_published_path(created, *identity) {
                    Ok(Some(path)) => recoveries.push(path),
                    Ok(None) => {}
                    Err(error) => rollback_errors.push(format!("{error:#}")),
                }
            }
            for (original, backup, identity) in displaced.iter().rev() {
                if let Err(error) = restore_skill_backup(backup, original, *identity) {
                    rollback_errors.push(format!("原内容保留在 {}：{error:#}", backup.display()));
                }
            }
            if !rollback_errors.is_empty() || !recoveries.is_empty() {
                let paths = recoveries.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join("、");
                return Err(error.context(format!(
                    "回滚检查：{}；新内容保留位置：{}",
                    rollback_errors.join("；"), paths
                )));
            }
            return Err(error);
        }
        if identity_changed {
            coordination.changed_skill(self.coordination_scope(), &id);
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

    /// 在同一把锁下捕获仓库、安装记录及其进程内代次。
    pub fn capture_download_guard(
        &self,
        id: &str,
        repo_key: &str,
        install_only: bool,
    ) -> anyhow::Result<SkillDownloadGuard> {
        validate_skill_id(id)?;
        let coordination = skills_coordination();
        let state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(id, &state)?;
        let repo = state.repos.iter().find(|repo| repo.key() == repo_key)
            .cloned().context("仓库已不存在，请刷新后重试")?;
        let installed = state.installed.get(id).cloned();
        match (&installed, install_only) {
            (Some(_), true) => anyhow::bail!("Skill 已安装，请使用更新"),
            (None, false) => anyhow::bail!("Skill 尚未安装，不能更新"),
            (Some(record), false) if record.id != id || record.repo_key != repo_key
                || record.repo_path.is_empty() =>
            {
                anyhow::bail!("已安装 Skill 的仓库或来源路径不匹配");
            }
            (None, true) if std::fs::symlink_metadata(self.source_dir.join(id)).is_ok()
                || std::fs::symlink_metadata(self.linked_dir().join(id)).is_ok() =>
            {
                anyhow::bail!("同名 Skill 目录已存在，不能作为新安装覆盖");
            }
            _ => {}
        }
        let scope = self.coordination_scope();
        let mut guard = SkillDownloadGuard {
            installed: installed.clone(),
            repo: repo.clone(),
            id: id.to_string(),
            install_only,
            state_path: self.state_path.clone(),
            source_dir: self.source_dir.clone(),
            backups_dir: self.backups_dir.clone(),
            codex_home: self.codex_home.clone(),
            agent_dirs: self.agent_dirs.clone(),
            installed_snapshot: installed,
            repo_snapshot: repo,
            contents: SkillContentSnapshot::default(),
            skill_generation: coordination.skill_generation(&scope, id),
            repo_generation: coordination.repo_generation(&scope, repo_key),
            coordination_scope: scope,
        };
        drop(coordination);
        guard.contents = self.skill_content_snapshot(id)?;
        let coordination = skills_coordination();
        self.validate_download_guard_unlocked(&guard, &coordination)?;
        Ok(guard)
    }

    fn skill_content_snapshot(&self, id: &str) -> anyhow::Result<SkillContentSnapshot> {
        let source = self.source_dir.join(id);
        let link = self.linked_dir().join(id);
        let source_digest = skill_directory_digest(&source)?;
        let codex_copy = match std::fs::symlink_metadata(&link) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let linked_target = link.canonicalize().context("无法核实 Codex Skill 软链")?;
                let source_target = source.canonicalize().context("无法核实 Skill 受控源")?;
                if linked_target != source_target || source_digest.is_none() {
                    anyhow::bail!("Codex Skill 软链未指向受控源，无法安全更新");
                }
                None
            }
            Ok(_) => skill_directory_digest(&link)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error).context("读取 Codex Skill 副本失败"),
        };
        Ok(SkillContentSnapshot { source: source_digest, codex_copy })
    }

    fn validate_download_guard_unlocked(
        &self,
        guard: &SkillDownloadGuard,
        coordination: &SkillsCoordination,
    ) -> anyhow::Result<SkillsState> {
        if guard.state_path != self.state_path || guard.source_dir != self.source_dir
            || guard.backups_dir != self.backups_dir || guard.codex_home != self.codex_home
            || guard.agent_dirs != self.agent_dirs || guard.installed != guard.installed_snapshot
            || guard.repo != guard.repo_snapshot
        {
            anyhow::bail!("下载快照与当前管理器不匹配");
        }
        let state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(&guard.id, &state)?;
        let key = guard.repo_snapshot.key();
        let repo_exists = state.repos.iter().any(|repo| {
            repo.owner == guard.repo_snapshot.owner
                && repo.name == guard.repo_snapshot.name
                && repo.branch == guard.repo_snapshot.branch
                && repo.subdir == guard.repo_snapshot.subdir
        });
        let current_scope = self.coordination_scope();
        let scope_current = coordination_scope_is_current(&guard.coordination_scope, &current_scope);
        let repo_current = coordination.repo_generation(&guard.coordination_scope, &key) == guard.repo_generation;
        let skill_current = coordination.skill_generation(&guard.coordination_scope, &guard.id) == guard.skill_generation;
        let installed_current = state.installed.get(&guard.id) == guard.installed_snapshot.as_ref();
        if !scope_current || !repo_exists || !repo_current || !skill_current || !installed_current {
            #[cfg(test)]
            eprintln!(
                "Skill guard: scope_current={scope_current}, captured_scope_known={}, current_scope_known={}, repo_exists={repo_exists}, repo_current={repo_current}, skill_current={skill_current}, installed_current={installed_current}",
                guard.coordination_scope.is_some(), current_scope.is_some(),
            );
            anyhow::bail!("下载期间仓库或 Skill 安装记录已改变，请重新下载");
        }
        if guard.install_only
            && (std::fs::symlink_metadata(self.source_dir.join(&guard.id)).is_ok()
                || std::fs::symlink_metadata(self.linked_dir().join(&guard.id)).is_ok())
        {
            anyhow::bail!("下载期间出现同名 Skill，不能覆盖");
        }
        Ok(state)
    }

    /// 解压在协调锁外进行；提交时重新检查 guard 并采用当时的启用状态。
    pub fn install_downloaded_zip_if_unchanged(
        &self,
        skill: &RemoteSkill,
        zip: &[u8],
        guard: &SkillDownloadGuard,
    ) -> anyhow::Result<SkillsState> {
        if skill.id != guard.id || skill.repo_key != guard.repo_snapshot.key()
            || guard.installed_snapshot.as_ref().is_some_and(|item| item.repo_path != skill.repo_path)
        {
            anyhow::bail!("下载内容与 Skill 快照不匹配");
        }
        {
            let coordination = skills_coordination();
            self.validate_download_guard_unlocked(guard, &coordination)?;
        }
        self.install_guarded_zip(zip, guard, Some(skill))?;
        Ok(self.load_state())
    }

    /// 从同一 ZIP 一次提取目标并生成元数据，不解析或覆盖无关仓库清单。
    pub fn install_downloaded_zip(
        &self, zip: &[u8], guard: &SkillDownloadGuard,
    ) -> anyhow::Result<RemoteSkill> {
        self.install_guarded_zip(zip, guard, None)
    }

    fn install_guarded_zip(
        &self, zip: &[u8], guard: &SkillDownloadGuard, expected: Option<&RemoteSkill>,
    ) -> anyhow::Result<RemoteSkill> {
        {
            let coordination = skills_coordination();
            self.validate_download_guard_unlocked(guard, &coordination)?;
        }
        let staging = self.staging_dir(&guard.id);
        std::fs::create_dir_all(&self.source_dir)?;
        std::fs::create_dir(&staging)
            .with_context(|| format!("创建 Skill 暂存目录失败：{}", staging.display()))?;
        let result = (|| {
            let skill = archive::stage_downloaded_zip(
                &guard.repo_snapshot, zip, &guard.id,
                guard.installed_snapshot.as_ref().map(|item| item.repo_path.as_str()), &staging,
            )?;
            if expected.is_some_and(|expected| expected.repo_path != skill.repo_path
                || expected.content_hash != skill.content_hash) {
                anyhow::bail!("Skill 路径或版本与下载的 ZIP 不匹配");
            }
            // 目录内容扫描不占用操作锁；非受控的跨进程写入仍不能与最后替换原子同步。
            let current = self.skill_content_snapshot(&guard.id)?;
            if current.source != guard.contents.source
                || match (&guard.contents.codex_copy, &current.codex_copy) {
                    (Some(before), Some(after)) => before != after,
                    (None, Some(after)) => Some(*after) != guard.contents.source,
                    _ => false,
                }
            {
                anyhow::bail!("下载期间 Skill 源目录或副本内容已改变，请重新下载");
            }
            let agent_snapshot = self.capture_agent_copies(&guard.id)?;
            let mut coordination = skills_coordination();
            self.validate_download_guard_unlocked(guard, &coordination)?;
            let enabled = guard.install_only || is_linked(&self.linked_dir().join(&guard.id));
            self.commit_staged_skill_unlocked(&staging, InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                repo_path: skill.repo_path.clone(),
                content_hash: skill.content_hash.clone(),
                installed_at: current_unix_timestamp_string(),
            }, enabled, true, Some(&agent_snapshot), guard.install_only, &mut coordination)?;
            // 安装记录保留 legacy ID，返回给目录缓存的必须是可用于新安装的稳定 ID。
            let mut catalog_skill = skill;
            let manifest = skill_manifest_path(&catalog_skill.repo_path);
            catalog_skill.id = parse_skills_from_tree(&guard.repo_snapshot,
                &serde_json::json!({ "tree": [{ "type": "blob", "path": manifest }] }))
                .into_iter().next().context("已安装 Skill 的目录身份缺失")?.id;
            Ok(catalog_skill)
        })();
        result.map_err(|error| retain_staging_context(error, &staging))
    }

    /// 把一份已下载的仓库 zip 里的某个 skill 子树装到 SSOT，并建立软链。
    pub fn install_from_zip(
        &self,
        skill: &RemoteSkill,
        zip_bytes: &[u8],
    ) -> anyhow::Result<SkillsState> {
        validate_skill_id(&skill.id)?;
        let staging = self.staging_dir(&skill.id);
        std::fs::create_dir_all(&self.source_dir)?;
        std::fs::create_dir(&staging)
            .with_context(|| format!("创建 Skill 暂存目录失败：{}", staging.display()))?;
        let result = extract_skill_subtree(zip_bytes, &skill.repo_path, &staging);
        result.map_err(|error| retain_staging_context(error, &staging))?;

        let result = (|| {
            let agent_snapshot = self.capture_agent_copies(&skill.id)?;
            let mut coordination = skills_coordination();
            let state = self.load_state_checked_unlocked()?;
            self.ensure_skill_name_not_aliased(&skill.id, &state)?;
            if let Some(installed) = state.installed.get(&skill.id) {
                if installed.repo_key != skill.repo_key || installed.repo_path != skill.repo_path {
                    anyhow::bail!("同名 Skill 的来源路径不同或旧记录缺少路径，不能覆盖；请先备份卸载原项");
                }
            }
            let enable_after_install = !state.installed.contains_key(&skill.id)
                || is_linked(&self.linked_dir().join(&skill.id));
            self.commit_staged_skill_unlocked(&staging, InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                repo_path: skill.repo_path.clone(),
                content_hash: skill.content_hash.clone(),
                installed_at: current_unix_timestamp_string(),
            }, enable_after_install, true, Some(&agent_snapshot), false, &mut coordination)
        })();
        result.map_err(|error| retain_staging_context(error, &staging))
    }

    /// 启用 = 在 `$CODEX_HOME/skills/` 下建一个指向源目录的软链；停用 = 删掉它。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let mut coordination = skills_coordination();
        self.ensure_skill_name_not_aliased(id, &self.load_state_checked_unlocked()?)?;
        let result = self.set_enabled_unlocked(id, enabled, &mut coordination);
        coordination.changed_nested_skill(self.coordination_scope(), id);
        result
    }

    fn set_enabled_unlocked(
        &self,
        id: &str,
        enabled: bool,
        coordination: &mut SkillsCoordination,
    ) -> anyhow::Result<()> {
        let source = self.source_dir.join(id);
        let link = self.linked_dir().join(id);
        if !enabled {
            let state = self.load_state_checked_unlocked()?;
            let physical_copy = std::fs::symlink_metadata(&link)
                .is_ok_and(|metadata| !metadata.file_type().is_symlink());
            if link.is_dir() && (physical_copy || !source.is_dir() || !state.installed.contains_key(id)) {
                // 复制回退后的安装位置可能已有用户修改；未纳管软链也需要保留可再启用的源。
                std::fs::create_dir_all(&self.source_dir)?;
                if self.source_dir.canonicalize()?.starts_with(link.canonicalize()?) {
                    anyhow::bail!("Skill 目录不能包含受控源目录，未执行停用");
                }
                let staging = self.staging_dir(id);
                std::fs::create_dir(&staging)
                    .with_context(|| format!("创建 Skill 暂存目录失败：{}", staging.display()))?;
                if let Err(error) = copy_dir_all(&link, &staging) {
                    return Err(retain_staging_context(error, &staging));
                }
                let contents_changed = match (
                    skill_directory_digest(&source),
                    skill_directory_digest(&staging),
                ) {
                    (Ok(Some(before)), Ok(Some(after))) => before != after,
                    _ => true,
                };
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
                self.commit_staged_skill_unlocked(
                    &staging, installed, false, contents_changed, None, false, coordination,
                ).map_err(|error| retain_staging_context(error, &staging))?;
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
        link_or_copy(&source, &link).map(|_| ())
    }

    /// 卸载：删软链，源目录整体移进备份目录。不做自动轮转删除。
    pub fn uninstall(&self, id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        let mut coordination = skills_coordination();
        let mut state = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(id, &state)?;
        // Agent 托管副本也参与事务，避免删除用户修改或卸载失败后无法回滚。
        let mut agent_paths = Vec::new();
        for (_, dir) in &self.agent_dirs {
            let link = dir.join(id);
            if std::fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink())
                || is_managed_copy(&link)
            {
                let identity = skill_path_identity(&link)?;
                agent_paths.push((link, identity));
            }
        }
        let link = self.linked_dir().join(id);
        let source = self.source_dir.join(id);
        state.installed.remove(id);
        let mut displaced = Vec::new();
        let result = (|| -> anyhow::Result<()> {
            let mut paths = Vec::new();
            let mut seen = BTreeSet::new();
            // 先按不跟随最后一级软链的身份去重，再移动；父目录别名也只处理一次。
            for original in [&link, &source] {
                if std::fs::symlink_metadata(original).is_ok() {
                    let identity = skill_path_identity(original)?;
                    if seen.insert(identity) { paths.push((original.clone(), identity)); }
                }
            }
            for (original, identity) in agent_paths {
                if seen.insert(identity) { paths.push((original, identity)); }
            }
            for (original, identity) in paths {
                let backup = self.backup_directory_checked(id, &original, identity)?;
                displaced.push((original, backup, identity));
            }
            self.save_state_unlocked(&state)?;
            Ok(())
        })();
        if let Err(error) = result {
            let mut rollback_errors = Vec::new();
            for (original, backup, identity) in displaced.iter().rev() {
                if let Err(rollback_error) = restore_skill_backup(backup, original, *identity) {
                    rollback_errors.push(format!("原内容保留在 {}：{rollback_error:#}", backup.display()));
                }
            }
            if !rollback_errors.is_empty() {
                return Err(error.context(format!("卸载回滚未完成：{}", rollback_errors.join("；"))));
            }
            return Err(error);
        }
        coordination.changed_skill(self.coordination_scope(), id);
        Ok(state)
    }

    fn backup_directory_checked(&self, id: &str, source: &Path, expected: SkillPathIdentity) -> anyhow::Result<PathBuf> {
        if skill_path_identity(source)? != expected {
            anyhow::bail!("Skill 路径已被其他内容替换，未备份：{}", source.display());
        }
        let backup = self.backup_directory(id, source)?;
        if skill_path_identity(&backup).ok() != Some(expected) {
            if let Err(error) = move_path_no_replace(&backup, source) {
                anyhow::bail!("Skill 备份期间路径发生变化，内容保留在 {}：{error:#}", backup.display());
            }
            anyhow::bail!("Skill 备份期间路径发生变化，未覆盖：{}", source.display());
        }
        Ok(backup)
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
        match move_skill_to_backup(source, &backup) {
            Ok(()) => Ok(backup),
            Err(error) if is_cross_device_error(&error) => {
                // rename 不能跨卷：保留在原目录的隐藏备份区，仍由备份列表统一管理。
                let parent = source.parent().context("Skill 路径没有父目录")?;
                let local = parent.join(".codex-plus-backups");
                std::fs::create_dir_all(&local)?;
                loop {
                    let target = local.join(format!("{id}-{timestamp}"));
                    if std::fs::symlink_metadata(&target).is_err() {
                        move_path_no_replace(source, &target)?;
                        return Ok(target);
                    }
                    timestamp += 1;
                }
            }
            Err(error) => Err(error).with_context(|| format!("备份 {} 到 {} 失败", source.display(), backup.display())),
        }
    }

    fn backup_roots(&self) -> Vec<(String, PathBuf)> {
        let mut roots = vec![(String::new(), self.backups_dir.clone()),
            ("@local/codex/".into(), self.linked_dir().join(".codex-plus-backups")),
            ("@local/source/".into(), self.source_dir.join(".codex-plus-backups"))];
        for (agent, dir) in &self.agent_dirs {
            roots.push((format!("@local/agent-{agent}/"), dir.join(".codex-plus-backups")));
        }
        roots
    }

    fn resolve_backup(&self, id: &str) -> anyhow::Result<(PathBuf, String)> {
        // 带 '/' 的命名空间不可能与合法旧 Skill ID 混淆；只接受已配置根的前缀。
        for (prefix, root) in self.backup_roots().into_iter().filter(|(prefix, _)| !prefix.is_empty()) {
            if let Some(name) = id.strip_prefix(&prefix) {
                validate_skill_id(name)?;
                return Ok((root.join(name), name.to_string()));
            }
        }
        validate_skill_id(id)?;
        Ok((self.backups_dir.join(id), id.to_string()))
    }

    pub fn list_backups(&self) -> Vec<SkillBackup> {
        let mut backups = Vec::new();
        let mut seen = BTreeSet::new();
        for (prefix, root) in self.backup_roots() {
            let Ok(resolved) = root.canonicalize() else { continue };
            let Ok(identity) = skill_path_identity(&resolved) else { continue };
            if !seen.insert(identity) { continue; }
            let Ok(dir) = std::fs::read_dir(root) else { continue };
            for entry in dir.flatten() {
                let path = entry.path();
                if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_dir()) { continue; }
                let Some(id) = path.file_name().and_then(|name| name.to_str()) else { continue };
                let (skill_id, backed_up_at) = split_backup_id(id);
                let (name, _) = read_skill_manifest(&path.join(SKILL_MANIFEST_FILE), &skill_id);
                backups.push(SkillBackup { id: format!("{prefix}{id}"), skill_id, name, backed_up_at });
            }
        }
        backups.sort_by(|a, b| b.id.cmp(&a.id));
        backups
    }

    /// 从备份恢复：目录移回 SSOT 并重新建链。
    pub fn restore_backup(&self, backup_id: &str) -> anyhow::Result<SkillsState> {
        let mut coordination = skills_coordination();
        let (backup, backup_name) = self.resolve_backup(backup_id)?;
        if !backup.is_dir() {
            anyhow::bail!("备份不存在：{}", backup.display());
        }
        let (skill_id, _) = split_backup_id(&backup_name);
        validate_skill_id(&skill_id)?;
        let current = self.load_state_checked_unlocked()?;
        self.ensure_skill_name_not_aliased(&skill_id, &current)?;
        let destination = self.source_dir.join(&skill_id);
        if current.installed.contains_key(&skill_id)
            || std::fs::symlink_metadata(&destination).is_ok()
            || std::fs::symlink_metadata(self.linked_dir().join(&skill_id)).is_ok()
        {
            anyhow::bail!("{skill_id} 已经装着了，先卸载再从备份恢复");
        }
        std::fs::create_dir_all(&self.source_dir)
            .with_context(|| format!("创建目录失败：{}", self.source_dir.display()))?;
        let staging = self.staging_dir(&skill_id);
        std::fs::create_dir(&staging)
            .with_context(|| format!("创建 Skill 暂存目录失败：{}", staging.display()))?;
        if let Err(error) = copy_dir_all(&backup, &staging) {
            return Err(retain_staging_context(error, &staging));
        }
        let (name, description) =
            read_skill_manifest(&staging.join(SKILL_MANIFEST_FILE), &skill_id);
        let result = self.commit_staged_skill_unlocked(
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
            true,
            None,
            false,
            &mut coordination,
        );
        let state = result.map_err(|error| retain_staging_context(error, &staging))?;
        // 提交前始终保留原备份；提交后清理失败也不撤销已恢复的安装。
        if let Err(error) = remove_link(&backup) {
            eprintln!("清理已恢复的 Skill 备份失败：{error:#}");
        }
        Ok(state)
    }

    /// 显式删除单个备份。AGENTS.md 禁止批量删除，所以只删指定的这一个。
    pub fn delete_backup(&self, backup_id: &str) -> anyhow::Result<Vec<SkillBackup>> {
        let _guard = skills_coordination();
        let (backup, _) = self.resolve_backup(backup_id)?;
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
    #[cfg(windows)]
    validate_windows_name(id)?;
    Ok(())
}

#[cfg(windows)]
fn validate_windows_name(name: &str) -> anyhow::Result<()> {
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty() || name.ends_with('.') || name.ends_with(' ') || reserved
        || name.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        anyhow::bail!("Windows 名称无效或可能是文件别名：{name}");
    }
    Ok(())
}

#[cfg(windows)]
fn resembles_windows_short_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let Some((prefix, number)) = stem.rsplit_once('~') else { return false };
    !prefix.is_empty()
        && !number.is_empty()
        && stem.len() <= 8
        && number.bytes().all(|byte| byte.is_ascii_digit())
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
        let Some(id) = path.file_name().and_then(|name| name.to_str()).map(str::to_string) else {
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

fn repo_zip_request(
    client: &reqwest::Client,
    zip_url: &str,
    authorize: GithubAuth,
) -> reqwest::RequestBuilder {
    authorize(client.get(zip_url))
        .header(reqwest::header::ACCEPT, "application/zip")
        .timeout(REPO_ZIP_HEADER_TIMEOUT + REPO_ZIP_BODY_TIMEOUT)
}

/// GitHub API 限流时，从公开仓库的 codeload 压缩包构造同样的文件树。
/// codeload 不占 GitHub API 配额，且能同时读取 SKILL.md 的元数据。
async fn fetch_repo_skills_from_zip(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    client: &reqwest::Client,
    zip_url: &str,
    authorize: GithubAuth,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let request = repo_zip_request(client, zip_url, authorize).send();
    let response = tokio::time::timeout(REPO_ZIP_HEADER_TIMEOUT, request)
        .await
        .context("等待仓库压缩包响应头超过 30 秒")?
        .with_context(|| format!("下载 {}/{} 的仓库压缩包失败", repo.owner, repo.name))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RepoArchiveHttpError {
            status: status.as_u16(),
            message: format!(
                "下载 {}/{} 的仓库压缩包失败：HTTP {}{}",
                repo.owner,
                repo.name,
                status.as_u16(),
                github_status_hint(status.as_u16())
            ),
        }
        .into());
    }
    let bytes = read_repo_zip_response(response).await?;
    let repo = repo.clone();
    let cached = cached.clone();
    tokio::task::spawn_blocking(move || parse_skills_from_zip(&repo, &bytes, &cached))
        .await
        .context("解析仓库压缩包时发生异常")?
}

pub fn parse_skills_from_zip(
    repo: &SkillRepo,
    bytes: &[u8],
    cached: &BTreeMap<String, RemoteSkill>,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).context("skill 仓库压缩包无法解析")?;
    let root = validate_zip_entries(&mut archive, None)?;
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
        let declared_size = file.size();
        let contents = read_bounded_zip_entry(
            &mut file, declared_size, &mut total_size, REPO_ZIP_UNPACKED_LIMIT_BYTES,
        )?;
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
    let text = (|| -> Option<String> {
        if !std::fs::metadata(manifest).ok()?.is_file() {
            return None;
        }
        let file = std::fs::File::open(manifest).ok()?;
        if !file.metadata().ok()?.is_file() {
            return None;
        }
        read_bounded_skill_manifest(file)
    })();
    text.map(|text| parse_skill_frontmatter(&text, fallback_id))
        .unwrap_or_else(|| (fallback_id.to_string(), String::new()))
}

fn read_bounded_skill_manifest(reader: impl Read) -> Option<String> {
    let mut bytes = Vec::new();
    reader
        .take((SKILL_FRONTMATTER_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() <= SKILL_FRONTMATTER_LIMIT_BYTES {
        return String::from_utf8(bytes).ok();
    }

    let mut reader = BufReader::new(bytes.as_slice());
    let mut text = String::new();
    let mut first = true;
    loop {
        let start = text.len();
        if reader.read_line(&mut text).ok()? == 0 || text.len() > SKILL_FRONTMATTER_LIMIT_BYTES {
            return None;
        }
        let delimiter = text[start..].trim() == "---";
        if first {
            if !delimiter {
                return None;
            }
            first = false;
        } else if delimiter {
            return Some(text);
        }
    }
}

/// 解析和提取必须看到完全相同的根与路径，拒绝可见的冲突条目及文件别名。
fn validate_zip_entries<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    target: Option<&str>,
) -> anyhow::Result<String> {
    let mut root: Option<String> = None;
    let mut files = BTreeSet::new();
    let mut directories = BTreeSet::new();
    #[cfg(any(windows, target_os = "macos"))]
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let name = file.name();
        if name.contains('\\') || name.starts_with('/') {
            anyhow::bail!("仓库压缩包包含不安全路径：{name}");
        }
        let parts: Vec<&str> = name.trim_end_matches('/').split('/').collect();
        if parts.iter().any(|part| part.is_empty() || *part == "." || *part == "..")
            || name.ends_with("//")
        {
            anyhow::bail!("仓库压缩包包含不安全路径：{name}");
        }
        if let Some(existing) = &root {
            if existing.as_str() != parts[0] {
                anyhow::bail!("仓库压缩包包含多个根目录");
            }
        } else {
            root = Some(parts[0].to_string());
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let relative = parts[1..].join("/");
            let inner = match target {
                Some(".") => relative.as_str(),
                Some(target) => relative.strip_prefix(&format!("{target}/")).unwrap_or(""),
                None => "",
            };
            let mut parent = String::new();
            for part in inner.split('/').filter(|part| !part.is_empty()) {
                #[cfg(windows)]
                {
                    validate_windows_name(part)?;
                    if resembles_windows_short_name(part) {
                        anyhow::bail!("Skill 包含可能冲突的 Windows 短名称：{name}");
                    }
                }
                let names = children.entry(parent.clone()).or_default();
                if names.iter().any(|old| old.as_str() != part && skill_names_may_alias(old, part)) {
                    anyhow::bail!("仓库压缩包包含大小写冲突的路径：{name}");
                }
                if !names.iter().any(|old| old.as_str() == part) {
                    names.push(part.to_string());
                }
                parent.push('/');
                parent.push_str(part);
            }
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        let _ = target;
        if parts.len() == 1 {
            if !file.is_dir() {
                anyhow::bail!("仓库压缩包根目录不是目录：{name}");
            }
            continue;
        }
        let mut path = String::new();
        for part in &parts[1..parts.len() - 1] {
            if !path.is_empty() { path.push('/'); }
            path.push_str(part);
            if files.contains(&path) {
                anyhow::bail!("仓库压缩包包含冲突路径：{name}");
            }
            directories.insert(path.clone());
        }
        if !path.is_empty() { path.push('/'); }
        path.push_str(parts[parts.len() - 1]);
        if file.is_dir() {
            if files.contains(&path) {
                anyhow::bail!("仓库压缩包包含冲突路径：{name}");
            }
            directories.insert(path);
        } else if directories.contains(&path) || !files.insert(path) {
            anyhow::bail!("仓库压缩包包含重复或冲突路径：{name}");
        }
    }
    root.context("仓库压缩包为空")
}

fn read_bounded_zip_entry<R: Read>(
    reader: &mut R,
    declared_size: u64,
    total_size: &mut u64,
    limit: u64,
) -> anyhow::Result<Vec<u8>> {
    if declared_size > limit.saturating_sub(*total_size) {
        anyhow::bail!("仓库解压内容超过 256 MiB");
    }
    let mut contents = Vec::new();
    reader.take(declared_size.saturating_add(1)).read_to_end(&mut contents)?;
    if contents.len() as u64 != declared_size {
        anyhow::bail!("仓库压缩包条目的实际解压大小与声明不符");
    }
    *total_size += contents.len() as u64;
    Ok(contents)
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
    let prefix = if repo_path == "." { String::new() } else { format!("{repo_path}/") };
    let mut archive =
        zip::ZipArchive::new(Cursor::new(zip_bytes)).context("skill 仓库压缩包无法解析")?;
    validate_zip_entries(&mut archive, Some(repo_path))?;
    let mut wrote_manifest = false;
    let mut total_size = 0;

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
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        let declared_size = file.size();
        let contents = read_bounded_zip_entry(
            &mut file, declared_size, &mut total_size, REPO_ZIP_UNPACKED_LIMIT_BYTES,
        )
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

fn github_https_proxy_from_env(
    get_env: &mut impl FnMut(&str) -> Option<String>,
    linux: bool,
) -> Option<String> {
    let keys = if linux {
        ["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"]
    } else {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    };
    keys.into_iter().find_map(|key| {
        get_env(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn build_github_client(
    agent: &str,
    mut get_env: impl FnMut(&str) -> Option<String>,
    linux: bool,
    mut system_proxy: impl FnMut(Option<&str>) -> Option<String>,
    builder: reqwest::ClientBuilder,
) -> anyhow::Result<reqwest::Client> {
    let environment_proxy = github_https_proxy_from_env(&mut get_env, linux);
    let http_proxy = ["http_proxy", "HTTP_PROXY"].into_iter().find_map(|key| {
        get_env(key)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    });
    let no_proxy = get_env("NO_PROXY").or_else(|| get_env("no_proxy"));
    let builder = builder.user_agent(agent);
    if environment_proxy.is_some() || http_proxy.is_some() {
        let proxy = if linux {
            environment_proxy
        } else {
            system_proxy(None).or(environment_proxy)
        };
        let bypass = || {
            no_proxy
                .as_deref()
                .and_then(reqwest::NoProxy::from_string)
                .or_else(|| reqwest::NoProxy::from_string("localhost,127.0.0.1,::1"))
        };
        let builder = if let Some(proxy) = proxy {
            builder.proxy(reqwest::Proxy::all(&proxy)?.no_proxy(bypass()))
        } else if linux {
            // HTTP 代理不能用于 HTTPS，也不能在降级重定向时明文发送凭据。
            builder.no_proxy().https_only(true)
        } else {
            builder
        };
        return Ok(crate::http_client::apply_configured_ca_certificate(builder)?.build()?);
    }

    let builder = builder
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(90));
    let mut proxies = BTreeMap::new();
    for host in [
        "api.github.com",
        "codeload.github.com",
        "raw.githubusercontent.com",
    ] {
        // Linux 的系统代理检测本身读取环境变量，不能让 HTTP-only 设置回流到 HTTPS。
        if !linux && let Some(proxy) = system_proxy(Some(&format!("https://{host}/"))) {
            proxies.insert(host.to_string(), proxy);
        }
    }
    let builder = builder.proxy(reqwest::Proxy::custom(move |url| {
        url.host_str().and_then(|host| proxies.get(host).cloned())
    }));
    Ok(builder.build()?)
}

fn github_system_proxy(url: Option<&str>) -> Option<String> {
    match url {
        Some(url) => crate::proxy::detect_system_proxy_for_url(url),
        None => crate::proxy::detect_system_proxy(),
    }
}

async fn github_client() -> anyhow::Result<reqwest::Client> {
    tokio::task::spawn_blocking(|| {
        let agent = format!("Codex++/{}", crate::version::VERSION);
        if !cfg!(target_os = "linux")
            && [
                "HTTPS_PROXY",
                "https_proxy",
                "ALL_PROXY",
                "all_proxy",
                "HTTP_PROXY",
                "http_proxy",
            ]
            .iter()
            .any(|key| std::env::var(key).is_ok_and(|value| !value.trim().is_empty()))
        {
            return crate::http_client::proxied_client(&agent);
        }
        build_github_client(
            &agent,
            |key| std::env::var(key).ok(),
            cfg!(target_os = "linux"),
            github_system_proxy,
            reqwest::Client::builder(),
        )
    })
    .await?
}

#[cfg(test)]
#[path = "skills_github_proxy_tests.rs"]
mod github_proxy_tests;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct RepoTreeHttpError {
    status: u16,
    message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct RepoArchiveHttpError {
    status: u16,
    message: String,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct RepoTreeFallbackError {
    archive_missing: bool,
    message: String,
}

fn should_try_alternative_branch(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<RepoTreeHttpError>()
        .is_some_and(|error| error.status == 404)
        || error
            .downcast_ref::<RepoTreeFallbackError>()
            .is_some_and(|error| error.archive_missing)
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
    ) -> futures_util::future::BoxFuture<'a, anyhow::Result<Vec<RemoteSkill>>>,
{
    let repo = normalize_repo(repo)?;
    match fetch(&repo, cached).await {
        Ok(skills) => Ok((repo, skills)),
        Err(error) if should_try_alternative_branch(&error)
            && matches!(repo.branch.as_str(), "main" | "master") =>
        {
            let mut alternative = repo.clone();
            alternative.branch = if repo.branch == "main" { "master" } else { "main" }.into();
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

type GithubAuth = fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder;

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
    fetch_repo_skills_with_client(
        repo,
        cached,
        &client,
        &repo_tree_url(repo),
        &repo_zip_url(repo),
        with_github_auth,
    )
    .await
}

async fn fetch_repo_skills_with_client(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    client: &reqwest::Client,
    tree_url: &str,
    zip_url: &str,
    authorize: GithubAuth,
) -> anyhow::Result<Vec<RemoteSkill>> {
    fetch_repo_skills_with_client_timeout(
        repo,
        cached,
        client,
        tree_url,
        zip_url,
        authorize,
        std::time::Duration::from_secs(20),
    )
    .await
}

async fn fetch_repo_skills_with_client_timeout(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    client: &reqwest::Client,
    tree_url: &str,
    zip_url: &str,
    authorize: GithubAuth,
    tree_timeout: std::time::Duration,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let response = authorize(client.get(tree_url))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(tree_timeout)
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            return fetch_repo_skills_from_zip(repo, cached, client, zip_url, authorize)
                .await
                .map_err(|zip_error| {
                    let archive_missing = zip_error
                        .downcast_ref::<RepoArchiveHttpError>()
                        .is_some_and(|error| error.status == 404);
                    zip_error.context(RepoTreeFallbackError {
                        archive_missing,
                        message: format!("文件树请求失败（{error}），压缩包回退也失败"),
                    })
                });
        }
    };
    // 不要用 error_for_status() 一把带过：403 限流、404 仓库/分支不存在、401 令牌
    // 无效，处理方式完全不同，但用户只会看到「返回错误状态」，没法自助也没法报障
    // （#1989 里四个仓库全失败，看不出到底是限流还是网络）。
    let status = response.status();
    if !status.is_success() && (matches!(status.as_u16(), 403 | 429) || status.is_server_error()) {
        return fetch_repo_skills_from_zip(repo, cached, client, zip_url, authorize)
            .await
            .map_err(|zip_error| {
                let archive_missing = zip_error
                    .downcast_ref::<RepoArchiveHttpError>()
                    .is_some_and(|error| error.status == 404);
                zip_error.context(RepoTreeFallbackError {
                    archive_missing,
                    message: format!("文件树请求失败：HTTP {}，压缩包回退也失败", status.as_u16()),
                })
            });
    }
    if !status.is_success() {
        return Err(RepoTreeHttpError {
            status: status.as_u16(),
            message: format!(
                "{}/{} 的文件树请求失败：HTTP {}{}",
                repo.owner,
                repo.name,
                status.as_u16(),
                github_status_hint(status.as_u16())
            ),
        }
        .into());
    }

    let tree = match response.bytes().await {
        Ok(bytes) => {
            let repo = repo.clone();
            tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Vec<RemoteSkill>>> {
                let tree: Value = serde_json::from_slice(&bytes)?;
                if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
                    return Ok(None);
                }
                anyhow::ensure!(
                    tree.get("tree").is_some_and(Value::is_array),
                    "文件树响应缺少有效的 tree 数组"
                );
                Ok(Some(parse_skills_from_tree(&repo, &tree)))
            })
            .await
            .context("解析 GitHub 文件树后台任务失败")?
        }
        Err(error) if error.is_timeout() => {
            let error = error.without_url();
            return fetch_repo_skills_from_zip(repo, cached, client, zip_url, authorize)
                .await
                .map_err(|zip_error| {
                    let archive_missing = zip_error
                        .downcast_ref::<RepoArchiveHttpError>()
                        .is_some_and(|error| error.status == 404);
                    zip_error.context(RepoTreeFallbackError {
                        archive_missing,
                        message: format!("读取 GitHub 文件树响应超时（{error:#}），压缩包回退也失败"),
                    })
                });
        }
        Err(error) => Err(anyhow::Error::new(error.without_url())),
    };
    let skills = match tree {
        Ok(Some(skills)) => skills,
        Ok(None) => {
            return fetch_repo_skills_from_zip(repo, cached, client, zip_url, authorize)
                .await
                .map_err(|zip_error| {
                    let archive_missing = zip_error
                        .downcast_ref::<RepoArchiveHttpError>()
                        .is_some_and(|error| error.status == 404);
                    zip_error.context(RepoTreeFallbackError {
                        archive_missing,
                        message: "文件树不完整，压缩包回退也失败".into(),
                    })
                });
        }
        Err(error) => {
            return fetch_repo_skills_from_zip(repo, cached, client, zip_url, authorize)
                .await
                .with_context(|| format!("解析 GitHub 文件树响应失败（{error:#}），压缩包回退也失败"));
        }
    };
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
            if let Ok(response) = authorize(client.get(manifest_url))
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
    let request = repo_zip_request(&client, &repo_zip_url(repo), with_github_auth).send();
    let response = tokio::time::timeout(REPO_ZIP_HEADER_TIMEOUT, request)
        .await
        .context("等待仓库压缩包响应头超过 30 秒")?
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

async fn read_repo_zip_response(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    read_repo_zip_response_with_timeout(response, REPO_ZIP_IDLE_TIMEOUT, REPO_ZIP_BODY_TIMEOUT).await
}

async fn read_repo_zip_response_with_timeout(
    mut response: reqwest::Response,
    idle_timeout: Duration,
    total_timeout: Duration,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= REPO_ZIP_DOWNLOAD_LIMIT_BYTES as u64),
        "仓库压缩包超过 64 MiB，未下载"
    );
    tokio::time::timeout(total_timeout, async move {
        let mut bytes = Vec::new();
        while let Some(chunk) = tokio::time::timeout(idle_timeout, response.chunk())
            .await
            .context("等待仓库压缩包数据超时")?
            .context("读取仓库压缩包内容失败")?
        {
            anyhow::ensure!(
                chunk.len() <= REPO_ZIP_DOWNLOAD_LIMIT_BYTES.saturating_sub(bytes.len()),
                "仓库压缩包超过 64 MiB，已停止下载"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    })
    .await
    .context("仓库压缩包下载总时间超限")?
}

/// 复制回退时放进副本的标记：有它说明这份副本是 Codex++ 建的，停用时可以删。
const MANAGED_COPY_MARKER: &str = ".codex-plus-managed";

#[cfg(test)]
thread_local! {
    static DIGEST_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}

fn skill_directory_digest(path: &Path) -> anyhow::Result<Option<[u8; 32]>> {
    #[cfg(test)]
    DIGEST_HOOK.with(|hook| { if let Some(hook) = hook.borrow_mut().take() { hook(); } });
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("读取 {} 失败", path.display())),
    };
    if !metadata.is_dir() {
        anyhow::bail!("Skill 内容不是可安全检查的实体目录：{}", path.display());
    }
    fn hash_directory(dir: &Path, hasher: &mut Sha256) -> anyhow::Result<()> {
        let mut entries = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            if name == MANAGED_COPY_MARKER { continue; }
            let name = name.to_str().context("Skill 文件名无法安全比较")?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!("Skill 包含无法安全比较的符号链接：{}", path.display());
            }
            hasher.update((name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            if metadata.is_dir() {
                hasher.update(b"D");
                hash_directory(&path, hasher)?;
                hasher.update(b"E");
            } else if metadata.is_file() {
                hasher.update(b"F");
                hasher.update(metadata.len().to_le_bytes());
                let mut file = std::fs::File::open(&path)?;
                let mut buffer = [0u8; 64 * 1024];
                let mut bytes_read = 0u64;
                loop {
                    let count = file.read(&mut buffer)?;
                    if count == 0 { break; }
                    bytes_read += count as u64;
                    hasher.update(&buffer[..count]);
                }
                let after = std::fs::metadata(&path)?;
                if bytes_read != metadata.len() || after.len() != metadata.len()
                    || after.modified().ok() != metadata.modified().ok()
                {
                    anyhow::bail!("读取期间 Skill 文件发生变化：{}", path.display());
                }
            } else {
                anyhow::bail!("Skill 包含无法安全比较的特殊文件：{}", path.display());
            }
        }
        Ok(())
    }
    let mut hasher = Sha256::new();
    hash_directory(path, &mut hasher)?;
    Ok(Some(hasher.finalize().into()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct SkillPathIdentity {
    device: u64,
    file: u64,
}

fn retain_staging_context(error: anyhow::Error, staging: &Path) -> anyhow::Error {
    if std::fs::symlink_metadata(staging).is_ok() {
        error.context(format!("暂存路径仍存在且未自动清理：{}", staging.display()))
    } else {
        error
    }
}

#[cfg(test)]
thread_local! {
    static SOURCE_PUBLICATION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce(&Path) -> anyhow::Result<()>>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_source_publication_hook(staging: &Path) -> anyhow::Result<()> {
    SOURCE_PUBLICATION_HOOK.with(|hook| match hook.borrow_mut().take() {
        Some(hook) => hook(staging),
        None => Ok(()),
    })
}

#[cfg(windows)]
fn skill_path_identity(path: &Path) -> anyhow::Result<SkillPathIdentity> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
        },
    };

    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags((FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).0)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)? };
    Ok(SkillPathIdentity {
        device: info.dwVolumeSerialNumber as u64,
        file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    })
}

#[cfg(unix)]
fn skill_path_identity(path: &Path) -> anyhow::Result<SkillPathIdentity> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::symlink_metadata(path)?;
    Ok(SkillPathIdentity { device: metadata.dev(), file: metadata.ino() })
}

#[cfg(not(any(windows, unix)))]
fn skill_path_identity(_path: &Path) -> anyhow::Result<SkillPathIdentity> {
    anyhow::bail!("当前平台不支持核验 Skill 路径身份")
}

/// 把本次发布的对象移到同卷的恢复位置；身份不符时保留现有路径。
fn recover_published_path(path: &Path, expected: SkillPathIdentity) -> anyhow::Result<Option<PathBuf>> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    if skill_path_identity(path)? != expected {
        anyhow::bail!("Skill 路径已被其他内容替换，未移动：{}", path.display());
    }
    let parent = path.parent().context("Skill 恢复位置没有父目录")?;
    let recovery = parent.join(format!(".codex-plus-recovery-{}", uuid::Uuid::new_v4()));
    move_path_no_replace(path, &recovery)?;
    if skill_path_identity(&recovery).ok() != Some(expected) {
        if let Err(error) = move_path_no_replace(&recovery, path) {
            anyhow::bail!("Skill 路径在回滚期间发生变化，内容保留在 {}：{error:#}", recovery.display());
        }
        anyhow::bail!("Skill 路径在回滚期间发生变化，未覆盖：{}", path.display());
    }
    Ok(Some(recovery))
}

fn restore_skill_backup(backup: &Path, original: &Path, expected: SkillPathIdentity) -> anyhow::Result<()> {
    if skill_path_identity(backup)? != expected {
        anyhow::bail!("Skill 备份已被其他内容替换，未恢复：{}", backup.display());
    }
    move_path_no_replace(backup, original)?;
    if skill_path_identity(original)? != expected {
        anyhow::bail!("Skill 恢复期间路径发生变化：{}", original.display());
    }
    Ok(())
}

fn is_cross_device_error(error: &anyhow::Error) -> bool {
    #[cfg(windows)]
    if error.downcast_ref::<windows::core::Error>().is_some_and(|error|
        error.code() == windows::core::HRESULT::from_win32(17)) { return true; }
    error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        #[cfg(unix)]
        { error.raw_os_error() == Some(libc::EXDEV) }
        #[cfg(windows)]
        { error.raw_os_error() == Some(17) } // ERROR_NOT_SAME_DEVICE
        #[cfg(not(any(unix, windows)))]
        { let _ = error; false }
    })
}

#[cfg(test)]
thread_local! {
    static FORCE_CROSS_DEVICE_BACKUP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn move_skill_to_backup(source: &Path, target: &Path) -> anyhow::Result<()> {
    #[cfg(test)]
    if FORCE_CROSS_DEVICE_BACKUP.with(|flag| flag.replace(false)) {
        #[cfg(unix)]
        return Err(std::io::Error::from_raw_os_error(libc::EXDEV).into());
        #[cfg(windows)]
        return Err(std::io::Error::from_raw_os_error(17).into());
    }
    move_path_no_replace(source, target)
}

/// 软链优先，在同目录暂存后以不覆盖现有目录的方式发布。
fn link_or_copy(source: &Path, link: &Path) -> anyhow::Result<SkillPathIdentity> {
    let mut published = None;
    match link_or_copy_tracked(source, link, &mut published) {
        Ok(identity) => Ok(identity),
        Err(error) => {
            if let Some(identity) = published {
                match recover_published_path(link, identity) {
                    Ok(Some(path)) => return Err(error.context(format!("未完成的 Skill 内容保留在 {}", path.display()))),
                    Ok(None) => {}
                    Err(recovery_error) => return Err(error.context(format!("Skill 回滚未完成：{recovery_error:#}"))),
                }
            }
            Err(error)
        }
    }
}

fn link_or_copy_tracked(
    source: &Path,
    link: &Path,
    published: &mut Option<SkillPathIdentity>,
) -> anyhow::Result<SkillPathIdentity> {
    let parent = link.parent().context("Skill 启用位置没有父目录")?;
    let staging = parent.join(format!(".codex-plus-copy-{}", uuid::Uuid::new_v4()));
    link_or_copy_at(source, link, &staging, published, || Ok(()))
}

fn link_or_copy_at(
    source: &Path,
    link: &Path,
    staging: &Path,
    published: &mut Option<SkillPathIdentity>,
    after_publish: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<SkillPathIdentity> {
    match std::fs::symlink_metadata(link) {
        Ok(_) => anyhow::bail!("Skill 启用位置已有其他内容：{}", link.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut created_staging = false;
    let result = (|| -> anyhow::Result<SkillPathIdentity> {
        if symlink_dir(source, staging).is_ok() {
            created_staging = true;
        } else {
            std::fs::create_dir(&staging)
                .with_context(|| format!("创建 Skill 副本暂存目录 {} 失败", staging.display()))?;
            created_staging = true;
            copy_dir_all(source, staging)?;
            std::fs::write(staging.join(MANAGED_COPY_MARKER), "")?;
        }
        let identity = skill_path_identity(staging)?;
        move_path_no_replace(staging, link)?;
        *published = Some(identity);
        after_publish()?;
        if skill_path_identity(link)? != identity {
            anyhow::bail!("Skill 发布期间路径发生变化，未覆盖：{}", link.display());
        }
        Ok(identity)
    })();
    match result {
        Err(error) if created_staging && published.is_none() => {
            Err(error.context(format!("未完成的 Skill 暂存内容保留在 {}", staging.display())))
        }
        other => other.with_context(|| format!("发布 Skill 到 {} 失败", link.display())),
    }
}

#[cfg(windows)]
fn move_path_no_replace(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::MoveFileW;

    // 只解析父目录，避免解析最后一级软链，同时支持 Windows 扩展长度路径。
    let extended = |path: &Path| -> anyhow::Result<PathBuf> {
        let name = path.file_name().context("Skill 路径没有文件名")?;
        let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
        Ok(parent.canonicalize()?.join(name))
    };
    let source = extended(source)?.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    let target = extended(target)?.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
    unsafe { MoveFileW(PCWSTR(source.as_ptr()), PCWSTR(target.as_ptr())) }?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn move_path_no_replace(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())?;
    let target = CString::new(target.as_os_str().as_bytes())?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2, libc::AT_FDCWD, source.as_ptr(),
            libc::AT_FDCWD, target.as_ptr(), libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(source.as_ptr(), target.as_ptr(), libc::RENAME_EXCL) } as libc::c_long;
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn move_path_no_replace(_source: &Path, _target: &Path) -> anyhow::Result<()> {
    anyhow::bail!("当前平台不支持安全发布 Skill 目录")
}

/// 复制回退建的副本（不是软链，但带标记）。
fn is_managed_copy(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
        && path.join(MANAGED_COPY_MARKER).is_file()
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
        // 副本标记只属于那一份副本，不跟着复制（例如停用 Codex 的副本时复制回受控源）。
        if entry.file_name() == MANAGED_COPY_MARKER {
            continue;
        }
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
    use std::sync::Arc;

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
        let metadata = std::fs::symlink_metadata(&link).unwrap();
        assert!(metadata.file_type().is_symlink() || is_managed_copy(&link));
        assert_eq!(std::fs::read_to_string(link.join(SKILL_MANIFEST_FILE)).unwrap(), "---\nname: Alpha\n---\n");
        let entries = manager.local_inventory();
        assert_eq!(entry(&entries, "alpha").agents, vec!["claude".to_string()]);
        assert!(entry(&entries, "alpha").managed);

        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert!(manager.source_dir.join("alpha/SKILL.md").is_file(), "停用不动源目录");
    }

    /// Windows 没有建软链的权限时退化成复制：带标记的副本停用时照样删掉，复制回受控源时不带标记。
    #[test]
    fn copied_fallback_can_be_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/alpha"), "---\nname: Alpha\n---\n");
        manager.import_from_directory("alpha", &temp.path().join("external/alpha")).unwrap();
        let copy = temp.path().join("claude-skills/alpha");
        copy_dir_all(&manager.source_dir.join("alpha"), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();

        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(std::fs::symlink_metadata(&copy).is_err(), "副本被删掉");
        assert!(manager.source_dir.join("alpha/SKILL.md").is_file(), "停用不动源目录");

        let again = temp.path().join("copy-again");
        std::fs::write(manager.source_dir.join("alpha").join(MANAGED_COPY_MARKER), "").unwrap();
        copy_dir_all(&manager.source_dir.join("alpha"), &again).unwrap();
        assert!(!again.join(MANAGED_COPY_MARKER).exists(), "标记不跟着复制");
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
    fn uninstall_deduplicates_shared_codex_and_agent_directories() {
        let temp = tempfile::tempdir().unwrap();
        let base = manager(&temp);
        let shared = base.linked_dir();
        let manager = base.with_agent_dirs(vec![("claude".into(), shared.clone()), ("grok".into(), shared)]);
        let external = temp.path().join("external/alpha");
        write_skill(&external, "original");
        manager.import_from_directory("alpha", &external).unwrap();
        manager.set_enabled("alpha", true).unwrap();
        manager.uninstall("alpha").unwrap();
        assert!(!manager.source_dir.join("alpha").exists());
        assert!(!manager.linked_dir().join("alpha").exists());
        assert!(!manager.load_state().installed.contains_key("alpha"));
    }

    #[cfg(unix)]
    #[test]
    fn uninstall_deduplicates_agent_parent_symlink_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("shared");
        std::fs::create_dir(&root).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        let manager = manager(&temp).with_agent_dirs(vec![("claude".into(), root.clone()), ("grok".into(), alias)]);
        let external = temp.path().join("external/alpha");
        write_skill(&external, "original");
        manager.import_from_directory("alpha", &external).unwrap();
        manager.set_enabled_for("claude", "alpha", true).unwrap();
        manager.uninstall("alpha").unwrap();
        assert!(std::fs::symlink_metadata(root.join("alpha")).is_err());
    }

    #[test]
    fn cross_volume_copy_disable_preserves_listable_restorable_backup() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let external = temp.path().join("external/alpha");
        write_skill(&external, "source");
        manager.import_from_directory("alpha", &external).unwrap();
        let copy = temp.path().join("claude-skills/alpha");
        write_skill(&copy, "agent edit");
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        FORCE_CROSS_DEVICE_BACKUP.with(|flag| flag.set(true));
        manager.set_enabled_for("claude", "alpha", false).unwrap();
        assert!(!copy.exists());
        let backup = manager.list_backups().into_iter().find(|backup| backup.id.starts_with("@local/agent-claude/")).unwrap();
        assert_eq!(backup.skill_id, "alpha");
        manager.uninstall("alpha").unwrap();
        manager.restore_backup(&backup.id).unwrap();
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "agent edit");
        assert!(!manager.list_backups().iter().any(|item| item.id == backup.id));
    }

    #[test]
    fn cross_volume_uninstall_keeps_backup_and_explicit_delete_resolves_it() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let copy = temp.path().join("claude-skills/alpha");
        write_skill(&copy, "only agent copy");
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        FORCE_CROSS_DEVICE_BACKUP.with(|flag| flag.set(true));
        manager.uninstall("alpha").unwrap();
        assert!(!copy.exists());
        let backup = manager.list_backups().pop().unwrap();
        assert!(backup.id.starts_with("@local/agent-claude/"));
        assert!(manager.delete_backup(&backup.id).unwrap().is_empty());
    }

    #[test]
    fn agent_copy_scan_runs_without_blocking_other_skill_state_reads() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let external = temp.path().join("external/alpha");
        write_skill(&external, "original");
        manager.import_from_directory("alpha", &external).unwrap();
        let count = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = count.clone();
        DIGEST_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || observed.set(1))));
        manager.capture_agent_copies("alpha").unwrap();
        assert_eq!(count.get(), 0, "没有副本时不应读取源文件");
        DIGEST_HOOK.with(|hook| hook.borrow_mut().take());

        let copy = temp.path().join("claude-skills/alpha");
        copy_dir_all(&manager.source_dir.join("alpha"), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        let reader = manager.clone();
        DIGEST_HOOK.with(|hook| *hook.borrow_mut() = Some(Box::new(move || {
            let (send, receive) = std::sync::mpsc::channel();
            std::thread::spawn(move || { let _ = send.send(reader.load_state()); });
            assert!(receive.recv_timeout(Duration::from_secs(5)).is_ok(), "摘要扫描持有全局锁");
        })));
        assert_eq!(manager.capture_agent_copies("alpha").unwrap().paths.len(), 1);
    }

    #[test]
    fn legacy_update_returns_catalog_identity_that_can_reinstall() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = guarded_repo(&manager);
        let zip = repo_zip(&[("kit-main/z/alpha/SKILL.md", "original")]);
        let remote = zip_skill(&repo, &zip, "z/alpha");
        let mut legacy = remote.clone();
        legacy.id = "alpha".into();
        manager.install_from_zip(&legacy, &zip).unwrap();
        let guard = manager.capture_download_guard("alpha", &repo.key(), false).unwrap();
        let catalog = manager.install_downloaded_zip(&zip, &guard).unwrap();
        assert_eq!(catalog.id, remote.id);
        assert!(manager.load_state().installed.contains_key("alpha"));
        manager.uninstall("alpha").unwrap();
        let guard = manager.capture_download_guard(&catalog.id, &repo.key(), true).unwrap();
        manager.install_downloaded_zip(&zip, &guard).unwrap();
        assert!(manager.load_state().installed.contains_key(&remote.id));
    }

    #[cfg(unix)]
    #[test]
    fn backup_listing_deduplicates_symlinked_backup_roots() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&manager.backups_dir.join("alpha-123"), "backup");
        let agent = temp.path().join("claude-skills");
        std::fs::create_dir_all(&agent).unwrap();
        std::os::unix::fs::symlink(&manager.backups_dir, agent.join(".codex-plus-backups")).unwrap();
        let backups = manager.list_backups();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].id, "alpha-123");
    }

    #[test]
    fn backup_namespace_cannot_shadow_legacy_names_or_escape_roots() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let id = "local--agent-claude--alpha-123";
        let old = manager.backups_dir.join(id);
        write_skill(&old, "legacy");
        assert_eq!(manager.resolve_backup(id).unwrap().0, old);
        for invalid in ["@local/unknown/alpha-123", "@local/agent-claude/../alpha", "@local/agent-claude//absolute", "../alpha"] {
            assert!(manager.resolve_backup(invalid).is_err());
        }
        manager.restore_backup(id).unwrap();
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("local--agent-claude--alpha/SKILL.md")).unwrap(), "legacy");
    }

    #[cfg(windows)]
    #[test]
    fn cross_volume_recognizes_native_windows_error() {
        let error: anyhow::Error = windows::core::Error::from(windows::core::HRESULT::from_win32(17)).into();
        assert!(is_cross_device_error(&error));
        let denied: anyhow::Error = windows::core::Error::from(windows::core::HRESULT::from_win32(5)).into();
        assert!(!is_cross_device_error(&denied));
    }

    #[test]
    fn agent_copy_snapshot_detects_toggle_before_commit() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let external = temp.path().join("external/alpha");
        write_skill(&external, "original");
        manager.import_from_directory("alpha", &external).unwrap();
        let snapshot = manager.capture_agent_copies("alpha").unwrap();
        manager.set_enabled_for("claude", "alpha", true).unwrap();
        let (staging, installed) = manager.stage_directory_import("alpha", &external, "local").unwrap();
        let mut coordination = skills_coordination();
        let result = manager.commit_staged_skill_unlocked(&staging, installed, false, true, Some(&snapshot), false, &mut coordination);
        assert!(format!("{:#}", result.unwrap_err()).contains("检查副本期间"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_zip_rejects_case_and_unicode_aliases_before_writing() {
        let temp = tempfile::tempdir().unwrap();
        for paths in [
            ("kit-main/alpha/SKILL.md", "kit-main/alpha/skill.md"),
            ("kit-main/alpha/é.txt", "kit-main/alpha/e\u{301}.txt"),
            ("kit-main/alpha/Dir/a", "kit-main/alpha/dir/b"),
        ] {
            let mut files = vec![("kit-main/alpha/SKILL.md", "manifest")];
            if paths.0 != files[0].0 { files.push((paths.0, "first")); }
            files.push((paths.1, "second"));
            let zip = repo_zip(&files);
            let destination = temp.path().join("target");
            assert!(extract_skill_subtree(&zip, "alpha", &destination).is_err());
            assert!(!destination.exists());
        }
    }

    #[test]
    fn uninstall_backs_up_modified_agent_copy() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/alpha"), "original");
        manager.import_from_directory("alpha", &temp.path().join("external/alpha")).unwrap();
        let copy = temp.path().join("claude-skills/alpha");
        copy_dir_all(&manager.source_dir.join("alpha"), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::write(copy.join(SKILL_MANIFEST_FILE), "agent edit").unwrap();

        manager.uninstall("alpha").unwrap();
        assert!(!copy.exists());
        let contents: BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join(SKILL_MANIFEST_FILE)).unwrap())
            .collect();
        assert!(contents.contains("original"));
        assert!(contents.contains("agent edit"));
    }

    #[test]
    fn failed_uninstall_preserves_agent_copy() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        write_skill(&temp.path().join("external/alpha"), "original");
        manager.import_from_directory("alpha", &temp.path().join("external/alpha")).unwrap();
        let copy = temp.path().join("claude-skills/alpha");
        copy_dir_all(&manager.source_dir.join("alpha"), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::write(copy.join(SKILL_MANIFEST_FILE), "agent edit").unwrap();
        std::fs::write(&manager.backups_dir, "blocked").unwrap();

        assert!(manager.uninstall("alpha").is_err());
        assert_eq!(std::fs::read_to_string(copy.join(SKILL_MANIFEST_FILE)).unwrap(), "agent edit");
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "original");
        assert!(manager.load_state().installed.contains_key("alpha"));
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
    fn local_import_accepts_a_skill_or_a_folder_of_skills_and_explains_missing_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);

        // 选中的目录本身就是一个 Skill。
        let single = temp.path().join("picked/my-skill");
        write_skill(&single, "---\nname: My Skill\ndescription: demo\n---\n");
        let candidates = local_skill_import_candidates(&single).unwrap();
        assert_eq!(candidates, vec![("my-skill".to_string(), single.clone())]);
        manager.import_from_local_directory("my-skill", &single).unwrap();
        let entries = manager.local_inventory();
        let skill = entry(&entries, "my-skill");
        assert!(skill.installed);
        assert!(!skill.enabled, "本地导入与 cc-switch 导入一样默认停用");
        assert_eq!(skill.repo_key, LOCAL_IMPORT_REPO_KEY);
        assert_eq!(skill.name, "My Skill");
        assert!(manager.source_dir().join("my-skill/SKILL.md").is_file());
        assert!(single.join("SKILL.md").is_file(), "导入是复制，原目录保持不动");

        // 选中的是一组 Skill 的父目录：只收含 SKILL.md 的子目录，跳过隐藏目录和普通文件夹。
        let bundle = temp.path().join("bundle");
        write_skill(&bundle.join("beta"), "---\nname: beta\n---\n");
        write_skill(&bundle.join("alpha"), "---\nname: alpha\n---\n");
        write_skill(&bundle.join(".hidden"), "---\nname: hidden\n---\n");
        std::fs::create_dir_all(bundle.join("notes")).unwrap();
        let ids: Vec<_> = local_skill_import_candidates(&bundle).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);

        // 没有 SKILL.md 时给出明确提示。
        let empty = temp.path().join("empty");
        std::fs::create_dir_all(empty.join("notes")).unwrap();
        let error = local_skill_import_candidates(&empty).unwrap_err().to_string();
        assert!(error.contains("SKILL.md"), "{error}");
        assert!(local_skill_import_candidates(&temp.path().join("missing")).is_err());

        // Codex 自己的 skills 目录里的 Skill 已会被扫描显示，不允许再「导入」把原目录挪走。
        let codex_local = manager.linked_dir().join("codex-local");
        write_skill(&codex_local, "---\nname: codex-local\n---\n");
        let error = manager.import_from_local_directory("codex-local", &codex_local).unwrap_err().to_string();
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
    fn failed_source_publication_preserves_a_recreated_staging_path() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        SOURCE_PUBLICATION_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|staging| {
                write_skill(staging, "external");
                anyhow::bail!("injected failure after source publication")
            }));
        });

        let error = manager.install_from_zip(
            &sample_skill("alpha", "alpha", "one"),
            &repo_zip(&[("kit-main/alpha/SKILL.md", "download")]),
        ).unwrap_err();
        let staging = std::fs::read_dir(&manager.source_dir).unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.file_name().unwrap().to_string_lossy().starts_with(".staging-alpha-"))
            .unwrap();
        assert_eq!(std::fs::read_to_string(staging.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert!(format!("{error:#}").contains("暂存路径仍存在且未自动清理"));
        assert!(!manager.source_dir.join("alpha").exists());
        assert!(!manager.load_state().installed.contains_key("alpha"));
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

    fn guarded_repo(manager: &SkillsManager) -> SkillRepo {
        let repo = parse_repo_key(&format!("test-{}/kit@main", uuid::Uuid::new_v4())).unwrap();
        manager.upsert_repo(repo.clone()).unwrap();
        repo
    }

    fn zip_skill(repo: &SkillRepo, zip: &[u8], path: &str) -> RemoteSkill {
        parse_skills_from_zip(repo, zip, &BTreeMap::new()).unwrap()
            .into_iter().find(|skill| skill.repo_path == path).unwrap()
    }

    #[test]
    fn mutations_do_not_replace_corrupt_skill_state_or_existing_content() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let source = manager.source_dir.join("alpha");
        let linked = manager.linked_dir().join("alpha");
        write_skill(&source, "source content");
        write_skill(&linked, "codex content");
        write_skill(&temp.path().join("external/beta"), "external content");
        std::fs::write(&manager.state_path, "{invalid json").unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[("kit-main/gamma/SKILL.md", "archive content")]);
        let skill = zip_skill(&repo, &zip, "gamma");

        assert!(manager.upsert_repo(repo.clone()).is_err());
        assert!(manager.delete_repo(&repo.key()).is_err());
        assert!(manager.set_enabled("alpha", false).is_err());
        assert!(manager.set_enabled_for("claude", "alpha", true).is_err());
        assert!(manager.uninstall("alpha").is_err());
        assert!(manager.import_from_local_directory("beta", &temp.path().join("external/beta")).is_err());
        assert!(manager.install_from_zip(&skill, &zip).is_err());
        assert_eq!(std::fs::read_to_string(&manager.state_path).unwrap(), "{invalid json");
        assert_eq!(std::fs::read_to_string(source.join(SKILL_MANIFEST_FILE)).unwrap(), "source content");
        assert_eq!(std::fs::read_to_string(linked.join(SKILL_MANIFEST_FILE)).unwrap(), "codex content");
    }

    #[test]
    fn staged_import_does_not_overwrite_a_newer_update() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let initial = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        first.install_from_zip(&sample_skill("alpha", "alpha", "one"), &initial).unwrap();
        let external = temp.path().join("external/alpha");
        write_skill(&external, "import content");
        let scope = first.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = first.stage_directory_import("alpha", &external, LOCAL_IMPORT_REPO_KEY).unwrap();

        let updated = repo_zip(&[("kit-main/alpha/SKILL.md", "updated")]);
        second.install_from_zip(&sample_skill("alpha", "alpha", "two"), &updated).unwrap();
        assert!(first.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert_eq!(std::fs::read_to_string(first.source_dir.join("alpha/SKILL.md")).unwrap(), "updated");
        assert_eq!(first.load_state().installed["alpha"].content_hash, "two");
    }

    #[test]
    fn staged_import_is_not_invalidated_by_an_independent_library() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        let second = manager(&second_temp);
        let id = format!("scope-{}", uuid::Uuid::new_v4());
        let first_external = first_temp.path().join("external").join(&id);
        let second_external = second_temp.path().join("external").join(&id);
        write_skill(&first_external, "first");
        write_skill(&second_external, "second");
        let scope = first.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, &id));
        let (staging, imported) = first.stage_directory_import(&id, &first_external, LOCAL_IMPORT_REPO_KEY).unwrap();

        second.import_from_directory(&id, &second_external).unwrap();
        first.commit_staged_import_if_unchanged(&staging, imported, generation).unwrap();
        assert_eq!(std::fs::read_to_string(first.source_dir.join(&id).join(SKILL_MANIFEST_FILE)).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(second.source_dir.join(&id).join(SKILL_MANIFEST_FILE)).unwrap(), "second");
    }

    #[test]
    fn independent_library_does_not_invalidate_repo_guard() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        let second = manager(&second_temp);
        let repo = SkillRepo {
            owner: "scope".into(),
            name: uuid::Uuid::new_v4().to_string(),
            branch: "main".into(),
            subdir: String::new(),
            enabled: true,
        };
        first.upsert_repo(repo.clone()).unwrap();
        let guard = first.capture_repo_guard(&repo.key()).unwrap();

        second.upsert_repo(repo).unwrap();
        assert!(first.repo_guard_is_current(&guard).unwrap());
    }

    #[test]
    fn staged_import_detects_shared_source_across_library_roots() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        let second = SkillsManager::new(
            first.source_dir.clone(),
            second_temp.path().join("backups"),
            second_temp.path().join("state.json"),
            second_temp.path().join("codex"),
        );
        let id = format!("scope-{}", uuid::Uuid::new_v4());
        let first_external = first_temp.path().join("external").join(&id);
        let second_external = second_temp.path().join("external").join(&id);
        write_skill(&first_external, "first");
        write_skill(&second_external, "second");
        let scope = first.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, &id));
        let (staging, imported) = first.stage_directory_import(&id, &first_external, LOCAL_IMPORT_REPO_KEY).unwrap();

        second.import_from_directory(&id, &second_external).unwrap();
        assert!(first.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert_eq!(std::fs::read_to_string(first.source_dir.join(&id).join(SKILL_MANIFEST_FILE)).unwrap(), "second");
    }

    #[test]
    fn shared_agent_directory_keeps_library_scopes_overlapping() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let shared = first_temp.path().join("shared-agent");
        let first = manager(&first_temp).with_agent_dirs(vec![("agent".into(), shared.clone())]);
        let second = manager(&second_temp).with_agent_dirs(vec![("agent".into(), shared)]);
        let first_scope = first.coordination_scope();
        let second_scope = second.coordination_scope();
        assert!(first_scope.is_some() && second_scope.is_some());
        assert!(coordination_scopes_overlap(&first_scope, &second_scope));
        let mut coordination = SkillsCoordination::default();
        coordination.changed_skill(second_scope, "alpha");
        assert_ne!(coordination.skill_generation(&first_scope, "alpha"), 0);
    }

    #[cfg(unix)]
    #[test]
    fn shared_source_symlink_keeps_library_scopes_overlapping() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        std::fs::create_dir_all(&first.source_dir).unwrap();
        let alias = second_temp.path().join("shared-source");
        std::os::unix::fs::symlink(&first.source_dir, &alias).unwrap();
        let second = SkillsManager::new(
            alias,
            second_temp.path().join("backups"),
            second_temp.path().join("state.json"),
            second_temp.path().join("codex"),
        );
        assert!(coordination_scopes_overlap(&first.coordination_scope(), &second.coordination_scope()));
    }

    #[test]
    fn unknown_coordination_scope_remains_conservative() {
        let temp = tempfile::tempdir().unwrap();
        assert!(coordination_path(Path::new("relative/skills")).is_none());
        let scope = manager(&temp).coordination_scope();
        assert!(scope.is_some());
        assert!(coordination_scopes_overlap(&None, &scope));
        assert!(coordination_scope_is_current(&None, &scope));
        assert!(!coordination_scope_is_current(&scope, &None));
    }

    #[test]
    fn coordination_path_tolerates_a_directory_created_during_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("created-during-resolution");
        let expected = path.clone();
        COORDINATION_MISSING_PATH_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move |missing| {
                assert_eq!(missing, expected);
                std::fs::create_dir(missing).unwrap();
            }));
        });
        assert!(coordination_path(&path).is_some());
    }

    #[test]
    fn disjoint_paths_remain_independent_when_they_share_a_common_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let first_root = temp.path().join("one");
        let second_root = temp.path().join("two");
        let first = SkillsManager::new(
            first_root.join("skills"), first_root.join("backups"),
            first_root.join("state.json"), second_root.join("home-one"),
        );
        let second = SkillsManager::new(
            second_root.join("skills"), second_root.join("backups"),
            second_root.join("state.json"), first_root.join("home-two"),
        );
        let first_scope = first.coordination_scope();
        let second_scope = second.coordination_scope();
        assert!(first_scope.is_some() && second_scope.is_some());
        assert!(!coordination_scopes_overlap(&first_scope, &second_scope));
    }

    #[test]
    fn shared_filesystem_ancestor_identifies_mount_aliases() {
        let identity = SkillPathIdentity { device: 11, file: 42 };
        let scope = |path: &str, suffix: &str| Some(vec![CoordinationPath {
            normalized: PathBuf::from(path),
            ancestors: vec![(identity, PathBuf::from(suffix))],
        }]);
        let first = scope("/mount-one/shared", "skills/alpha");
        let alias = scope("/mount-two/shared", "skills/alpha");
        let unrelated = scope("/mount-two/other", "skills/beta");
        assert!(coordination_scopes_overlap(&first, &alias));
        assert!(!coordination_scopes_overlap(&first, &unrelated));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unicode_aliases_share_coordination_and_are_not_overwritten() {
        let composed = "\u{e9}";
        let decomposed = "e\u{301}";
        assert!(skill_names_may_alias(composed, decomposed));
        assert!(!skill_names_may_alias("\u{7532}", "\u{4e59}"));
        assert!(coordination_path_starts_with(
            Path::new("/skills/\u{e9}/child"),
            Path::new("/skills/e\u{301}"),
        ));

        let scope = |path: &str| Some(vec![CoordinationPath {
            normalized: PathBuf::from(path),
            ancestors: Vec::new(),
        }]);
        let mut coordination = SkillsCoordination::default();
        coordination.changed_skill(scope("/skills/\u{e9}"), composed);
        assert!(coordination.skill_generation(&scope("/skills/e\u{301}"), decomposed) > 0);
        assert!(!coordination_scopes_overlap(&scope("/skills/\u{7532}"), &scope("/skills/\u{4e59}")));

        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        write_skill(&first, "first");
        write_skill(&second, "second");
        manager.import_from_directory(composed, &first).unwrap();
        assert!(manager.import_from_directory(decomposed, &second).is_err());
        assert_eq!(std::fs::read_to_string(manager.source_dir.join(composed).join("SKILL.md")).unwrap(), "first");
    }

    #[test]
    fn generation_entries_are_reused_when_a_path_identity_changes() {
        let scope = |file| Some(vec![CoordinationPath {
            normalized: PathBuf::from("/state/skills.json"),
            ancestors: vec![(SkillPathIdentity { device: 11, file }, PathBuf::new())],
        }]);
        let mut coordination = SkillsCoordination::default();
        coordination.changed_skill(scope(1), "alpha");
        let captured = coordination.skill_generation(&scope(1), "alpha");
        coordination.changed_skill(scope(2), "alpha");
        assert_eq!(coordination.skills.len(), 1);
        assert!(coordination.skill_generation(&scope(1), "alpha") > captured);
    }

    #[test]
    fn atomic_state_replacements_do_not_accumulate_file_identities() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut coordination = SkillsCoordination::default();
        for _ in 0..8 {
            manager.save_state_unlocked(&SkillsState::default()).unwrap();
            coordination.changed_skill(manager.coordination_scope(), "alpha");
        }
        assert_eq!(coordination.skills.len(), 1);
        let scope = coordination.skills.keys().next().unwrap().0.as_ref().unwrap();
        assert_eq!(scope[2].ancestors, manager.coordination_scope().unwrap()[2].ancestors);
    }

    #[test]
    fn replaced_path_preserves_generation_visible_through_an_old_alias() {
        let scope = |path: &str, file| Some(vec![CoordinationPath {
            normalized: PathBuf::from(path),
            ancestors: vec![(SkillPathIdentity { device: 11, file }, PathBuf::new())],
        }]);
        let old = scope("/mount-one/skills", 1);
        let alias = scope("/mount-two/skills", 1);
        let replacement = scope("/mount-one/skills", 2);
        let mut coordination = SkillsCoordination::default();

        coordination.changed_skill(old.clone(), "alpha");
        let skill_generation = coordination.skill_generation(&alias, "alpha");
        coordination.changed_skill(replacement.clone(), "alpha");
        assert!(skill_generation > 0);
        assert!(coordination.skill_generation(&alias, "alpha") >= skill_generation);

        coordination.changed_repo(old.clone(), "owner/repo");
        let repo_generation = coordination.repo_generation(&alias, "owner/repo");
        let repo_change_generation = coordination.repo_change_generation(&alias, "owner/repo");
        coordination.changed_repo(replacement.clone(), "owner/repo");
        assert!(repo_generation > 0);
        assert!(coordination.repo_generation(&alias, "owner/repo") >= repo_generation);
        assert!(coordination.repo_change_generation(&alias, "owner/repo") >= repo_change_generation);

        coordination.changed_repo_settings(old, "owner/other");
        let settings_generation = coordination.repo_change_generation(&alias, "owner/other");
        coordination.changed_repo_settings(replacement, "owner/other");
        assert!(settings_generation > 0);
        assert!(coordination.repo_change_generation(&alias, "owner/other") >= settings_generation);
    }

    #[test]
    fn nested_agent_update_invalidates_a_different_skill_import() {
        let temp = tempfile::tempdir().unwrap();
        let base = manager(&temp);
        let agent_dir = base.source_dir.join("alpha");
        let manager = base.with_agent_dirs(vec![("nested".into(), agent_dir.clone())]);
        let alpha = temp.path().join("external/alpha");
        let beta = temp.path().join("external/beta");
        write_skill(&alpha, "alpha");
        write_skill(&beta, "beta");
        manager.import_from_directory("beta", &beta).unwrap();
        manager.set_enabled_for("nested", "beta", true).unwrap();

        let scope = manager.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = manager.stage_directory_import("alpha", &alpha, LOCAL_IMPORT_REPO_KEY).unwrap();
        write_skill(&beta, "updated beta");
        manager.import_from_directory("beta", &beta).unwrap();

        assert!(manager.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert!(agent_dir.join("beta/SKILL.md").is_file());
    }

    #[test]
    fn nested_agent_enable_invalidates_a_different_skill_import() {
        let temp = tempfile::tempdir().unwrap();
        let base = manager(&temp);
        let agent_dir = base.source_dir.join("alpha");
        let manager = base.with_agent_dirs(vec![("nested".into(), agent_dir.clone())]);
        let alpha = temp.path().join("external/alpha");
        let beta = temp.path().join("external/beta");
        write_skill(&alpha, "alpha");
        write_skill(&beta, "beta");
        manager.import_from_directory("beta", &beta).unwrap();

        let scope = manager.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = manager.stage_directory_import("alpha", &alpha, LOCAL_IMPORT_REPO_KEY).unwrap();
        manager.set_enabled_for("nested", "beta", true).unwrap();

        assert!(manager.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert!(agent_dir.join("beta/SKILL.md").is_file());
    }

    #[test]
    fn nested_agent_disable_invalidates_a_same_skill_import() {
        let temp = tempfile::tempdir().unwrap();
        let base = manager(&temp);
        let agent_dir = base.source_dir.join("alpha/agents");
        let manager = base.with_agent_dirs(vec![("nested".into(), agent_dir.clone())]);
        let alpha = temp.path().join("external/alpha");
        write_skill(&alpha, "alpha");
        manager.import_from_directory("alpha", &alpha).unwrap();
        write_skill(&agent_dir.join("alpha"), "agent copy");
        std::fs::write(agent_dir.join("alpha/.codex-plus-managed"), "").unwrap();

        let scope = manager.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = manager.stage_directory_import("alpha", &alpha, LOCAL_IMPORT_REPO_KEY).unwrap();
        manager.set_enabled_for("nested", "alpha", false).unwrap();

        assert!(manager.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert_eq!(std::fs::read_to_string(manager.source_dir.join("alpha/SKILL.md")).unwrap(), "alpha");
    }

    #[test]
    fn shared_agent_enable_invalidates_a_same_skill_import() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        let second = manager(&second_temp).with_agent_dirs(vec![("shared".into(), first.source_dir.clone())]);
        let first_external = first_temp.path().join("external/alpha");
        let second_external = second_temp.path().join("external/alpha");
        write_skill(&first_external, "first");
        write_skill(&second_external, "second");
        second.import_from_directory("alpha", &second_external).unwrap();

        let scope = first.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = first.stage_directory_import("alpha", &first_external, LOCAL_IMPORT_REPO_KEY).unwrap();
        second.set_enabled_for("shared", "alpha", true).unwrap();

        assert!(first.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert_eq!(std::fs::read_to_string(first.source_dir.join("alpha/SKILL.md")).unwrap(), "second");
    }

    #[test]
    fn nested_library_invalidates_a_different_skill_import() {
        let first_temp = tempfile::tempdir().unwrap();
        let second_temp = tempfile::tempdir().unwrap();
        let first = manager(&first_temp);
        let second = SkillsManager::new(
            first.source_dir.join("alpha"),
            second_temp.path().join("backups"),
            second_temp.path().join("state.json"),
            second_temp.path().join("codex"),
        );
        let first_external = first_temp.path().join("external/alpha");
        let second_external = second_temp.path().join("external/beta");
        write_skill(&first_external, "first");
        write_skill(&second_external, "second");
        let scope = first.coordination_scope();
        let generation = (scope.clone(), skills_coordination().skill_generation(&scope, "alpha"));
        let (staging, imported) = first.stage_directory_import("alpha", &first_external, LOCAL_IMPORT_REPO_KEY).unwrap();

        second.import_from_directory("beta", &second_external).unwrap();
        assert!(first.commit_staged_import_if_unchanged(&staging, imported, generation).is_err());
        assert_eq!(std::fs::read_to_string(second.source_dir.join("beta/SKILL.md")).unwrap(), "second");
    }

    #[test]
    fn independent_managers_commit_different_downloads_without_losing_state() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let repo = guarded_repo(&first);
        let zip = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "alpha"),
            ("kit-main/beta/SKILL.md", "beta"),
        ]);
        let skills = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(skills.len()));
        let mut workers = Vec::new();
        for (manager, skill) in [first.clone(), second].into_iter().zip(skills) {
            let guard = manager.capture_download_guard(&skill.id, &repo.key(), true).unwrap();
            assert!(guard.installed.is_none());
            assert_eq!(guard.repo, repo);
            let zip = zip.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                manager.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).unwrap();
                skill
            }));
        }
        let installed: Vec<_> = workers.into_iter().map(|worker| worker.join().unwrap()).collect();
        let state = first.load_state();
        for skill in installed {
            assert_eq!(state.installed[&skill.id].content_hash, skill.content_hash);
            assert!(first.linked_dir().join(&skill.id).join(SKILL_MANIFEST_FILE).is_file());
        }
    }

    #[test]
    fn download_guard_preserves_latest_skill_and_repo_enabled_states() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let mut repo = guarded_repo(&first);
        let old_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "old")]);
        let old = zip_skill(&repo, &old_zip, "alpha");
        first.install_from_zip(&old, &old_zip).unwrap();
        let guard = first.capture_download_guard(&old.id, &repo.key(), false).unwrap();
        assert_eq!(guard.installed.as_ref(), first.load_state().installed.get(&old.id));
        second.set_enabled(&old.id, false).unwrap();
        repo.enabled = false;
        second.update_repo_enabled_if_unchanged(&repo, None).unwrap();
        let new_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "new")]);
        let new = zip_skill(&repo, &new_zip, "alpha");
        first.install_downloaded_zip_if_unchanged(&new, &new_zip, &guard).unwrap();
        assert!(!is_linked(&first.linked_dir().join(&old.id)));
        assert!(!first.list_repos().iter().find(|item| item.key() == repo.key()).unwrap().enabled);

        let guard = first.capture_download_guard(&old.id, &repo.key(), false).unwrap();
        second.set_enabled(&old.id, true).unwrap();
        let newer_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "newer")]);
        let newer = zip_skill(&repo, &newer_zip, "alpha");
        first.install_downloaded_zip_if_unchanged(&newer, &newer_zip, &guard).unwrap();
        assert!(is_linked(&first.linked_dir().join(&old.id)));
        assert_eq!(first.load_state().installed[&old.id].content_hash, newer.content_hash);
    }

    #[test]
    fn download_guard_rejects_uninstall_then_reinstall_of_identical_record() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let repo = guarded_repo(&first);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "same")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        first.install_from_zip(&skill, &zip).unwrap();
        let guard = first.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        second.uninstall(&skill.id).unwrap();
        second.install_from_zip(&skill, &zip).unwrap();
        {
            let _coordination = skills_coordination();
            let mut state = first.load_state_unlocked();
            state.installed.insert(skill.id.clone(), guard.installed.clone().unwrap());
            first.save_state_unlocked(&state).unwrap();
        }
        assert_eq!(first.load_state().installed.get(&skill.id), guard.installed.as_ref());
        assert!(first.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
    }

    #[test]
    fn download_guard_rejects_repo_delete_and_readd() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let repo = guarded_repo(&first);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "alpha")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        let guard = first.capture_download_guard(&skill.id, &repo.key(), true).unwrap();
        second.delete_repo(&repo.key()).unwrap();
        second.upsert_repo(repo.clone()).unwrap();
        assert!(first.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
        assert!(!first.source_dir.join(&skill.id).exists());
    }

    #[test]
    fn download_guard_rejects_direct_edits_to_source_and_copy() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let repo = guarded_repo(&manager);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        manager.install_from_zip(&skill, &zip).unwrap();

        let guard = manager.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        std::fs::write(manager.source_dir.join(&skill.id).join(SKILL_MANIFEST_FILE), "source edit").unwrap();
        assert!(manager.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
        assert_eq!(
            std::fs::read_to_string(manager.source_dir.join(&skill.id).join(SKILL_MANIFEST_FILE)).unwrap(),
            "source edit",
        );

        let link = manager.linked_dir().join(&skill.id);
        remove_link(&link).unwrap();
        copy_dir_all(&manager.source_dir.join(&skill.id), &link).unwrap();
        std::fs::write(link.join(MANAGED_COPY_MARKER), "").unwrap();
        let guard = manager.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        std::fs::write(link.join(SKILL_MANIFEST_FILE), "copy edit").unwrap();
        assert!(manager.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
        assert_eq!(std::fs::read_to_string(link.join(SKILL_MANIFEST_FILE)).unwrap(), "copy edit");
    }

    #[test]
    fn disabling_edited_copy_preserves_content_and_invalidates_download() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let repo = guarded_repo(&first);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        first.install_from_zip(&skill, &zip).unwrap();
        let link = first.linked_dir().join(&skill.id);
        remove_link(&link).unwrap();
        copy_dir_all(&first.source_dir.join(&skill.id), &link).unwrap();
        std::fs::write(link.join(MANAGED_COPY_MARKER), "").unwrap();
        let guard = first.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        std::fs::write(link.join(SKILL_MANIFEST_FILE), "copy edit").unwrap();
        second.set_enabled(&skill.id, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(first.source_dir.join(&skill.id).join(SKILL_MANIFEST_FILE)).unwrap(),
            "copy edit",
        );
        assert!(first.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
        let contents: BTreeSet<_> = first.list_backups().iter()
            .map(|backup| std::fs::read_to_string(first.backups_dir.join(&backup.id).join(SKILL_MANIFEST_FILE)).unwrap())
            .collect();
        assert!(contents.contains("original"));
        assert!(contents.contains("copy edit"));
    }

    #[test]
    fn disabling_unchanged_copy_keeps_download_guard_valid() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let repo = guarded_repo(&first);
        let old_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let skill = zip_skill(&repo, &old_zip, "alpha");
        first.install_from_zip(&skill, &old_zip).unwrap();
        let link = first.linked_dir().join(&skill.id);
        remove_link(&link).unwrap();
        copy_dir_all(&first.source_dir.join(&skill.id), &link).unwrap();
        std::fs::write(link.join(MANAGED_COPY_MARKER), "").unwrap();
        let guard = first.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        second.set_enabled(&skill.id, false).unwrap();
        let new_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "updated")]);
        let updated = zip_skill(&repo, &new_zip, "alpha");
        first.install_downloaded_zip_if_unchanged(&updated, &new_zip, &guard).unwrap();
        assert!(!is_linked(&link));
        assert_eq!(first.load_state().installed[&skill.id].content_hash, updated.content_hash);
    }

    #[test]
    fn update_backs_up_preexisting_source_and_codex_copy_edits() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let repo = guarded_repo(&manager);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        manager.install_from_zip(&skill, &zip).unwrap();
        let source = manager.source_dir.join(&skill.id);
        let link = manager.linked_dir().join(&skill.id);
        remove_link(&link).unwrap();
        std::fs::write(source.join(SKILL_MANIFEST_FILE), "source edit").unwrap();
        copy_dir_all(&source, &link).unwrap();
        std::fs::write(link.join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::write(link.join(SKILL_MANIFEST_FILE), "codex copy edit").unwrap();
        let guard = manager.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        let updated_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "updated")]);
        let updated = zip_skill(&repo, &updated_zip, "alpha");
        manager.install_downloaded_zip_if_unchanged(&updated, &updated_zip, &guard).unwrap();
        let backups: BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join(SKILL_MANIFEST_FILE)).unwrap())
            .collect();
        assert!(backups.contains("source edit"));
        assert!(backups.contains("codex copy edit"));
    }

    #[test]
    fn update_refreshes_agent_copy_created_during_download() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let repo = guarded_repo(&manager);
        let old_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let old = zip_skill(&repo, &old_zip, "alpha");
        manager.install_from_zip(&old, &old_zip).unwrap();
        let guard = manager.capture_download_guard(&old.id, &repo.key(), false).unwrap();

        let copy = temp.path().join("claude-skills").join(&old.id);
        copy_dir_all(&manager.source_dir.join(&old.id), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        let new_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "updated")]);
        let updated = zip_skill(&repo, &new_zip, "alpha");
        manager.install_downloaded_zip_if_unchanged(&updated, &new_zip, &guard).unwrap();
        assert_eq!(std::fs::read_to_string(copy.join(SKILL_MANIFEST_FILE)).unwrap(), "updated");
        assert!(entry(&manager.local_inventory(), &old.id).agents.contains(&"claude".to_string()));
    }

    #[test]
    fn modified_agent_copy_prevents_update_and_can_be_backed_up_on_disable() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let repo = guarded_repo(&manager);
        let old_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "original")]);
        let old = zip_skill(&repo, &old_zip, "alpha");
        manager.install_from_zip(&old, &old_zip).unwrap();
        let guard = manager.capture_download_guard(&old.id, &repo.key(), false).unwrap();

        let copy = temp.path().join("claude-skills").join(&old.id);
        copy_dir_all(&manager.source_dir.join(&old.id), &copy).unwrap();
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        std::fs::write(copy.join(SKILL_MANIFEST_FILE), "agent edit").unwrap();
        let new_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "updated")]);
        let updated = zip_skill(&repo, &new_zip, "alpha");
        let error = manager.install_downloaded_zip_if_unchanged(&updated, &new_zip, &guard).unwrap_err();
        assert!(format!("{error:#}").contains("副本与受控源内容不一致"), "{error:#}");
        assert_eq!(std::fs::read_to_string(copy.join(SKILL_MANIFEST_FILE)).unwrap(), "agent edit");
        assert_eq!(std::fs::read_to_string(manager.source_dir.join(&old.id).join(SKILL_MANIFEST_FILE)).unwrap(), "original");
        assert_eq!(manager.load_state().installed[&old.id].content_hash, old.content_hash);
        manager.set_enabled_for("claude", &old.id, false).unwrap();
        assert!(!copy.exists());
        let backups: BTreeSet<_> = manager.list_backups().iter()
            .map(|backup| std::fs::read_to_string(manager.backups_dir.join(&backup.id).join(SKILL_MANIFEST_FILE)).unwrap())
            .collect();
        assert!(backups.contains("agent edit"));
        manager.install_downloaded_zip_if_unchanged(&updated, &new_zip, &guard).unwrap();
    }

    #[test]
    fn publishing_copy_does_not_replace_a_new_destination() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        write_skill(&source, "managed");
        write_skill(&target, "external");

        assert!(link_or_copy(&source, &target).is_err());
        assert_eq!(std::fs::read_to_string(target.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert_eq!(std::fs::read_to_string(source.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2);
    }

    #[test]
    fn publishing_does_not_remove_an_existing_staging_directory() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let staging = temp.path().join("staging");
        write_skill(&source, "managed");
        write_skill(&staging, "external");
        let mut published = None;

        assert!(link_or_copy_at(&source, &target, &staging, &mut published, || Ok(())).is_err());
        assert!(published.is_none());
        assert_eq!(std::fs::read_to_string(staging.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert!(!target.exists());
    }

    #[test]
    fn publication_failure_tracks_the_published_copy_without_cleaning_a_replaced_staging_path() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let staging = temp.path().join("staging");
        write_skill(&source, "managed");
        let mut published = None;

        let result = link_or_copy_at(&source, &target, &staging, &mut published, || {
            write_skill(&staging, "external");
            anyhow::bail!("injected failure after publication")
        });
        assert!(result.is_err());
        let identity = published.expect("published copy must be tracked before the failure");
        assert_eq!(std::fs::read_to_string(staging.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        let recovery = recover_published_path(&target, identity).unwrap().unwrap();
        assert!(!target.exists());
        assert_eq!(std::fs::read_to_string(recovery.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
    }

    #[test]
    fn moving_directory_does_not_replace_an_existing_target() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        write_skill(&source, "source");
        write_skill(&target, "target");

        assert!(move_path_no_replace(&source, &target).is_err());
        assert_eq!(std::fs::read_to_string(source.join(SKILL_MANIFEST_FILE)).unwrap(), "source");
        assert_eq!(std::fs::read_to_string(target.join(SKILL_MANIFEST_FILE)).unwrap(), "target");
    }

    #[test]
    fn rollback_does_not_move_a_replaced_published_path() {
        let temp = tempfile::tempdir().unwrap();
        let published = temp.path().join("published");
        let owned = temp.path().join("owned");
        write_skill(&published, "managed");
        let identity = skill_path_identity(&published).unwrap();
        move_path_no_replace(&published, &owned).unwrap();
        write_skill(&published, "external");

        assert!(recover_published_path(&published, identity).is_err());
        assert_eq!(std::fs::read_to_string(published.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert_eq!(std::fs::read_to_string(owned.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
    }

    #[test]
    fn rollback_recovers_published_content_on_its_own_volume() {
        let temp = tempfile::tempdir().unwrap();
        let published = temp.path().join("agent").join("published");
        write_skill(&published, "managed");
        let identity = skill_path_identity(&published).unwrap();

        let recovery = recover_published_path(&published, identity).unwrap().unwrap();
        assert_eq!(recovery.parent(), published.parent());
        assert!(!published.exists());
        assert_eq!(std::fs::read_to_string(recovery.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires SKILL_TEST_SECOND_VOLUME on a different volume"]
    fn rollback_recovers_cross_volume_agent_copy() {
        let local = tempfile::tempdir().unwrap();
        let second_volume = std::env::var_os("SKILL_TEST_SECOND_VOLUME").unwrap();
        let remote = tempfile::tempdir_in(second_volume).unwrap();
        assert_ne!(skill_path_identity(local.path()).unwrap().device, skill_path_identity(remote.path()).unwrap().device);
        let manager = manager(&local);
        let published = remote.path().join("published");
        write_skill(&published, "managed");
        let identity = skill_path_identity(&published).unwrap();

        let backup = manager.backup_directory("alpha", &published).unwrap();
        assert_eq!(backup.parent().unwrap(), remote.path().join(".codex-plus-backups"));
        restore_skill_backup(&backup, &published, identity).unwrap();
        let recovery = recover_published_path(&published, identity).unwrap().unwrap();
        assert_eq!(recovery.parent(), published.parent());
        assert_eq!(std::fs::read_to_string(recovery.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
    }

    #[test]
    fn checked_backup_rejects_a_replaced_path() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let source = temp.path().join("source");
        let previous = temp.path().join("previous");
        write_skill(&source, "managed");
        let identity = skill_path_identity(&source).unwrap();
        move_path_no_replace(&source, &previous).unwrap();
        write_skill(&source, "external");

        assert!(manager.backup_directory_checked("alpha", &source, identity).is_err());
        assert_eq!(std::fs::read_to_string(source.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert_eq!(std::fs::read_to_string(previous.join(SKILL_MANIFEST_FILE)).unwrap(), "managed");
    }

    #[test]
    fn new_download_does_not_displace_a_newly_created_source() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let source = manager.source_dir.join("alpha");
        let staging = temp.path().join("staging");
        write_skill(&source, "external");
        write_skill(&staging, "download");
        let installed = InstalledSkill {
            id: "alpha".into(),
            name: "Alpha".into(),
            description: String::new(),
            repo_key: "acme/kit@main".into(),
            repo_path: "alpha".into(),
            content_hash: "hash".into(),
            installed_at: "1".into(),
        };

        let mut coordination = skills_coordination();
        assert!(manager.commit_staged_skill_unlocked(
            &staging, installed, true, true, None, true, &mut coordination,
        ).is_err());
        assert_eq!(std::fs::read_to_string(source.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
        assert!(!manager.state_path.exists());
    }

    #[test]
    fn new_download_rejects_an_existing_managed_agent_copy() {
        let temp = tempfile::tempdir().unwrap();
        let manager = multi_agent_manager(&temp);
        let copy = temp.path().join("claude-skills/alpha");
        let staging = temp.path().join("staging");
        write_skill(&copy, "existing agent copy");
        std::fs::write(copy.join(MANAGED_COPY_MARKER), "").unwrap();
        write_skill(&staging, "download");
        let installed = InstalledSkill {
            id: "alpha".into(),
            name: "Alpha".into(),
            description: String::new(),
            repo_key: "acme/kit@main".into(),
            repo_path: "alpha".into(),
            content_hash: "hash".into(),
            installed_at: "1".into(),
        };

        let mut coordination = skills_coordination();
        assert!(manager.commit_staged_skill_unlocked(
            &staging, installed, true, true, None, true, &mut coordination,
        ).is_err());
        assert_eq!(std::fs::read_to_string(copy.join(SKILL_MANIFEST_FILE)).unwrap(), "existing agent copy");
        assert!(!manager.source_dir.join("alpha").exists());
    }

    #[test]
    fn rollback_does_not_restore_a_replaced_backup() {
        let temp = tempfile::tempdir().unwrap();
        let backup = temp.path().join("backup");
        let previous = temp.path().join("previous");
        let original = temp.path().join("original");
        write_skill(&backup, "managed");
        let identity = skill_path_identity(&backup).unwrap();
        move_path_no_replace(&backup, &previous).unwrap();
        write_skill(&backup, "external");

        assert!(restore_skill_backup(&backup, &original, identity).is_err());
        assert!(!original.exists());
        assert_eq!(std::fs::read_to_string(backup.join(SKILL_MANIFEST_FILE)).unwrap(), "external");
    }

    #[cfg(windows)]
    #[test]
    fn moving_directory_supports_long_windows_paths() {
        let temp = tempfile::tempdir().unwrap();
        let mut parent = temp.path().to_path_buf();
        for _ in 0..8 {
            parent.push("long-directory-name-for-skill-test");
        }
        std::fs::create_dir_all(&parent).unwrap();
        let source = parent.join("source");
        let target = parent.join("target");
        assert!(source.to_string_lossy().len() > 260);
        write_skill(&source, "source");

        move_path_no_replace(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(target.join(SKILL_MANIFEST_FILE)).unwrap(), "source");
    }

    #[test]
    fn shared_source_with_different_state_paths_invalidates_old_download() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = SkillsManager::new(
            first.source_dir.clone(), first.backups_dir.clone(),
            temp.path().join("other-skills.json"), first.codex_home.clone(),
        );
        let repo = guarded_repo(&first);
        second.upsert_repo(repo.clone()).unwrap();
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "same")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        first.install_from_zip(&skill, &zip).unwrap();
        let guard = first.capture_download_guard(&skill.id, &repo.key(), false).unwrap();
        second.install_from_zip(&skill, &zip).unwrap();
        assert!(first.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).is_err());
    }

    #[test]
    fn repository_fallback_guard_rejects_toggles_and_delete_readd() {
        let temp = tempfile::tempdir().unwrap();
        let first = manager(&temp);
        let second = manager(&temp);
        let mut repo = guarded_repo(&first);
        let mut resolved = repo.clone();
        resolved.branch = "master".into();
        let guard = first.capture_repo_guard(&repo.key()).unwrap();
        assert!(first.repo_guard_is_current(&guard).unwrap());
        repo.enabled = false;
        second.update_repo_enabled_if_unchanged(&repo, None).unwrap();
        assert!(!first.repo_guard_is_current(&guard).unwrap());
        repo.enabled = true;
        second.update_repo_enabled_if_unchanged(&repo, None).unwrap();
        assert!(!first.repo_guard_is_current(&guard).unwrap());
        assert!(!first.save_resolved_repo_if_unchanged(&guard, resolved.clone()).unwrap());

        let guard = first.capture_repo_guard(&repo.key()).unwrap();
        assert!(first.repo_guard_is_current(&guard).unwrap());
        second.delete_repo(&repo.key()).unwrap();
        second.upsert_repo(repo.clone()).unwrap();
        assert!(!first.repo_guard_is_current(&guard).unwrap());
        assert!(!first.save_resolved_repo_if_unchanged(&guard, resolved.clone()).unwrap());

        let guard = first.capture_repo_guard(&repo.key()).unwrap();
        assert!(first.save_resolved_repo_if_unchanged(&guard, resolved.clone()).unwrap());
        assert!(!first.list_repos().iter().any(|item| item.key() == repo.key()));
        assert!(first.list_repos().iter().any(|item| item.key() == resolved.key()));
    }

    #[test]
    fn guarded_install_requires_matching_mode_path_and_zip_version() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let repo = guarded_repo(&manager);
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "one")]);
        let skill = zip_skill(&repo, &zip, "alpha");
        assert!(manager.capture_download_guard(&skill.id, &repo.key(), false).is_err());
        let guard = manager.capture_download_guard(&skill.id, &repo.key(), true).unwrap();
        let other = repo_zip(&[("kit-main/alpha/SKILL.md", "different")]);
        assert!(manager.install_downloaded_zip_if_unchanged(&skill, &other, &guard).is_err());
        manager.install_downloaded_zip_if_unchanged(&skill, &zip, &guard).unwrap();
        assert!(manager.capture_download_guard(&skill.id, &repo.key(), true).is_err());
    }

    fn without_github_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
    }

    const FIXTURE_GITHUB_AUTH: &str = "token fixture-github-token";

    fn with_fixture_github_auth(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request.header(reqwest::header::AUTHORIZATION, FIXTURE_GITHUB_AUTH)
    }

    #[tokio::test]
    async fn github_tree_body_timeout_falls_back_to_zip() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stalled = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"tree\":")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        });
        let server = MockServer::builder().disable_request_recording().start().await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                "kit-main/alpha/SKILL.md",
                "---\nname: Alpha\n---\n",
            )])))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let skills = fetch_repo_skills_with_client_timeout(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("http://{address}/tree"),
            &format!("{}/zip", server.uri()),
            without_github_auth,
            std::time::Duration::from_millis(200),
        )
        .await
        .unwrap();
        stalled.await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Alpha");
    }

    #[tokio::test]
    async fn github_tree_body_timeout_and_missing_zip_try_alternative_branch() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for zip_status in [404, 502] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let stalled = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 2048];
                socket.read(&mut request).await.unwrap();
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"tree\":")
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(800)).await;
            });
            let server = MockServer::builder().disable_request_recording().start().await;
            Mock::given(method("GET"))
                .and(path("/tree-master"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"truncated": true})))
                .expect(u64::from(zip_status == 404))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/main"))
                .respond_with(ResponseTemplate::new(zip_status))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/master"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                    "kit-master/alpha/SKILL.md",
                    "---\nname: Alpha\n---\n",
                )])))
                .expect(u64::from(zip_status == 404))
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let base = server.uri();
            let attempts = AtomicUsize::new(0);
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let result = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
                attempts.fetch_add(1, Ordering::SeqCst);
                let client = client.clone();
                let base = base.clone();
                Box::pin(async move {
                    let tree_url = if repo.branch == "main" {
                        format!("http://{address}/tree?fixture=tree-secret")
                    } else {
                        format!("{base}/tree-master")
                    };
                    fetch_repo_skills_with_client_timeout(
                        repo,
                        cached,
                        &client,
                        &tree_url,
                        &format!("{base}/{}", repo.branch),
                        without_github_auth,
                        Duration::from_millis(200),
                    )
                    .await
                })
            })
            .await;
            if zip_status == 404 {
                let (resolved, skills) = result.unwrap();
                assert_eq!(resolved.branch, "master");
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0].name, "Alpha");
                assert_eq!(attempts.load(Ordering::SeqCst), 2);
            } else {
                let error = result.unwrap_err();
                let message = format!("{error:#}");
                assert!(message.contains("文件树响应超时"));
                assert!(message.contains("HTTP 502"));
                assert!(!message.contains("tree-secret"));
                assert!(!message.contains(&address.to_string()));
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
            }
            stalled.await.unwrap();
        }
    }

    #[tokio::test]
    async fn github_tree_body_interruption_and_missing_zip_do_not_change_branch() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let interrupted = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{\"tree\":")
                .await
                .unwrap();
        });
        let server = MockServer::builder().disable_request_recording().start().await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = server.uri();
        let attempts = AtomicUsize::new(0);
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let error = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
            attempts.fetch_add(1, Ordering::SeqCst);
            let client = client.clone();
            let base = base.clone();
            Box::pin(async move {
                fetch_repo_skills_with_client(
                    repo,
                    cached,
                    &client,
                    &format!("http://{address}/tree"),
                    &format!("{base}/zip"),
                    without_github_auth,
                )
                .await
            })
        })
        .await
        .unwrap_err();
        interrupted.await.unwrap();
        assert!(format!("{error:#}").contains("解析 GitHub 文件树响应失败"));
        assert!(format!("{error:#}").contains("HTTP 404"));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn github_tree_invalid_response_falls_back_but_empty_tree_is_valid() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for body in ["{", "{}", r#"{"tree":null}"#] {
            let server = MockServer::builder().disable_request_recording().start().await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                    "kit-main/alpha/SKILL.md",
                    "---\nname: Alpha\n---\n",
                )])))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let skills = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await
            .unwrap();
            assert_eq!(skills.len(), 1);
            assert_eq!(skills[0].name, "Alpha");
        }

        let server = MockServer::builder().disable_request_recording().start().await;
        Mock::given(method("GET"))
            .and(path("/tree"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": []})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        assert!(
            fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await
            .unwrap()
            .is_empty()
        );
    }

    #[tokio::test]
    async fn github_rate_limit_and_missing_main_zip_try_master() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for status in [403, 429] {
            let server = MockServer::builder().disable_request_recording().start().await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(status))
                .expect(2)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/main"))
                .respond_with(ResponseTemplate::new(404))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/master"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                    "kit-master/alpha/SKILL.md",
                    "---\nname: Alpha\n---\n",
                )])))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let base = server.uri();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let (resolved, skills) = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
                let client = client.clone();
                let base = base.clone();
                Box::pin(async move {
                    fetch_repo_skills_with_client(
                        repo,
                        cached,
                        &client,
                        &format!("{base}/tree"),
                        &format!("{base}/{}", repo.branch),
                        without_github_auth,
                    )
                    .await
                })
            })
            .await
            .unwrap();
            assert_eq!(resolved.branch, "master");
            assert_eq!(skills.len(), 1);
        }
    }

    #[tokio::test]
    async fn github_truncated_tree_and_missing_main_zip_try_master() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::builder().disable_request_recording().start().await;
        Mock::given(method("GET"))
            .and(path("/tree"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"truncated": true})))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/main"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/master"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                "kit-master/alpha/SKILL.md",
                "---\nname: Alpha\n---\n",
            )])))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = server.uri();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let (resolved, skills) = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
            let client = client.clone();
            let base = base.clone();
            Box::pin(async move {
                fetch_repo_skills_with_client(
                    repo,
                    cached,
                    &client,
                    &format!("{base}/tree"),
                    &format!("{base}/{}", repo.branch),
                    without_github_auth,
                )
                .await
            })
        })
        .await
        .unwrap();
        assert_eq!(resolved.branch, "master");
        assert_eq!(skills.len(), 1);
    }

    #[tokio::test]
    async fn github_zip_body_idle_timeout_is_bounded() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nz")
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client.get(format!("http://{address}/zip")).send().await.unwrap();
        let error = read_repo_zip_response_with_timeout(
            response,
            std::time::Duration::from_millis(200),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(format!("{error:#}").contains("等待仓库压缩包数据超时"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn github_zip_body_total_timeout_bounds_continuous_transfer() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let chunks_sent = Arc::new(AtomicUsize::new(0));
        let server_chunks_sent = Arc::clone(&chunks_sent);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nz")
                .await
                .unwrap();
            server_chunks_sent.fetch_add(1, Ordering::SeqCst);
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_millis(70)).await;
                if socket.write_all(b"z").await.is_err() {
                    break;
                }
                server_chunks_sent.fetch_add(1, Ordering::SeqCst);
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let response = client.get(format!("http://{address}/zip")).send().await.unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            read_repo_zip_response_with_timeout(
                response,
                Duration::from_millis(400),
                Duration::from_millis(900),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(format!("{error:#}").contains("仓库压缩包下载总时间超限"));
        assert!(chunks_sent.load(Ordering::SeqCst) >= 4);
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn github_zip_response_retains_declared_and_streaming_size_limits() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for chunked in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 2048];
                socket.read(&mut request).await.unwrap();
                if chunked {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .await
                        .unwrap();
                    let chunk = vec![0u8; 1024 * 1024];
                    for _ in 0..65 {
                        if socket.write_all(b"100000\r\n").await.is_err()
                            || socket.write_all(&chunk).await.is_err()
                            || socket.write_all(b"\r\n").await.is_err()
                        {
                            break;
                        }
                    }
                } else {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                        REPO_ZIP_DOWNLOAD_LIMIT_BYTES + 1
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                }
            });
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let response = client.get(format!("http://{address}/zip")).send().await.unwrap();
            let error = read_repo_zip_response_with_timeout(
                response,
                Duration::from_secs(5),
                Duration::from_secs(20),
            )
            .await
            .unwrap_err();
            let message = format!("{error:#}");
            assert!(message.contains("仓库压缩包超过 64 MiB"));
            assert_eq!(message.contains("已停止下载"), chunked);
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn github_zip_request_overrides_short_client_timeout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let archive = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: Alpha\n---\n")]);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 2048];
            socket.read(&mut request).await.unwrap();
            let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", archive.len());
            socket.write_all(header.as_bytes()).await.unwrap();
            let middle = archive.len() / 2;
            socket.write_all(&archive[..middle]).await.unwrap();
            tokio::time::sleep(Duration::from_millis(350)).await;
            socket.write_all(&archive[middle..]).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let skills = fetch_repo_skills_from_zip(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("http://{address}/zip"),
            without_github_auth,
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Alpha");
    }

    #[tokio::test]
    async fn github_missing_tree_tries_alternative_branch() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::builder().disable_request_recording().start().await;
        Mock::given(method("GET"))
            .and(path("/main"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/master"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": []})))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let base = server.uri();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let (resolved, skills) = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
            let client = client.clone();
            let base = base.clone();
            Box::pin(async move {
                fetch_repo_skills_with_client(
                    repo,
                    cached,
                    &client,
                    &format!("{base}/{}", repo.branch),
                    &format!("{base}/zip"),
                    without_github_auth,
                )
                .await
            })
        })
        .await
        .unwrap();
        assert_eq!(resolved.branch, "master");
        assert!(skills.is_empty());
    }

    #[tokio::test]
    async fn github_invalid_tree_and_missing_zip_do_not_change_branch() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for body in ["{", "{}", r#"{"tree":null}"#] {
            let server = MockServer::builder().disable_request_recording().start().await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(404))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let base = server.uri();
            let attempts = AtomicUsize::new(0);
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let error = resolve_repo_skills_with(repo, &BTreeMap::new(), |repo, cached| {
                attempts.fetch_add(1, Ordering::SeqCst);
                let client = client.clone();
                let base = base.clone();
                Box::pin(async move {
                    fetch_repo_skills_with_client(
                        repo,
                        cached,
                        &client,
                        &format!("{base}/tree"),
                        &format!("{base}/zip"),
                        without_github_auth,
                    )
                    .await
                })
            })
            .await
            .unwrap_err();
            assert!(format!("{error:#}").contains("解析 GitHub 文件树响应失败"));
            assert!(format!("{error:#}").contains("HTTP 404"));
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn github_tree_zip_mock_requests_use_only_explicit_auth() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let cases: [(GithubAuth, Option<&str>); 2] = [
            (without_github_auth, None),
            (with_fixture_github_auth, Some(FIXTURE_GITHUB_AUTH)),
        ];
        for (authorize, expected) in cases {
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            let tree = Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(403))
                .expect(1)
                .mount_as_scoped(&server)
                .await;
            let zip = Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(502))
                .expect(1)
                .mount_as_scoped(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let error = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                authorize,
            )
            .await
            .unwrap_err();
            assert!(format!("{error:#}").contains("HTTP 403"));
            assert!(server.received_requests().await.is_none());
            let tree_requests = tree.received_requests().await;
            let zip_requests = zip.received_requests().await;
            assert_eq!(tree_requests.len(), 1);
            assert_eq!(zip_requests.len(), 1);
            for request in tree_requests.iter().chain(&zip_requests) {
                let authorization = request
                    .headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok());
                assert!(authorization == expected, "unexpected mock authorization");
            }
        }
    }

    #[tokio::test]
    async fn github_tree_zip_mock_failure_does_not_print_request_headers() {
        use futures_util::FutureExt;
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::builder()
            .disable_request_recording()
            .start()
            .await;
        Mock::given(method("GET"))
            .and(path("/tree"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(502))
            .expect(2)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let _error = fetch_repo_skills_with_client(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("{}/tree", server.uri()),
            &format!("{}/zip", server.uri()),
            with_fixture_github_auth,
        )
        .await
        .unwrap_err();
        let failure = std::panic::AssertUnwindSafe(server.verify())
            .catch_unwind()
            .await;
        server.reset().await;
        let payload = failure.err().expect("expected a mock verification failure");
        let message = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or_default();
        assert!(message.contains("Enable request recording"));
        assert!(!message.contains(FIXTURE_GITHUB_AUTH));
        assert!(!message.to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn github_tree_zip_tree_success_401_and_404_do_not_fallback() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for status in [200, 401, 404] {
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            let tree = json!({"tree": [{"type": "blob", "path": "alpha/SKILL.md", "sha": "abc"}]});
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(status).set_body_json(tree.clone()))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let mut cached_skill = parse_skills_from_tree(&repo, &tree).remove(0);
            cached_skill.name = "Cached alpha".into();
            cached_skill.description = "Cached description".into();
            let cached = BTreeMap::from([(cached_skill.id.clone(), cached_skill)]);
            let result = fetch_repo_skills_with_client(
                &repo,
                &cached,
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await;
            if status == 200 {
                let skills = result.unwrap();
                assert_eq!(skills.len(), 1);
                assert_eq!(skills[0].name, "Cached alpha");
            } else {
                let error = result.unwrap_err();
                assert!(format!("{error:#}").contains(&format!("HTTP {status}")));
                assert_eq!(format!("{error:#}").contains("HTTP 404"), status == 404);
            }
        }
    }

    #[tokio::test]
    async fn github_tree_zip_status_failures_can_fallback_successfully() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for status in [403, 429, 500, 503, 599] {
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                    "kit-main/alpha/SKILL.md",
                    "---\nname: Alpha\ndescription: Example\n---\n",
                )])))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let skills = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await
            .unwrap();
            assert_eq!(skills.len(), 1);
            assert_eq!(skills[0].name, "Alpha");
        }
    }

    #[tokio::test]
    async fn github_tree_zip_double_status_failure_keeps_both_and_zip_404_semantics() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for (tree_status, zip_status) in [
            (403, 502),
            (403, 404),
            (429, 503),
            (429, 404),
            (500, 502),
            (500, 404),
            (599, 502),
            (599, 404),
        ] {
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(tree_status))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(zip_status))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let error = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await
            .unwrap_err();
            let message = format!("{error:#}");
            assert!(
                message.contains(&format!("文件树请求失败：HTTP {tree_status}")),
                "{message}"
            );
            assert!(
                message.contains(&format!("压缩包失败：HTTP {zip_status}")),
                "{message}"
            );
            assert_eq!(message.contains("HTTP 404"), zip_status == 404);
            let chain: Vec<_> = error.chain().map(ToString::to_string).collect();
            assert_eq!(chain.len(), 2);
            assert!(chain[0].contains(&format!("文件树请求失败：HTTP {tree_status}")));
            assert!(chain[1].contains(&format!("压缩包失败：HTTP {zip_status}")));
        }
    }

    #[tokio::test]
    async fn github_tree_zip_invalid_archive_keeps_both_causes() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::builder()
            .disable_request_recording()
            .start()
            .await;
        Mock::given(method("GET"))
            .and(path("/tree"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not a zip".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let error = fetch_repo_skills_with_client(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("{}/tree", server.uri()),
            &format!("{}/zip", server.uri()),
            without_github_auth,
        )
        .await
        .unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("HTTP 503"), "{message}");
        assert!(message.contains("压缩包无法解析"), "{message}");
        assert!(error.chain().count() >= 3);
        assert!(error.downcast_ref::<zip::result::ZipError>().is_some());
    }

    #[tokio::test]
    async fn github_tree_zip_transport_failure_keeps_original_context() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for zip_status in [404, 502] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let closer = tokio::spawn(async move {
                drop(listener.accept().await.unwrap());
            });
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            Mock::given(method("GET"))
                .and(path("/zip"))
                .respond_with(ResponseTemplate::new(zip_status))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let tree_url = format!("http://{address}/tree");
            let error = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &tree_url,
                &format!("{}/zip", server.uri()),
                without_github_auth,
            )
            .await
            .unwrap_err();
            closer.await.unwrap();
            let context = error.chain().next().unwrap().to_string();
            let message = format!("{error:#}");
            assert!(
                context.contains("文件树请求失败（error sending request"),
                "{context}"
            );
            assert!(context.contains(&tree_url), "{context}");
            assert!(message.contains(&format!("HTTP {zip_status}")), "{message}");
            assert_eq!(message.contains("HTTP 404"), zip_status == 404);
            assert!(error.downcast_ref::<reqwest::Error>().is_none());
        }
    }

    #[tokio::test]
    async fn github_tree_zip_transport_failure_can_fallback_successfully() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let closer = tokio::spawn(async move {
            drop(listener.accept().await.unwrap());
        });
        let server = MockServer::builder()
            .disable_request_recording()
            .start()
            .await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                "kit-main/alpha/SKILL.md",
                "---\nname: Alpha\n---\n",
            )])))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let skills = fetch_repo_skills_with_client(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("http://{address}/tree"),
            &format!("{}/zip", server.uri()),
            without_github_auth,
        )
        .await
        .unwrap();
        closer.await.unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Alpha");
    }

    #[tokio::test]
    async fn github_tree_zip_status_and_zip_transport_failure_preserve_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        for partial_body in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let closer = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                if partial_body {
                    let mut buffer = [0u8; 2048];
                    let _ = socket.read(&mut buffer).await.unwrap();
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
                        .await
                        .unwrap();
                }
            });
            let server = MockServer::builder()
                .disable_request_recording()
                .start()
                .await;
            Mock::given(method("GET"))
                .and(path("/tree"))
                .respond_with(ResponseTemplate::new(429))
                .expect(1)
                .mount(&server)
                .await;
            let client = reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(3))
                .build()
                .unwrap();
            let repo = parse_repo_key("acme/kit@main").unwrap();
            let zip_url = format!("http://{address}/zip");
            let error = fetch_repo_skills_with_client(
                &repo,
                &BTreeMap::new(),
                &client,
                &format!("{}/tree", server.uri()),
                &zip_url,
                without_github_auth,
            )
            .await
            .unwrap_err();
            closer.await.unwrap();
            let message = format!("{error:#}");
            assert!(message.contains("HTTP 429"), "{message}");
            let expected = if partial_body {
                "读取仓库压缩包内容失败"
            } else {
                "下载 acme/kit 的仓库压缩包失败"
            };
            assert!(message.contains(expected), "{message}");
            let zip_error = error.downcast_ref::<reqwest::Error>().unwrap();
            assert!(std::error::Error::source(zip_error).is_some());
            if !partial_body {
                assert_eq!(zip_error.url().unwrap().as_str(), zip_url);
            }
            let contexts: Vec<_> = error.chain().map(ToString::to_string).collect();
            assert!(contexts[0].contains("文件树请求失败：HTTP 429"));
            assert!(contexts[1].contains(expected));
        }
    }

    #[tokio::test]
    async fn github_tree_zip_truncated_tree_still_falls_back() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let server = MockServer::builder()
            .disable_request_recording()
            .start()
            .await;
        Mock::given(method("GET"))
            .and(path("/tree"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"truncated": true})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/zip"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(repo_zip(&[(
                "kit-main/alpha/SKILL.md",
                "---\nname: Alpha\n---\n",
            )])))
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let skills = fetch_repo_skills_with_client(
            &repo,
            &BTreeMap::new(),
            &client,
            &format!("{}/tree", server.uri()),
            &format!("{}/zip", server.uri()),
            without_github_auth,
        )
        .await
        .unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "Alpha");
    }

    #[tokio::test]
    async fn github_tree_zip_status_context_does_not_include_proxy_or_response_secrets() {
        use base64::Engine as _;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let proxy_secret =
            base64::engine::general_purpose::STANDARD.encode("proxy-user:proxy-password");
        let proxy_header = format!(
            "proxy-authorization: basic {}",
            proxy_secret.to_ascii_lowercase()
        );
        let proxy = tokio::spawn(async move {
            for status in ["403 Forbidden", "502 Bad Gateway"] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                let headers = String::from_utf8_lossy(&request).to_ascii_lowercase();
                assert!(headers.contains(&proxy_header));
                assert!(
                    !headers
                        .lines()
                        .any(|line| line.starts_with("authorization:"))
                );
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: 0\r\nX-Private: response-secret\r\nConnection: close\r\n\r\n"
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = reqwest::Client::builder()
            .proxy(
                reqwest::Proxy::all(format!("http://proxy-user:proxy-password@{address}")).unwrap(),
            )
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let error = fetch_repo_skills_with_client(
            &repo,
            &BTreeMap::new(),
            &client,
            "http://example.invalid/tree",
            "http://example.invalid/zip",
            without_github_auth,
        )
        .await
        .unwrap_err();
        tokio::time::timeout(std::time::Duration::from_secs(5), proxy)
            .await
            .unwrap()
            .unwrap();
        let message = format!("{error:#}");
        assert!(message.contains("HTTP 403"));
        assert!(message.contains("HTTP 502"));
        for rendered in [
            format!("{error}"),
            message,
            format!("{error:?}"),
            format!("{error:#?}"),
        ] {
            for secret in [
                "proxy-user",
                "proxy-password",
                "response-secret",
                proxy_secret.as_str(),
            ] {
                assert!(!rendered.contains(secret));
            }
        }
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
    fn local_manifest_does_not_decode_large_body() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(b"---\nname: Sample\ndescription: Metadata\n---\n")
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        file.write_all(&vec![b'x'; 512 * 1024]).unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("Sample".into(), "Metadata".into())
        );
    }

    #[test]
    fn local_manifest_preserves_invalid_utf8_fallback_for_short_body() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(b"---\nname: Sample\n---\ninvalid: ")
            .unwrap();
        file.write_all(&[0xff]).unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("fallback".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_reads_at_most_budget_plus_probe_byte() {
        struct CountingReader<'a> {
            bytes: &'a [u8],
            count: &'a mut usize,
        }

        impl Read for CountingReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let count = self.bytes.read(buffer)?;
                *self.count += count;
                Ok(count)
            }
        }

        let mut bytes = b"---\nname: Sample\n---\n".to_vec();
        bytes.resize(SKILL_FRONTMATTER_LIMIT_BYTES * 2, b'x');
        let mut count = 0;
        let text = read_bounded_skill_manifest(CountingReader {
            bytes: &bytes,
            count: &mut count,
        })
        .unwrap();

        assert_eq!(count, SKILL_FRONTMATTER_LIMIT_BYTES + 1);
        assert_eq!(parse_skill_frontmatter(&text, "fallback").0, "Sample");
    }

    #[test]
    fn local_manifest_preserves_crlf_and_multibyte_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        std::fs::write(&manifest, "---\r\nname: Caf\u{e9}\r\ndescription: Example\r\n---\r\nbody")
            .unwrap();

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("Caf\u{e9}".into(), "Example".into())
        );
    }

    #[test]
    fn local_manifest_falls_back_when_frontmatter_exceeds_budget() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(b"---\nname: Partial\nother: ").unwrap();
        file.write_all(&vec![b'x'; 256 * 1024]).unwrap();
        file.write_all(b"\n---\n").unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("fallback".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_preserves_short_unclosed_frontmatter() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        std::fs::write(&manifest, "---\nname: Incomplete\n").unwrap();

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("Incomplete".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_accepts_frontmatter_at_budget_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let prefix = b"---\nname: Boundary\nother: ";
        let suffix = b"\n---\n";
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(&vec![b'x'; SKILL_FRONTMATTER_LIMIT_BYTES - prefix.len() - suffix.len()])
            .unwrap();
        file.write_all(suffix).unwrap();
        file.write_all(b"body after boundary").unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("Boundary".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_falls_back_on_utf8_split_at_budget() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let prefix = b"---\nname: Partial\nother: ";
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(&vec![b'x'; SKILL_FRONTMATTER_LIMIT_BYTES - prefix.len()])
            .unwrap();
        file.write_all(&[0xc3, 0xa9]).unwrap();
        file.write_all(b"\n---\n").unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("fallback".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_accepts_large_crlf_body_with_utf8_split_at_budget() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let prefix = "---\r\nname: Caf\u{e9}\r\n---\r\n".as_bytes();
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(&vec![b'x'; SKILL_FRONTMATTER_LIMIT_BYTES - 1 - prefix.len()])
            .unwrap();
        file.write_all(&[0xc3, 0xa9]).unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("Caf\u{e9}".into(), String::new())
        );
    }

    #[test]
    fn local_manifest_rejects_truncated_delimiter_at_budget() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = temp.path().join("SKILL.md");
        let prefix = b"---\nname: Partial\nother: ";
        let suffix = b"\n---";
        let mut file = std::fs::File::create(&manifest).unwrap();
        file.write_all(prefix).unwrap();
        file.write_all(&vec![b'x'; SKILL_FRONTMATTER_LIMIT_BYTES - prefix.len() - suffix.len()])
            .unwrap();
        file.write_all(suffix).unwrap();
        file.write_all(b"x\n").unwrap();
        drop(file);

        assert_eq!(
            read_skill_manifest(&manifest, "fallback"),
            ("fallback".into(), String::new())
        );
    }

    #[cfg(windows)]
    #[test]
    fn local_inventory_reads_manifest_through_directory_junction() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write_skill(&source, "---\nname: Shared\ndescription: Linked\n---\nbody");
        let skills = temp.path().join("codex-home").join("skills");
        std::fs::create_dir_all(&skills).unwrap();
        let link = skills.join("shared");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&source)
            .output()
            .unwrap();
        assert!(output.status.success(), "could not create directory junction: {:?}", output);

        let entries = manager(&temp).local_inventory();
        assert_eq!(entry(&entries, "shared").name, "Shared");
        assert_eq!(entry(&entries, "shared").description, "Linked");
        std::fs::remove_dir(link).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn local_manifest_reads_through_shared_directory_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        write_skill(&source, "---\nname: Shared\ndescription: Linked\n---\nbody");
        let link = temp.path().join("linked");
        std::os::unix::fs::symlink(&source, &link).unwrap();

        assert_eq!(
            read_skill_manifest(&link.join("SKILL.md"), "fallback"),
            ("Shared".into(), "Linked".into())
        );
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
    fn mixed_root_zip_is_rejected_by_both_catalog_and_extraction() {
        let temp = tempfile::tempdir().unwrap();
        let repo = parse_repo_key("acme/kit@main").unwrap();
        for files in [
            [("good-main/alpha/SKILL.md", "good"), ("evil-main/alpha/SKILL.md", "evil")],
            [("evil-main/alpha/SKILL.md", "evil"), ("good-main/alpha/SKILL.md", "good")],
        ] {
            let zip = repo_zip(&files);
            let error = parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).unwrap_err();
            assert!(format!("{error:#}").contains("多个根"));
            let destination = temp.path().join("rejected");
            let error = extract_skill_subtree(&zip, "alpha", &destination).unwrap_err();
            assert!(format!("{error:#}").contains("多个根"));
            assert!(!destination.exists());
        }
    }

    #[test]
    fn zip_paths_that_normalize_to_a_different_tree_are_rejected() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "good"),
            ("kit-main/alpha/./SKILL.md", "different"),
        ]);
        assert!(parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).is_err());
        let temp = tempfile::tempdir().unwrap();
        assert!(extract_skill_subtree(&zip, "alpha", temp.path()).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_zip_and_skill_name_aliases_are_rejected() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "good"),
            ("kit-main/alpha/skill.md", "different"),
        ]);
        assert!(parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).is_ok());
        let temp = tempfile::tempdir().unwrap();
        assert!(extract_skill_subtree(&zip, "alpha", temp.path()).is_err());
        let manager = manager(&temp);
        write_skill(&manager.source_dir.join("Existing"), "original");
        assert!(manager.set_enabled("existing", false).is_err());
        if std::fs::symlink_metadata(manager.source_dir.join("EXISTI~1")).is_ok() {
            assert!(manager.set_enabled("EXISTI~1", false).is_err());
        }
        assert!(manager.source_dir.join("Existing/SKILL.md").is_file());
    }

    #[cfg(windows)]
    #[test]
    fn unrelated_tilde_names_do_not_block_skill_operations_or_extraction() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let mut state = SkillsState::default();
        state.installed.insert(
            "notes~old".into(),
            InstalledSkill {
                id: "notes~old".into(),
                name: "notes~old".into(),
                description: String::new(),
                repo_key: "acme/kit@main".into(),
                repo_path: "notes~old".into(),
                content_hash: "old".into(),
                installed_at: String::new(),
            },
        );
        manager.save_state_unlocked(&state).unwrap();
        assert!(manager.set_enabled("unrelated", false).is_ok());
        assert!(validate_skill_id("notes~old").is_ok());

        let repo = parse_repo_key("acme/kit@main").unwrap();
        let zip = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "good"),
            ("kit-main/notes~1/document.txt", "unrelated"),
        ]);
        assert!(parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).is_ok());
        extract_skill_subtree(&zip, "alpha", &temp.path().join("extracted")).unwrap();

        let ambiguous = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "good"),
            ("kit-main/alpha/NOTES~1.txt", "ambiguous"),
        ]);
        assert!(extract_skill_subtree(&ambiguous, "alpha", &temp.path().join("rejected")).is_err());
    }

    #[test]
    fn zip_read_enforces_actual_and_cumulative_uncompressed_sizes() {
        let mut total = 0;
        let mut first = Cursor::new(b"abc");
        assert_eq!(read_bounded_zip_entry(&mut first, 3, &mut total, 4).unwrap(), b"abc");
        let mut second = Cursor::new(b"de");
        assert!(read_bounded_zip_entry(&mut second, 2, &mut total, 4).is_err());
        let mut forged = Cursor::new(b"extra");
        assert!(read_bounded_zip_entry(&mut forged, 1, &mut total, 4).is_err());
        assert_eq!(total, 3);

        let repo = parse_repo_key("acme/kit@main").unwrap();
        let mut zip = repo_zip(&[("kit-main/alpha/SKILL.md", "actual content")]);
        let central = zip.windows(4).position(|bytes| bytes == b"PK\x01\x02").unwrap();
        zip[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());
        assert!(parse_skills_from_zip(&repo, &zip, &BTreeMap::new()).is_err());
        let temp = tempfile::tempdir().unwrap();
        assert!(extract_skill_subtree(&zip, "alpha", temp.path()).is_err());
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
