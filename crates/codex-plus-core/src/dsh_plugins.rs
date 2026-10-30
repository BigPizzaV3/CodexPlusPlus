//! 导入 dsh（deepseek-harness）插件，让它们在 Codex App 里可用。
//!
//! dsh 插件是 Cordis 插件包，依赖 dsh 的 agent 循环与服务；Codex App 里没有这些，所以按层落地
//! （设计见 `docs/plans/2026-10-30-dsh-plugin-import-design.md`）：
//!
//! - 工具插件：在 Node 子进程里的兼容宿主（`assets/dsh-host/`）中运行，经托管 MCP
//!   `[mcp_servers.codex_plus_dsh]` 把工具暴露给 Codex；
//! - skill：包里的 `SKILL.md` 目录交给 Skills 管理导入；
//! - MCP 配置行：转换成标准 `mcpServers` JSON，由用户走现有「导入 MCP」流程；
//! - 系统提示词片段与能对上的 dsh 事件（工具前后、回合前、回合停止）：生成配套拓展包
//!   `dsh-<插件 id>`，用网关请求中间件追加 instructions、用 Codex 钩子转发事件（宿主的 `bridge.mjs`）。
//!
//! 本模块负责来源解析与获取（本地目录 / npm / git）、依赖安装、调用宿主分析、注册表，以及 MCP 启动入口。
//! 插件代码以用户权限运行、没有沙箱（与 dsh 相同）；依赖安装一律 `--ignore-scripts`。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::managed_mcp::{self, ManagedMcp};
use crate::user_scripts::{ExtensionWrite, UserScriptManager};

pub use crate::managed_mcp::RegistrationStatus;

pub const SERVER_ID: &str = "codex_plus_dsh";
pub const MCP_FLAG: &str = "--codex-plus-dsh-mcp";

pub const SPEC: ManagedMcp = ManagedMcp {
    server_id: SERVER_ID,
    flag: MCP_FLAG,
    label: "dsh 插件 MCP",
    comment: "Managed by Codex++: dsh plugins MCP",
    lock_file: ".codex-plus-dsh-mcp.lock",
    backup_dir: ".codex-plus-dsh-mcp-backups",
    legacy: &[],
};

/// 兼容宿主的文件，随程序内嵌，运行前写到状态目录（内容不变不重写）。
const HOST_FILES: &[(&str, &str)] = &[
    ("host.mjs", include_str!("../../../assets/dsh-host/host.mjs")),
    ("loader.mjs", include_str!("../../../assets/dsh-host/loader.mjs")),
    ("yaml.mjs", include_str!("../../../assets/dsh-host/yaml.mjs")),
    ("bridge.mjs", include_str!("../../../assets/dsh-host/bridge.mjs")),
    (
        "shims/dsh-tools.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-tools.mjs"),
    ),
    (
        "shims/schemastery.mjs",
        include_str!("../../../assets/dsh-host/shims/schemastery.mjs"),
    ),
    (
        "shims/cordis.mjs",
        include_str!("../../../assets/dsh-host/shims/cordis.mjs"),
    ),
    (
        "shims/dsh-skill.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-skill.mjs"),
    ),
    (
        "shims/dsh-system-prompt.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-system-prompt.mjs"),
    ),
    (
        "shims/dsh-llm.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-llm.mjs"),
    ),
];

const REGISTRY_VERSION: u32 = 1;
/// 分析一个包会执行插件的 apply，给足时间但不能无限等。
const INSPECT_TIMEOUT: Duration = Duration::from_secs(45);
const FETCH_TIMEOUT: Duration = Duration::from_secs(180);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_SOURCE_LEN: usize = 512;
const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;
/// 宿主自己提供的 peer 包；只依赖它们时不需要跑 npm install。
/// schemastery 按 dsh 约定是普通运行时依赖，声明了就装真包，替身只兜底没装的情况。
const HOST_PROVIDED_PACKAGES: &[&str] = &["@deepseek-ai/cordis", "@deepseek-ai/dsh-tools"];

/// 插件来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginSource {
    /// 本地目录，原地链接（同 dsh 的 `link:`），不复制。
    Local { path: String },
    /// npm 包 spec，如 `@scope/name@1.2.3`。
    Npm { spec: String },
    /// git 仓库，可指定分支/标签与包所在子目录。
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub source: PluginSource,
    /// 包目录（宿主从这里加载）。
    pub root: String,
    /// 包目录由 Codex++ 管理（npm / git），移除时一并移入备份；本地目录只解除登记。
    #[serde(default)]
    pub managed: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 用户在管理器里停用的 bundle 行。
    #[serde(default)]
    pub disabled_rows: Vec<String>,
    /// 按行覆盖的插件配置，浅合并到 bundle patch 的配置之上。
    #[serde(default)]
    pub configs: BTreeMap<String, Value>,
    #[serde(default)]
    pub imported_at: u64,
    /// 宿主最近一次的分析报告。
    #[serde(default)]
    pub report: Value,
    /// 已导入 Codex Skills 的 skill id。
    #[serde(default)]
    pub imported_skills: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// 解析用户输入的来源：绝对路径 / `~/` 是本地目录；URL、`git@`、`.git` 结尾是 git；其余按 npm spec。
/// git 地址可用 `#` 片段指定分支与子目录：`#main&path:packages/foo` 或 `#path:packages/foo`。
pub fn parse_source(input: &str) -> anyhow::Result<PluginSource> {
    let text = input.trim();
    if text.is_empty() {
        bail!("请输入 npm 包名、git 地址或本地目录");
    }
    if text.len() > MAX_SOURCE_LEN || text.chars().any(char::is_control) {
        bail!("来源过长或包含控制字符");
    }
    if let Some(rest) = text.strip_prefix("~/") {
        let home = directories::BaseDirs::new()
            .context("无法定位用户目录")?
            .home_dir()
            .to_path_buf();
        return Ok(PluginSource::Local {
            path: home.join(rest).to_string_lossy().into_owned(),
        });
    }
    if Path::new(text).is_absolute() {
        return Ok(PluginSource::Local {
            path: text.to_string(),
        });
    }
    let git_like = text.starts_with("https://")
        || text.starts_with("http://")
        || text.starts_with("git+")
        || text.starts_with("git@")
        || text.starts_with("ssh://")
        || text.split('#').next().is_some_and(|url| url.ends_with(".git"));
    if git_like {
        let (url, fragment) = match text.split_once('#') {
            Some((url, fragment)) => (url, Some(fragment)),
            None => (text, None),
        };
        let url = url.strip_prefix("git+").unwrap_or(url).to_string();
        if url.starts_with('-') || url.chars().any(char::is_whitespace) {
            bail!("git 地址不合法");
        }
        let mut reference = None;
        let mut path = None;
        for part in fragment.unwrap_or("").split('&').filter(|part| !part.is_empty()) {
            if let Some(sub) = part.strip_prefix("path:") {
                path = Some(validate_relative_subdir(sub)?);
            } else {
                if part.starts_with('-') || part.chars().any(char::is_whitespace) {
                    bail!("git 分支或标签不合法");
                }
                reference = Some(part.to_string());
            }
        }
        return Ok(PluginSource::Git {
            url,
            reference,
            path,
        });
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || "@/._~^<>=*|+-".contains(c);
    if text.starts_with('-') || text.starts_with('.') || !text.chars().all(allowed) {
        bail!("无法识别的来源：npm 包名只能包含字母、数字与 @/._-，本地目录请用绝对路径");
    }
    Ok(PluginSource::Npm {
        spec: text.to_string(),
    })
}

fn validate_relative_subdir(sub: &str) -> anyhow::Result<String> {
    let sub = sub.trim_matches('/');
    let path = Path::new(sub);
    if sub.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        bail!("git 子目录必须是仓库内的相对路径");
    }
    Ok(sub.to_string())
}

/// 由包名生成稳定的插件 id：`@scope/name` → `scope__name`。
pub fn plugin_id_from_name(name: &str) -> String {
    let mut id = String::new();
    for c in name.trim().trim_start_matches('@').chars() {
        match c {
            'a'..='z' | '0'..='9' | '-' | '_' | '.' => id.push(c),
            'A'..='Z' => id.push(c.to_ascii_lowercase()),
            '/' => id.push_str("__"),
            _ => id.push('-'),
        }
    }
    let id = id.trim_matches(|c| c == '.' || c == '-' || c == '_').to_string();
    if !id.chars().any(|c| c.is_ascii_alphanumeric()) {
        "plugin".to_string()
    } else {
        id.chars().take(96).collect()
    }
}

fn validate_plugin_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty()
        || id.len() > 96
        || id.starts_with('.')
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c))
    {
        bail!("插件 id 不合法：{id}");
    }
    Ok(())
}

/// 运行宿主与获取插件用到的外部程序。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Toolchain {
    pub node: Option<PathBuf>,
    pub npm: Option<PathBuf>,
    pub git: Option<PathBuf>,
}

impl Toolchain {
    pub fn detect() -> Self {
        Self {
            node: locate_node(),
            npm: locate_program("npm"),
            git: locate_program("git"),
        }
    }

    fn require_node(&self) -> anyhow::Result<&Path> {
        self.node.as_deref().context(
            "没有找到 Node 运行时：请安装 Codex App（自带 Node）或 Node.js，也可以用 CODEX_PLUS_NODE 指定",
        )
    }
}

/// 找 Node：`CODEX_PLUS_NODE` → Codex App 自带的 cua_node → PATH 与常见安装目录。
pub fn locate_node() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_PLUS_NODE").map(PathBuf::from) {
        if path.is_absolute() && path.is_file() {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Codex.app/Contents/Resources/cua_node/bin/node",
        ));
        if let Some(base) = directories::BaseDirs::new() {
            candidates.push(
                base.home_dir()
                    .join("Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node"),
            );
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| locate_program("node"))
}

/// 在 PATH 与常见安装目录里找程序。访达启动的 App 拿不到 shell 的 PATH，补上常见位置。
pub fn locate_program(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    if let Some(base) = directories::BaseDirs::new() {
        for extra in [".local/bin", ".volta/bin", ".npm-global/bin", ".bun/bin"] {
            dirs.push(base.home_dir().join(extra));
        }
    }
    if cfg!(unix) {
        dirs.push(PathBuf::from("/opt/homebrew/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs.push(PathBuf::from("/usr/bin"));
    }
    dirs.into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// 子进程的 PATH：npm 是 Node 脚本，需要能找到 node。
fn child_path(toolchain: &Toolchain) -> OsString {
    let mut dirs = Vec::new();
    for program in [&toolchain.npm, &toolchain.node, &toolchain.git]
        .into_iter()
        .flatten()
    {
        if let Some(parent) = program.parent() {
            dirs.push(parent.to_path_buf());
        }
        if let Some(parent) = std::fs::canonicalize(program)
            .ok()
            .and_then(|real| real.parent().map(Path::to_path_buf))
        {
            dirs.push(parent);
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// 运行子进程并限时；超时会杀掉它。stdout/stderr 在后台线程读，避免管道写满卡死。
fn run_with_timeout(mut command: Command, timeout: Duration, label: &str) -> anyhow::Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("无法启动{label}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let read = |pipe: Option<std::process::ChildStdout>| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
            }
            buffer
        })
    };
    let stdout_reader = read(stdout);
    let stderr_reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = stderr {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        }
        buffer
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{label}超过 {} 秒未完成，已终止", timeout.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let output = Output {
        stdout: stdout_reader.join().unwrap_or_default(),
        stderr: stderr_reader.join().unwrap_or_default(),
    };
    if !status.success() {
        bail!("{label}失败：{}", tail_text(&output.stderr, &output.stdout));
    }
    Ok(output)
}

fn tail_text(stderr: &[u8], stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(if stderr.iter().any(|b| !b.is_ascii_whitespace()) {
        stderr
    } else {
        stdout
    });
    let lines: Vec<&str> = text.lines().filter(|line| !line.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(8);
    let tail = lines[start..].join("\n");
    if tail.is_empty() {
        "（没有输出）".to_string()
    } else {
        tail.chars().take(2000).collect()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

/// dsh 插件的本地存储：`<state>/dsh-plugins/`。
#[derive(Debug, Clone)]
pub struct DshPlugins {
    root: PathBuf,
}

impl DshPlugins {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            root: state_dir.join("dsh-plugins"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn registry_path(&self) -> PathBuf {
        self.root.join("registry.json")
    }

    fn packages_dir(&self) -> PathBuf {
        self.root.join("packages")
    }

    fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    fn staging_dir(&self) -> PathBuf {
        self.root.join(".staging")
    }

    /// 插件在代码里注册的 skill 被宿主写成 SKILL.md 目录放在这里，再导入 Codex Skills。
    fn skills_dir(&self, id: &str) -> PathBuf {
        self.root.join("skills").join(id)
    }

    /// 写出内嵌的兼容宿主，返回 `host.mjs` 路径。内容没变不重写：每个 Codex 会话都会启动一次 MCP。
    pub fn ensure_host(&self) -> anyhow::Result<PathBuf> {
        let directory = self.root.join("host");
        for (name, contents) in HOST_FILES {
            let path = directory.join(name);
            if std::fs::read_to_string(&path).is_ok_and(|current| current == *contents) {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("无法创建目录 {}", parent.display()))?;
            }
            crate::settings::atomic_write(&path, contents.as_bytes())
                .with_context(|| format!("无法写入兼容宿主 {name}"))?;
        }
        Ok(directory.join("host.mjs"))
    }

    pub fn load(&self) -> anyhow::Result<Registry> {
        match std::fs::read_to_string(self.registry_path()) {
            Ok(text) => {
                let mut registry: Registry =
                    serde_json::from_str(&text).context("dsh 插件注册表损坏")?;
                registry.version = REGISTRY_VERSION;
                Ok(registry)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry {
                version: REGISTRY_VERSION,
                plugins: Vec::new(),
            }),
            Err(error) => Err(error).context("无法读取 dsh 插件注册表"),
        }
    }

    fn save(&self, registry: &Registry) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.root)
            .with_context(|| format!("无法创建目录 {}", self.root.display()))?;
        crate::settings::atomic_write(
            &self.registry_path(),
            serde_json::to_string_pretty(registry)?.as_bytes(),
        )
        .context("无法保存 dsh 插件注册表")
    }

    /// 导入一个插件包：获取 → 安装依赖 → 分析 → 登记。同名插件视为更新，旧的托管目录移入备份。
    pub fn import(&self, source: &PluginSource, toolchain: &Toolchain) -> anyhow::Result<PluginRecord> {
        let host = self.ensure_host()?;
        let node = toolchain.require_node()?;
        let staging = self
            .staging_dir()
            .join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&staging)
            .with_context(|| format!("无法创建临时目录 {}", staging.display()))?;
        let result = self.import_in(source, toolchain, node, &host, &staging);
        let _ = remove_dir_quietly(&staging);
        result
    }

    fn import_in(
        &self,
        source: &PluginSource,
        toolchain: &Toolchain,
        node: &Path,
        host: &Path,
        staging: &Path,
    ) -> anyhow::Result<PluginRecord> {
        let (fetched, managed) = match source {
            PluginSource::Local { path } => {
                let directory = Path::new(path)
                    .canonicalize()
                    .with_context(|| format!("目录不存在：{path}"))?;
                if !directory.join("package.json").is_file() {
                    bail!("{} 下没有 package.json，不是 dsh 插件包", directory.display());
                }
                (directory, false)
            }
            PluginSource::Npm { spec } => (fetch_npm(spec, toolchain, staging)?, true),
            PluginSource::Git {
                url,
                reference,
                path,
            } => (
                fetch_git(url, reference.as_deref(), path.as_deref(), toolchain, staging)?,
                true,
            ),
        };
        let package = read_package_json(&fetched)?;
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                fetched
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "plugin".to_string())
            });
        let id = plugin_id_from_name(&name);
        validate_plugin_id(&id)?;
        if managed && needs_dependency_install(&package) {
            install_dependencies(&fetched, toolchain)?;
        }
        let mut registry = self.load()?;
        let previous = registry.plugins.iter().position(|plugin| plugin.id == id);
        // 托管包落到 packages/<id>；先分析暂存目录，失败就不动已有版本。
        let root = if managed {
            self.packages_dir().join(&id)
        } else {
            fetched.clone()
        };
        let mut record = PluginRecord {
            id: id.clone(),
            name: name.clone(),
            version: package
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            source: source.clone(),
            root: root.to_string_lossy().into_owned(),
            managed,
            enabled: true,
            disabled_rows: Vec::new(),
            configs: BTreeMap::new(),
            imported_at: now_millis(),
            report: Value::Null,
            imported_skills: Vec::new(),
        };
        if let Some(index) = previous {
            let old = &registry.plugins[index];
            record.enabled = old.enabled;
            record.disabled_rows = old.disabled_rows.clone();
            record.configs = old.configs.clone();
            record.imported_skills = old.imported_skills.clone();
        }
        if managed {
            // 先在暂存位置确认包可分析，再替换正式目录。
            inspect_package(node, host, &fetched, &record, staging, &staging.join("skills"))?;
            std::fs::create_dir_all(self.packages_dir())?;
            if std::fs::symlink_metadata(&root).is_ok() {
                self.backup(&id, &root)?;
            }
            move_dir(&fetched, &root)?;
        }
        record.report = inspect_package(node, host, &root, &record, staging, &self.skills_dir(&id))?;
        if let Some(index) = previous {
            let old = registry.plugins.remove(index);
            // 换了位置的旧托管目录同样移入备份，不留孤儿目录。
            if old.managed && old.root != record.root && Path::new(&old.root).exists() {
                let _ = self.backup(&old.id, Path::new(&old.root));
            }
        }
        registry.plugins.push(record.clone());
        self.save(&registry)?;
        Ok(record)
    }

    /// 重新分析（改配置、改了本地源码之后）。
    pub fn reinspect(&self, id: &str, toolchain: &Toolchain) -> anyhow::Result<PluginRecord> {
        let host = self.ensure_host()?;
        let node = toolchain.require_node()?;
        let mut registry = self.load()?;
        let record = registry
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == id)
            .with_context(|| format!("没有 id 为 {id} 的 dsh 插件"))?;
        let staging = self
            .staging_dir()
            .join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&staging)?;
        let skills_out = self.skills_dir(&record.id);
        let report = inspect_package(node, &host, Path::new(&record.root), record, &staging, &skills_out);
        let _ = remove_dir_quietly(&staging);
        record.report = report?;
        let record = record.clone();
        self.save(&registry)?;
        Ok(record)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        self.update(id, |record| {
            record.enabled = enabled;
            Ok(())
        })
    }

    pub fn set_row_enabled(&self, id: &str, row_id: &str, enabled: bool) -> anyhow::Result<()> {
        self.update(id, |record| {
            record.disabled_rows.retain(|row| row != row_id);
            if !enabled {
                record.disabled_rows.push(row_id.to_string());
            }
            Ok(())
        })
    }

    /// 设置某一行的配置覆盖；`None` 或空对象表示清除。
    pub fn set_row_config(&self, id: &str, row_id: &str, config: Option<Value>) -> anyhow::Result<()> {
        self.update(id, |record| {
            match config {
                Some(Value::Object(map)) if !map.is_empty() => {
                    record.configs.insert(row_id.to_string(), Value::Object(map));
                }
                Some(Value::Object(_)) | Some(Value::Null) | None => {
                    record.configs.remove(row_id);
                }
                Some(_) => bail!("配置必须是 JSON 对象"),
            }
            Ok(())
        })
    }

    pub fn set_imported_skills(&self, id: &str, skills: Vec<String>) -> anyhow::Result<()> {
        self.update(id, |record| {
            record.imported_skills = skills;
            Ok(())
        })
    }

    fn update(
        &self,
        id: &str,
        change: impl FnOnce(&mut PluginRecord) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let mut registry = self.load()?;
        let record = registry
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == id)
            .with_context(|| format!("没有 id 为 {id} 的 dsh 插件"))?;
        change(record)?;
        self.save(&registry)
    }

    /// 移除插件：托管目录移入备份（可恢复），本地目录只解除登记。
    pub fn remove(&self, id: &str) -> anyhow::Result<PluginRecord> {
        let mut registry = self.load()?;
        let index = registry
            .plugins
            .iter()
            .position(|plugin| plugin.id == id)
            .with_context(|| format!("没有 id 为 {id} 的 dsh 插件"))?;
        let record = registry.plugins.remove(index);
        if record.managed {
            let root = PathBuf::from(&record.root);
            // 只处理落在自己 packages 目录里的路径，注册表被改坏也不会挪走别处的目录。
            if root.starts_with(self.packages_dir()) && std::fs::symlink_metadata(&root).is_ok() {
                self.backup(id, &root)?;
            }
        }
        self.save(&registry)?;
        Ok(record)
    }

    fn backup(&self, id: &str, directory: &Path) -> anyhow::Result<PathBuf> {
        let backups = self.backups_dir();
        std::fs::create_dir_all(&backups)?;
        let target = backups.join(format!("{id}-{}", now_millis()));
        move_dir(directory, &target)?;
        Ok(target)
    }
}

fn read_package_json(directory: &Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(directory.join("package.json"))
        .with_context(|| format!("{} 下没有可读的 package.json", directory.display()))?;
    serde_json::from_str(&text).context("package.json 不是有效 JSON")
}

fn needs_dependency_install(package: &Value) -> bool {
    package
        .get("dependencies")
        .and_then(Value::as_object)
        .is_some_and(|dependencies| {
            dependencies
                .keys()
                .any(|name| !HOST_PROVIDED_PACKAGES.contains(&name.as_str()))
        })
}

fn npm_command(toolchain: &Toolchain, directory: &Path) -> anyhow::Result<Command> {
    let npm = toolchain.npm.as_deref().context(
        "从 npm 获取或安装依赖需要 npm：请安装 Node.js（含 npm），或改用已安装依赖的本地目录导入",
    )?;
    let mut command = Command::new(npm);
    command
        .current_dir(directory)
        .env("PATH", child_path(toolchain))
        .env("npm_config_update_notifier", "false")
        .env("npm_config_fund", "false")
        .env("npm_config_audit", "false");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    Ok(command)
}

/// `npm pack` 只下载 tarball，不执行任何脚本；随后用 tar 解包。
fn fetch_npm(spec: &str, toolchain: &Toolchain, staging: &Path) -> anyhow::Result<PathBuf> {
    let download = staging.join("download");
    std::fs::create_dir_all(&download)?;
    let mut command = npm_command(toolchain, &download)?;
    command
        .arg("pack")
        .arg(spec)
        .arg("--json")
        .arg("--pack-destination")
        .arg(&download);
    let output = run_with_timeout(command, FETCH_TIMEOUT, &format!("下载 npm 包 {spec}"))?;
    let listing: Value = serde_json::from_slice(&output.stdout)
        .ok()
        .or_else(|| {
            // 部分 npm 版本会在 JSON 前打印提示，取第一个 `[` 之后的内容。
            let text = String::from_utf8_lossy(&output.stdout);
            text.find('[')
                .and_then(|start| serde_json::from_str(&text[start..]).ok())
        })
        .context("无法解析 npm pack 的输出")?;
    let filename = listing
        .get(0)
        .and_then(|entry| entry.get("filename"))
        .and_then(Value::as_str)
        .context("npm pack 没有给出 tarball 文件名")?;
    // npm 对带 scope 的包名会把 `@scope/` 改写成 `scope-`，文件名只取最后一段。
    let tarball = download.join(Path::new(filename).file_name().context("tarball 文件名不合法")?);
    if !tarball.is_file() {
        bail!("npm pack 没有生成 {}", tarball.display());
    }
    let extract = staging.join("extract");
    std::fs::create_dir_all(&extract)?;
    let mut tar = Command::new(if cfg!(windows) { "tar.exe" } else { "tar" });
    tar.arg("-xzf").arg(&tarball).arg("-C").arg(&extract);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        tar.creation_flags(crate::windows_create_no_window());
    }
    run_with_timeout(tar, FETCH_TIMEOUT, "解包 npm tarball")?;
    let package = extract.join("package");
    if !package.join("package.json").is_file() {
        bail!("npm 包里没有 package/package.json");
    }
    reject_links_outside(&package)?;
    Ok(package)
}

fn fetch_git(
    url: &str,
    reference: Option<&str>,
    subdir: Option<&str>,
    toolchain: &Toolchain,
    staging: &Path,
) -> anyhow::Result<PathBuf> {
    let git = toolchain
        .git
        .as_deref()
        .context("从 git 导入需要 git，请先安装 git")?;
    let checkout = staging.join("repo");
    let mut command = Command::new(git);
    command
        .env("PATH", child_path(toolchain))
        .env("GIT_TERMINAL_PROMPT", "0")
        .arg("clone")
        .arg("--depth")
        .arg("1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    if let Some(reference) = reference {
        command.arg("--branch").arg(reference);
    }
    command.arg("--").arg(url).arg(&checkout);
    run_with_timeout(command, FETCH_TIMEOUT, &format!("克隆 {url}"))?;
    let package = match subdir {
        Some(subdir) => checkout.join(subdir),
        None => checkout.clone(),
    };
    if !package.join("package.json").is_file() {
        bail!("仓库{}里没有 package.json", subdir.map(|sub| format!("的 {sub} 目录")).unwrap_or_default());
    }
    let package = package.canonicalize()?;
    if !package.starts_with(checkout.canonicalize()?) {
        bail!("git 子目录指向仓库以外");
    }
    reject_links_outside(&package)?;
    // 不带 .git 进包目录，后续按普通文件管理。
    let cleaned = staging.join("package");
    copy_dir_filtered(&package, &cleaned)?;
    Ok(cleaned)
}

/// 包里的符号链接不能指向包外：宿主会从包目录加载代码与 SKILL.md。
fn reject_links_outside(root: &Path) -> anyhow::Result<()> {
    let root = root.canonicalize()?;
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                let target = path.canonicalize().ok();
                if !target.is_some_and(|target| target.starts_with(&root)) {
                    bail!("包里的符号链接 {} 指向包外，已拒绝", path.display());
                }
            } else if metadata.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(())
}

fn copy_dir_filtered(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let source = entry.path();
        let target = to.join(&name);
        let metadata = std::fs::symlink_metadata(&source)?;
        if metadata.is_dir() {
            copy_dir_filtered(&source, &target)?;
        } else if metadata.is_file() {
            std::fs::copy(&source, &target)?;
        }
        // 符号链接已在 reject_links_outside 里确认在包内，复制时跳过，避免带出悬空链接。
    }
    Ok(())
}

/// 安装包的 `dependencies`：peer（dsh 宿主包）不装，交给兼容宿主；不执行安装脚本。
fn install_dependencies(directory: &Path, toolchain: &Toolchain) -> anyhow::Result<()> {
    let mut command = npm_command(toolchain, directory)?;
    command.args([
        "install",
        "--omit=dev",
        "--omit=peer",
        "--legacy-peer-deps",
        "--ignore-scripts",
        "--no-audit",
        "--no-fund",
        "--no-package-lock",
    ]);
    run_with_timeout(command, INSTALL_TIMEOUT, "安装插件依赖").map(|_| ())
}

/// 调用宿主分析包：执行插件的 apply 收集工具与 skill，输出 JSON 报告。
/// 代码注册的 skill 写到 `skills_out` 下。
fn inspect_package(
    node: &Path,
    host: &Path,
    root: &Path,
    record: &PluginRecord,
    staging: &Path,
    skills_out: &Path,
) -> anyhow::Result<Value> {
    let options = staging.join(format!("inspect-{}.json", uuid::Uuid::new_v4().simple()));
    std::fs::write(
        &options,
        serde_json::to_vec(&json!({
            "disabledRows": record.disabled_rows,
            "configs": record.configs,
            "skillsOut": skills_out,
        }))?,
    )?;
    let mut command = Command::new(node);
    command
        .current_dir(root)
        .arg(host)
        .arg("inspect")
        .arg(root)
        .arg(&options);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    let result = run_with_timeout(command, INSPECT_TIMEOUT, "分析插件");
    let _ = std::fs::remove_file(&options);
    let output = result?;
    if output.stdout.len() > MAX_REPORT_BYTES {
        bail!("插件分析报告过大");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // 报告是最后一行；插件若绕过 console 直接写 stdout，前面可能有杂讯。
    let line = text
        .lines()
        .rev()
        .find(|line| line.trim_start().starts_with('{'))
        .context("插件分析没有输出报告")?;
    serde_json::from_str(line).context("插件分析报告不是有效 JSON")
}

fn move_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            // 跨卷时 rename 会失败，退回复制（源目录随暂存目录一起清掉）。
            copy_dir_filtered(from, to)?;
            Ok(())
        }
    }
}

/// 清理 Codex++ 自己创建的暂存目录。
fn remove_dir_quietly(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)
    } else {
        Ok(())
    }
}

/// 报告里给 Codex 的 MCP 行 → 标准 `mcpServers` JSON（可交给「导入 MCP」）。
pub fn mcp_servers_json(report: &Value) -> Option<Value> {
    let mut servers = serde_json::Map::new();
    for row in report.get("rows").and_then(Value::as_array)? {
        let Some(mcp) = row.get("mcp") else { continue };
        let (Some(name), Some(server)) = (
            mcp.get("serverName").and_then(Value::as_str),
            mcp.get("server"),
        ) else {
            continue;
        };
        servers.insert(name.to_string(), server.clone());
    }
    (!servers.is_empty()).then(|| json!({ "mcpServers": servers }))
}

/// 报告里的 skill 目录。
pub fn report_skill_dirs(report: &Value) -> Vec<PathBuf> {
    report
        .get("skills")
        .and_then(Value::as_array)
        .map(|skills| {
            skills
                .iter()
                .filter_map(|skill| skill.get("directory").and_then(Value::as_str))
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// 报告里注册出的工具数。
pub fn report_tool_count(report: &Value) -> usize {
    report
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get("tools").and_then(Value::as_array))
                .map(Vec::len)
                .sum()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 配套拓展包：系统提示词 → 网关 instructions，dsh 事件 → Codex 钩子
// ---------------------------------------------------------------------------

/// 配套拓展包里的标记文件：写着注册表位置与插件 id，钩子 / 网关脚本据此加载插件。
const COMPANION_MARKER: &str = "dsh-bridge.json";
const COMPANION_HOOK_SCRIPT: &str = "hooks/dsh.mjs";
const COMPANION_GATEWAY_SCRIPT: &str = "gateway/instructions.mjs";
const COMPANION_PROMPT_FILE: &str = "prompt.txt";
/// 钩子要加载插件再派发，给足时间；Codex 那边的分发条目固定 120 秒。
const COMPANION_HOOK_TIMEOUT_SECS: u64 = 30;
/// 静态提示词只是读文件；动态的要加载插件，取网关中间件允许的上限。
const COMPANION_STATIC_GATEWAY_TIMEOUT_SECS: u64 = 3;
const COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS: u64 = 10;

/// 插件对应的配套拓展包 id。
pub fn companion_id(plugin_id: &str) -> String {
    let mut id = format!("dsh-{plugin_id}");
    id.truncate(64);
    id
}

/// 报告里要转发的 Codex 钩子事件、提示词是否追加到 instructions、能否用静态文本。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgePlan {
    pub hooks: Vec<String>,
    pub instructions: bool,
    pub static_prompt: Option<String>,
}

impl BridgePlan {
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty() && !self.instructions
    }
}

pub fn report_bridge(report: &Value) -> BridgePlan {
    let bridge = report.get("bridge");
    let hooks = bridge
        .and_then(|bridge| bridge.get("hooks"))
        .and_then(Value::as_array)
        .map(|hooks| {
            hooks
                .iter()
                .filter_map(Value::as_str)
                .filter(|event| crate::user_scripts::EXTENSION_HOOK_EVENTS.contains(event))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    BridgePlan {
        hooks,
        instructions: bridge
            .and_then(|bridge| bridge.get("instructions"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        static_prompt: bridge
            .and_then(|bridge| bridge.get("staticPrompt"))
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .map(str::to_string),
    }
}

/// JS 字符串字面量（JSON 字符串即合法的 JS 字符串）。
fn js_string(value: &str) -> String {
    Value::String(value.to_string()).to_string()
}

/// 由插件报告生成配套拓展包的内容；没有要映射的东西时返回 `None`。
pub fn companion_write(record: &PluginRecord, registry_path: &Path, bridge_path: &Path) -> Option<ExtensionWrite> {
    let plan = report_bridge(&record.report);
    if plan.is_empty() {
        return None;
    }
    let bridge_url = reqwest::Url::from_file_path(bridge_path).ok()?.to_string();
    let display = if record.name.is_empty() { &record.id } else { &record.name };
    let header = format!(
        "// 由 Codex++ 根据 dsh 插件「{display}」生成，重新分析插件时会覆盖，不要手改。\n"
    );
    let mut files = BTreeMap::new();
    files.insert(
        COMPANION_MARKER.to_string(),
        serde_json::to_string_pretty(&json!({
            "registry": registry_path.to_string_lossy(),
            "plugin": record.id,
            "bridge": bridge_path.to_string_lossy(),
        }))
        .ok()?,
    );
    let config_line = "const CONFIG = new URL('../dsh-bridge.json', import.meta.url)\n";
    let mut hooks = serde_json::Map::new();
    if !plan.hooks.is_empty() {
        files.insert(
            COMPANION_HOOK_SCRIPT.to_string(),
            format!(
                "{header}// 把插件的事件监听器转发成 Codex 钩子：{}。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function hook(payload, {{ event }}) {{\n  const {{ runHook }} = await import({})\n  return runHook(fileURLToPath(CONFIG), event, payload)\n}}\n",
                plan.hooks.join("、"),
                js_string(&bridge_url),
            ),
        );
        for event in &plan.hooks {
            hooks.insert(
                event.clone(),
                json!([{ "script": COMPANION_HOOK_SCRIPT, "timeout": COMPANION_HOOK_TIMEOUT_SECS }]),
            );
        }
    }
    let gateway = if plan.instructions {
        let (script, timeout) = match &plan.static_prompt {
            Some(text) => {
                files.insert(COMPANION_PROMPT_FILE.to_string(), text.clone());
                (
                    format!(
                        "{header}// 把插件的系统提示词片段（静态文本，见 prompt.txt）追加到每次模型请求的 instructions。\nimport {{ readFileSync }} from 'node:fs'\n\nconst TEXT = readFileSync(new URL('../prompt.txt', import.meta.url), 'utf8').trim()\n\nexport default function request(body) {{\n  if (!TEXT || !body || typeof body !== 'object') return null\n  const current = typeof body.instructions === 'string' ? body.instructions : ''\n  if (current.includes(TEXT)) return null\n  return {{ ...body, instructions: current ? `${{current}}\\n\\n${{TEXT}}` : TEXT }}\n}}\n"
                    ),
                    COMPANION_STATIC_GATEWAY_TIMEOUT_SECS,
                )
            }
            None => (
                format!(
                    "{header}// 插件的系统提示词有动态片段：每次模型请求加载插件求值，追加到 instructions。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function request(body) {{\n  const {{ requestInstructions }} = await import({})\n  return requestInstructions(fileURLToPath(CONFIG), body)\n}}\n",
                    js_string(&bridge_url),
                ),
                COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS,
            ),
        };
        files.insert(COMPANION_GATEWAY_SCRIPT.to_string(), script);
        Some(json!({ "request": [{ "script": COMPANION_GATEWAY_SCRIPT, "timeout": timeout }] }))
    } else {
        None
    };
    let mut targets = plan.hooks.clone();
    if plan.instructions {
        targets.push("instructions".to_string());
    }
    Some(ExtensionWrite {
        id: companion_id(&record.id),
        name: format!("dsh：{display}"),
        version: if record.version.is_empty() { "0.1.0".to_string() } else { record.version.clone() },
        description: format!(
            "由 Codex++ 为 dsh 插件「{display}」生成：{}。在「dsh 插件」里管理，随插件启停与移除。",
            targets.join("、")
        ),
        routes: Vec::new(),
        renderer: Vec::new(),
        hooks: (!hooks.is_empty()).then(|| Value::Object(hooks)),
        gateway,
        // 工具仍走 dsh 插件自己的 MCP（codex_plus_dsh），不重复声明成拓展工具。
        tools: None,
        config: None,
        files,
    })
}

impl DshPlugins {
    /// 让插件的配套拓展包与最新报告、启停状态一致：需要就写入（覆盖），不需要就移入备份。
    /// 返回配套拓展包的现状；调用方随后应同步 Codex 钩子配置（`extension_hooks::sync_default`）。
    pub fn sync_companion(&self, manager: &UserScriptManager, id: &str) -> anyhow::Result<Value> {
        let registry = self.load()?;
        let record = registry
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .with_context(|| format!("没有 id 为 {id} 的 dsh 插件"))?;
        let host = self.ensure_host()?;
        let bridge = host.with_file_name("bridge.mjs");
        let Some(write) = companion_write(record, &self.registry_path(), &bridge) else {
            self.remove_companion(manager, id)?;
            return Ok(Value::Null);
        };
        let written = manager.write_extension_package(&write)?;
        let key = crate::user_scripts::extension_package_key(&write.id);
        manager.set_script_enabled(&key, record.enabled)?;
        let plan = report_bridge(&record.report);
        Ok(json!({
            "id": write.id,
            "key": key,
            "directory": written.get("directory"),
            "manifestError": written.get("manifestError"),
            "hooks": plan.hooks,
            "instructions": plan.instructions,
            "staticPrompt": plan.static_prompt.is_some(),
            "enabled": record.enabled,
        }))
    }

    /// 移除插件的配套拓展包（移入 dsh 插件备份目录，可恢复）。只动带标记文件、指向本插件的目录。
    pub fn remove_companion(&self, manager: &UserScriptManager, id: &str) -> anyhow::Result<bool> {
        let directory = manager.user_dir().join(companion_id(id));
        let Ok(metadata) = std::fs::symlink_metadata(&directory) else {
            return Ok(false);
        };
        if !metadata.is_dir() {
            return Ok(false);
        }
        let owned = std::fs::read_to_string(directory.join(COMPANION_MARKER))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .is_some_and(|marker| marker.get("plugin").and_then(Value::as_str) == Some(id));
        if !owned {
            return Ok(false);
        }
        self.backup(&format!("companion-{}", companion_id(id)), &directory)?;
        Ok(true)
    }

    /// 配套拓展包的现状（给管理器显示）：期望的映射，以及包是否在、是否启用。
    pub fn companion_status(&self, manager: &UserScriptManager, record: &PluginRecord) -> Value {
        let plan = report_bridge(&record.report);
        if plan.is_empty() {
            return Value::Null;
        }
        let id = companion_id(&record.id);
        let key = crate::user_scripts::extension_package_key(&id);
        let config = manager.load_config();
        json!({
            "id": id,
            "hooks": plan.hooks,
            "instructions": plan.instructions,
            "staticPrompt": plan.static_prompt.is_some(),
            "installed": manager.user_dir().join(&id).join(COMPANION_MARKER).is_file(),
            "enabled": config.enabled && config.scripts.get(&key).copied().unwrap_or(true),
            "scriptsEnabled": config.enabled,
        })
    }
}

// ---------------------------------------------------------------------------
// MCP 接入与启动
// ---------------------------------------------------------------------------

pub fn status(home: &Path, executable: &Path, state_dir: &Path) -> anyhow::Result<RegistrationStatus> {
    managed_mcp::status(&SPEC, home, executable, state_dir)
}

/// `--codex-plus-dsh-mcp --state-dir <dir>`：写出宿主、定位 Node，以继承的 stdio 运行 MCP 服务。
pub fn run_mcp(args: &[OsString]) -> anyhow::Result<i32> {
    let state_dir = match args {
        [flag, directory] if flag == "--state-dir" => PathBuf::from(directory),
        [] => crate::paths::default_app_state_dir(),
        _ => bail!("用法：{MCP_FLAG} --state-dir <目录>"),
    };
    if !state_dir.is_absolute() {
        bail!("状态目录必须是绝对路径");
    }
    let plugins = DshPlugins::new(&state_dir);
    let host = plugins.ensure_host()?;
    let node = locate_node().context("没有找到 Node 运行时，dsh 插件 MCP 无法启动")?;
    let mut command = Command::new(node);
    command
        .arg(host)
        .arg("serve")
        .arg(plugins.registry_path())
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // stdio 已接到 Codex 的管道上，不需要控制台窗口。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    let status = command.status().context("无法启动 dsh 插件宿主")?;
    Ok(status.code().unwrap_or(1))
}
