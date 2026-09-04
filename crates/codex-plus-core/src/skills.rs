//! Codex Skills 管理。
//!
//! codex 的 skill 是文件系统约定，不是配置项：它扫描 `$CODEX_HOME/skills/<id>/SKILL.md`，
//! 从 YAML frontmatter 读 `name` / `description`，然后把清单注入 `<skills_instructions>`。
//! `config.toml` 里的 `[skills]` 是一个三字段结构体（bundled / include_instructions /
//! max_context_tokens），写 `[skills.<id>]` 会被 serde 当未知字段静默丢弃，什么都不会发生。
//!
//! 所以这里采用「SSOT + 软链」模型（与 cc-switch 一致）：
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
use std::path::{Path, PathBuf};
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
/// 用的就是它），后三个与 cc-switch 预置的 `skill_repos` 重合。
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
            state_lock: Arc::new(Mutex::new(())),
        }
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
        let repo = normalize_repo(repo)?;
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        let key = repo.key();
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
        let mut entries: BTreeMap<String, SkillEntry> = BTreeMap::new();

        for skill in remote {
            let installed = state.installed.get(&skill.id);
            entries.insert(
                skill.id.clone(),
                SkillEntry {
                    id: skill.id.clone(),
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    repo_key: skill.repo_key.clone(),
                    repo_path: skill.repo_path.clone(),
                    installed: installed.is_some(),
                    enabled: is_linked(&linked.join(&skill.id)),
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
                },
            );
        }

        // 已安装但远端清单里没有的（仓库删了、或本地手工放的），仍要能看见和卸载。
        for (id, installed) in &state.installed {
            entries.entry(id.clone()).or_insert_with(|| SkillEntry {
                id: id.clone(),
                name: if installed.name.is_empty() {
                    id.clone()
                } else {
                    installed.name.clone()
                },
                description: installed.description.clone(),
                repo_key: installed.repo_key.clone(),
                repo_path: String::new(),
                installed: true,
                enabled: is_linked(&linked.join(id)),
                bundled: false,
                content_hash: installed.content_hash.clone(),
                remote_hash: String::new(),
                update_available: false,
            });
        }

        for skill in self.list_bundled_skills() {
            entries.entry(skill.id.clone()).or_insert(skill);
        }

        entries.into_values().collect()
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

        let destination = self.source_dir.join(&skill.id);
        if destination.exists() {
            // 更新场景：先撤掉旧的，再把暂存目录顶上去。
            std::fs::remove_dir_all(&destination)
                .with_context(|| format!("移除旧 skill 目录失败：{}", destination.display()))?;
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败：{}", parent.display()))?;
        }
        std::fs::rename(&staging, &destination).with_context(|| {
            format!(
                "移动 skill 到 {} 失败（暂存目录 {}）",
                destination.display(),
                staging.display()
            )
        })?;

        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.installed.insert(
            skill.id.clone(),
            InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                content_hash: skill.content_hash.clone(),
                installed_at: current_unix_timestamp_string(),
            },
        );
        self.save_state_unlocked(&state)?;
        drop(_guard);

        self.set_enabled(&skill.id, true)?;
        Ok(self.load_state())
    }

    /// 启用 = 在 `$CODEX_HOME/skills/` 下建一个指向源目录的软链；停用 = 删掉它。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let source = self.source_dir.join(id);
        let link = self.linked_dir().join(id);
        if !enabled {
            return remove_link(&link);
        }
        if !source.is_dir() {
            anyhow::bail!("skill 源目录不存在：{}", source.display());
        }
        std::fs::create_dir_all(self.linked_dir())
            .with_context(|| format!("创建 {} 失败", self.linked_dir().display()))?;
        remove_link(&link)?;
        link_or_copy(&source, &link)
    }

    /// 卸载：删软链，源目录整体移进备份目录。不做自动轮转删除。
    pub fn uninstall(&self, id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        remove_link(&self.linked_dir().join(id))?;

        let source = self.source_dir.join(id);
        if source.is_dir() {
            std::fs::create_dir_all(&self.backups_dir)
                .with_context(|| format!("创建备份目录失败：{}", self.backups_dir.display()))?;
            let backup = self
                .backups_dir
                .join(format!("{id}-{}", current_unix_timestamp_string()));
            std::fs::rename(&source, &backup).with_context(|| {
                format!(
                    "备份 skill 到 {} 失败（源 {}）",
                    backup.display(),
                    source.display()
                )
            })?;
        }

        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.installed.remove(id);
        self.save_state_unlocked(&state)?;
        Ok(state)
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
        if destination.exists() {
            anyhow::bail!("{skill_id} 已经装着了，先卸载再从备份恢复");
        }
        std::fs::create_dir_all(&self.source_dir)
            .with_context(|| format!("创建目录失败：{}", self.source_dir.display()))?;
        std::fs::rename(&backup, &destination)
            .with_context(|| format!("恢复到 {} 失败", destination.display()))?;

        let (name, description) =
            read_skill_manifest(&destination.join(SKILL_MANIFEST_FILE), &skill_id);
        {
            let _guard = self.state_lock.lock().unwrap();
            let mut state = self.load_state_unlocked();
            state.installed.insert(
                skill_id.clone(),
                InstalledSkill {
                    id: skill_id.clone(),
                    name,
                    description,
                    repo_key: String::new(),
                    content_hash: String::new(),
                    installed_at: current_unix_timestamp_string(),
                },
            );
            self.save_state_unlocked(&state)?;
        }
        self.set_enabled(&skill_id, true)?;
        Ok(self.load_state())
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
            .join(format!(".staging-{id}-{}", current_unix_timestamp_string()))
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

fn normalize_repo(repo: SkillRepo) -> anyhow::Result<SkillRepo> {
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

    // skill 目录 -> (相对路径, blob sha) 列表
    let mut subtrees: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("blob") {
            continue;
        }
        let Some(path) = item.get("path").and_then(Value::as_str) else {
            continue;
        };
        let Some(rest) = path.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let Some((skill_dir, relative)) = rest.split_once('/') else {
            continue;
        };
        if skill_dir.is_empty() || skill_dir.starts_with('.') {
            continue;
        }
        let sha = item
            .get("sha")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        subtrees
            .entry(skill_dir.to_string())
            .or_default()
            .push((relative.to_string(), sha));
    }

    subtrees
        .into_iter()
        .filter_map(|(id, mut files)| {
            // 只有直接放着 SKILL.md 的目录才是 skill，嵌套更深的忽略。
            if !files.iter().any(|(path, _)| path == SKILL_MANIFEST_FILE) {
                return None;
            }
            files.sort();
            let mut hasher = Sha256::new();
            for (path, sha) in &files {
                hasher.update(path.as_bytes());
                hasher.update([0]);
                hasher.update(sha.as_bytes());
                hasher.update([0]);
            }
            let repo_path = if prefix.is_empty() {
                id.clone()
            } else {
                format!("{prefix}{id}")
            };
            Some(RemoteSkill {
                id: id.clone(),
                name: id.clone(),
                description: String::new(),
                repo_key: repo_key.clone(),
                repo_path,
                content_hash: format!("{:x}", hasher.finalize()),
            })
        })
        .collect()
}

/// 解析 SKILL.md 的 YAML frontmatter。只取 `name` 和 `description` 两个标量，
/// 不引 YAML 依赖——codex 自己也只读这两个字段。
pub fn parse_skill_frontmatter(text: &str, fallback_id: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let mut in_frontmatter = false;
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
        // 嵌套字段（如 metadata 下的键）缩进了，跳过，只认顶层。
        if line.starts_with(' ') || line.starts_with('\t') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = unquote_scalar(value);
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = unquote_scalar(value);
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
    let prefix = format!("{repo_path}/");
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
        let Some(relative) = crate::plugin_marketplace::zip_entry_relative_path(file.name()) else {
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

fn github_client() -> anyhow::Result<reqwest::Client> {
    crate::http_client::proxied_client(&format!("Codex++/{}", crate::version::VERSION))
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
    let client = github_client()?;
    let response = with_github_auth(client.get(repo_tree_url(repo)))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .with_context(|| format!("请求 {}/{} 的文件树失败", repo.owner, repo.name))?;
    // 不要用 error_for_status() 一把带过：403 限流、404 仓库/分支不存在、401 令牌
    // 无效，处理方式完全不同，但用户只会看到「返回错误状态」，没法自助也没法报障
    // （#1989 里四个仓库全失败，看不出到底是限流还是网络）。
    let status = response.status();
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

    let mut skills = parse_skills_from_tree(repo, &tree);
    for skill in &mut skills {
        if let Some(hit) = cached.get(&skill.id) {
            if hit.content_hash == skill.content_hash && !hit.description.is_empty() {
                skill.name = hit.name.clone();
                skill.description = hit.description.clone();
                continue;
            }
        }
        let manifest_url =
            raw_file_url(repo, &format!("{}/{SKILL_MANIFEST_FILE}", skill.repo_path));
        if let Ok(response) = with_github_auth(client.get(manifest_url)).send().await
            && let Ok(response) = response.error_for_status()
            && let Ok(text) = response.text().await
        {
            let (name, description) = parse_skill_frontmatter(&text, &skill.id);
            skill.name = name;
            skill.description = description;
        }
    }
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
    let client = github_client()?;
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
        assert_eq!(skills[0].id, "alpha");
        assert_eq!(skills[0].repo_path, "skills/.curated/alpha");
        assert!(!skills[0].content_hash.is_empty());
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

        let state = manager.uninstall("alpha").unwrap();
        assert!(!state.installed.contains_key("alpha"));
        assert!(!manager.source_dir().join("alpha").exists());
        assert!(!manager.linked_dir().join("alpha").exists());

        let backups = manager.list_backups();
        assert_eq!(backups.len(), 1);
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
        assert!(manager.list_backups().is_empty());
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
