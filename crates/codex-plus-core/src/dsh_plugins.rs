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
    (
        "services.mjs",
        include_str!("../../../assets/dsh-host/services.mjs"),
    ),
    (
        "host-call.mjs",
        include_str!("../../../assets/dsh-host/host-call.mjs"),
    ),
    (
        "shims/dsh-session.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-session.mjs"),
    ),
    (
        "shims/dsh-fs.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-fs.mjs"),
    ),
    (
        "shims/dsh-credentials.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-credentials.mjs"),
    ),
    (
        "shims/dsh-storage-domain.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-storage-domain.mjs"),
    ),
    (
        "shims/dsh-errors.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-errors.mjs"),
    ),
    (
        "shims/dsh-services.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-services.mjs"),
    ),
    (
        "shims/dsh-token-meter-estimate.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-token-meter-estimate.mjs"),
    ),
    (
        "shims/dsh-client-connection.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-client-connection.mjs"),
    ),
    (
        "shims/dsh-settings.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-settings.mjs"),
    ),
    (
        "shims/dsh-skill-filesystem.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-skill-filesystem.mjs"),
    ),
    (
        "settings-services.mjs",
        include_str!("../../../assets/dsh-host/settings-services.mjs"),
    ),
    (
        "shims/dsh-session-surface.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-session-surface.mjs"),
    ),
    (
        "shims/dsh-agent.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-agent.mjs"),
    ),
    (
        "shims/dsh-subagent.mjs",
        include_str!("../../../assets/dsh-host/shims/dsh-subagent.mjs"),
    ),
];

const REGISTRY_VERSION: u32 = 1;
/// 分析一个包会执行插件的 apply，给足时间但不能无限等。
const INSPECT_TIMEOUT: Duration = Duration::from_secs(45);
const FETCH_TIMEOUT: Duration = Duration::from_secs(180);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_SOURCE_LEN: usize = 512;
const MAX_REPORT_BYTES: usize = 4 * 1024 * 1024;
/// dsh 自己的纯工具包（dsh 仓库 `packages/util` 下与 typert 协议包）：发布在 npm 上、只把 cordis 列为 peer，
/// 不碰 dsh 的 agent 循环与会话。插件把它们写成 peer（指望 dsh 宿主带着）时，导入时照样从 npm 装真包。
pub const DSH_UTILITY_PACKAGES: &[&str] = &[
    "@deepseek-ai/dsh-atomic-write",
    "@deepseek-ai/dsh-brand",
    "@deepseek-ai/dsh-chunked-list",
    "@deepseek-ai/dsh-deque",
    "@deepseek-ai/dsh-home-paths",
    "@deepseek-ai/dsh-http-proxy",
    "@deepseek-ai/dsh-launch-environment",
    "@deepseek-ai/dsh-lazy-require",
    "@deepseek-ai/dsh-native-command",
    "@deepseek-ai/dsh-output-retention",
    "@deepseek-ai/dsh-package-manifest",
    "@deepseek-ai/dsh-plugin-detail",
    "@deepseek-ai/dsh-plugin-market-index",
    "@deepseek-ai/dsh-scope",
    "@deepseek-ai/dsh-timeout",
    "@deepseek-ai/dsh-typert-protocol",
    "@deepseek-ai/dsh-util-code-language",
    "@deepseek-ai/dsh-util-crypto",
    "@deepseek-ai/dsh-util-file-drag",
    "@deepseek-ai/dsh-util-time",
    "@deepseek-ai/dsh-util-values",
    "@deepseek-ai/dsh-util-workspace-path",
    // 只建在替身与上面工具包之上的纯逻辑库：装真包，它 import 的宿主包照常落到替身。
    "@deepseek-ai/dsh-session-title-llm",
];

/// package.json 里记下装依赖时跳过的依赖（名字 → 原因），重新分析时也能在报告里说明。
const SKIPPED_DEPENDENCIES_KEY: &str = "codexPlusSkippedDependencies";

/// dsh 前端插件入口的上限（与 host.mjs 的 CLIENT_MAX_BYTES 一致，测试守住两边）。打包后的前端插件常在 0.3~3 MiB：
/// 它整个夹进配套拓展包的页面脚本，随 Codex 页面注入一次。
pub const DSH_CLIENT_MAX_BYTES: usize = 4 * 1024 * 1024;

/// 宿主自己提供的 peer 包（`host.mjs` 的 SHIMS 去掉 schemastery，测试守住两边一致）；只依赖它们时不需要跑 npm install。
/// schemastery 按 dsh 约定是普通运行时依赖，声明了就装真包，替身只兜底没装的情况。
pub const HOST_PROVIDED_PACKAGES: &[&str] = &[
    "@deepseek-ai/cordis",
    "@deepseek-ai/dsh-tools",
    "@deepseek-ai/dsh-skill",
    "@deepseek-ai/dsh-system-prompt",
    "@deepseek-ai/dsh-llm",
    "@deepseek-ai/dsh-session",
    "@deepseek-ai/dsh-fs",
    "@deepseek-ai/dsh-credentials",
    "@deepseek-ai/dsh-storage-domain",
    "@deepseek-ai/dsh-web",
    "@deepseek-ai/dsh-user-questions",
    "@deepseek-ai/dsh-session-query",
    "@deepseek-ai/dsh-subprocess",
    "@deepseek-ai/dsh-shell",
    "@deepseek-ai/dsh-commands",
    "@deepseek-ai/dsh-spill",
    "@deepseek-ai/dsh-user-approval",
    "@deepseek-ai/dsh-session-title",
    "@deepseek-ai/dsh-token-meter",
    "@deepseek-ai/dsh-shell-env",
    "@deepseek-ai/dsh-storage",
    "@deepseek-ai/dsh-session-telemetry",
    "@deepseek-ai/dsh-plan-mode",
    "@deepseek-ai/dsh-goal",
    "@deepseek-ai/dsh-subagent",
    "@deepseek-ai/dsh-attachment",
    "@deepseek-ai/dsh-file-reference",
    "@deepseek-ai/dsh-host-webserver",
    "@deepseek-ai/dsh-mcp-resources",
    "@deepseek-ai/dsh-client-connection",
    "@deepseek-ai/dsh-settings",
    "@deepseek-ai/dsh-skill-filesystem",
    "@deepseek-ai/dsh-agent",
];

/// 插件来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PluginSource {
    /// 本地目录，原地链接（同 dsh 的 `link:`），不复制。
    Local { path: String },
    /// npm 包 spec，如 `@scope/name@1.2.3`；`registry` 是自定义 npm registry（同 dsh 市场索引的
    /// `source.registry`），下载包与安装依赖都用它。
    Npm {
        spec: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registry: Option<String>,
    },
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
    /// 由拓展包（codexpp.json 的 `dsh`）带来的插件：拓展的脚本 key。随拓展启停、删除。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// 拓展包的版本与插件目录的修改时间：变了就重新分析。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_fingerprint: Option<String>,
}

fn default_true() -> bool {
    true
}

/// dsh 插件市场索引（格式 1）里的一项：取最新版本的来源，翻成「导入」能直接用的来源字符串。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub version: String,
    /// 交给 [`parse_source`] 的字符串：npm 是 `name@version`（自定义 registry 时加 `#registry:<地址>`），
    /// git 是 `url#commit&path:子目录`。
    pub source: String,
    /// 来源里 Codex++ 不支持、被忽略的部分；目前都支持，留作兼容（总是 `None`）。
    pub note: Option<String>,
}

/// 解析插件市场索引（格式 1：`{ format: 1, plugins: [{ name, versions: [{ version, source }] }] }`）。
pub fn parse_market_index(index: &Value) -> anyhow::Result<Vec<MarketEntry>> {
    if index.get("format").and_then(Value::as_u64) != Some(1) {
        bail!("不是格式 1 的 dsh 插件市场索引");
    }
    let plugins = index.get("plugins").and_then(Value::as_array).context("索引里没有 plugins")?;
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut entries = Vec::new();
    for plugin in plugins.iter().take(500) {
        let name = text(plugin, "name");
        let Some(latest) = plugin.get("versions").and_then(Value::as_array).and_then(|versions| versions.last()) else { continue };
        let version = text(latest, "version");
        let source = &latest["source"];
        let (spec, note) = match source.get("type").and_then(Value::as_str) {
            Some("npm") => {
                let package = source.get("name").and_then(Value::as_str).unwrap_or(&name);
                let spec = if version.is_empty() { package.to_string() } else { format!("{package}@{version}") };
                // 自定义 registry 随来源字符串带上，导入时 npm pack / install 都用它。
                let spec = match source.get("registry").and_then(Value::as_str).map(str::trim).filter(|registry| !registry.is_empty()) {
                    Some(registry) => format!("{spec}#registry:{registry}"),
                    None => spec,
                };
                (spec, None)
            }
            Some("git") => {
                let url = text(source, "url");
                if url.is_empty() {
                    continue;
                }
                let mut fragment = Vec::new();
                if let Some(commit) = source.get("commit").and_then(Value::as_str) {
                    fragment.push(commit.to_string());
                }
                if let Some(path) = source.get("path").and_then(Value::as_str) {
                    fragment.push(format!("path:{path}"));
                }
                (if fragment.is_empty() { url } else { format!("{url}#{}", fragment.join("&")) }, None)
            }
            _ => continue,
        };
        if name.is_empty() || parse_source(&spec).is_err() {
            continue;
        }
        let meta = plugin.get("metadata").unwrap_or(&Value::Null);
        entries.push(MarketEntry {
            title: [text(plugin, "title"), text(meta, "title")].into_iter().find(|title| !title.is_empty()).unwrap_or_else(|| name.clone()),
            description: [text(plugin, "description"), text(meta, "description")].into_iter().find(|text| !text.is_empty()).unwrap_or_default(),
            name,
            version,
            source: spec,
            note,
        });
    }
    Ok(entries)
}

/// 下载并解析插件市场索引；只认 http(s)，最大 4 MiB。
pub async fn fetch_market_index(url: &str) -> anyhow::Result<Vec<MarketEntry>> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("索引地址必须是 http(s)");
    }
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("CodexPlusPlus/dsh-market")
        .build()?
        .get(url)
        .send()
        .await
        .context("下载索引失败")?;
    if !response.status().is_success() {
        bail!("下载索引失败：HTTP {}", response.status());
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_REPORT_BYTES {
        bail!("索引超过 4 MiB");
    }
    parse_market_index(&serde_json::from_slice(&bytes).context("索引不是有效 JSON")?)
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
    // Windows 上 `/x`、`\x` 不算绝对路径，但 npm 包名不会以它们开头：也当本地目录。
    if Path::new(text).is_absolute() || text.starts_with('/') || text.starts_with('\\') {
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
    // npm 包可在 `#registry:<地址>` 里指定自定义 registry（同 dsh 市场索引的 source.registry）。
    let (spec, registry) = match text.split_once("#registry:") {
        Some((spec, registry)) => (spec.trim(), Some(validate_registry(registry)?)),
        None => (text, None),
    };
    let allowed = |c: char| c.is_ascii_alphanumeric() || "@/._~^<>=*|+-".contains(c);
    if spec.is_empty() || spec.starts_with('-') || spec.starts_with('.') || !spec.chars().all(allowed) {
        bail!("无法识别的来源：npm 包名只能包含字母、数字与 @/._-，本地目录请用绝对路径");
    }
    Ok(PluginSource::Npm {
        spec: spec.to_string(),
        registry,
    })
}

/// 自定义 npm registry 只认 http(s) 地址（作为 `--registry` 的值交给 npm，不能是别的选项）。
fn validate_registry(raw: &str) -> anyhow::Result<String> {
    let text = raw.trim();
    let url = reqwest::Url::parse(text).map_err(|_| anyhow::anyhow!("npm registry 必须是 http(s) 地址"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || text.chars().any(char::is_whitespace) {
        bail!("npm registry 必须是 http(s) 地址");
    }
    Ok(text.to_string())
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
    let mut candidates: Vec<PathBuf> = Vec::new();
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

/// 去掉 Windows canonicalize 带上的 `\\?\` verbatim 前缀（`\\?\UNC\` 换回 `\\`）。
/// 交给 Node 的插件根目录使用普通路径，避免与依赖扫描得到的路径混用两种表示。
fn strip_verbatim_prefix(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        // 只处理盘符路径；`\\?\Volume{...}` 之类去掉前缀就不是合法路径了。
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
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

    /// 有没有「接上之后真的能提供工具」的插件：登记过、开着、包目录还在。
    ///
    /// 没有的话这条 MCP 就只剩空转：Codex 每开一个会话/重连一次就 spawn 一个宿主
    /// （一个 Node 进程 68 MB），宿主却没有任何工具可以响应。实测一天能攒到 45 个常驻
    /// 孤儿、2.2 GB。调用方据此决定要不要真的把宿主拉起来。
    pub fn has_enabled_plugins(&self) -> anyhow::Result<bool> {
        Ok(self
            .load()?
            .plugins
            .iter()
            .any(|record| record.enabled && Path::new(&record.root).exists()))
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
        let result = self.import_in(source, toolchain, node, &host, &staging, None);
        let _ = remove_dir_quietly(&staging);
        result
    }

    /// 以指定 id 导入本地目录（拓展包带来的插件用 `ext-<拓展 id>`，不和用户自己导入的同名包冲突）。
    fn import_local_as(&self, root: &Path, id: &str, toolchain: &Toolchain) -> anyhow::Result<PluginRecord> {
        let host = self.ensure_host()?;
        let node = toolchain.require_node()?;
        let staging = self.staging_dir().join(uuid::Uuid::new_v4().simple().to_string());
        std::fs::create_dir_all(&staging).with_context(|| format!("无法创建临时目录 {}", staging.display()))?;
        let source = PluginSource::Local { path: root.to_string_lossy().into_owned() };
        let result = self.import_in(&source, toolchain, node, &host, &staging, Some(id));
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
        id_override: Option<&str>,
    ) -> anyhow::Result<PluginRecord> {
        let (fetched, managed) = match source {
            PluginSource::Local { path } => {
                let directory = Path::new(path)
                    .canonicalize()
                    .map(|directory| strip_verbatim_prefix(&directory))
                    .with_context(|| format!("目录不存在：{path}"))?;
                if !directory.join("package.json").is_file() {
                    bail!("{} 下没有 package.json，不是 dsh 插件包", directory.display());
                }
                (directory, false)
            }
            PluginSource::Npm { spec, registry } => (fetch_npm(spec, registry.as_deref(), toolchain, staging)?, true),
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
        let id = id_override.map(str::to_string).unwrap_or_else(|| plugin_id_from_name(&name));
        validate_plugin_id(&id)?;
        if managed && needs_dependency_install(&package, &fetched) {
            let registry = match source {
                PluginSource::Npm { registry, .. } => registry.as_deref(),
                _ => None,
            };
            install_dependencies(&fetched, registry, toolchain)?;
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
            owner: None,
            owner_fingerprint: None,
        };
        if let Some(index) = previous {
            let old = &registry.plugins[index];
            record.owner = old.owner.clone();
            record.owner_fingerprint = old.owner_fingerprint.clone();
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

fn needs_dependency_install(package: &Value, directory: &Path) -> bool {
    let wants = |key: &str, keep: &dyn Fn(&str) -> bool| {
        package.get(key).and_then(Value::as_object).is_some_and(|dependencies| dependencies.keys().any(|name| keep(name)))
    };
    wants("dependencies", &|name| !HOST_PROVIDED_PACKAGES.contains(&name))
        || wants("peerDependencies", &|name| DSH_UTILITY_PACKAGES.contains(&name))
        || !imported_dsh_utilities(directory).is_empty()
}

/// 装依赖前整理导入副本的 package.json：
/// - 写成 peer 的 dsh 纯工具包挪进 dependencies 装真包（版本范围写的是 workspace: 之类时装 latest）；
/// - 发布不了的依赖写法（`workspace:`、`link:`、`catalog:`、指向不存在路径的 `file:`）挪进 peerDependencies，不装。
/// 返回跳过的依赖（名字 → 原因）。
fn prepare_dependencies(directory: &Path, package: &mut Value) -> BTreeMap<String, String> {
    let mut skipped = BTreeMap::new();
    let Some(object) = package.as_object_mut() else { return skipped };
    let mut dependencies = object.get("dependencies").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut peers = object.get("peerDependencies").and_then(Value::as_object).cloned().unwrap_or_default();
    // 插件常直接 import dsh 纯工具包而不写进 package.json（dsh 宿主本来就带着）：扫一遍包里的脚本补上。
    for name in imported_dsh_utilities(directory) {
        if !dependencies.contains_key(name) && !peers.contains_key(name) {
            dependencies.insert(name.to_string(), Value::String("latest".into()));
        }
    }
    for name in DSH_UTILITY_PACKAGES {
        if let Some(range) = peers.remove(*name) {
            let range = range.as_str().filter(|range| !unpublishable_range(directory, range)).unwrap_or("latest").to_string();
            dependencies.entry(name.to_string()).or_insert(Value::String(range));
        }
    }
    for (name, range) in dependencies.clone() {
        let Some(text) = range.as_str() else { continue };
        if unpublishable_range(directory, text) {
            dependencies.remove(&name);
            if DSH_UTILITY_PACKAGES.contains(&name.as_str()) {
                dependencies.insert(name.clone(), Value::String("latest".into()));
                continue;
            }
            peers.insert(name.clone(), range.clone());
            skipped.insert(name, format!("版本写的是 {text}（只在作者自己的工作区里有效），没有安装"));
        }
    }
    // 导入副本用不到开发依赖，整个去掉：`npm install --omit=dev` 仍会先解析 dev 依赖树（dsh 插件常把宿主包写进 dev
    // 方便本地编译，其中有没发布的包，解析就报 404），同名的还会让 dependencies 里的那个被当成开发依赖跳过。
    object.remove("devDependencies");
    object.insert("dependencies".into(), Value::Object(dependencies));
    object.insert("peerDependencies".into(), Value::Object(peers));
    skipped
}

/// 包里的脚本（不进 node_modules，最多看 2000 个文件、每个 2 MiB 以内）引用到的 dsh 纯工具包。
fn imported_dsh_utilities(directory: &Path) -> Vec<&'static str> {
    let mut found = std::collections::BTreeSet::new();
    let mut stack = vec![directory.to_path_buf()];
    let mut seen = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                if entry.file_name() != "node_modules" && !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            let script = path.extension().and_then(|ext| ext.to_str()).is_some_and(|ext| matches!(ext, "js" | "mjs" | "cjs"));
            if !kind.is_file() || !script || entry.metadata().map(|meta| meta.len()).unwrap_or(u64::MAX) > 2 * 1024 * 1024 {
                continue;
            }
            seen += 1;
            if seen > 2000 {
                return found.into_iter().collect();
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for name in DSH_UTILITY_PACKAGES {
                if [format!("'{name}'"), format!("\"{name}\""), format!("'{name}/"), format!("\"{name}/")].iter().any(|quoted| text.contains(quoted.as_str())) {
                    found.insert(*name);
                }
            }
        }
    }
    found.into_iter().collect()
}

fn unpublishable_range(directory: &Path, range: &str) -> bool {
    let range = range.trim();
    ["workspace:", "link:", "catalog:", "portal:"].iter().any(|prefix| range.starts_with(prefix))
        || range.strip_prefix("file:").is_some_and(|path| !directory.join(path).exists())
}

/// npm 报「这个依赖装不上」（404 没发布、没有匹配的版本）时，是哪个包。
fn unresolvable_dependency(error: &str) -> Option<(String, &'static str)> {
    let decode = |name: &str| name.replace("%2f", "/").replace("%2F", "/");
    if error.contains("E404") {
        let start = error.find("404 Not Found - GET ")? + "404 Not Found - GET ".len();
        let url = error[start..].split_whitespace().next()?;
        let name = url.rsplit_once(".org/").map(|(_, name)| name).unwrap_or(url);
        return Some((decode(name.trim_end_matches(" - Not found")), "npm 上没有这个包（多为 dsh 宿主自带、没有单独发布的包）"));
    }
    if error.contains("ETARGET") {
        let start = error.find("No matching version found for ")? + "No matching version found for ".len();
        let spec = error[start..].split_whitespace().next()?.trim_end_matches('.');
        let name = if let Some(rest) = spec.strip_prefix('@') { format!("@{}", rest.split('@').next()?) } else { spec.split('@').next()?.to_string() };
        return Some((name, "npm 上没有匹配的版本"));
    }
    None
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

/// `npm pack` 只下载 tarball，不执行任何脚本；随后用 tar 解包。`registry` 是自定义 npm registry。
fn fetch_npm(spec: &str, registry: Option<&str>, toolchain: &Toolchain, staging: &Path) -> anyhow::Result<PathBuf> {
    let download = staging.join("download");
    std::fs::create_dir_all(&download)?;
    let mut command = npm_command(toolchain, &download)?;
    command
        .arg("pack")
        .arg(spec)
        .arg("--json")
        .arg("--pack-destination")
        .arg(&download);
    if let Some(registry) = registry {
        command.arg("--registry").arg(registry);
    }
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
    let package = canonical_dir(&package)?;
    if !package.starts_with(canonical_dir(&checkout)?) {
        bail!("git 子目录指向仓库以外");
    }
    reject_links_outside(&package)?;
    // 不带 .git 进包目录，后续按普通文件管理。
    let cleaned = staging.join("package");
    copy_dir_filtered(&package, &cleaned)?;
    Ok(cleaned)
}

/// 规范化路径并去掉 Windows 的 `\\?\` 前缀：这些路径要交给 Node 宿主，Node 的 `pathToFileURL` /
/// `import()` 会把 `\\?\D:\x` 变成无效的 `file:////?/D:/x`，插件代码就加载不了。
fn canonical_dir(path: &Path) -> std::io::Result<PathBuf> {
    Ok(strip_verbatim_prefix(&path.canonicalize()?))
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
/// 包来自自定义 registry 时依赖也从它装（同 dsh 安装组合包时整次安装用同一个 registry）。
/// 直接依赖里有装不上的（npm 上没有、版本写法只在作者工作区有效），挪成 peer 后重试：用到它的插件行在分析时
/// 逐行报出来，别的行照常可用，而不是整个包导入失败。跳过的依赖记进 package.json 的 [`SKIPPED_DEPENDENCIES_KEY`]。
fn install_dependencies(directory: &Path, registry: Option<&str>, toolchain: &Toolchain) -> anyhow::Result<()> {
    let mut package = read_package_json(directory)?;
    let mut skipped = prepare_dependencies(directory, &mut package);
    let mut attempts = 0;
    let mut utility_rounds = 0;
    loop {
        if let Some(object) = package.as_object_mut() {
            if skipped.is_empty() {
                object.remove(SKIPPED_DEPENDENCIES_KEY);
            } else {
                object.insert(SKIPPED_DEPENDENCIES_KEY.into(), json!(skipped));
            }
        }
        std::fs::write(directory.join("package.json"), serde_json::to_vec_pretty(&package)?)?;
        let error = match run_npm_install(directory, registry, toolchain) {
            Ok(()) => {
                // 装上的 dsh 包（bundle 行引用的官方模块、纯逻辑库）把工具包写成 peer、指望 dsh 宿主带着：补装后再装一轮。
                let missing = missing_utility_peers(directory);
                if missing.is_empty() || utility_rounds >= 2 {
                    return Ok(());
                }
                utility_rounds += 1;
                if let Some(dependencies) = package.as_object_mut().and_then(|object| {
                    object.entry("dependencies").or_insert_with(|| json!({})).as_object_mut()
                }) {
                    for (name, range) in missing {
                        dependencies.entry(name).or_insert(Value::String(range));
                    }
                }
                continue;
            }
            Err(error) => error,
        };
        attempts += 1;
        let text = format!("{error:#}");
        let direct = unresolvable_dependency(&text).filter(|(name, _)| {
            package.pointer(&format!("/dependencies/{}", name.replace('~', "~0").replace('/', "~1"))).is_some()
        });
        let Some((name, reason)) = direct.filter(|_| attempts <= 8) else {
            return Err(error);
        };
        if let Some(object) = package.as_object_mut() {
            let range = object.get_mut("dependencies").and_then(Value::as_object_mut).and_then(|dependencies| dependencies.remove(&name));
            if let (Some(range), Some(peers)) = (range, object.entry("peerDependencies").or_insert_with(|| json!({})).as_object_mut()) {
                peers.insert(name.clone(), range);
            }
        }
        skipped.insert(name, reason.to_string());
    }
}

/// 已装进 `node_modules` 的包（一层，含 `@scope/`）里，声明为 peer、却没装上的 dsh 工具包（名字 → 版本范围）。
/// 版本范围是只在作者工作区有效的写法时装 latest。
fn missing_utility_peers(directory: &Path) -> BTreeMap<String, String> {
    let modules = directory.join("node_modules");
    let mut packages = Vec::new();
    for entry in std::fs::read_dir(&modules).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('@') {
            packages.extend(std::fs::read_dir(&path).into_iter().flatten().flatten().map(|scoped| scoped.path()));
        } else {
            packages.push(path);
        }
    }
    let mut missing = BTreeMap::new();
    for package in packages {
        let Ok(manifest) = read_package_json(&package) else { continue };
        let Some(peers) = manifest.get("peerDependencies").and_then(Value::as_object) else { continue };
        for (name, range) in peers {
            if !DSH_UTILITY_PACKAGES.contains(&name.as_str()) || modules.join(name).join("package.json").is_file() {
                continue;
            }
            let range = range.as_str().filter(|range| !unpublishable_range(&package, range)).unwrap_or("latest");
            missing.entry(name.clone()).or_insert_with(|| range.to_string());
        }
    }
    missing
}

fn run_npm_install(directory: &Path, registry: Option<&str>, toolchain: &Toolchain) -> anyhow::Result<()> {
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
    if let Some(registry) = registry {
        command.arg("--registry").arg(registry);
    }
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
    let mut report: Value = serde_json::from_str(line).context("插件分析报告不是有效 JSON")?;
    // 装依赖时跳过的依赖：用到它们的行分析时已逐行报出，这里在包级说明原因。
    if let Some(skipped) = read_package_json(root).ok().and_then(|package| package.get(SKIPPED_DEPENDENCIES_KEY).and_then(Value::as_object).cloned()) {
        if let Some(diagnostics) = report.get_mut("diagnostics").and_then(Value::as_array_mut) {
            for (name, reason) in skipped {
                diagnostics.push(json!(format!("依赖 {name} 没有安装：{}；用到它的插件行不会加载", reason.as_str().unwrap_or(""))));
            }
        }
    }
    Ok(report)
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
const COMPANION_ERROR_SCRIPT: &str = "gateway/error.mjs";
const COMPANION_STEP_SCRIPT: &str = "gateway/step.mjs";
const COMPANION_RESPONSE_SCRIPT: &str = "gateway/response.mjs";
const COMPANION_RENDERER_SCRIPT: &str = "renderer.js";
const COMPANION_BACKEND_SCRIPT: &str = "backend/main.mjs";
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
    /// 每一步的模型请求都交给插件：agent/request（改模型 / 推理强度）、按步的 agent/pre-step、
    /// tools/post-execute 改写的工具结果、钩子之外排下的上下文。
    pub gateway_step: bool,
    /// 插件监听 llm/stream / agent/assistant-stream：挂网关响应流中间件。
    pub gateway_response: bool,
    /// 插件监听 agent/request-error：挂网关失败重试。
    pub gateway_error: bool,
    /// 插件注册的斜杠命令：在 Codex 页面里注册同名命令，经配套拓展包的后台转给插件。
    pub commands: Vec<BridgeCommand>,
    /// 插件宿主半边注册了 HTTP 路由（ctx.connection.fetch）：前端半边的请求经后台转过去。
    pub connection: bool,
    /// 插件用到 ctx.schedule：后台每分钟检查一次到点的定时任务。
    pub schedule: bool,
    /// 插件注册了 webhook 规则：后台开 /ext/<配套拓展>/webhook/<kind>。
    pub webhook: bool,
    /// 插件监听插件之间的自定义事件：别的包 emit 时经常驻进程转过来，要有配套拓展包（标记文件）才找得到它。
    pub events: bool,
    /// dsh 前端插件（dsh.client）的入口文件与包名：夹上胶水作为配套拓展包的页面脚本。
    pub client: Option<BridgeClient>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeClient {
    pub entry: PathBuf,
    pub package_name: String,
    /// 前端插件 apply 的第二个参数：包自己那一行的配置。
    pub config: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeCommand {
    pub name: String,
    pub description: String,
    pub hint: String,
}

impl BridgePlan {
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
            && !self.instructions
            && !self.gateway_step
            && !self.gateway_response
            && !self.gateway_error
            && self.commands.is_empty()
            && !self.connection
            && !self.schedule
            && !self.webhook
            && !self.events
            && self.client.is_none()
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
        // 旧报告里叫 request。
        gateway_step: bridge
            .and_then(|bridge| bridge.pointer("/gateway/step").or_else(|| bridge.pointer("/gateway/request")))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        gateway_response: bridge
            .and_then(|bridge| bridge.pointer("/gateway/response"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        gateway_error: bridge
            .and_then(|bridge| bridge.pointer("/gateway/error"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        commands: bridge
            .and_then(|bridge| bridge.get("commands"))
            .and_then(Value::as_array)
            .map(|commands| {
                let mut seen = std::collections::BTreeSet::new();
                commands
                    .iter()
                    .filter_map(|command| {
                        let name = command.get("name")?.as_str()?;
                        // 与页面斜杠命令的命名规则一致；重名只留第一个。
                        let valid = name.len() <= 32
                            && name.chars().next().is_some_and(|ch| ch.is_ascii_alphanumeric())
                            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
                        (valid && seen.insert(name.to_string())).then(|| BridgeCommand {
                            name: name.to_string(),
                            description: command.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                            hint: command.get("hint").and_then(Value::as_str).unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        connection: bridge
            .and_then(|bridge| bridge.get("connection"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        schedule: bridge.and_then(|bridge| bridge.get("schedule")).and_then(Value::as_bool).unwrap_or(false),
        webhook: bridge.and_then(|bridge| bridge.get("webhook")).and_then(Value::as_bool).unwrap_or(false),
        events: bridge.and_then(|bridge| bridge.get("events")).and_then(Value::as_bool).unwrap_or(false),
        client: bridge.and_then(|bridge| bridge.get("client")).and_then(|client| {
            Some(BridgeClient {
                entry: PathBuf::from(client.get("entry")?.as_str()?),
                package_name: client.get("packageName").and_then(Value::as_str).unwrap_or_default().to_string(),
                config: client.get("config").filter(|config| config.is_object()).cloned().unwrap_or_else(|| json!({})),
            })
        }),
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
    let mut gateway = serde_json::Map::new();
    let mut requests = Vec::new();
    if plan.instructions {
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
                    "{header}// 插件的系统提示词有动态片段（或 system-prompt/assemble）：每次模型请求求值，追加到 instructions。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function request(body) {{\n  const {{ requestInstructions }} = await import({})\n  return requestInstructions(fileURLToPath(CONFIG), body)\n}}\n",
                    js_string(&bridge_url),
                ),
                COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS,
            ),
        };
        files.insert(COMPANION_GATEWAY_SCRIPT.to_string(), script);
        requests.push(json!({ "script": COMPANION_GATEWAY_SCRIPT, "timeout": timeout }));
    }
    if plan.gateway_step {
        files.insert(
            COMPANION_STEP_SCRIPT.to_string(),
            format!(
                "{header}// 每一步的模型请求交给插件：agent/request 改模型与推理强度，工具调用之后的每一步跑 agent/pre-step（可拒绝这一步），\n// tools/post-execute 改写的工具结果换进 input，钩子之外 agent.inject 排下的上下文带上。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function request(body, context) {{\n  const {{ requestStep }} = await import({})\n  return requestStep(fileURLToPath(CONFIG), body, {{ step: context?.step }})\n}}\n",
                js_string(&bridge_url),
            ),
        );
        requests.push(json!({ "script": COMPANION_STEP_SCRIPT, "timeout": COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS }));
    }
    if !requests.is_empty() {
        gateway.insert("request".into(), Value::Array(requests));
    }
    if plan.gateway_response {
        files.insert(
            COMPANION_RESPONSE_SCRIPT.to_string(),
            format!(
                "{header}// 模型回复流交给插件：llm/stream 改写、agent/assistant-stream 观察。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function* response(context, next) {{\n  const {{ responseStream }} = await import({})\n  yield* responseStream(fileURLToPath(CONFIG), context, next())\n}}\n",
                js_string(&bridge_url),
            ),
        );
        gateway.insert("response".into(), json!([{ "script": COMPANION_RESPONSE_SCRIPT, "timeout": COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS }]));
    }
    if plan.gateway_error {
        files.insert(
            COMPANION_ERROR_SCRIPT.to_string(),
            format!(
                "{header}// 上游出错时问插件的 agent/request-error：返回 {{ kind: 'retry' }} 就重发。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nexport default async function error(failure) {{\n  const {{ requestError }} = await import({})\n  return requestError(fileURLToPath(CONFIG), failure)\n}}\n",
                js_string(&bridge_url),
            ),
        );
        gateway.insert("error".into(), json!([{ "script": COMPANION_ERROR_SCRIPT, "timeout": COMPANION_DYNAMIC_GATEWAY_TIMEOUT_SECS }]));
    }
    let gateway = (!gateway.is_empty()).then_some(Value::Object(gateway));
    // 页面脚本：斜杠命令在 Codex 输入框里同名注册，dsh 前端插件夹上胶水在页面里运行；
    // 两者的请求都经配套拓展包的后台转给插件常驻进程（后台 → bridge.mjs → 常驻进程）。
    let client_source = match &plan.client {
        Some(client) => match std::fs::read_to_string(&client.entry) {
            Ok(source) => Some((client, source)),
            Err(error) => {
                eprintln!("[dsh] 读取前端插件 {} 失败：{error}", client.entry.display());
                None
            }
        },
        None => None,
    };
    let mut renderer = Vec::new();
    if !plan.commands.is_empty() || client_source.is_some() {
        let commands = serde_json::to_string(&plan.commands).ok()?;
        let mut script = format!(
            "{header}// 插件注册的 dsh 斜杠命令，在 Codex 输入框里同名可用；结果用提示条显示。\nconst COMMANDS = {commands}\n\ncodexPlus.extension((ctx) => {{\n  for (const command of COMMANDS) {{\n    ctx.registerCommand({{\n      name: command.name,\n      description: command.hint ? `${{command.description}} ${{command.hint}}` : command.description,\n      async run(rawInput, {{ toast, sessionId }}) {{\n        try {{\n          const result = await ctx.backend.call('command', {{ name: command.name, rawInput, sessionId }}, {{ timeout: 120000 }})\n          if (result?.kind === 'error') toast(result.text || `/${{command.name}} 失败`, {{ type: 'error' }})\n          else toast(result?.text || `/${{command.name}} 已完成`, {{ type: 'success' }})\n        }} catch (error) {{\n          toast(`/${{command.name}} 失败：${{error?.message ?? error}}`, {{ type: 'error' }})\n        }}\n      }},\n    }})\n  }}\n"
        );
        if let Some((client, source)) = &client_source {
            script.push_str(&format!(
                "\n  // dsh 前端插件：截住它的 __ModuleLoader__.load，把发往 /api/ 的 fetch 经后台转给插件的宿主半边，\n  // 再交给 Codex++ 的 dsh 前端运行时（React 用 Codex 页面自己的，dsh 插槽映射到 Codex++ 的插槽）。\n  const modules = []\n  const loader = {{ load: (module) => modules.push(module) }}\n  const routed = async (input, init = {{}}) => {{\n    const url = typeof input === 'string' ? input : String(input?.url ?? input)\n    if (!url.startsWith('/api/')) return globalThis.fetch(input, init)\n    const reply = await ctx.backend.call('fetch', {{ path: url, method: init.method || 'GET', headers: init.headers ?? {{}}, body: init.body ?? null }}, {{ timeout: 120000 }})\n    return new Response(reply?.body ?? '', {{ status: reply?.status ?? 502, headers: reply?.headers ?? {{}} }})\n  }}\n  const window = new Proxy(globalThis.window, {{\n    get: (target, key) => key === '__ModuleLoader__' ? loader : key === 'fetch' ? routed : Reflect.get(target, key),\n  }})\n  const fetch = routed\n  ;(function () {{\n// ---- {name} 的前端入口 ----\n{source}\n// ---- 前端入口结束 ----\n  }}).call(window)\n  void codexPlus.dshClient(ctx, modules, {{ packageName: {package}, config: {config} }}).catch((error) => ctx.fail(error))\n",
                name = client.package_name,
                package = js_string(&client.package_name),
                config = client.config,
            ));
        }
        script.push_str("})\n");
        files.insert(COMPANION_RENDERER_SCRIPT.to_string(), script);
        renderer.push(COMPANION_RENDERER_SCRIPT.to_string());
    }
    let mut extra = serde_json::Map::new();
    // 只监听插件间事件的包也挂后台脚本：拓展清单至少要声明一种入口；后台按需才启动，不常驻。
    let backend = if !plan.commands.is_empty() || plan.connection || plan.schedule || plan.webhook || plan.events {
        files.insert(
            COMPANION_BACKEND_SCRIPT.to_string(),
            format!(
                "{header}// 后台：页面斜杠命令、前端插件的请求、定时任务（每分钟）、webhook，都转给插件。\nimport {{ fileURLToPath }} from 'node:url'\n{config_line}\nconst bridge = () => import({})\n\nexport default () => ({{\n  async command({{ name, rawInput, sessionId }} = {{}}) {{\n    return (await bridge()).runCommand(fileURLToPath(CONFIG), name, rawInput, sessionId)\n  }},\n  async fetch(request = {{}}) {{\n    return (await bridge()).runFetch(fileURLToPath(CONFIG), request)\n  }},\n  async rpc(request = {{}}) {{\n    return (await bridge()).runRpc(fileURLToPath(CONFIG), request)\n  }},\n  async rpcOpen(request = {{}}) {{\n    return (await bridge()).runRpcStream(fileURLToPath(CONFIG), 'open', request)\n  }},\n  async rpcPull(request = {{}}) {{\n    return (await bridge()).runRpcStream(fileURLToPath(CONFIG), 'pull', request)\n  }},\n  async rpcPush(request = {{}}) {{\n    return (await bridge()).runRpcStream(fileURLToPath(CONFIG), 'push', request)\n  }},\n  async rpcClose(request = {{}}) {{\n    return (await bridge()).runRpcStream(fileURLToPath(CONFIG), 'close', request)\n  }},\n  async tick() {{\n    return (await bridge()).runTick(fileURLToPath(CONFIG))\n  }},\n  async http(request) {{\n    return (await bridge()).runWebhook(fileURLToPath(CONFIG), request)\n  }},\n  async upgrade(request) {{\n    return (await bridge()).runUpgrade(fileURLToPath(CONFIG), request)\n  }},\n}})\n",
                js_string(&bridge_url),
            ),
        );
        if plan.schedule {
            extra.insert("schedules".into(), json!([{ "method": "tick", "everyMinutes": 1 }]));
        }
        Some(if plan.webhook { json!({ "script": COMPANION_BACKEND_SCRIPT, "http": true }) } else { json!({ "script": COMPANION_BACKEND_SCRIPT }) })
    } else {
        None
    };
    let mut targets = plan.hooks.clone();
    if plan.instructions {
        targets.push("instructions".to_string());
    }
    if plan.gateway_step {
        targets.push("每一步的模型请求".to_string());
    }
    if plan.gateway_response {
        targets.push("回复流".to_string());
    }
    if plan.gateway_error {
        targets.push("失败重试".to_string());
    }
    if plan.client.is_some() {
        targets.push("前端界面".to_string());
    }
    if plan.schedule {
        targets.push("定时任务".to_string());
    }
    if plan.webhook {
        targets.push("webhook".to_string());
    }
    if plan.events {
        targets.push("插件间事件".to_string());
    }
    if !plan.commands.is_empty() {
        targets.push(format!("命令 {}", plan.commands.iter().map(|command| format!("/{}", command.name)).collect::<Vec<_>>().join(" ")));
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
        renderer,
        hooks: (!hooks.is_empty()).then(|| Value::Object(hooks)),
        gateway,
        // 工具仍走 dsh 插件自己的 MCP（codex_plus_dsh），不重复声明成拓展工具。
        tools: None,
        config: None,
        files,
        backend,
        tool_pipeline: None,
        providers: None,
        extra,
        assets: Default::default(),
        // 页面脚本里夹着前端插件的打包产物（宿主已限制入口不超过 4 MiB），再留出胶水代码的余量。
        copied_text: Default::default(),
        max_file_bytes: Some(DSH_CLIENT_MAX_BYTES + 64 * 1024),
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
            "gatewayStep": plan.gateway_step,
            "gatewayResponse": plan.gateway_response,
            "gatewayError": plan.gateway_error,
            "commands": plan.commands,
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
            "gatewayStep": plan.gateway_step,
            "gatewayResponse": plan.gateway_response,
            "gatewayError": plan.gateway_error,
            "commands": plan.commands,
            "installed": manager.user_dir().join(&id).join(COMPANION_MARKER).is_file(),
            "enabled": config.enabled && config.scripts.get(&key).copied().unwrap_or(true),
            "scriptsEnabled": config.enabled,
        })
    }
}

// ---------------------------------------------------------------------------
// 拓展包带来的 dsh 插件（codexpp.json 的 `dsh`）
// ---------------------------------------------------------------------------

/// 拓展包带来的插件 id。
pub fn extension_plugin_id(extension_id: &str) -> String {
    plugin_id_from_name(&format!("ext-{extension_id}"))
}

fn dsh_fingerprint(root: &Path, version: &str) -> String {
    let modified = |path: PathBuf| {
        std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or(0)
    };
    format!("{version}|{}|{}", modified(root.join("package.json")), modified(root.to_path_buf()))
}

impl DshPlugins {
    /// 让拓展包带来的插件与拓展的启停一致：启用的导入（插件目录或拓展版本变了就重新分析）、停用的停用、
    /// 拓展删掉的移除，并同步各自的配套拓展包。没有拓展声明 `dsh` 时什么也不做。
    pub fn sync_extension_plugins(&self, manager: &UserScriptManager, toolchain: &Toolchain) -> anyhow::Result<Value> {
        let declared = manager.extension_dsh_plugins()?;
        let registry = self.load()?;
        let owned: Vec<PluginRecord> = registry.plugins.iter().filter(|plugin| plugin.owner.is_some()).cloned().collect();
        if declared.is_empty() && owned.is_empty() {
            return Ok(json!({ "changed": [] }));
        }
        let mut changed = Vec::new();
        let mut errors = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (key, extension_id, root, enabled, version) in declared {
            // 包边界已由清单解析验证；比较时沿用导入记录交给 Node 的路径表示。
            let root = strip_verbatim_prefix(&root);
            let id = extension_plugin_id(&extension_id);
            seen.insert(id.clone());
            let fingerprint = dsh_fingerprint(&root, &version);
            let existing = registry.plugins.iter().find(|plugin| plugin.id == id);
            let outcome = (|| -> anyhow::Result<bool> {
                if enabled && existing.is_none_or(|plugin| plugin.owner_fingerprint.as_deref() != Some(fingerprint.as_str()) || strip_verbatim_prefix(Path::new(&plugin.root)) != root) {
                    self.import_local_as(&root, &id, toolchain)?;
                    self.update(&id, |record| {
                        record.owner = Some(key.clone());
                        record.owner_fingerprint = Some(fingerprint.clone());
                        record.enabled = true;
                        Ok(())
                    })?;
                } else if existing.is_some_and(|plugin| plugin.enabled != enabled) {
                    self.set_enabled(&id, enabled)?;
                } else {
                    return Ok(false);
                }
                self.sync_companion(manager, &id)?;
                Ok(true)
            })();
            match outcome {
                Ok(true) => changed.push(id),
                Ok(false) => {}
                Err(error) => errors.push(json!({ "extension": key, "error": format!("{error:#}") })),
            }
        }
        for plugin in owned.iter().filter(|plugin| !seen.contains(&plugin.id)) {
            let _ = self.remove_companion(manager, &plugin.id);
            match self.remove(&plugin.id) {
                Ok(_) => changed.push(plugin.id.clone()),
                Err(error) => errors.push(json!({ "plugin": plugin.id, "error": format!("{error:#}") })),
            }
        }
        Ok(json!({ "changed": changed, "errors": errors }))
    }
}

/// 用默认状态目录同步拓展包带来的插件（拓展增删启停之后调用，失败只记日志）。
pub fn sync_extension_plugins_default(manager: &UserScriptManager) -> Value {
    let plugins = DshPlugins::new(&crate::paths::default_app_state_dir());
    match plugins.sync_extension_plugins(manager, &Toolchain::detect()) {
        Ok(report) => report,
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log("dsh_plugins.extension_sync_failed", json!({ "error": format!("{error:#}") }));
            json!({ "error": format!("{error:#}") })
        }
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
    // 一个能用的插件都没有时不要再拉 Node 宿主：Codex 每开一个会话就 spawn 一次这条 MCP，
    // 空宿主每个 68 MB（加上这层包装进程 12 MB），一天下来能堆几十个常驻孤儿。
    // 直接以「没有工具」收场，Codex 侧看到的就是一份空工具表。用户装了插件再接入即可。
    if !plugins.has_enabled_plugins()? {
        return Ok(0);
    }
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
    // 插件经 Codex++ 可执行文件发起宿主调用（读写会话、调模型、问用户）。
    if let Ok(exe) = std::env::current_exe() {
        command.env(crate::extension_backend::HOST_EXE_ENV, exe);
    }
    // stdio 已接到 Codex 的管道上，不需要控制台窗口。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    let status = command.status().context("无法启动 dsh 插件宿主")?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefix_is_stripped_for_drive_and_unc_paths() {
        assert_eq!(strip_verbatim_prefix(Path::new(r"\\?\D:\a\plugin")), PathBuf::from(r"D:\a\plugin"));
        assert_eq!(strip_verbatim_prefix(Path::new(r"\\?\UNC\server\share\p")), PathBuf::from(r"\\server\share\p"));
        // 非盘符的 verbatim 路径与普通路径原样保留。
        assert_eq!(strip_verbatim_prefix(Path::new(r"\\?\Volume{x}\p")), PathBuf::from(r"\\?\Volume{x}\p"));
        assert_eq!(strip_verbatim_prefix(Path::new("/tmp/plugin")), PathBuf::from("/tmp/plugin"));
    }

    #[test]
    fn has_enabled_plugins_follows_registry_and_package_dir() {
        // 这个判断决定 MCP 要不要真的拉起 Node 宿主：判错就会白养一堆 68 MB 的孤儿进程。
        let temp = tempfile::tempdir().unwrap();
        let plugins = DshPlugins::new(temp.path());
        assert!(
            !plugins.has_enabled_plugins().unwrap(),
            "注册表不存在时视为没有插件"
        );

        let present = temp.path().join("pkg-present");
        std::fs::create_dir_all(&present).unwrap();
        let missing = temp.path().join("pkg-missing");
        let record = |id: &str, enabled: bool, root: &Path| PluginRecord {
            id: id.to_string(),
            name: id.to_string(),
            version: String::new(),
            source: PluginSource::Local {
                path: root.display().to_string(),
            },
            root: root.display().to_string(),
            managed: false,
            enabled,
            disabled_rows: Vec::new(),
            configs: BTreeMap::new(),
            imported_at: 0,
            report: Value::Null,
            imported_skills: Vec::new(),
            owner: None,
            owner_fingerprint: None,
        };
        let save = |records: Vec<PluginRecord>| {
            std::fs::create_dir_all(plugins.root()).unwrap();
            std::fs::write(
                plugins.registry_path(),
                serde_json::to_string(&Registry {
                    version: REGISTRY_VERSION,
                    plugins: records,
                })
                .unwrap(),
            )
            .unwrap();
        };

        save(vec![record("off", false, &present)]);
        assert!(!plugins.has_enabled_plugins().unwrap(), "停用的插件不算");

        save(vec![record("gone", true, &missing)]);
        assert!(
            !plugins.has_enabled_plugins().unwrap(),
            "包目录被删掉的插件不算"
        );

        save(vec![
            record("gone", true, &missing),
            record("live", true, &present),
        ]);
        assert!(plugins.has_enabled_plugins().unwrap(), "有一个能用的就算");
    }
}
