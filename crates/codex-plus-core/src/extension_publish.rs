//! 拓展一键发布：把本地拓展包发布成 GitHub 公开仓库（`codexpp-extension` topic + release 包文件），
//! 出现在社区栏。设计见 docs/plans/2026-10-30-extension-publish-discovery-design.md 第十节。
//!
//! 鉴权全部交给 `gh`：Codex++ 不读、不存 token，也不改用户的全局 git 配置；推送时只在那一条
//! git 命令上临时指定 `gh auth git-credential` 作凭据助手。每一步失败都停下并给出原始输出，
//! 已完成的步骤不回滚（仓库已建、提交已推都可以安全重试）。

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Value, json};

use crate::user_scripts::{EXTENSION_API_VERSION, EXTENSION_MANIFEST_FILE, UserScriptManager, validate_extension_id};

/// dsh 导入生成的配套包带这个标记文件，不提供发布入口。
const DSH_COMPANION_MARKER: &str = "dsh-bridge.json";
const PUBLISH_DIR: &str = "extension_publish";
const PUBLISH_STATE: &str = "extension_publish.json";
const MAX_FILES: usize = crate::extension_discovery::PACKAGE_MAX_FILES;
const MAX_BYTES: u64 = crate::extension_discovery::PACKAGE_MAX_BYTES;

/// 一处疑似密钥：只给文件与行号，不回传命中内容。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SecretHit {
    pub file: String,
    pub line: usize,
    pub kind: String,
}

/// 发布前检查的结果。`errors` 非空时不能发布。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub secrets: Vec<SecretHit>,
    pub files: usize,
    pub bytes: u64,
    pub has_readme: bool,
    pub has_license: bool,
    /// 之前发布过的仓库（`owner/repo`）。
    pub published_repo: Option<String>,
    /// 默认仓库名。
    pub suggested_repo_name: String,
}

/// 拓展包目录与清单；不是可发布的用户拓展包时报错。
fn package_dir(manager: &UserScriptManager, id: &str) -> anyhow::Result<(PathBuf, Value)> {
    validate_extension_id(id)?;
    let dir = manager.user_dir().join(id);
    let manifest_path = dir.join(EXTENSION_MANIFEST_FILE);
    if std::fs::symlink_metadata(&dir).map(|meta| !meta.is_dir()).unwrap_or(true) || !manifest_path.is_file() {
        bail!("{id} 不是用户拓展包（单文件脚本请先转成拓展包，内置拓展不能发布）");
    }
    if dir.join(DSH_COMPANION_MARKER).exists() {
        bail!("{id} 是 dsh 插件导入生成的配套包，请发布原来的 dsh 插件");
    }
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)
        .with_context(|| format!("{EXTENSION_MANIFEST_FILE} 不是合法的 JSON"))?;
    Ok((dir, manifest))
}

fn is_semver(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or("");
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3 && parts.iter().all(|part| !part.is_empty() && part.chars().all(|ch| ch.is_ascii_digit()))
}

/// 包内文件（相对路径、大小），不跟随软链；遇到软链记一条错误。
pub fn package_files(dir: &Path, errors: &mut Vec<String>) -> Vec<(String, u64)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".git" || name == ".DS_Store" {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            let relative = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            if meta.file_type().is_symlink() {
                errors.push(format!("{relative} 是软链，发布前请换成真实文件"));
            } else if meta.is_dir() {
                stack.push(path);
            } else {
                files.push((relative, meta.len()));
            }
        }
    }
    files
}

/// 文件名与内容的密钥扫描。精选审核也用它扫待审的包。
pub fn scan_secrets(dir: &Path, files: &[(String, u64)]) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    for (file, size) in files {
        let name = file.rsplit('/').next().unwrap_or(file).to_lowercase();
        let sensitive_name = name.starts_with(".env")
            || matches!(name.as_str(), "auth.json" | "config.toml" | "user_scripts.json")
            || name.ends_with(".pem")
            || name.ends_with(".key")
            || name.starts_with("id_rsa")
            || name.starts_with("id_ed25519");
        if sensitive_name {
            hits.push(SecretHit { file: file.clone(), line: 0, kind: "敏感文件名".into() });
            continue;
        }
        if *size > 2 * 1024 * 1024 {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            if let Some(kind) = secret_kind(line) {
                hits.push(SecretHit { file: file.clone(), line: index + 1, kind: kind.into() });
            }
        }
    }
    hits
}

fn secret_kind(line: &str) -> Option<&'static str> {
    if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
        return Some("私钥");
    }
    let tokens = line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'));
    for token in tokens {
        if token.starts_with("sk-") && token.len() >= 24 {
            return Some("API 密钥（sk-）");
        }
        if (token.starts_with("ghp_") || token.starts_with("gho_") || token.starts_with("ghs_")) && token.len() >= 30 {
            return Some("GitHub token");
        }
        if token.starts_with("github_pat_") && token.len() >= 30 {
            return Some("GitHub token");
        }
        if token.starts_with("AKIA") && token.len() == 20 && token.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
            return Some("AWS 访问密钥");
        }
    }
    None
}

fn load_publish_state(config_dir: &Path) -> BTreeMap<String, Value> {
    std::fs::read(config_dir.join(PUBLISH_STATE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// 发布前检查。
pub fn check(manager: &UserScriptManager, config_dir: &Path, id: &str) -> anyhow::Result<CheckReport> {
    let (dir, manifest) = package_dir(manager, id)?;
    let text = |key: &str| manifest.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for (key, label) in [("name", "名称"), ("description", "描述"), ("version", "版本号")] {
        if text(key).is_empty() {
            errors.push(format!("清单缺少{label}（{key}）"));
        }
    }
    if !text("version").is_empty() && !is_semver(&text("version")) {
        errors.push(format!("版本号 {} 不是 x.y.z 形式", text("version")));
    }
    if text("id") != id {
        errors.push(format!("清单 id 应为 {id}"));
    }
    let engine = manifest.pointer("/engines/codexpp").and_then(Value::as_u64).unwrap_or(1);
    if engine > EXTENSION_API_VERSION {
        errors.push(format!("清单要求拓展 API v{engine}，高于本机 v{EXTENSION_API_VERSION}"));
    }
    // config 里 secret 字段的默认值必须为空：默认值会随包发出去。
    for (key, field) in manifest.get("config").and_then(Value::as_object).into_iter().flatten() {
        let secret = field.get("secret").and_then(Value::as_bool).unwrap_or(false);
        let default = field.get("default").map(|value| !value.is_null() && value != "").unwrap_or(false);
        if secret && default {
            errors.push(format!("配置项 {key} 标了 secret，default 必须为空"));
        }
    }
    let files = package_files(&dir, &mut errors);
    let bytes = files.iter().map(|(_, size)| size).sum::<u64>();
    if files.len() > MAX_FILES {
        errors.push(format!("文件超过 {MAX_FILES} 个"));
    }
    if bytes > MAX_BYTES {
        errors.push(format!("总大小超过 {} MiB", MAX_BYTES / 1024 / 1024));
    }
    let secrets = scan_secrets(&dir, &files);
    if !secrets.is_empty() {
        errors.push(format!("发现 {} 处疑似密钥或敏感文件，移除后再发布", secrets.len()));
    }
    let has = |names: &[&str]| files.iter().any(|(file, _)| names.iter().any(|name| file.eq_ignore_ascii_case(name)));
    let has_readme = has(&["README.md", "README", "README.zh.md"]);
    let has_license = has(&["LICENSE", "LICENSE.md", "LICENSE.txt"]);
    if !has_readme {
        warnings.push("没有 README，发布时会按清单生成一份".into());
    }
    if !has_license {
        warnings.push("没有 LICENSE，可以在发布时选择许可证".into());
    }
    let state = load_publish_state(config_dir);
    let published_repo = state
        .get(id)
        .and_then(|record| record.get("repo"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| Some(text("repository")).filter(|repo| repo.split('/').count() == 2));
    Ok(CheckReport {
        id: id.to_string(),
        name: text("name"),
        version: text("version"),
        description: text("description"),
        errors,
        warnings,
        secrets,
        files: files.len(),
        bytes,
        has_readme,
        has_license,
        suggested_repo_name: published_repo
            .as_deref()
            .and_then(|repo| repo.split('/').nth(1))
            .map(str::to_string)
            .unwrap_or_else(|| format!("codexpp-{id}")),
        published_repo,
    })
}

/// 版本号 +1：`part` 为 `patch` 或 `minor`。直接改本地 codexpp.json，返回新版本。
pub fn bump_version(manager: &UserScriptManager, id: &str, part: &str) -> anyhow::Result<String> {
    let (dir, mut manifest) = package_dir(manager, id)?;
    let current = manifest.get("version").and_then(Value::as_str).unwrap_or("0.0.0").to_string();
    let core = current.split(['-', '+']).next().unwrap_or("0.0.0");
    let mut numbers = core.split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    numbers.resize(3, 0);
    match part {
        "patch" => numbers[2] += 1,
        "minor" => {
            numbers[1] += 1;
            numbers[2] = 0;
        }
        other => bail!("不认识的版本部分 {other}"),
    }
    let next = format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
    manifest["version"] = json!(next);
    crate::settings::atomic_write(&dir.join(EXTENSION_MANIFEST_FILE), serde_json::to_string_pretty(&manifest)?.as_bytes())?;
    Ok(next)
}

/// 按清单生成 README：名称、描述、安装方式与能力表。
pub fn generate_readme(manifest: &Value, repo: &str) -> String {
    let text = |key: &str| manifest.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let permissions = crate::extension_discovery::manifest_permissions(manifest);
    let mut rows = Vec::new();
    if let Some(backend) = permissions.get("backend").and_then(Value::as_str) {
        rows.push(format!("| 常驻后台进程 | `{backend}` |"));
    }
    for hook in permissions.get("hooks").and_then(Value::as_array).into_iter().flatten() {
        rows.push(format!("| Codex 钩子 | `{}` {} |", hook["event"].as_str().unwrap_or(""), hook["matcher"].as_str().unwrap_or("")));
    }
    for (key, label) in [("gatewayRequest", "网关请求中间件"), ("gatewayResponse", "网关响应中间件")] {
        if permissions.get(key).and_then(Value::as_u64).unwrap_or(0) > 0 {
            rows.push(format!("| {label} | {} 个 |", permissions[key]));
        }
    }
    for tool in permissions.get("tools").and_then(Value::as_array).into_iter().flatten() {
        rows.push(format!("| 模型工具 | `{}` |", tool.as_str().unwrap_or("")));
    }
    for route in permissions.get("routes").and_then(Value::as_array).into_iter().flatten() {
        rows.push(format!("| 本地路由 | `{}` |", route.as_str().unwrap_or("")));
    }
    for entry in permissions.get("renderer").and_then(Value::as_array).into_iter().flatten() {
        rows.push(format!("| 页面脚本 | `{}` |", entry.as_str().unwrap_or("")));
    }
    let table = if rows.is_empty() {
        "这个拓展没有声明额外能力。".to_string()
    } else {
        format!("| 能力 | 内容 |\n| --- | --- |\n{}", rows.join("\n"))
    };
    format!(
        "# {name}\n\n{description}\n\n## 安装\n\n在 Codex++ 管理器打开「拓展 → 社区拓展」，搜索 `{id}` 点安装；\n或从 [Releases](https://github.com/{repo}/releases) 下载 `{id}-v<版本>.zip`，解压到 Codex++ 的拓展目录。\n\n## 需要的能力\n\n{table}\n\n---\n\n这是一个 [Codex++](https://github.com/BigPizzaV3/CodexPlusPlus) 拓展（topic：`codexpp-extension`）。\n",
        name = text("name"),
        description = text("description"),
        id = text("id"),
    )
}

/// 打包：压缩包根目录就是拓展包根目录。
pub fn build_archive(dir: &Path, destination: &Path) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    let files = package_files(dir, &mut errors);
    if !errors.is_empty() {
        bail!("{}", errors.join("；"));
    }
    let file = std::fs::File::create(destination)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (relative, _) in files {
        writer.start_file(relative.as_str(), options)?;
        writer.write_all(&std::fs::read(dir.join(&relative))?)?;
    }
    writer.finish()?;
    Ok(())
}

/// 在给定目录里找命令文件：目录按给定顺序优先，同一目录内先试带扩展名的候选
/// （Windows 上 `gh` 实际是 `gh.exe`），最后试 `name` 本身；`name` 自带扩展名时
/// 不再追加扩展名。
fn locate_tool_in(
    name: &str,
    dirs: impl IntoIterator<Item = PathBuf>,
    extensions: &[String],
) -> Option<PathBuf> {
    let bare = Path::new(name).extension().is_none() && !extensions.is_empty();
    for dir in dirs {
        if bare {
            for extension in extensions {
                let candidate = dir.join(format!("{name}{extension}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 在常见位置找命令：图形界面启动的应用拿不到 shell 的 PATH。
pub fn locate_tool(name: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // Windows 上可执行文件带扩展名（gh.exe / gh.cmd），按 PATHEXT 尝试；
        // home 在 USERPROFILE 而不是 HOME（#2995）。
        let extensions = std::env::var("PATHEXT")
            .map(|value| {
                value
                    .split(';')
                    .filter(|ext| !ext.trim().is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|_| {
                [".COM", ".EXE", ".BAT", ".CMD"]
                    .iter()
                    .map(|ext| ext.to_string())
                    .collect()
            });
        let mut dirs = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .unwrap_or_default();
        for key in ["USERPROFILE", "HOME"] {
            if let Some(home) = std::env::var_os(key) {
                dirs.push(PathBuf::from(home).join(".local").join("bin"));
            }
        }
        // winget / MSI 的常见安装位置；取不到环境变量就跳过。
        if let Some(dir) = std::env::var_os("ProgramFiles") {
            dirs.push(PathBuf::from(dir).join("GitHub CLI"));
        }
        if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(dir).join("Programs").join("GitHub CLI"));
        }
        return locate_tool_in(name, dirs, &extensions);
    }
    #[cfg(not(windows))]
    {
        let mut dirs = std::env::var_os("PATH")
            .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            dirs.push(home.join(".local/bin"));
        }
        for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"] {
            dirs.push(PathBuf::from(dir));
        }
        locate_tool_in(name, dirs, &[])
    }
}

/// 发布选项。
#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    /// 仓库名（不含 owner），默认 `codexpp-<id>`。
    pub repo_name: String,
    /// 没有 LICENSE 时加的许可证：`mit` / `apache-2.0`；None 不加。
    pub license: Option<String>,
    /// 没有 README 时生成。
    pub generate_readme: bool,
}

/// 发布器。`gh` / `git` 路径、远端地址模板、git 环境都可替换，便于测试。
#[derive(Debug, Clone)]
pub struct Publisher {
    pub gh: PathBuf,
    pub git: PathBuf,
    pub config_dir: PathBuf,
    /// 远端地址模板，`{repo}` 替换为 `owner/name`。
    pub remote_template: String,
    /// 只加给 git 命令的环境变量（测试用来指定 git 配置文件）。
    pub git_env: Vec<(String, String)>,
}

/// 一步的结果，按顺序返回给界面。
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub step: String,
    pub ok: bool,
    pub output: String,
}

/// 发布进度：每一步开始时发一次 `running`，结束时发一次 `ok` / `error`，界面据此逐步刷新。
#[derive(Debug, Clone, Serialize)]
pub struct StepEvent {
    pub index: usize,
    pub step: String,
    pub state: &'static str,
    pub output: String,
}

/// 记录各步结果，同时把进度推给回调。
struct StepLog<'a> {
    steps: Vec<Step>,
    on_progress: &'a dyn Fn(&StepEvent),
}

impl StepLog<'_> {
    /// 跑一步并记下结果，原样返回：失败时调用方用 `?` 停下。
    fn run(&mut self, name: &str, action: impl FnOnce() -> anyhow::Result<String>) -> anyhow::Result<String> {
        let index = self.steps.len();
        (self.on_progress)(&StepEvent { index, step: name.into(), state: "running", output: String::new() });
        let result = action();
        let (ok, output) = match &result {
            Ok(text) => (true, text.clone()),
            Err(error) => (false, format!("{error:#}")),
        };
        (self.on_progress)(&StepEvent { index, step: name.into(), state: if ok { "ok" } else { "error" }, output: output.clone() });
        self.steps.push(Step { step: name.into(), ok, output });
        result
    }
}

impl Publisher {
    pub fn new(config_dir: impl Into<PathBuf>) -> anyhow::Result<Self> {
        Ok(Self {
            gh: locate_tool("gh").context("没有找到 GitHub CLI（gh）：先安装 gh（brew install gh），再运行 gh auth login 登录")?,
            git: locate_tool("git").context("没有找到 git")?,
            config_dir: config_dir.into(),
            remote_template: "https://github.com/{repo}.git".into(),
            git_env: Vec::new(),
        })
    }

    fn run(&self, program: &Path, args: &[&str], cwd: Option<&Path>) -> anyhow::Result<String> {
        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        if program == self.git {
            for (key, value) in &self.git_env {
                command.env(key, value);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_create_no_window());
        }
        let output = command.output().with_context(|| format!("无法运行 {}", program.display()))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            bail!("{} {} 失败：{}", program.file_name().unwrap_or_default().to_string_lossy(), args.join(" "), if stderr.is_empty() { &stdout } else { &stderr });
        }
        Ok(if stdout.is_empty() { stderr } else { stdout })
    }

    pub(crate) fn gh(&self, args: &[&str]) -> anyhow::Result<String> {
        self.run(&self.gh, args, None)
    }

    fn git(&self, args: &[&str], cwd: &Path) -> anyhow::Result<String> {
        self.run(&self.git, args, Some(cwd))
    }

    /// gh 是否登录、当前账号。
    pub fn account(&self) -> anyhow::Result<String> {
        self.gh(&["auth", "status"]).context("gh 没有登录：在终端运行 gh auth login")?;
        let login = self.gh(&["api", "user", "--jq", ".login"])?;
        if login.is_empty() {
            bail!("读不到 gh 当前账号");
        }
        Ok(login)
    }

    /// 发布。返回每一步的结果；失败时停在那一步。
    pub fn publish(&self, manager: &UserScriptManager, id: &str, options: &PublishOptions) -> Value {
        self.publish_with_progress(manager, id, options, &|_| {})
    }

    /// 同 [`Self::publish`]，每一步开始、结束时调用 `on_progress`。
    pub fn publish_with_progress(&self, manager: &UserScriptManager, id: &str, options: &PublishOptions, on_progress: &dyn Fn(&StepEvent)) -> Value {
        let mut log = StepLog { steps: Vec::new(), on_progress };
        let result = self.publish_steps(manager, id, options, &mut log);
        let steps = log.steps;
        match result {
            Ok((repo, version)) => json!({
                "ok": true, "repo": repo, "version": version,
                "url": format!("https://github.com/{repo}"),
                "steps": steps,
            }),
            Err(error) => json!({ "ok": false, "error": format!("{error:#}"), "steps": steps }),
        }
    }

    fn publish_steps(
        &self,
        manager: &UserScriptManager,
        id: &str,
        options: &PublishOptions,
        steps: &mut StepLog,
    ) -> anyhow::Result<(String, String)> {
        let report = check(manager, &self.config_dir, id)?;
        steps.run("发布前检查", || {
            if !report.errors.is_empty() {
                bail!("{}", report.errors.join("；"));
            }
            Ok(format!("{} 个文件，{} 字节", report.files, report.bytes))
        })?;
        let (dir, mut manifest) = package_dir(manager, id)?;
        let version = report.version.clone();
        let tag = format!("v{version}");
        let owner = steps.run("确认 GitHub 账号", || self.account())?;
        let name = if options.repo_name.trim().is_empty() { report.suggested_repo_name.clone() } else { options.repo_name.trim().to_string() };
        if name.is_empty() || !name.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.')) {
            bail!("仓库名只能包含字母、数字、- _ .");
        }
        let repo = format!("{owner}/{name}");
        std::fs::create_dir_all(&self.config_dir)?;
        steps.run("确认 git 身份", || {
            self.git(&["config", "--get", "user.name"], &self.config_dir)
                .and_then(|_| self.git(&["config", "--get", "user.email"], &self.config_dir))
                .map(|_| "已配置 user.name / user.email".into())
                .context("git 没有配置提交身份：在终端运行 git config --global user.name \"你的名字\" 和 git config --global user.email \"你的邮箱\"")
        })?;

        let mut exists = false;
        steps.run("检查仓库与版本", || {
            exists = self.gh(&["repo", "view", &repo, "--json", "name"]).is_ok();
            if !exists {
                return Ok(format!("{repo} 还不存在，将新建"));
            }
            if self.gh(&["api", &format!("repos/{repo}/git/ref/tags/{tag}")]).is_ok() {
                bail!("{repo} 已经有 {tag}，先把版本号 +1 再发布");
            }
            Ok(format!("{repo} 已存在，发布 {tag}"))
        })?;
        let work_root = self.config_dir.join(PUBLISH_DIR);
        std::fs::create_dir_all(&work_root)?;
        let work = work_root.join(id);
        if work.exists() {
            std::fs::remove_dir_all(&work).with_context(|| format!("无法清理工作副本 {}", work.display()))?;
        }
        let remote = self.remote_template.replace("{repo}", &repo);
        let work_text = work.to_string_lossy().to_string();
        if exists {
            steps.run("取回仓库", || self.git(&["clone", &remote, &work_text], &work_root))?;
        } else {
            let description = report.description.clone();
            steps.run("创建仓库", || self.gh(&["repo", "create", &repo, "--public", "--description", &description]))?;
            steps.run("初始化工作副本", || self.git(&["init", "-b", "main", &work_text], &work_root).and_then(|_| self.git(&["remote", "add", "origin", &remote], &work)))?;
        }

        // 用拓展包当前文件整体替换工作副本（保留 .git）。
        for entry in std::fs::read_dir(&work)?.flatten() {
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        let mut copy_errors = Vec::new();
        let files = package_files(&dir, &mut copy_errors);
        if !copy_errors.is_empty() {
            bail!("{}", copy_errors.join("；"));
        }
        for (relative, _) in &files {
            let target = work.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(dir.join(relative), &target)?;
        }
        // 清单写回 repository，本地包也写一份，下次「发布新版本」用。
        manifest["repository"] = json!(repo);
        let manifest_text = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(work.join(EXTENSION_MANIFEST_FILE), &manifest_text)?;
        crate::settings::atomic_write(&dir.join(EXTENSION_MANIFEST_FILE), manifest_text.as_bytes())?;
        let mut extras = Vec::new();
        if !report.has_readme && options.generate_readme {
            let readme = generate_readme(&manifest, &repo);
            std::fs::write(work.join("README.md"), &readme)?;
            std::fs::write(dir.join("README.md"), &readme)?;
            extras.push("生成 README.md");
        }
        if let Some(license) = options.license.as_deref().filter(|_| !report.has_license) {
            let key = match license {
                "mit" | "MIT" => "mit",
                "apache-2.0" | "Apache-2.0" => "apache-2.0",
                other => bail!("不支持的许可证 {other}"),
            };
            let mut body = String::new();
            steps.run("取许可证文本", || {
                body = self.gh(&["api", &format!("licenses/{key}"), "--jq", ".body"])?;
                // 发布框里只显示许可证名，不刷整段正文。
                Ok(body.lines().next().unwrap_or(key).to_string())
            })?;
            let year = 1970 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0) / 31_557_600;
            let author = manifest.get("author").and_then(Value::as_str).filter(|author| !author.is_empty()).unwrap_or(&owner).to_string();
            let body = body.replace("[year]", &year.to_string()).replace("[fullname]", &author);
            std::fs::write(work.join("LICENSE"), &body)?;
            std::fs::write(dir.join("LICENSE"), &body)?;
            extras.push("添加 LICENSE");
        }
        steps.run("整理文件", || Ok(format!("{} 个文件{}", files.len(), if extras.is_empty() { String::new() } else { format!("，{}", extras.join("、")) })))?;

        let message = format!("release {tag}");
        steps.run("提交", || self.git(&["add", "-A"], &work).and_then(|_| self.git(&["commit", "--allow-empty", "-m", &message], &work)))?;
        steps.run("打标签", || self.git(&["tag", "-a", &tag, "-m", &tag], &work))?;
        let helper = format!("credential.helper=!'{}' auth git-credential", self.gh.display());
        let tag_ref = format!("refs/tags/{tag}");
        steps.run("推送", || self.git(&["-c", "credential.helper=", "-c", &helper, "push", "origin", "HEAD", &tag_ref], &work))?;

        let archive = work_root.join(crate::extension_discovery::asset_name(id, &tag));
        let archive_text = archive.to_string_lossy().to_string();
        steps.run("创建 release", || {
            build_archive(&work, &archive).context("打包失败")?;
            self.gh(&["release", "create", &tag, &archive_text, "--repo", &repo, "--title", &tag, "--generate-notes"])
        })?;
        steps.run("添加 topic", || self.gh(&["repo", "edit", &repo, "--add-topic", crate::extension_discovery::TOPIC]))?;

        let mut state = load_publish_state(&self.config_dir);
        state.insert(id.to_string(), json!({ "repo": repo, "version": version, "publishedAt": now_secs() }));
        crate::settings::atomic_write(&self.config_dir.join(PUBLISH_STATE), serde_json::to_string_pretty(&state)?.as_bytes())?;
        Ok((repo, version))
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 「我发布的」：列出、下架、重新上架、删除仓库
// ---------------------------------------------------------------------------

impl Publisher {
    /// 当前 gh 账号名下的拓展仓库：带 `codexpp-extension` topic 的（社区栏里能看到），
    /// 加上本机发布记录里有、但已经下架的（仓库还在，可以重新上架或删除）。
    pub fn list_published(&self) -> anyhow::Result<Value> {
        let account = self.account()?;
        let raw = self.gh(&[
            "repo", "list", &account, "--limit", "1000", "--json",
            "nameWithOwner,description,url,pushedAt,stargazerCount,isArchived,visibility,repositoryTopics",
        ])?;
        let repos: Vec<Value> = serde_json::from_str(&raw).context("gh repo list 返回的不是 JSON")?;
        let state = load_publish_state(&self.config_dir);
        let topic = crate::extension_discovery::TOPIC;
        let mut entries = repos
            .iter()
            .filter_map(|item| {
                let repo = item.get("nameWithOwner")?.as_str()?.to_string();
                let listed = item
                    .get("repositoryTopics")
                    .and_then(Value::as_array)
                    .is_some_and(|topics| topics.iter().any(|t| t.get("name").and_then(Value::as_str) == Some(topic)));
                let record = state.iter().find(|(_, record)| {
                    record.get("repo").and_then(Value::as_str).is_some_and(|r| r.eq_ignore_ascii_case(&repo))
                });
                if !listed && record.is_none() {
                    return None;
                }
                let text = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                Some(json!({
                    "repo": repo,
                    "url": text("url"),
                    "description": text("description"),
                    "stars": item.get("stargazerCount").and_then(Value::as_u64).unwrap_or(0),
                    "pushedAt": text("pushedAt"),
                    "visibility": text("visibility"),
                    "archived": item.get("isArchived").and_then(Value::as_bool).unwrap_or(false),
                    "listed": listed,
                    "id": record.map(|(id, _)| id.clone()),
                    "version": record.and_then(|(_, r)| r.get("version").cloned()),
                    "publishedAt": record.and_then(|(_, r)| r.get("publishedAt").cloned()),
                }))
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| b["pushedAt"].as_str().cmp(&a["pushedAt"].as_str()));
        Ok(json!({ "account": account, "entries": entries }))
    }

    /// 下架：去掉 topic，社区栏不再列出；仓库、release 都保留，可以重新上架。
    pub fn unlist(&self, repo: &str) -> anyhow::Result<()> {
        self.own_repo(repo)?;
        self.gh(&["repo", "edit", repo, "--remove-topic", crate::extension_discovery::TOPIC])?;
        Ok(())
    }

    /// 重新上架：加回 topic。
    pub fn relist(&self, repo: &str) -> anyhow::Result<()> {
        self.own_repo(repo)?;
        self.gh(&["repo", "edit", repo, "--add-topic", crate::extension_discovery::TOPIC])?;
        Ok(())
    }

    /// 删除 GitHub 仓库（不可恢复），并清掉本机的发布记录。本地拓展包不动。
    pub fn delete_repo(&self, repo: &str) -> anyhow::Result<()> {
        self.own_repo(repo)?;
        if let Err(error) = self.gh(&["repo", "delete", repo, "--yes"]) {
            let message = format!("{error:#}");
            if message.contains("delete_repo") {
                bail!("gh 的令牌没有删除仓库的权限：在终端运行 gh auth refresh -h github.com -s delete_repo 授权后再试");
            }
            return Err(error);
        }
        let mut state = load_publish_state(&self.config_dir);
        let before = state.len();
        state.retain(|_, record| !record.get("repo").and_then(Value::as_str).is_some_and(|r| r.eq_ignore_ascii_case(repo)));
        if state.len() != before {
            crate::settings::atomic_write(&self.config_dir.join(PUBLISH_STATE), serde_json::to_string_pretty(&state)?.as_bytes())?;
        }
        Ok(())
    }

    /// 只能动当前 gh 账号自己名下的仓库。
    fn own_repo(&self, repo: &str) -> anyhow::Result<()> {
        let parts = repo.split('/').collect::<Vec<_>>();
        let valid = parts.len() == 2
            && parts.iter().all(|part| {
                !part.is_empty() && *part != "." && *part != ".." && part.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            });
        if !valid {
            bail!("仓库必须写成 owner/repo");
        }
        let account = self.account()?;
        if !parts[0].eq_ignore_ascii_case(&account) {
            bail!("{repo} 不属于当前 gh 账号 {account}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #2995：Windows 上 `gh` 实际是 `gh.exe`，按 PATHEXT 扩展名候选能找到。
    #[test]
    fn locate_tool_finds_extension_candidates_in_dir_order() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("gh.EXE"), "").unwrap();
        let extensions = vec![".EXE".to_string(), ".CMD".to_string()];
        assert_eq!(
            locate_tool_in("gh", vec![first.clone(), second.clone()], &extensions),
            Some(second.join("gh.EXE")),
            "目录顺序优先，命中靠前目录里没有扩展名候选时继续往后找"
        );
        std::fs::write(first.join("gh.EXE"), "").unwrap();
        assert_eq!(
            locate_tool_in("gh", vec![first.clone(), second], &extensions),
            Some(first.join("gh.EXE"))
        );
    }

    /// 同一目录里扩展名候选排在裸名之前（Windows 创建进程需要带扩展名的路径更稳）。
    #[test]
    fn locate_tool_prefers_extension_candidate_over_bare_name() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        std::fs::write(dir.join("gh"), "").unwrap();
        std::fs::write(dir.join("gh.EXE"), "").unwrap();
        let extensions = vec![".EXE".to_string()];
        assert_eq!(
            locate_tool_in("gh", vec![dir.clone()], &extensions),
            Some(dir.join("gh.EXE"))
        );
    }

    /// 扩展名列表为空（非 Windows）时只找裸名，行为与原来一致。
    #[test]
    fn locate_tool_without_extensions_only_matches_bare_name() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        std::fs::write(dir.join("gh.exe"), "").unwrap();
        assert_eq!(locate_tool_in("gh", vec![dir.clone()], &[]), None);
        std::fs::write(dir.join("gh"), "").unwrap();
        assert_eq!(locate_tool_in("gh", vec![dir.clone()], &[]), Some(dir.join("gh")));
    }

    /// `name` 自带扩展名时不再追加：找 `gh.exe` 不会变成 `gh.exe.exe`。
    #[test]
    fn locate_tool_name_with_extension_is_not_extended() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        std::fs::write(dir.join("gh.exe"), "").unwrap();
        let extensions = vec![".EXE".to_string()];
        assert_eq!(
            locate_tool_in("gh.exe", vec![dir.clone()], &extensions),
            Some(dir.join("gh.exe"))
        );
    }

    /// 什么都找不到时返回 None。
    #[test]
    fn locate_tool_returns_none_when_absent() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            locate_tool_in("gh", vec![temp.path().to_path_buf()], &[".EXE".to_string()]),
            None
        );
    }
}
