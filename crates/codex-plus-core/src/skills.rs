//! Codex Skills 管理。
//!
//! codex 的 skill 是文件系统约定：这里支持 `$CODEX_HOME/skills` 和用户级 `.agents/skills`，
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
    /// 用户确认接受的完整本地文件树；与远端安装基线分开保存。
    #[serde(default)]
    pub local_hash: String,
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
    /// 插件市场拥有的文件只读展示，启停和更新由插件管理器负责。
    pub plugin: bool,
    pub content_hash: String,
    pub remote_hash: String,
    pub update_available: bool,
    pub local_difference: bool,
    pub comparison_error: String,
    pub local_path: String,
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
pub struct SkillOriginHint {
    pub id: String,
    pub repo: SkillRepo,
    pub repo_path: Option<String>,
}

/// CC Switch 的明确安装来源；绝不从介绍中的任意 GitHub 链接猜测来源。
pub fn read_ccs_skill_origins(database: &Path) -> anyhow::Result<Vec<SkillOriginHint>> {
    let connection = rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement = connection.prepare(
        "SELECT directory, repo_owner, repo_name, COALESCE(repo_branch, 'main'), readme_url FROM skills WHERE repo_owner IS NOT NULL AND repo_name IS NOT NULL")?;
    let rows = statement.query_map([], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?,
        row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?,
    )))?;
    let mut hints = Vec::new();
    for row in rows {
        let (id, owner, name, branch, readme) = row?;
        if validate_skill_id(&id).is_err() { continue; }
        let Ok(repo) = normalize_repo(SkillRepo { owner, name, branch, subdir: String::new(), enabled: true }) else { continue; };
        let prefix = format!("https://github.com/{}/{}/blob/{}/", repo.owner, repo.name, repo.branch);
        let repo_path = readme.as_deref().and_then(|url| url.strip_prefix(&prefix))
            .and_then(|path| path.strip_suffix("/SKILL.md").map(str::to_string)
                .or_else(|| (path == "SKILL.md").then(|| ".".into())));
        hints.push(SkillOriginHint { id, repo, repo_path });
    }
    Ok(hints)
}

/// 与 GitHub 子树指纹保持一致，同时用于检测本地完整文件树差异。
pub fn skill_directory_hash(root: &Path) -> anyhow::Result<String> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<String, String>, total: &mut u64) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            anyhow::ensure!(!kind.is_symlink(), "Skill 内含符号链接，无法安全比较：{}", entry.path().display());
            if kind.is_dir() {
                visit(root, &entry.path(), files, total)?;
            } else if kind.is_file() {
                *total = total.saturating_add(entry.metadata()?.len());
                anyhow::ensure!(*total <= 256 * 1024 * 1024, "Skill 内容超过 256 MiB，未完成比较");
                let contents = std::fs::read(entry.path())?;
                let mut blob = sha1::Sha1::new();
                blob.update(format!("blob {}\0", contents.len()).as_bytes());
                blob.update(contents);
                files.insert(entry.path().strip_prefix(root)?.to_string_lossy().replace('\\', "/"), format!("{:x}", blob.finalize()));
            } else {
                anyhow::bail!("Skill 内含不支持的文件类型");
            }
        }
        Ok(())
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files, &mut 0)?;
    let mut hasher = Sha256::new();
    for (path, sha) in files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(sha.as_bytes());
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Debug, Clone)]
pub struct SkillsManager {
    source_dir: PathBuf,
    backups_dir: PathBuf,
    state_path: PathBuf,
    codex_home: PathBuf,
    agents_skills_dir: Option<PathBuf>,
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
            agents_skills_dir: None,
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

    pub fn with_agents_skills_dir(mut self, directory: impl Into<PathBuf>) -> Self {
        self.agents_skills_dir = Some(directory.into());
        self
    }

    /// 命名空间只影响管理 ID，磁盘上的技能目录名保持不变。
    fn local_path(&self, id: &str) -> anyhow::Result<PathBuf> {
        validate_skill_id(id)?;
        if let Some(rest) = id.strip_prefix("local@") {
            let (root, leaf) = rest.split_once('@').context("非法的本地 Skill ID")?;
            return match root {
                "agents" => Ok(self.agents_skills_dir.as_ref().context("未配置 agents Skills 目录")?.join(leaf)),
                "codex" => Ok(self.linked_dir().join(leaf)),
                _ => anyhow::bail!("未知的 Skill 安装位置"),
            };
        }
        Ok(self.linked_dir().join(id))
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
                    enabled: installed.is_some() && self.local_path(&id).is_ok_and(|path| is_linked(&path)),
                    bundled: false,
                    plugin: false,
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
                    local_difference: false,
                    comparison_error: String::new(),
                    local_path: String::new(),
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
                enabled: self.local_path(id).is_ok_and(|path| is_linked(&path)),
                bundled: false,
                plugin: false,
                content_hash: installed.content_hash.clone(),
                remote_hash: String::new(),
                update_available: false,
                local_difference: false,
                comparison_error: String::new(),
                local_path: String::new(),
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

        entries.into_values().map(|mut entry| {
            if entry.installed && !entry.bundled && !entry.plugin {
                if let Some(remote) = remote.iter().find(|remote| remote.repo_key == entry.repo_key && remote.repo_path == entry.repo_path) {
                    entry.remote_hash = remote.content_hash.clone();
                    entry.update_available = !entry.content_hash.is_empty() && !remote.content_hash.is_empty()
                        && entry.content_hash != remote.content_hash;
                }
                entry.local_path = self.local_path(&entry.id).map(|path| path.to_string_lossy().into_owned()).unwrap_or_default();
                match self.local_difference(&entry.id) {
                    Ok(different) => entry.local_difference = different,
                    Err(error) => entry.comparison_error = format!("{error:#}"),
                }
            }
            entry
        }).collect()
    }

    /// 本地目录 inventory 不混入仅出现在远端/缓存清单中的未安装条目，也不写 TOML。
    pub fn local_inventory(&self) -> Vec<SkillEntry> {
        self.merge_entries(&[])
    }

    /// 扫描已配置的用户技能目录；仅发现不写入安装状态，后续显式操作才接管。
    fn list_local_skills(&self) -> Vec<SkillEntry> {
        let mut skills = Vec::new();
        let mut roots = vec![("codex", self.linked_dir())];
        if let Some(root) = &self.agents_skills_dir {
            if root.canonicalize().ok().zip(self.linked_dir().canonicalize().ok())
                .is_none_or(|(a, b)| a != b) {
                roots.push(("agents", root.clone()));
            }
        }
        for (kind, root) in roots {
        let Ok(dir) = std::fs::read_dir(&root) else { continue; };
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
            let leaf = id;
            let id = if kind == "agents" || leaf.starts_with("local@") {
                format!("local@{kind}@{leaf}")
            } else { leaf.to_string() };
            let (name, description) = read_skill_manifest(&manifest, leaf);
            skills.push(SkillEntry {
                id,
                name,
                description,
                repo_key: String::new(),
                repo_path: String::new(),
                installed: true,
                enabled: is_linked(&path),
                bundled: false,
                plugin: false,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                local_difference: false,
                comparison_error: String::new(),
                local_path: path.to_string_lossy().into_owned(),
            });
        }
        }
        skills
    }

    pub fn origin_identity(&self, id: &str) -> anyhow::Result<String> {
        validate_skill_id(id)?;
        for directory in [self.local_path(id)?, self.source_dir.join(id)] {
            let manifest = directory.join(SKILL_MANIFEST_FILE);
            if manifest.is_file() { return Ok(read_skill_manifest(&manifest, id).0); }
        }
        anyhow::bail!("本地 Skill 不存在")
    }

    pub fn origin_fingerprint(&self, id: &str) -> anyhow::Result<String> {
        self.origin_identity(id)?;
        for directory in [self.local_path(id)?, self.source_dir.join(id)] {
            if directory.join(SKILL_MANIFEST_FILE).is_file() {
                return skill_directory_hash(&directory);
            }
        }
        anyhow::bail!("本地 Skill 不存在")
    }

    /// 仅提供待确认线索；文件中的链接不能直接更改安装来源。
    pub fn origin_search_repos(&self, id: &str) -> anyhow::Result<Vec<SkillRepo>> {
        self.origin_identity(id)?;
        let mut repos = BTreeMap::new();
        for root in [self.local_path(id)?, self.source_dir.join(id)] {
            for relative in [SKILL_MANIFEST_FILE, "README.md", "readme.md", ".git/config"] {
                let path = root.join(relative);
                if !path.metadata().is_ok_and(|meta| meta.len() <= 256 * 1024) { continue; }
                let Ok(text) = std::fs::read_to_string(path) else { continue; };
                for (_, tail) in text.match_indices("https://github.com/").map(|(index, _)| (index, &text[index..])) {
                    let token = tail.split(|c: char| c.is_whitespace() || matches!(c, ')' | '(' | ']' | '[' | '>' | '<' | '"' | '\'' | '`')).next().unwrap_or("");
                    let Ok(url) = reqwest::Url::parse(token.trim_end_matches(['.', ',', ';'])) else { continue; };
                    let parts: Vec<_> = url.path_segments().into_iter().flatten().collect();
                    if parts.len() < 2 { continue; }
                    let branch = if parts.len() >= 4 && matches!(parts[2], "tree" | "blob") { parts[3] } else { "main" };
                    if let Ok(repo) = normalize_repo(SkillRepo { owner: parts[0].into(), name: parts[1].trim_end_matches(".git").into(), branch: branch.into(), subdir: String::new(), enabled: true }) {
                        repos.insert(repo.key(), repo);
                    }
                    if repos.len() >= 8 { break; }
                }
            }
        }
        Ok(repos.into_values().take(8).collect())
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
            local_hash: String::new(),
            installed_at: current_unix_timestamp_string(),
        }, false)
    }

    /// 比较完整文件树，包括软链目标本身相对接受基线的修改。
    pub fn local_difference(&self, id: &str) -> anyhow::Result<bool> {
        validate_skill_id(id)?;
        let source = self.source_dir.join(id);
        let local = self.local_path(id)?;
        if local.join(SKILL_MANIFEST_FILE).is_file() {
            let local_hash = skill_directory_hash(&local)?;
            if source.join(SKILL_MANIFEST_FILE).is_file() && source.canonicalize()? != local.canonicalize()?
                && skill_directory_hash(&source)? != local_hash { return Ok(true); }
            if let Some(installed) = self.load_state().installed.get(id) {
                if !installed.local_hash.is_empty() { return Ok(installed.local_hash != local_hash); }
            }
        } else if source.join(SKILL_MANIFEST_FILE).is_file() {
            if let Some(installed) = self.load_state().installed.get(id) {
                if !installed.local_hash.is_empty() {
                    return Ok(installed.local_hash != skill_directory_hash(&source)?);
                }
            }
        }
        Ok(false)
    }

    pub fn ensure_update_unmodified(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!self.local_difference(id)?, "Skill 有本地修改，请先明确保留版本，再执行更新");
        if !self.load_state().installed.contains_key(id) && self.local_path(id)?.exists() {
            anyhow::bail!("安装位置已有本地 Skill，请先验证并关联来源，未覆盖文件");
        }
        Ok(())
    }

    pub fn resolve_local_version(&self, id: &str, keep_local: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        if !self.local_difference(id)? { return Ok(()); }
        let local = self.local_path(id)?;
        let source = self.source_dir.join(id);
        let mut installed = self.load_state().installed.get(id).cloned().unwrap_or_else(|| {
            let (name, description) = read_skill_manifest(&local.join(SKILL_MANIFEST_FILE), id);
            InstalledSkill { id: id.into(), name, description, repo_key: String::new(),
                repo_path: String::new(), content_hash: String::new(), local_hash: String::new(), installed_at: current_unix_timestamp_string() }
        });
        let selected = if keep_local && local.join(SKILL_MANIFEST_FILE).is_file() { &local } else { &source };
        anyhow::ensure!(selected.join(SKILL_MANIFEST_FILE).is_file(), "没有可保留的对应版本");
        let (name, description) = read_skill_manifest(&selected.join(SKILL_MANIFEST_FILE), id);
        installed.name = name;
        installed.description = description;
        installed.local_hash = skill_directory_hash(selected)?;
        // 保留远端安装基线，接受本地修改不能误认为已更新到远端。
        let staging = self.staging_dir(id);
        if let Err(error) = copy_dir_all(selected, &staging) {
            let _ = remove_link(&staging);
            return Err(error);
        }
        self.commit_staged_skill(&staging, installed, local.join(SKILL_MANIFEST_FILE).is_file())?;
        Ok(())
    }

    /// 仅在已验证的远端清单中找到唯一来源后修复元数据，不替换本地文件。
    pub fn associate_origin(&self, id: &str, repo: &SkillRepo, skill: &RemoteSkill) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        anyhow::ensure!(repo.key() == skill.repo_key, "仓库与 Skill 来源不一致");
        let local = self.local_path(id)?;
        let source = self.source_dir.join(id);
        let directory = if local.join(SKILL_MANIFEST_FILE).is_file() { &local } else { &source };
        anyhow::ensure!(directory.join(SKILL_MANIFEST_FILE).is_file(), "本地 Skill 不存在");
        let baseline = skill_directory_hash(&directory)?;
        let (name, description) = read_skill_manifest(&directory.join(SKILL_MANIFEST_FILE), id);
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        if !state.repos.iter().any(|item| item.key() == repo.key()) {
            state.repos.push(repo.clone());
        }
        state.installed.insert(id.into(), InstalledSkill { id: id.into(), name, description,
            repo_key: repo.key(), repo_path: skill.repo_path.clone(), content_hash: baseline.clone(),
            local_hash: baseline,
            installed_at: current_unix_timestamp_string() });
        // 首次接管仅复制基线，不移动或重新链接原安装。
        let snapshot = !source.exists();
        if snapshot {
            std::fs::create_dir_all(&self.source_dir)?;
            anyhow::ensure!(!self.source_dir.canonicalize()?.starts_with(directory.canonicalize()?),
                "受控源目录不能位于待接管 Skill 内部，未保存来源");
            let staging = self.staging_dir(id);
            copy_dir_all(directory, &staging)?;
            std::fs::rename(&staging, &source)?;
        }
        if let Err(error) = self.save_state_unlocked(&state) {
            if snapshot {
                self.backup_directory(id, &source).context(format!("来源保存失败：{error:#}；基线备份失败"))?;
            }
            return Err(error);
        }
        Ok(())
    }

    /// 目录、链接和安装记录一起提交；失败时恢复两处原内容。
    fn commit_staged_skill(&self, staging: &Path, mut installed: InstalledSkill, enabled: bool) -> anyhow::Result<SkillsState> {
        if installed.local_hash.is_empty() { installed.local_hash = skill_directory_hash(staging)?; }
        let id = installed.id.clone();
        let destination = self.source_dir.join(&id);
        let _guard = self.state_lock.lock().unwrap();
        let mut state = self.load_state_unlocked();
        state.installed.insert(id.clone(), installed);
        let link = self.local_path(&id)?;
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
                std::fs::create_dir_all(link.parent().context("Skill 安装位置缺少父目录")?)?;
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
                plugin: false,
                content_hash: String::new(),
                remote_hash: String::new(),
                update_available: false,
                local_difference: false,
                comparison_error: String::new(),
                local_path: path.to_string_lossy().into_owned(),
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
            || is_linked(&self.local_path(&skill.id)?);
        self.ensure_update_unmodified(&skill.id)?;
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
        self.ensure_update_unmodified(&skill.id)?;

        self.commit_staged_skill(&staging,
            InstalledSkill {
                id: skill.id.clone(),
                name: skill.name.clone(),
                description: skill.description.clone(),
                repo_key: skill.repo_key.clone(),
                repo_path: skill.repo_path.clone(),
                content_hash: skill.content_hash.clone(),
                local_hash: skill_directory_hash(&staging)?,
                installed_at: current_unix_timestamp_string(),
            },
            enable_after_install,
        )
    }

    /// 启用 = 在 `$CODEX_HOME/skills/` 下建一个指向源目录的软链；停用 = 删掉它。
    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        validate_skill_id(id)?;
        let source = self.source_dir.join(id);
        let link = self.local_path(id)?;
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
                    local_hash: String::new(),
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
        std::fs::create_dir_all(link.parent().context("Skill 安装位置缺少父目录")?)
            .with_context(|| format!("创建 {} 失败", link.display()))?;
        if let Err(error) = link_or_copy(&source, &link) {
            remove_link(&link).context(format!("启用失败且清理未完成：{error:#}"))?;
            return Err(error);
        }
        Ok(())
    }

    /// 卸载：删软链，源目录整体移进备份目录。不做自动轮转删除。
    pub fn uninstall(&self, id: &str) -> anyhow::Result<SkillsState> {
        validate_skill_id(id)?;
        let link = self.local_path(id)?;
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
            || std::fs::symlink_metadata(self.local_path(&skill_id)?).is_ok()
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
                local_hash: String::new(),
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
    anyhow::ensure!(!id.starts_with("codex-plugin:"), "插件 Skill 请在 Codex 插件管理中操作");
    if id.is_empty() {
        anyhow::bail!("skill id 不能为空");
    }
    if id == "." || id == ".." || id.contains('/') || id.contains('\\') {
        anyhow::bail!("非法的 skill id：{id}");
    }
    anyhow::ensure!(!id.contains([':', '\0']) && !id.ends_with(['.', ' ']), "非法的 skill id：{id}");
    if let Some(rest) = id.strip_prefix("local@") {
        let (root, leaf) = rest.split_once('@').context("非法的本地 Skill ID")?;
        anyhow::ensure!(matches!(root, "agents" | "codex") && !leaf.is_empty()
            && !leaf.starts_with('.') && !leaf.ends_with(['.', ' ']), "非法的本地 Skill ID");
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
    zip_url: &str,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let response = with_github_auth(client.get(zip_url))
        .header(reqwest::header::ACCEPT, "application/zip")
        .timeout(std::time::Duration::from_secs(90))
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

/// 从同一份下载快照读取清单和内容指纹，供安装/更新复用，避免分支变化造成错配。
pub fn parse_skills_from_zip(
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
/// 并同时挡住路径逃逸。目标子树内的符号链接一律拒绝，避免装进来的 skill
/// 指到仓库外面；其他子树的链接不影响当前 skill 的安装。
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
        if file.is_symlink() {
            anyhow::bail!("skill 中不允许包含符号链接：{}", file.name());
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
                Err(alternative_error) => Err(alternative_error.context(format!(
                    "默认分支 {} 不可用，尝试 {} 也失败", repo.branch, alternative.branch))),
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
    fetch_repo_skills_from_endpoints(repo, cached, &client, &repo_tree_url(repo), &repo_zip_url(repo)).await
}

async fn fetch_repo_skills_from_endpoints(
    repo: &SkillRepo,
    cached: &BTreeMap<String, RemoteSkill>,
    client: &reqwest::Client,
    tree_url: &str,
    zip_url: &str,
) -> anyhow::Result<Vec<RemoteSkill>> {
    let response = with_github_auth(client.get(tree_url))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) => return fetch_repo_skills_from_zip(repo, cached, client, zip_url).await
            .with_context(|| format!("文件树请求失败（{error}），压缩包回退也失败")),
    };
    // 不要用 error_for_status() 一把带过：403 限流、404 仓库/分支不存在、401 令牌
    // 无效，处理方式完全不同，但用户只会看到「返回错误状态」，没法自助也没法报障
    // （#1989 里四个仓库全失败，看不出到底是限流还是网络）。
    let status = response.status();
    if !status.is_success() && (matches!(status.as_u16(), 403 | 429) || status.is_server_error()) {
        return fetch_repo_skills_from_zip(repo, cached, client, zip_url).await;
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
    // 响应头成功后仍可能正文超时、截断或返回非文件树 JSON；同样走 ZIP 回退。
    let tree = match response.json::<Value>().await {
        Ok(tree) if tree.get("tree").is_some_and(Value::is_array) => tree,
        Ok(_) => return fetch_repo_skills_from_zip(repo, cached, client, zip_url).await
            .context("GitHub 文件树格式无效，压缩包回退也失败"),
        Err(error) => return fetch_repo_skills_from_zip(repo, cached, client, zip_url).await
            .with_context(|| format!("解析 GitHub 文件树响应失败（{error}），压缩包回退也失败")),
    };
    if tree.get("truncated").and_then(Value::as_bool) == Some(true) {
        return fetch_repo_skills_from_zip(repo, cached, client, zip_url).await;
    }

    let skills = parse_skills_from_tree(repo, &tree);
    use futures_util::{StreamExt, stream};
    let skills = stream::iter(skills.into_iter().map(|mut skill| {
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
        .timeout(std::time::Duration::from_secs(90))
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
        assert!(manager.install_from_zip(&sample_skill("alpha", "alpha", "two"), &repo_zip(&[("kit-main/alpha/SKILL.md", "replacement")])).is_err());
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "local edits");
        manager.resolve_local_version("alpha", true).unwrap();
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

    async fn catalog_response_fixture(
        status: u16, body: &str, truncated_body: bool, archive: Option<Vec<u8>>,
    ) -> (anyhow::Result<Vec<RemoteSkill>>, Vec<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.as_bytes().to_vec();
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            let mut responses = vec![(status, body.clone(), body.len() + usize::from(truncated_body) * 100)];
            if let Some(archive) = archive { responses.push((200, archive.clone(), archive.len())); }
            for (status, bytes, length) in responses {
                let (mut socket, _) = tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept()).await.unwrap().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                }
                requests.push(String::from_utf8_lossy(&request).lines().next().unwrap().to_string());
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                socket.write_all(&bytes).await.unwrap();
                socket.shutdown().await.unwrap();
            }
            requests
        });
        let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(3)).build().unwrap();
        let result = fetch_repo_skills_from_endpoints(
            &parse_repo_key("acme/kit@main").unwrap(), &BTreeMap::new(), &client,
            &format!("http://{address}/tree"), &format!("http://{address}/archive"),
        ).await;
        (result, server.await.unwrap())
    }

    #[tokio::test]
    async fn invalid_or_incomplete_tree_responses_fall_back_to_zip() {
        let zip = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\ndescription: ZIP fallback\n---\n")]);
        for (body, truncated) in [
            ("<html>proxy error</html>", false),
            ("{\"tree\":", false),
            ("{\"message\":\"invalid response\"}", false),
            ("{\"tree\":[]}", true),
            ("{\"truncated\":true,\"tree\":[]}", false),
        ] {
            let (result, requests) = catalog_response_fixture(200, body, truncated, Some(zip.clone())).await;
            let skills = result.unwrap();
            assert_eq!(skills.len(), 1);
            assert_eq!(skills[0].repo_path, "alpha");
            assert_eq!(skills[0].description, "ZIP fallback");
            assert_eq!(requests, ["GET /tree HTTP/1.1", "GET /archive HTTP/1.1"]);
        }
    }

    #[tokio::test]
    async fn tree_authentication_and_missing_repo_errors_do_not_fall_back() {
        for status in [401, 404] {
            let (result, requests) = catalog_response_fixture(status, "{}", false, None).await;
            assert!(result.unwrap_err().to_string().contains(&format!("HTTP {status}")));
            assert_eq!(requests, ["GET /tree HTTP/1.1"]);
        }
    }

    #[tokio::test]
    async fn failed_tree_and_zip_responses_preserve_both_error_causes() {
        let (result, _) = catalog_response_fixture(200, "invalid json", false, Some(b"invalid zip".to_vec())).await;
        let error = format!("{:#}", result.unwrap_err());
        assert!(error.contains("解析 GitHub 文件树响应失败"));
        assert!(error.contains("压缩包回退也失败"));
        assert!(error.contains("仓库压缩包无法解析"));
    }

    #[test]
    fn repository_archive_update_uses_one_snapshot_for_catalog_hash_and_installed_files() {
        let repo = parse_repo_key("acme/kit@main").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let old_zip = repo_zip(&[("kit-main/alpha/SKILL.md", "---\nname: alpha\ndescription: Old\n---\n")]);
        let old = parse_skills_from_zip(&repo, &old_zip, &BTreeMap::new()).unwrap().remove(0);
        manager.install_from_zip(&old, &old_zip).unwrap();
        manager.set_enabled(&old.id, false).unwrap();
        let cached = BTreeMap::from([(old.id.clone(), old.clone())]);
        let new_zip = repo_zip(&[
            ("kit-main/alpha/SKILL.md", "---\nname: alpha\ndescription: New snapshot\n---\n"),
            ("kit-main/alpha/references/update.md", "New reference"),
        ]);
        let updated = parse_skills_from_zip(&repo, &new_zip, &cached).unwrap().remove(0);
        assert_eq!(updated.id, old.id);
        assert_ne!(updated.content_hash, old.content_hash);
        manager.install_from_zip(&updated, &new_zip).unwrap();
        let entries = manager.merge_entries(&[updated.clone()]);
        let installed = entries.iter().find(|entry| entry.id == old.id).unwrap();
        assert!(installed.installed);
        assert!(!installed.enabled);
        assert!(!installed.update_available);
        assert!(!installed.local_difference);
        assert!(installed.comparison_error.is_empty());
        assert_eq!(installed.description, "New snapshot");
        assert_eq!(skill_directory_hash(&manager.source_dir().join(&old.id)).unwrap(), updated.content_hash);
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

    fn zip_with_skill_symlink(path: &str) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        writer.add_symlink(path, "../../outside", options).unwrap();
        writer.start_file("kit-main/skills/alpha/SKILL.md", options).unwrap();
        writer.write_all(b"replacement").unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn skill_update_ignores_symlinks_outside_requested_subtree() {
        for path in [
            "kit-main/skills/academic-paper",
            "kit-main/skills/alpha-alias",
            "kit-main/skills/alpha-other/link",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let manager = manager(&temp);
            manager.install_from_zip(
                &sample_skill("alpha", "skills/alpha", "v1"),
                &repo_zip(&[("kit-main/skills/alpha/SKILL.md", "original")]),
            ).unwrap();
            let state = manager.install_from_zip(
                &sample_skill("alpha", "skills/alpha", "v2"),
                &zip_with_skill_symlink(path),
            ).unwrap();
            assert_eq!(state.installed["alpha"].content_hash, "v2");
            assert_eq!(std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(), "replacement");
            assert!(!temp.path().join("outside").exists());
        }
    }

    #[test]
    fn skill_update_rejects_internal_symlinks_and_preserves_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        manager.install_from_zip(
            &sample_skill("alpha", "skills/alpha", "v1"),
            &repo_zip(&[("kit-main/skills/alpha/SKILL.md", "original")]),
        ).unwrap();
        let before = manager.load_state();
        let zip = zip_with_skill_symlink("kit-main/skills/alpha/link");
        let error = manager.install_from_zip(
            &sample_skill("alpha", "skills/alpha", "v2"), &zip,
        ).unwrap_err();
        assert!(error.to_string().contains("符号链接"));
        assert_eq!(manager.load_state(), before);
        assert_eq!(std::fs::read_to_string(manager.source_dir().join("alpha/SKILL.md")).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(manager.linked_dir().join("alpha/SKILL.md")).unwrap(), "original");
        assert!(extract_skill_subtree(&zip, ".", &temp.path().join("root")).unwrap_err().to_string().contains("符号链接"));
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

    #[test]
    fn plugin_marketplace_skills_are_excluded_without_changing_plugin_files() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        for (plugin, version) in [("pdf", "current"), ("unused", "old")] {
            let skill = manager.codex_home.join("plugins/cache/market").join(plugin).join(version).join("skills/pdf");
            std::fs::create_dir_all(&skill).unwrap();
            std::fs::write(skill.join(SKILL_MANIFEST_FILE), "---\nname: pdf\ndescription: Plugin PDF\n---\n").unwrap();
        }
        let config = manager.codex_home.join("config.toml");
        std::fs::write(&config, "[plugins.\"pdf@market\"]\nenabled = false\n").unwrap();
        let before = std::fs::read(&config).unwrap();
        let entries = manager.local_inventory();
        assert!(entries.is_empty());
        assert!(manager.merge_entries(&[]).is_empty());
        assert_eq!(std::fs::read(&config).unwrap(), before);
        assert!(manager.codex_home.join("plugins/cache/market/pdf/current/skills/pdf/SKILL.md").is_file());
    }

    #[test]
    fn source_hints_are_readonly_and_confirmed_origin_can_be_reassigned_without_file_changes() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp);
        let local = manager.linked_dir().join("planning-with-files");
        std::fs::create_dir_all(&local).unwrap();
        let text = "---\nname: planning-with-files\n---\n[Source](https://github.com/acme/planner/tree/master/skills/planning-with-files)\n";
        std::fs::write(local.join(SKILL_MANIFEST_FILE), text).unwrap();
        std::fs::write(local.join("local.txt"), "preserve").unwrap();
        let hints = manager.origin_search_repos("planning-with-files").unwrap();
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].key(), "acme/planner@master");
        assert!(!manager.state_path.exists());
        assert!(manager.origin_identity("../escape").is_err());
        let mut remote = sample_skill("planning-with-files", "skills/planning-with-files", "upstream");
        remote.repo_key = hints[0].key();
        manager.associate_origin("planning-with-files", &hints[0], &remote).unwrap();
        let other = parse_repo_key("other/planner@main").unwrap();
        remote.repo_key = other.key();
        manager.associate_origin("planning-with-files", &other, &remote).unwrap();
        assert_eq!(manager.load_state().installed["planning-with-files"].repo_key, other.key());
        assert_eq!(std::fs::read_to_string(local.join(SKILL_MANIFEST_FILE)).unwrap(), text);
        assert_eq!(std::fs::read_to_string(local.join("local.txt")).unwrap(), "preserve");
        assert_eq!(skill_directory_hash(&manager.source_dir.join("planning-with-files")).unwrap(), skill_directory_hash(&local).unwrap());
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

    #[test]
    fn agents_adoption_update_and_restore_keep_original_location_and_same_named_codex_skill() {
        let temp = tempfile::tempdir().unwrap();
        let agents = temp.path().join("agents/skills");
        let manager = manager(&temp).with_agents_skills_dir(&agents);
        let local = agents.join("alpha");
        let codex = manager.linked_dir().join("alpha");
        for (directory, text) in [(&local, "agents original"), (&codex, "codex original")] {
            std::fs::create_dir_all(directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), text).unwrap();
        }
        let outside = agents.parent().unwrap().join("hooks.json");
        std::fs::write(&outside, "outside config").unwrap();
        let inventory = manager.local_inventory();
        assert_eq!(inventory.len(), 2);
        let id = inventory.iter().find(|entry| entry.local_path == local.to_string_lossy()).unwrap().id.clone();
        assert_ne!(id, "alpha");
        let repo = parse_repo_key("openai/skills@main:skills/.curated").unwrap();
        let mut remote = sample_skill("alpha", "host/alpha", "upstream-v2");
        manager.associate_origin(&id, &repo, &remote).unwrap();
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "agents original");
        assert_eq!(std::fs::read_to_string(manager.source_dir.join(&id).join("SKILL.md")).unwrap(), "agents original");
        assert!(manager.merge_entries(&[remote.clone()]).iter().any(|entry| entry.id == id && entry.update_available));
        remote.id = id.clone();
        assert!(manager.install_from_zip(&remote, b"invalid zip").is_err());
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "agents original");
        manager.install_from_zip(&remote, &repo_zip(&[("kit-main/host/alpha/SKILL.md", "version two")])).unwrap();
        std::fs::write(local.join("SKILL.md"), "edited version two").unwrap();
        assert!(manager.local_difference(&id).unwrap());
        assert!(manager.install_from_zip(&remote, &repo_zip(&[("kit-main/host/alpha/SKILL.md", "overwrite")])).is_err());
        manager.resolve_local_version(&id, true).unwrap();
        assert!(!manager.local_difference(&id).unwrap());
        manager.set_enabled(&id, false).unwrap();
        assert!(!local.exists());
        remote.content_hash = "upstream-v3".into();
        manager.install_from_zip(&remote, &repo_zip(&[("kit-main/host/alpha/SKILL.md", "version three")])).unwrap();
        assert!(!local.exists(), "updating a disabled Skill must not enable it");
        manager.set_enabled(&id, true).unwrap();
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "version three");
        manager.uninstall(&id).unwrap();
        let backup = manager.list_backups().into_iter().find(|backup| backup.skill_id == id
            && std::fs::read_to_string(manager.backups_dir.join(&backup.id).join("SKILL.md")).unwrap() == "version three").unwrap();
        manager.restore_backup(&backup.id).unwrap();
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "version three");
        assert_eq!(std::fs::read_to_string(codex.join("SKILL.md")).unwrap(), "codex original");
        assert_eq!(std::fs::read_to_string(outside).unwrap(), "outside config");
        assert!(!manager.linked_dir().join(&id).exists());
    }

    #[test]
    fn multiple_local_installations_of_same_remote_both_receive_update_status() {
        let temp = tempfile::tempdir().unwrap();
        let agents = temp.path().join("agents");
        let manager = manager(&temp).with_agents_skills_dir(&agents);
        for directory in [agents.join("alpha"), manager.linked_dir().join("alpha")] {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("SKILL.md"), "local").unwrap();
        }
        let repo = parse_repo_key("openai/skills@main:skills/.curated").unwrap();
        let remote = sample_skill("alpha", "alpha", "new-upstream");
        for id in ["alpha", "local@agents@alpha"] { manager.associate_origin(id, &repo, &remote).unwrap(); }
        let entries = manager.merge_entries(&[remote]);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| entry.update_available && entry.enabled));
    }

    #[test]
    fn agents_adoption_state_failure_preserves_original_and_does_not_register_source() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp).with_agents_skills_dir(temp.path().join("agents"));
        let local = temp.path().join("agents/alpha");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "original").unwrap();
        std::fs::create_dir(&manager.state_path).unwrap();
        let repo = parse_repo_key("openai/skills@main:skills/.curated").unwrap();
        assert!(manager.associate_origin("local@agents@alpha", &repo, &sample_skill("alpha", "alpha", "h")).is_err());
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "original");
        assert!(manager.load_state().installed.is_empty());
        assert!(!manager.source_dir.join("local@agents@alpha").exists());
    }

    #[test]
    fn origin_association_rejects_recursive_snapshot_without_persisting_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("home/skills/alpha");
        std::fs::create_dir_all(&local).unwrap();
        std::fs::write(local.join("SKILL.md"), "original").unwrap();
        let manager = SkillsManager::new(local.join("managed"), temp.path().join("backups"),
            temp.path().join("state.json"), temp.path().join("home"));
        let repo = parse_repo_key("openai/skills@main:skills/.curated").unwrap();
        assert!(manager.associate_origin("alpha", &repo, &sample_skill("alpha", "alpha", "hash")).is_err());
        assert!(!manager.state_path.exists());
        assert_eq!(std::fs::read_to_string(local.join("SKILL.md")).unwrap(), "original");
        assert_eq!(std::fs::read_dir(manager.source_dir()).unwrap().count(), 0);
    }

    #[test]
    fn local_namespace_rejects_escape_and_preserves_unusual_codex_directory_names() {
        let temp = tempfile::tempdir().unwrap();
        let manager = manager(&temp).with_agents_skills_dir(temp.path().join("agents"));
        for id in ["local@agents@..", "local@agents@../escape", "local@other@alpha", "local@agents@", "C:escape"] {
            assert!(manager.set_enabled(id, true).is_err(), "{id}");
        }
        let unusual = manager.linked_dir().join("local@agents@alpha");
        std::fs::create_dir_all(&unusual).unwrap();
        std::fs::write(unusual.join("SKILL.md"), "unusual name").unwrap();
        let entry = manager.local_inventory().pop().unwrap();
        assert_eq!(entry.id, "local@codex@local@agents@alpha");
        manager.set_enabled(&entry.id, false).unwrap();
        assert!(!unusual.exists());
        manager.set_enabled(&entry.id, true).unwrap();
        assert_eq!(std::fs::read_to_string(unusual.join("SKILL.md")).unwrap(), "unusual name");
    }
}
