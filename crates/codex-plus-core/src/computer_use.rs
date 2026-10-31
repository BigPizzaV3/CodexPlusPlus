//! 复用 Codex app 的 Computer Use（`cua_repl` MCP server），把它接入 Claude Code /
//! Claude Desktop / Codex。
//!
//! # 为什么是「接入」而不是「实现」
//!
//! Codex 桌面版的 computer use 本身就是一个标准 stdio MCP server：
//!
//! ```text
//! node <ChatGPT.app>/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs
//!   → tools: js / js_add_node_module_dir / js_reset / turn_ended
//!   → JS 里 await import("@oai/sky") 后 sky.* 才真正操作窗口
//!   → node_repl 通过 unix socket 连到 OpenAI 签名的 SkyComputerUseService 执行 AX / CGEvent
//! ```
//!
//! 它对外只有一个 MCP 握手，没有绑定宿主；实测在外部 Node 进程里 `initialize` 正常、
//! `sky.get_app_state()` 能读到真实 AX 树。所以这里做的是「定位运行时 + 写配置 +
//! 自检」，不重写任何 AX / CGEvent 逻辑——重写只会得到更差的还原度。
//!
//! # 边界
//!
//! - 只读/写三个客户端自己的 MCP 配置文件，不碰它们的二进制与 asar。
//! - 不复制 OpenAI 的签名产物；运行时永远指向本机已安装的 ChatGPT.app。
//! - 所有写入都走「锁 → 读 → 比对 → 备份 → 原子写」，同名条目不是我们写的一律拒绝覆盖。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Claude / Codex 配置里的 MCP server 名字。改动会影响已接入用户的配置，不要改。
pub const SERVER_ID: &str = "codex-plus-cu";
/// 浏览器控制用的第二个 server 名。
pub const BROWSER_SERVER_ID: &str = "codex-plus-browser";
/// Codex `config.toml` 里的键名（TOML 里不能用连字符之外的字符，这里用下划线保持一致）。
/// 只用于识别并移除旧版本留下的条目，不再写入。
pub const CODEX_SERVER_ID: &str = "codex_plus_cu";
const CODEX_NOT_INSTALLABLE: &str =
    "Codex 自带 Computer Use，不接入 Codex++ 的：同时存在会出现两套电脑控制工具";
/// 非 macOS 平台的明确提示：运行时只随 macOS 版 ChatGPT.app 提供，别的平台不是「没装」而是「不支持」。
pub const UNSUPPORTED_PLATFORM: &str =
    "Computer Use 接入目前只支持 macOS，暂不支持 Windows：它依赖 macOS 版 ChatGPT.app 自带的 Computer Use 服务";

/// 当前平台能否接入 Computer Use。
pub fn platform_supported() -> bool {
    cfg!(target_os = "macos")
}

const MAX_CONFIG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BACKUPS: usize = 5;
const PROBE_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_PROBE_FRAME: usize = 512 * 1024;

/// 直通 shim 的源码。运行时写到 state 目录下再由 OpenAI 签名的 node 执行，
/// 见 `assets/cu/shim.mjs` 顶部的说明。
const SHIM_SOURCE: &str = include_str!("../../../assets/cu/shim.mjs");
const SHIM_FILE: &str = "cu/shim.mjs";

mod dsh;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 接入目标。字符串形式与前端 `target` 参数一一对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    ClaudeCode,
    ClaudeDesktop,
    Codex,
    /// DeepSeek Harness：cordis 补丁层（YAML），每个 server 一条 `dsh-mcp-client` 条目。
    Dsh,
    /// ZCode：`~/.zcode/cli/config.json` 的 `mcp.servers`。
    Zcode,
    /// WorkBuddy AI：`~/.workbuddy-ai/mcp.json` 的 `mcpServers`（旧版是 `~/.workbuddy`）。
    WorkBuddy,
}

impl Target {
    pub const ALL: [Target; 6] = [
        Self::ClaudeCode,
        Self::ClaudeDesktop,
        Self::Codex,
        Self::Dsh,
        Self::Zcode,
        Self::WorkBuddy,
    ];

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        Self::ALL
            .into_iter()
            .find(|target| target.id() == value)
            .with_context(|| format!("未知的接入目标：{value}"))
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claudeCode",
            Self::ClaudeDesktop => "claudeDesktop",
            Self::Codex => "codex",
            Self::Dsh => "dsh",
            Self::Zcode => "zcode",
            Self::WorkBuddy => "workBuddy",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Codex => "Codex",
            Self::Dsh => "DeepSeek Harness",
            Self::Zcode => "ZCode",
            Self::WorkBuddy => "WorkBuddy AI",
        }
    }

    /// JSON 目标里 server 表所在的键路径。非 JSON 目标返回 `None`。
    fn json_key_path(self) -> Option<&'static [&'static str]> {
        match self {
            Self::ClaudeCode | Self::ClaudeDesktop | Self::WorkBuddy => Some(&["mcpServers"]),
            // ZCode 优先读 `mcp.servers`（嵌套），为空才退回 `~/.agents/mcp.json`。
            Self::Zcode => Some(&["mcp", "servers"]),
            Self::Codex | Self::Dsh => None,
        }
    }

    /// 配置文件所在目录不存在，说明这台机器没用过该工具：不主动替它建目录。
    fn tool_home_missing(self) -> Option<String> {
        let home = home_dir();
        let (dir, name) = match self {
            Self::Dsh => (home.join(".dsh/profiles/desktop"), "DeepSeek Harness"),
            Self::Zcode => (home.join(".zcode"), "ZCode"),
            // 新版 WorkBuddy 的数据目录是 `.workbuddy-ai`，但保留旧版目录的探测。
            Self::WorkBuddy => (workbuddy_config_dir(), "WorkBuddy AI"),
            Self::ClaudeCode | Self::ClaudeDesktop | Self::Codex => return None,
        };
        (!dir.is_dir()).then(|| format!("没检测到 {name}（{} 不存在），先打开一次该应用", dir.display()))
    }
}

pub(crate) fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// WorkBuddy 的配置目录。
///
/// 应用按 `WORKBUDDY_CONFIG_DIR` → `CODEBUDDY_CONFIG_DIR` → `~/<dataFolderName>` 的顺序解析，
/// 而 `dataFolderName` 从随包的产品配置里读。当前版本的取值是 `.workbuddy-ai`（实测运行中的
/// 进程打开的全部句柄都在这个目录下），旧版本才是 `.workbuddy`。这里按同样的优先级还原：
/// 环境变量优先；否则两个目录里挑真实存在的那个（优先 `.workbuddy-ai`）；都没有时按新版默认。
fn workbuddy_config_dir() -> PathBuf {
    for key in ["WORKBUDDY_CONFIG_DIR", "CODEBUDDY_CONFIG_DIR"] {
        if let Some(value) = std::env::var_os(key) {
            let path = PathBuf::from(value);
            if !path.as_os_str().is_empty() {
                return path;
            }
        }
    }
    let home = home_dir();
    let current = home.join(".workbuddy-ai");
    let legacy = home.join(".workbuddy");
    if current.is_dir() || !legacy.is_dir() {
        current
    } else {
        legacy
    }
}

/// 拉起原生 CU server 时从当前环境透传的变量，与 Codex 的 `DEFAULT_ENV_VARS`（Unix）一致。
const NATIVE_ENV_PASSTHROUGH: [&str; 11] = [
    "HOME",
    "LOGNAME",
    "PATH",
    "SHELL",
    "USER",
    "__CF_USER_TEXT_ENCODING",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "TZ",
];

/// 代理变量。浏览器面打开新网站前要联网做一次安全检查（Codex 里借道应用自己的通道，
/// 这里没有），国内网络缺了代理会一直卡到 30 秒超时（实测 2026-10-25）。
const PROXY_ENV_KEYS: [&str; 8] = [
    "http_proxy",
    "HTTP_PROXY",
    "https_proxy",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
];

/// 子进程用的代理，按顺序取第一个有的：
///
/// 1. 调用方环境里的代理变量，原样透传；
/// 2. 系统代理（图形界面客户端如 DeepSeek Harness 拉起时没有终端里的代理变量）；
/// 3. 都没有、而且直连 chatgpt.com 连不上（国内无代理）：把代理指到本机一个没人监听的
///    端口，让 Codex 组件访问 OpenAI 的请求（功能开关、错误上报）当场失败。
///
/// 第 3 条不是绕过验证：实测（2026-10-25）OpenAI 的地址连不上时，Codex 的浏览器组件本来
/// 就会继续执行，打开新网站照样会发授权请求；问题只在于国内这些连接不报错、一直挂着，
/// 整个调用卡到 30 秒超时。改成立刻失败后，新开标签页 0.6 秒完成。网页由 Chrome 自己加载，
/// 不受这些变量影响。
pub(crate) fn proxy_env() -> BTreeMap<String, String> {
    let inherited: BTreeMap<String, String> = PROXY_ENV_KEYS
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (key.to_string(), value))
        })
        .collect();
    if !inherited.is_empty() {
        return inherited;
    }
    if let Some(proxy) = crate::proxy::detect_system_proxy() {
        return proxy_vars(&proxy);
    }
    if openai_reachable() {
        return BTreeMap::new();
    }
    proxy_vars(UNREACHABLE_PROXY)
}

/// 本机不会有人监听的端口（discard）：连接当场被拒。
const UNREACHABLE_PROXY: &str = "http://127.0.0.1:9";

fn proxy_vars(proxy: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    for key in ["http_proxy", "https_proxy", "all_proxy"] {
        env.insert(key.to_string(), proxy.to_string());
        env.insert(key.to_uppercase(), proxy.to_string());
    }
    // 本机回环始终直连：Codex 组件之间有本地通信。
    for key in ["no_proxy", "NO_PROXY"] {
        env.insert(key.to_string(), "localhost,127.0.0.1,::1".to_string());
    }
    env
}

/// 直连 chatgpt.com:443 能否在 1.5 秒内建立 TCP 连接。国内无代理时这一步就会超时，
/// 正是 Codex 组件会被挂住的情形。
fn openai_reachable() -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let Ok(addresses) = ("chatgpt.com", 443).to_socket_addrs() else {
        return false;
    };
    addresses
        .take(2)
        .any(|address| TcpStream::connect_timeout(&address, Duration::from_millis(1500)).is_ok())
}

/// 自定义 CA 证书路径，非空时透传，与 Codex 的 `CUSTOM_CA_ENV_KEYS` 一致。
/// 原生 server 会联网上报统计，公司代理换了根证书时要靠它们。
const CUSTOM_CA_ENV_KEYS: [&str; 11] = [
    "CODEX_CA_CERTIFICATE",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "GIT_SSL_CAINFO",
    "CARGO_HTTP_CAINFO",
    "PIP_CERT",
    "BUNDLE_SSL_CA_CERT",
    "npm_config_cafile",
    "NPM_CONFIG_CAFILE",
];

/// 本机 Codex CU 运行时。全部为绝对路径，写进配置后不再二次推导。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLocation {
    pub app_path: String,
    pub node: String,
    /// 原生 CU MCP server（`SkyComputerUseClient mcp`）。直通路径的执行器入口。
    pub client: String,
    /// `cua-repl` 入口，仅保留给 Codex 目标的兼容条目使用。
    pub cua_repl: String,
    pub node_repl: String,
    pub node_module_dirs: String,
    pub codex_home: String,
    pub service_path: Option<String>,
    pub codex_cli: Option<String>,
    pub app_version: Option<String>,
    pub package_version: Option<String>,
}

impl RuntimeLocation {
    pub(crate) fn env(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("NODE_REPL_NODE_PATH".into(), self.node.clone());
        env.insert(
            "NODE_REPL_NODE_MODULE_DIRS".into(),
            self.node_module_dirs.clone(),
        );
        env.insert(
            "NODE_REPL_TRUSTED_CODE_PATHS".into(),
            format!("{}:{}", self.codex_home, self.node_module_dirs),
        );
        env.insert("CODEX_HOME".into(), self.codex_home.clone());
        env.insert("CUA_REPL_NODE_REPL_PATH".into(), self.node_repl.clone());
        // 浏览器面走 Codex 自己的 Chrome 扩展桥；代理会补上它要求的 turn 元数据。
        env.insert("CUA_REPL_ENABLED_SURFACES".into(), "browser,computer".into());
        env.insert("BROWSER_USE_AVAILABLE_BACKENDS".into(), "chrome".into());
        env.insert(
            "NODE_REPL_NATIVE_PIPE_CONNECT_TIMEOUT_MS".into(),
            "1000".into(),
        );
        env.insert(
            "NODE_REPL_TRUSTED_SERVICES".into(),
            "{\"browser\":\"@oai/browser-desktop/service\",\"sky\":\"@oai/sky/service\"}".into(),
        );
        if let Some(service) = &self.service_path {
            env.insert("SKY_CUA_SERVICE_PATH".into(), service.clone());
        }
        // Codex 桌面版的 CU 需要同包的 CLI 作为 app-server 侧审批通道；
        // 缺这一项会直接报 -10005「codex app-server exited」。
        if let Some(codex_cli) = &self.codex_cli {
            env.insert("CODEX_CLI_PATH".into(), codex_cli.clone());
        }
        if let Some(version) = &self.app_version {
            env.insert("BROWSER_USE_CODEX_APP_VERSION".into(), version.clone());
            env.insert("BROWSER_USE_CODEX_APP_BUILD_FLAVOR".into(), "prod".into());
        }
        env.insert(
            "NODE_REPL_INSTRUCTIONS_USE_CASE_COMPUTER_USE".into(),
            "Control desktop apps on macOS through Computer Use.".into(),
        );
        env
    }

    /// 浏览器面的环境。与 CU 的 `env()` 分开：
    ///
    /// - 只开 `browser` 面（CU 走原生直通，不需要 cua-repl 的 computer 面）；
    /// - `BROWSER_USE_TINYSKY_ENABLED=1` 才会下发 `tab.ax`，否则 Chrome 后端只有
    ///   Playwright/DOM，元素索引点击直接不可用。
    pub(crate) fn browser_env(&self) -> BTreeMap<String, String> {
        let mut env = self.env();
        env.insert("CUA_REPL_ENABLED_SURFACES".into(), "browser".into());
        env.insert("BROWSER_USE_AVAILABLE_BACKENDS".into(), "chrome".into());
        env.insert("BROWSER_USE_TINYSKY_ENABLED".into(), "1".into());
        // 与 Codex 插件配置（unified-computer-use 的 .mcp.json）原文一致。
        env.insert(
            "NODE_REPL_INSTRUCTIONS_USE_CASE_CHROME".into(),
            "Control the Chrome browser in conjunction with the Chrome Plugin. Prefer this method of controlling Chrome over alternatives (such as Computer Use) unless the user explicitly mentions an alternative.".into(),
        );
        env
    }

    /// 原生 CU server 的**完整**环境：调用方的其它变量一律不传。
    ///
    /// 照 Codex 自己拉起 stdio MCP server 的做法（`codex-rs/rmcp-client/src/utils.rs`
    /// 的 `create_env_for_mcp_server`，rust-v0.163.0-alpha.11.2）：只透传
    /// [`NATIVE_ENV_PASSTHROUGH`] 白名单和自定义 CA 变量。Claude Code 会把用户
    /// `settings.json` 里的 `env`（含 `ANTHROPIC_AUTH_TOKEN` 等）继承给 MCP 进程，
    /// 原样传下去等于把 Key 交给第三方程序。
    ///
    /// 在白名单之上只加两项（实测 2026-10-25，只读探针）：
    /// - `CODEX_CLI_PATH`：**必须**。缺了连 `list_apps` 都失败为
    ///   `-10005 codex app-server exited before returning a response`。
    ///   Codex 里不用传，推测是因为那边的直接父进程就是 `codex` 本身。
    /// - `CODEX_HOME`：默认 `~/.codex` 时可有可无，Codex 目录不在默认位置时才需要。
    ///
    /// `SKY_CUA_SERVICE_PATH` 实测去掉后结果完全相同，不再传。
    ///
    /// 代理变量另算（见 [`proxy_env`]）：JS 档的浏览器面没有代理会卡死。
    pub(crate) fn native_client_env(&self) -> BTreeMap<String, String> {
        let mut env: BTreeMap<String, String> = NATIVE_ENV_PASSTHROUGH
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_string(), value)))
            .collect();
        for key in CUSTOM_CA_ENV_KEYS {
            if let Ok(value) = std::env::var(key)
                && !value.is_empty()
            {
                env.insert(key.to_string(), value);
            }
        }
        env.insert("CODEX_HOME".into(), self.codex_home.clone());
        if let Some(cli) = &self.codex_cli {
            env.insert("CODEX_CLI_PATH".into(), cli.clone());
        }
        env.extend(proxy_env());
        env
    }

    /// JS 档 `cua_repl` 的完整环境：直通档的白名单（含 `CODEX_CLI_PATH`）加上
    /// `cua_repl` / `node_repl` 需要的那几项，同样不继承调用方的环境。
    ///
    /// 和 Codex 一样桌面、浏览器两个面都开：一个 `js` 工具里既能 `cua.getApp` 也能
    /// `cua.getTab`。这时 `codex-plus-browser` 不再暴露工具（见 `browser_use::serve`），
    /// 免得两个进程抢同一批标签。Codex 的 `iab` / `mcpapps` 后端只在 Codex 应用里有，不开。
    pub(crate) fn js_env(&self) -> BTreeMap<String, String> {
        let mut env = self.native_client_env();
        env.extend(self.browser_env());
        env.insert("CUA_REPL_ENABLED_SURFACES".into(), "browser,computer".into());
        env
    }

    fn self_command(&self) -> String {
        std::env::current_exe()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| self.node.clone())
    }

    /// 写进客户端配置的条目。
    ///
    /// 指向 Codex++ 自己的二进制而不是 `cua-repl`：中间需要一层 MCP 代理处理
    /// `elicitation`（Claude Code 的 MCP 客户端不声明这个能力，直连会在接管 app 的
    /// 瞬间断掉）。代理内部仍原样拉起 `cua-repl`，工具面与执行器都没有变化。
    fn server_entry(&self) -> Value {
        entry_with_state_dir(
            json!({
                "command": self.self_command(),
                "args": ["--codex-plus-cu-mcp"],
            }),
            crate::paths::state_dir_override(),
        )
    }

    /// 本功能会写进每个客户端的全部条目：桌面操控 + 浏览器。
    fn managed_entries(&self) -> [(&'static str, Value); 2] {
        [
            (SERVER_ID, self.server_entry()),
            (BROWSER_SERVER_ID, self.browser_server_entry()),
        ]
    }

    /// 浏览器控制的条目。单独一个 MCP server，和 CU 并列。
    fn browser_server_entry(&self) -> Value {
        entry_with_state_dir(
            json!({
                "command": self.self_command(),
                "args": ["--codex-plus-browser-mcp"],
            }),
            crate::paths::state_dir_override(),
        )
    }
}

/// 管理器用了显式状态目录时，把它带进 MCP 条目的 `env`。
///
/// 本地开发实例靠 `Info.plist` 的 `LSEnvironment` 设置 `CODEX_PLUS_STATE_DIR`，
/// 这只在经 LaunchServices 打开时生效；Claude 拉起 MCP 进程时没有它，于是
/// 管理器写的审批策略 / 审批记忆与 MCP 进程读的不是同一份文件（实测 2026-10-24）。
/// 正式安装没有这个覆盖，条目保持不带 `env`。
fn entry_with_state_dir(mut entry: Value, state_dir: Option<PathBuf>) -> Value {
    if let (Some(dir), Some(object)) = (state_dir, entry.as_object_mut()) {
        object.insert(
            "env".into(),
            json!({ crate::paths::STATE_DIR_ENV: dir.to_string_lossy() }),
        );
    }
    entry
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub id: String,
    pub label: String,
    pub config_path: String,
    pub registered: bool,
    /// registered / missing / needs_update / foreign / unsupported
    pub state: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    /// 非 macOS 为 false，前端据此显示「暂不支持」而不是「找不到 ChatGPT」。
    pub platform_supported: bool,
    pub runtime_found: bool,
    /// Claude Desktop 运行中会重写自己的配置，这时改 `mcpServers` 容易在它退出时被覆盖。
    pub claude_desktop_running: bool,
    pub app_path: Option<String>,
    pub package_version: Option<String>,
    pub service_path: Option<String>,
    pub config_hint: Option<String>,
    pub targets: BTreeMap<String, TargetStatus>,
    /// 用户自定义的目标，按添加顺序排列。
    pub custom_targets: Vec<CustomTargetStatus>,
}

/// 自定义目标的状态：在 `TargetStatus` 之外带上类型与用户填写的路径。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTargetStatus {
    #[serde(flatten)]
    pub status: TargetStatus,
    pub kind: Target,
    pub path: String,
    /// 用户起的名字（可能为空）。`status.label` 已经按它取了显示名。
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallReceipt {
    pub target: String,
    pub changed: bool,
    pub backup_path: Option<String>,
    pub targets: BTreeMap<String, TargetStatus>,
    pub custom_targets: Vec<CustomTargetStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub ok: bool,
    pub target: String,
    pub app_count: Option<usize>,
    pub browser_count: Option<usize>,
    pub sample_apps: Vec<String>,
    pub duration_ms: u64,
    pub failure_kind: Option<String>,
    pub failure_detail: Option<String>,
}

// ---------------------------------------------------------------------------
// 运行时定位
// ---------------------------------------------------------------------------

/// 找本机的 Codex CU 运行时。找不到就返回 `Ok(None)`，由调用方决定怎么提示。
pub fn locate_runtime() -> anyhow::Result<Option<RuntimeLocation>> {
    let codex_home = crate::codex_home::default_codex_home_dir();
    // 原生 CU server 由 Codex 的安装器放在 codex home 下，不随 app 走。
    let client = codex_home
        .join("computer-use/Codex Computer Use.app")
        .join("Contents/SharedSupport/SkyComputerUseClient.app")
        .join("Contents/MacOS/SkyComputerUseClient");
    for app_path in candidate_app_paths() {
        let resources = app_path.join("Contents/Resources");
        let node = resources.join("cua_node/bin/node");
        let cua_repl = resources.join("cua_node/lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs");
        let node_repl = resources.join("cua_node/bin/node_repl");
        // 直通路径只需要 node + 原生 client；cua-repl 只服务 Codex 目标的兼容条目。
        if !(node.is_file() && client.is_file()) {
            continue;
        }
        let module_dirs = resources.join("cua_node/lib/node_modules");
        let package_version = read_package_version(&module_dirs.join("@oai/cua-repl/package.json"));
        let service_path = codex_home
            .join("computer-use/Codex Computer Use.app")
            .is_dir()
            .then(|| codex_home.join("computer-use/Codex Computer Use.app"));
        let codex_cli = resources
            .join("codex-cli/CodexCLI.app/Contents/MacOS/codex")
            .is_file()
            .then(|| {
                resources
                    .join("codex-cli/CodexCLI.app/Contents/MacOS/codex")
                    .to_string_lossy()
                    .into_owned()
            });
        let app_version = read_bundle_version(&app_path);
        return Ok(Some(RuntimeLocation {
            app_path: app_path.to_string_lossy().into_owned(),
            node: node.to_string_lossy().into_owned(),
            client: client.to_string_lossy().into_owned(),
            cua_repl: cua_repl.to_string_lossy().into_owned(),
            node_repl: node_repl.to_string_lossy().into_owned(),
            node_module_dirs: module_dirs.to_string_lossy().into_owned(),
            codex_home: codex_home.to_string_lossy().into_owned(),
            service_path: service_path.map(|path| path.to_string_lossy().into_owned()),
            codex_cli,
            app_version,
            package_version,
        }));
    }
    Ok(None)
}

/// 把内嵌的 shim 写到 state 目录，返回它的绝对路径。
///
/// 内容没变就不重写：MCP 进程每个会话启动一次，无谓的写入会让并发会话互相踩。
pub fn ensure_shim() -> anyhow::Result<PathBuf> {
    let path = crate::paths::default_app_state_dir().join(SHIM_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    let unchanged = fs::read_to_string(&path)
        .map(|existing| existing == SHIM_SOURCE)
        .unwrap_or(false);
    if !unchanged {
        fs::write(&path, SHIM_SOURCE).with_context(|| format!("无法写入 {}", path.display()))?;
    }
    Ok(path)
}

fn candidate_app_paths() -> Vec<PathBuf> {
    let mut paths = vec![
        PathBuf::from("/Applications/ChatGPT.app"),
        PathBuf::from("/Applications/Codex.app"),
    ];
    if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        paths.push(home.join("Applications/ChatGPT.app"));
        paths.push(home.join("Applications/Codex.app"));
    }
    // Spotlight 兜底：应用可能被改过名字或装在非标准位置。
    if let Ok(output) = Command::new("mdfind")
        .arg("kMDItemCFBundleIdentifier == \"com.openai.chat\"")
        .stderr(Stdio::null())
        .output()
        && output.status.success()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let line = line.trim();
            if line.ends_with(".app") {
                paths.push(PathBuf::from(line));
            }
        }
    }
    paths
}

/// 读 `CFBundleShortVersionString`。CU 服务用它做客户端版本校验。
fn read_bundle_version(app_path: &Path) -> Option<String> {
    let plist = app_path.join("Contents/Info.plist");
    if !plist.is_file() {
        return None;
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(&plist)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn read_package_version(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// 路径
// ---------------------------------------------------------------------------

pub(crate) fn claude_code_config_path() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude.json")
}

/// Claude Desktop 的配置路径可能有多个：官方版用 `Claude/`，Codex++ 的增强版副本用
/// `Claude-3p/`，两者互不读取。存在几个就写几个，避免「官方装了、用户实际跑副本」时
/// 接入看起来成功却没生效。
fn claude_desktop_config_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("CLAUDE_USER_DATA_DIR") {
        let dir = PathBuf::from(dir);
        if !dir.as_os_str().is_empty() {
            paths.push(dir.join("claude_desktop_config.json"));
        }
    }
    let base = if cfg!(target_os = "macos") {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join("Library/Application Support"))
    } else {
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().to_path_buf())
    };
    if let Some(base) = base {
        for name in ["Claude", "Claude-3p"] {
            let dir = base.join(name);
            if dir.is_dir() {
                paths.push(dir.join("claude_desktop_config.json"));
            }
        }
    }
    if paths.is_empty() {
        paths.push(
            directories::BaseDirs::new()
                .map(|dirs| dirs.config_dir().join("Claude/claude_desktop_config.json"))
                .unwrap_or_else(|| PathBuf::from("claude_desktop_config.json")),
        );
    }
    paths.dedup();
    paths
}

fn codex_config_path() -> PathBuf {
    crate::codex_home::default_codex_home_dir().join("config.toml")
}

/// DSH 桌面版 profile 的用户补丁层。`cordis.yml` 本身由 DSH 生成、注释里明确写着
/// 「Edit cordis.patch.yml, not this file」。
fn dsh_config_path() -> PathBuf {
    home_dir().join(".dsh/profiles/desktop/cordis.patch.yml")
}

fn config_paths(target: Target) -> Vec<PathBuf> {
    match target {
        Target::ClaudeCode => vec![claude_code_config_path()],
        Target::ClaudeDesktop => claude_desktop_config_paths(),
        Target::Codex => vec![codex_config_path()],
        Target::Dsh => vec![dsh_config_path()],
        Target::Zcode => vec![home_dir().join(".zcode/cli/config.json")],
        Target::WorkBuddy => vec![workbuddy_config_dir().join("mcp.json")],
    }
}

// ---------------------------------------------------------------------------
// 自定义接入目标
// ---------------------------------------------------------------------------

/// 允许自定义目录的客户端类型。与前端列出的内置目标一致：Codex 自己会写
/// `[mcp_servers.computer-use]`，不提供。
pub const CUSTOM_KINDS: [Target; 5] = [
    Target::ClaudeCode,
    Target::ClaudeDesktop,
    Target::Dsh,
    Target::Zcode,
    Target::WorkBuddy,
];

const CUSTOM_STORE_FILE: &str = "computer-use-targets.json";
const CUSTOM_ID_PREFIX: &str = "custom-";

/// 用户自定义的接入目标：同一种客户端，换一个配置目录。
///
/// 典型场景是客户端被包装器换了配置目录（例如 reclaude 给 Claude Code 设置
/// `CLAUDE_CONFIG_DIR`），内置目标写的默认路径它根本不读。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomTarget {
    pub id: String,
    pub kind: Target,
    /// 用户填写的路径，原样保存：可以是配置目录，也可以直接是配置文件。
    pub path: String,
    /// 用户起的名字，只用于界面显示；为空时显示客户端类型名。
    /// 写进客户端配置的 MCP 服务名不受影响（仍是 `codex-plus-cu`）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CustomStore {
    #[serde(default)]
    targets: Vec<CustomTarget>,
}

impl Target {
    /// 配置目录里该客户端的配置文件名。
    fn config_file_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => ".claude.json",
            Self::ClaudeDesktop => "claude_desktop_config.json",
            Self::Codex => "config.toml",
            Self::Dsh => "cordis.patch.yml",
            Self::Zcode => "config.json",
            Self::WorkBuddy => "mcp.json",
        }
    }
}

/// 自定义目标名字的长度上限（按字符计）。
const CUSTOM_NAME_MAX_CHARS: usize = 40;

/// 规整用户起的名字：去掉首尾空白；不允许控制字符和过长的名字。空字符串表示不起名。
fn normalize_custom_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim();
    ensure!(
        !trimmed.chars().any(char::is_control),
        "名称不能包含换行等控制字符"
    );
    ensure!(
        trimmed.chars().count() <= CUSTOM_NAME_MAX_CHARS,
        "名称最多 {CUSTOM_NAME_MAX_CHARS} 个字"
    );
    Ok(trimmed.to_string())
}

impl CustomTarget {
    /// 界面上显示的名字：起了名用名字，否则用客户端类型名。
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.kind.label().to_string()
        } else {
            self.name.clone()
        }
    }

    /// 实际读写的配置文件。
    pub fn config_path(&self) -> PathBuf {
        self.config_path_in(&home_dir())
    }

    fn config_path_in(&self, home: &Path) -> PathBuf {
        resolve_custom_path(self.kind, &self.path, home)
    }

    /// Claude Code 回合钩子所在的 settings.json：与配置文件同目录，
    /// 和 `CLAUDE_CONFIG_DIR` 的布局一致。其他类型没有钩子。
    pub fn claude_settings_path(&self) -> Option<PathBuf> {
        (self.kind == Target::ClaudeCode)
            .then(|| self.config_path().with_file_name("settings.json"))
    }
}

/// 把用户输入解析成配置文件路径。
///
/// `~` 展开为家目录；已存在的目录、或没有配置扩展名的路径按目录处理并拼上该类型的
/// 文件名；已存在的文件或带 json/toml/yml 扩展名的按文件原样使用。
fn resolve_custom_path(kind: Target, input: &str, home: &Path) -> PathBuf {
    let trimmed = input.trim();
    let expanded = if trimmed == "~" {
        home.to_path_buf()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(trimmed)
    };
    if expanded.is_dir() {
        return expanded.join(kind.config_file_name());
    }
    let looks_like_file = expanded.is_file()
        || expanded
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "json" | "toml" | "yml" | "yaml"));
    if looks_like_file {
        expanded
    } else {
        expanded.join(kind.config_file_name())
    }
}

fn custom_store_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(CUSTOM_STORE_FILE)
}

fn load_custom_from(path: &Path) -> anyhow::Result<Vec<CustomTarget>> {
    let Some(text) = read_text(path)? else {
        return Ok(Vec::new());
    };
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let store: CustomStore = serde_json::from_str(&text)
        .with_context(|| format!("{} 不是合法 JSON，未读取自定义目标", path.display()))?;
    Ok(store.targets)
}

fn save_custom_to(path: &Path, targets: &[CustomTarget]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    let store = CustomStore { targets: targets.to_vec() };
    let text = format!("{}\n", serde_json::to_string_pretty(&store)?);
    crate::settings::atomic_write(path, text.as_bytes())
        .with_context(|| format!("无法保存 {}", path.display()))
}

/// 已保存的自定义目标。存储文件损坏时返回空列表，由增删操作报错，不在查询时打断页面。
pub fn custom_targets() -> Vec<CustomTarget> {
    load_custom_from(&custom_store_path()).unwrap_or_default()
}

pub fn custom_target(id: &str) -> anyhow::Result<CustomTarget> {
    load_custom_from(&custom_store_path())?
        .into_iter()
        .find(|target| target.id == id)
        .with_context(|| format!("找不到自定义接入目标：{id}"))
}

/// 新增一个自定义目标（只登记，不写客户端配置）。`name` 可以为空。
pub fn add_custom_target(kind: Target, path: &str, name: &str) -> anyhow::Result<CustomTarget> {
    with_lock(|| add_custom_in(&custom_store_path(), kind, path, name, &home_dir()))
}

/// 给自定义目标改名（只改显示名，不动客户端配置）。`name` 为空表示恢复默认显示。
pub fn rename_custom_target(id: &str, name: &str) -> anyhow::Result<StatusReport> {
    with_lock(|| rename_custom_in(&custom_store_path(), id, name))?;
    status()
}

fn rename_custom_in(store: &Path, id: &str, name: &str) -> anyhow::Result<CustomTarget> {
    let name = normalize_custom_name(name)?;
    let mut targets = load_custom_from(store)?;
    let target = targets
        .iter_mut()
        .find(|target| target.id == id)
        .with_context(|| format!("找不到自定义接入目标：{id}"))?;
    target.name = name;
    let renamed = target.clone();
    save_custom_to(store, &targets)?;
    Ok(renamed)
}

fn add_custom_in(
    store: &Path,
    kind: Target,
    input: &str,
    name: &str,
    home: &Path,
) -> anyhow::Result<CustomTarget> {
    ensure!(CUSTOM_KINDS.contains(&kind), "{} 不支持自定义配置目录", kind.label());
    let name = normalize_custom_name(name)?;
    let trimmed = input.trim();
    ensure!(!trimmed.is_empty(), "请填写配置目录");
    let file = resolve_custom_path(kind, trimmed, home);
    ensure!(
        file.is_absolute(),
        "请填写绝对路径（可用 ~ 表示家目录）：{trimmed}"
    );
    let dir = file.parent().unwrap_or(&file);
    ensure!(dir.is_dir(), "目录不存在：{}", dir.display());
    ensure!(
        !config_paths(kind).contains(&file),
        "{} 是 {} 的默认配置，内置目标已经覆盖",
        file.display(),
        kind.label()
    );
    let mut targets = load_custom_from(store)?;
    ensure!(
        !targets
            .iter()
            .any(|target| target.kind == kind && target.config_path_in(home) == file),
        "已经添加过：{}",
        file.display()
    );
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let mut id = format!("{CUSTOM_ID_PREFIX}{stamp}");
    let mut suffix = 1;
    while targets.iter().any(|target| target.id == id) {
        id = format!("{CUSTOM_ID_PREFIX}{stamp}-{suffix}");
        suffix += 1;
    }
    let target = CustomTarget { id, kind, path: trimmed.to_string(), name };
    targets.push(target.clone());
    save_custom_to(store, &targets)?;
    Ok(target)
}

/// 删除自定义目标：先撤掉我们写进它配置里的条目，再从列表里去掉。
/// 别人写的条目一律不动，与内置目标的「移除」一致。
pub fn remove_custom_target(id: &str) -> anyhow::Result<StatusReport> {
    let target = custom_target(id)?;
    uninstall_at(target.kind, &[target.config_path()])?;
    with_lock(|| {
        let store = custom_store_path();
        let mut targets = load_custom_from(&store)?;
        targets.retain(|item| item.id != id);
        save_custom_to(&store, &targets)
    })?;
    status()
}

pub fn install_custom(id: &str) -> anyhow::Result<InstallReceipt> {
    let target = custom_target(id)?;
    ensure!(platform_supported(), "{UNSUPPORTED_PLATFORM}");
    let runtime =
        locate_runtime()?.context("未找到 Codex 运行时（ChatGPT.app），无法接入 Computer Use")?;
    let file = target.config_path();
    if let Some(dir) = file.parent() {
        ensure!(dir.is_dir(), "目录不存在：{}", dir.display());
    }
    let mut receipt = install_at(&runtime, target.kind, &[file])?;
    receipt.target = target.id;
    Ok(receipt)
}

pub fn uninstall_custom(id: &str) -> anyhow::Result<InstallReceipt> {
    let target = custom_target(id)?;
    let mut receipt = uninstall_at(target.kind, &[target.config_path()])?;
    receipt.target = target.id;
    Ok(receipt)
}


fn lock_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(".computer-use.lock")
}

// ---------------------------------------------------------------------------
// 状态查询
// ---------------------------------------------------------------------------

/// 汇总三个客户端的接入状态（不启动任何进程）。
pub fn status() -> anyhow::Result<StatusReport> {
    let runtime = locate_runtime()?;
    let mut targets = BTreeMap::new();
    for target in Target::ALL {
        targets.insert(
            target.id().to_string(),
            target_status(target, runtime.as_ref()),
        );
    }
    let custom_targets = custom_targets()
        .iter()
        .map(|custom| custom_status(custom, runtime.as_ref()))
        .collect();
    Ok(StatusReport {
        platform_supported: platform_supported(),
        runtime_found: runtime.is_some(),
        claude_desktop_running: claude_desktop_running(),
        app_path: runtime.as_ref().map(|item| item.app_path.clone()),
        package_version: runtime
            .as_ref()
            .and_then(|item| item.package_version.clone()),
        service_path: runtime.as_ref().and_then(|item| item.service_path.clone()),
        config_hint: runtime.is_none().then(|| {
            if platform_supported() {
                "未找到 ChatGPT.app：请先安装 Codex 桌面版，或把应用放进 /Applications".to_string()
            } else {
                UNSUPPORTED_PLATFORM.to_string()
            }
        }),
        targets,
        custom_targets,
    })
}

/// Claude Desktop 是否在运行。只在 macOS/Windows 上探测，失败一律当作没运行。
fn claude_desktop_running() -> bool {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/pgrep")
            .args(["-f", "Contents/MacOS/Claude"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return matches!(output, Ok(status) if status.success());
    }
    #[cfg(windows)]
    {
        // Windows 上进程名即 Claude.exe。直接枚举进程快照，不再起 tasklist：
        // 管理器是 GUI 子系统，起控制台程序不加 CREATE_NO_WINDOW 就会闪黑色终端，
        // 而 Agent 页每次切到 Claude Code 都会查这里（issue #2778）。
        // 区分大小写：小写的 claude.exe 是 Claude Code 命令行，不是桌面端（与原 tasklist 判断一致）。
        return crate::windows_enumerate_processes()
            .iter()
            .any(|process| process.exe_file == "Claude.exe");
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

fn target_status(target: Target, runtime: Option<&RuntimeLocation>) -> TargetStatus {
    status_at(target, &config_paths(target), runtime, target.tool_home_missing())
}

fn custom_status(custom: &CustomTarget, runtime: Option<&RuntimeLocation>) -> CustomTargetStatus {
    let file = custom.config_path();
    // 自定义目录不存在时不替用户建目录，和内置目标「工具没装」同样处理。
    let dir_missing = file
        .parent()
        .filter(|dir| !dir.is_dir())
        .map(|dir| format!("目录不存在：{}", dir.display()));
    let mut status = status_at(custom.kind, &[file], runtime, dir_missing);
    status.id = custom.id.clone();
    status.label = custom.display_name();
    CustomTargetStatus {
        status,
        kind: custom.kind,
        path: custom.path.clone(),
        name: custom.name.clone(),
    }
}

/// 按给定的配置文件判定接入状态。`home_missing` 非空时直接判为不支持。
fn status_at(
    target: Target,
    config_paths: &[PathBuf],
    runtime: Option<&RuntimeLocation>,
    home_missing: Option<String>,
) -> TargetStatus {
    let path_text = config_paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("、");
    let unsupported = |message: String| TargetStatus {
        id: target.id().into(),
        label: target.label().into(),
        config_path: path_text.clone(),
        registered: false,
        state: "unsupported".into(),
        message,
    };
    let Some(runtime) = runtime else {
        return unsupported("未找到 Codex 运行时（ChatGPT.app）".into());
    };
    if let Some(message) = home_missing {
        return unsupported(message);
    }
    let outcome = match target {
        Target::ClaudeCode | Target::ClaudeDesktop | Target::Zcode | Target::WorkBuddy => {
            let key_path = target.json_key_path().unwrap_or(&["mcpServers"]);
            // 每个 (配置文件, server) 组合算一项：CU 与浏览器任一缺失都要提示更新。
            let mut registered = 0usize;
            let mut stale = 0usize;
            let mut foreign: Option<String> = None;
            let mut failure: Option<String> = None;
            for path in config_paths {
                for (id, expected) in runtime.managed_entries() {
                    match read_json_entry(path, key_path, id) {
                        Ok(Some(existing)) if entry_matches(&existing, &expected) => {
                            registered += 1
                        }
                        Ok(Some(existing)) if looks_like_ours(&existing, runtime) => {
                            registered += 1;
                            stale += 1;
                        }
                        Ok(Some(_)) => {
                            foreign = Some(format!(
                                "{} 里的同名条目 {id} 由其他工具写入，未改动",
                                path.display()
                            ));
                        }
                        Ok(None) => {}
                        Err(error) => failure = Some(error.to_string()),
                    }
                }
            }
            let total = config_paths.len() * runtime.managed_entries().len();
            if let Some(failure) = failure {
                (false, "foreign", failure)
            } else if let Some(foreign) = foreign {
                (false, "foreign", foreign)
            } else if registered == 0 {
                let mut message = "尚未接入".to_string();
                // WorkBuddy 只在 mcp.json 不存在时写入示例服务器；由我们先建文件就不会再种。
                if target == Target::WorkBuddy && !config_paths[0].exists() {
                    message.push_str("；接入会新建 mcp.json，WorkBuddy 之后不会再写入它的示例 MCP 服务");
                }
                (false, "missing", message)
            } else if registered < total {
                (true, "needs_update", "部分条目仍未接入（桌面操控或浏览器）".to_string())
            } else if stale > 0 {
                (true, "needs_update", "已接入，但运行时路径或版本已变化".to_string())
            } else if config_paths.len() > 1 {
                (true, "registered", "已接入（多份配置）".to_string())
            } else {
                (true, "registered", "已接入".to_string())
            }
        }
        Target::Dsh => match dsh::status(&config_paths[0], runtime) {
            Ok(dsh::State::Missing) => (false, "missing", "尚未接入".to_string()),
            Ok(dsh::State::Current) => (true, "registered", "已接入".to_string()),
            Ok(dsh::State::Stale) => (
                true,
                "needs_update",
                "已接入，但运行时路径或版本已变化".to_string(),
            ),
            Ok(dsh::State::Foreign) => (
                false,
                "foreign",
                "补丁层里已有同名条目且不是本功能写入的，未改动".to_string(),
            ),
            Err(error) => (false, "foreign", error.to_string()),
        },
        // Codex 自带 Computer Use，不提供接入；这里只检测旧版本或命令行留下的残留，
        // 残留会让 Codex 同时出现两套电脑控制工具。
        Target::Codex => match read_codex_entry(&config_paths[0], CODEX_SERVER_ID) {
            Ok(None) => (
                false,
                "unsupported",
                "Codex 使用自带的 Computer Use，无需接入".to_string(),
            ),
            Ok(Some(entry)) if codex_entry_looks_like_ours(&entry) => (
                true,
                "registered",
                "Codex 配置里残留 Codex++ 的 Computer Use 条目，会与 Codex 自带的重复，建议移除".to_string(),
            ),
            Ok(Some(_)) => (
                false,
                "foreign",
                "同名条目由其他工具写入，未改动".to_string(),
            ),
            Err(error) => (false, "foreign", error.to_string()),
        },
    };
    TargetStatus {
        id: target.id().into(),
        label: target.label().into(),
        config_path: path_text,
        registered: outcome.0,
        state: outcome.1.into(),
        message: outcome.2,
    }
}

// ---------------------------------------------------------------------------
// 安装 / 卸载
// ---------------------------------------------------------------------------

pub fn install(target: Target) -> anyhow::Result<InstallReceipt> {
    ensure!(target != Target::Codex, "{CODEX_NOT_INSTALLABLE}");
    ensure!(platform_supported(), "{UNSUPPORTED_PLATFORM}");
    let runtime =
        locate_runtime()?.context("未找到 Codex 运行时（ChatGPT.app），无法接入 Computer Use")?;
    if let Some(message) = target.tool_home_missing() {
        bail!("{message}");
    }
    install_at(&runtime, target, &config_paths(target))
}

/// 把条目写进给定的配置文件。内置目标与自定义目标共用。
fn install_at(
    runtime: &RuntimeLocation,
    target: Target,
    config_paths: &[PathBuf],
) -> anyhow::Result<InstallReceipt> {
    with_lock(|| match target {
        Target::ClaudeCode | Target::ClaudeDesktop | Target::Zcode | Target::WorkBuddy => {
            let key_path = target.json_key_path().unwrap_or(&["mcpServers"]);
            let mut changed = false;
            let mut backup: Option<PathBuf> = None;
            for path in config_paths {
                let receipt = install_json_entry(path, key_path, runtime, target)?;
                changed |= receipt.changed;
                if backup.is_none() {
                    backup = receipt.backup_path.map(PathBuf::from);
                }
            }
            receipt(target, changed, backup)
        }
        Target::Codex => bail!("{CODEX_NOT_INSTALLABLE}"),
        Target::Dsh => {
            let path = &config_paths[0];
            let before = read_text(path)?;
            match dsh::install(before.as_deref(), runtime, path)? {
                None => receipt(target, false, None),
                Some(rendered) => {
                    let backup = write_with_backup(path, before.as_deref(), &rendered)?;
                    receipt(target, true, backup)
                }
            }
        }
    })
}

pub fn uninstall(target: Target) -> anyhow::Result<InstallReceipt> {
    uninstall_at(target, &config_paths(target))
}

/// 从给定的配置文件里撤掉本功能写入的条目。内置目标与自定义目标共用。
fn uninstall_at(target: Target, config_paths: &[PathBuf]) -> anyhow::Result<InstallReceipt> {
    with_lock(|| match target {
        Target::ClaudeCode | Target::ClaudeDesktop | Target::Zcode | Target::WorkBuddy => {
            let key_path = target.json_key_path().unwrap_or(&["mcpServers"]);
            let mut changed = false;
            let mut backup: Option<PathBuf> = None;
            for path in config_paths {
                let receipt = uninstall_json_entry(path, key_path, target)?;
                changed |= receipt.changed;
                if backup.is_none() {
                    backup = receipt.backup_path.map(PathBuf::from);
                }
            }
            receipt(target, changed, backup)
        }
        Target::Codex => uninstall_codex_entry(&config_paths[0], target),
        Target::Dsh => {
            let path = &config_paths[0];
            let Some(before) = read_text(path)? else {
                return receipt(target, false, None);
            };
            match dsh::uninstall(&before) {
                None => receipt(target, false, None),
                Some(rendered) => {
                    let backup = write_with_backup(path, Some(&before), &rendered)?;
                    receipt(target, true, backup)
                }
            }
        }
    })
}

/// 按键路径取（必要时创建）server 表。中间层不是对象时拒绝修改。
fn servers_at<'a>(
    root: &'a mut Map<String, Value>,
    key_path: &[&str],
    path: &Path,
) -> anyhow::Result<&'a mut Map<String, Value>> {
    let mut node = root;
    for key in key_path {
        node = node
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .with_context(|| {
                format!("{} 的 {} 不是对象，未修改", path.display(), key_path.join("."))
            })?;
    }
    Ok(node)
}

fn install_json_entry(
    path: &Path,
    key_path: &[&str],
    runtime: &RuntimeLocation,
    target: Target,
) -> anyhow::Result<InstallReceipt> {
    let before = read_text(path)?;
    let mut root = parse_json_object(before.as_deref().unwrap_or("{}"), path)?;
    let servers = servers_at(&mut root, key_path, path)?;

    // CU 与浏览器是两个独立的 server，各自判定、各自写；只改自己那条。
    let mut changed = false;
    for (id, expected) in runtime.managed_entries() {
        if let Some(existing) = servers.get(id) {
            ensure!(
                looks_like_ours(existing, runtime),
                "{} 里已有同名 {} 且不是本功能写入的，未覆盖",
                path.display(),
                id
            );
            if entry_matches(existing, &expected) {
                continue;
            }
        }
        servers.insert(id.to_string(), expected);
        changed = true;
    }
    if !changed {
        return receipt(target, false, None);
    }
    let rendered = format!("{}\n", serde_json::to_string_pretty(&root)?);
    let backup = write_with_backup(path, before.as_deref(), &rendered)?;
    receipt(target, true, backup)
}

fn uninstall_json_entry(
    path: &Path,
    key_path: &[&str],
    target: Target,
) -> anyhow::Result<InstallReceipt> {
    let Some(before) = read_text(path)? else {
        return receipt(target, false, None);
    };
    let mut root = parse_json_object(&before, path)?;
    let mut removed = false;
    let mut node = Some(&mut root);
    for key in key_path {
        node = node.and_then(|map| map.get_mut(*key)).and_then(Value::as_object_mut);
    }
    if let Some(servers) = node {
        for id in [SERVER_ID, BROWSER_SERVER_ID] {
            removed |= servers.remove(id).is_some();
        }
    }
    if !removed {
        return receipt(target, false, None);
    }
    let rendered = format!("{}\n", serde_json::to_string_pretty(&root)?);
    let backup = write_with_backup(path, Some(&before), &rendered)?;
    receipt(target, true, backup)
}

fn uninstall_codex_entry(path: &Path, target: Target) -> anyhow::Result<InstallReceipt> {
    use toml_edit::{DocumentMut, Item};
    let Some(before) = read_text(path)? else {
        return receipt(target, false, None);
    };
    let mut document: DocumentMut = before
        .parse()
        .with_context(|| format!("{} 不是合法 TOML，未修改", path.display()))?;
    if let Some(existing) = document
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .and_then(|servers| servers.get(CODEX_SERVER_ID))
    {
        ensure!(
            codex_entry_looks_like_ours(existing),
            "{} 里的 {} 由其他工具写入，未改动",
            path.display(),
            CODEX_SERVER_ID
        );
    }
    let removed = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .and_then(|servers| servers.remove(CODEX_SERVER_ID))
        .is_some();
    if !removed {
        return receipt(target, false, None);
    }
    let rendered = document.to_string();
    let backup = write_with_backup(path, Some(&before), &rendered)?;
    receipt(target, true, backup)
}

fn receipt(
    target: Target,
    changed: bool,
    backup_path: Option<PathBuf>,
) -> anyhow::Result<InstallReceipt> {
    let status = status()?;
    Ok(InstallReceipt {
        target: target.id().into(),
        changed,
        backup_path: backup_path.map(|path| path.to_string_lossy().into_owned()),
        targets: status.targets,
        custom_targets: status.custom_targets,
    })
}

// ---------------------------------------------------------------------------
// 健康检查（真跑一次 MCP 握手 + sky.list_apps()）
// ---------------------------------------------------------------------------

pub fn check() -> HealthReport {
    let started = Instant::now();
    if !platform_supported() {
        return unsupported_platform_report(elapsed_ms(started));
    }
    let runtime = match locate_runtime() {
        Ok(Some(runtime)) => runtime,
        Ok(None) => {
            return HealthReport {
                ok: false,
                target: "unknown".into(),
                app_count: None,
                browser_count: None,
                sample_apps: vec![],
                duration_ms: elapsed_ms(started),
                failure_kind: Some("runtime_missing".into()),
                failure_detail: Some("未找到 ChatGPT.app 及其 cua_node 运行时".into()),
            };
        }
        Err(error) => {
            return HealthReport {
                ok: false,
                target: "unknown".into(),
                app_count: None,
                browser_count: None,
                sample_apps: vec![],
                duration_ms: elapsed_ms(started),
                failure_kind: Some("error".into()),
                failure_detail: Some(error.to_string()),
            };
        }
    };
    let outcome = run_probe(&runtime);
    HealthReport {
        duration_ms: elapsed_ms(started),
        ..outcome
    }
}

/// 非 macOS 的自检结果：直接报「不支持」，不去找运行时，也不报成「未找到」。
fn unsupported_platform_report(duration_ms: u64) -> HealthReport {
    HealthReport {
        ok: false,
        target: "unknown".into(),
        app_count: None,
        browser_count: None,
        sample_apps: vec![],
        duration_ms,
        failure_kind: Some("unsupported_platform".into()),
        failure_detail: Some(UNSUPPORTED_PLATFORM.into()),
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn run_probe(_runtime: &RuntimeLocation) -> HealthReport {
    // 自检走与客户端完全相同的入口：本二进制 + `--codex-plus-cu-mcp` 代理，
    // 并且**刻意不声明 elicitation**——这样代理一旦退化，自检立刻失败，
    // 而不是像直连那样要等到真正接管窗口时才炸。
    let command_path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            return probe_failure("runtime_missing", &format!("无法定位自身可执行文件：{error}"));
        }
    };
    let mut command = Command::new(command_path);
    command
        .arg("--codex-plus-cu-mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    // 独立进程组：cua-repl 会再拉起 node_repl，只杀父进程会留下孙进程占着管道。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return probe_failure("runtime_missing", &format!("无法启动 node：{error}")),
    };
    let mut stdin = child.stdin.take();
    let (sender, receiver) = mpsc::channel::<String>();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_thread = stdout.map(|stream| {
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        })
    });
    let stderr_buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(stream) = stderr {
        let buffer = std::sync::Arc::clone(&stderr_buffer);
        std::thread::spawn(move || {
            for line in BufReader::new(stream).lines().take(64) {
                let Ok(line) = line else { break };
                if let Ok(mut guard) = buffer.lock() {
                    guard.push_str(&line);
                    guard.push('\n');
                }
            }
        });
    }

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut handshake_done = false;
    let mut request_sent = false;
    // JS 档（`computerUseMode = js`）的工具面是 `cua_repl` 的 `js`，按工具面自动判断。
    let mut js_mode = false;
    let mut failure: Option<HealthReport> = None;

    if let Err(error) = send_json(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": crate::computer_use_approval::SELF_CHECK_CLIENT,
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
    ) {
        failure = Some(probe_failure("service_unavailable", &error.to_string()));
    }

    while failure.is_none() {
        let now = Instant::now();
        if now >= deadline {
            failure = Some(probe_failure(
                "service_unavailable",
                "自检超时：45 秒内没有拿到 Computer Use 结果",
            ));
            break;
        }
        let wait = deadline - now;
        let Ok(line) = receiver.recv_timeout(wait.min(Duration::from_secs(5))) else {
            continue;
        };
        if line.len() > MAX_PROBE_FRAME {
            failure = Some(probe_failure("error", "自检响应过大，已中止"));
            break;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        // 服务端请求：elicitation 是审批表单，自检里直接接受（只做只读列举）。
        if let Some(method) = message.get("method").and_then(Value::as_str) {
            let id = message.get("id").cloned();
            if let (Some(id), true) = (id, method == "elicitation/create") {
                let _ = send_json(
                    &mut stdin,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": { "action": "accept", "content": {} } }),
                );
            }
            continue;
        }
        let Some(id) = message.get("id").and_then(Value::as_i64) else {
            continue;
        };
        if id == 1 {
            handshake_done = true;
            let _ = send_json(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            );
            // 先验工具面：直通路径下这里必须是原生那 10 个离散工具，
            // 而不是 cua-repl 的 `js`。工具面一错，后面全是白搭。
            let _ = send_json(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
            );
            continue;
        }
        if id == 2 {
            let names = tool_names(&message);
            let text = names.join(", ");
            js_mode = names.iter().any(|name| name == "js");
            let tools_ok = if js_mode {
                true
            } else {
                text.contains("get_app_state") && text.contains("click")
            };
            if !tools_ok {
                failure = Some(probe_failure(
                    "service_unavailable",
                    &format!("Computer Use 工具面不符（拿到：{text}）"),
                ));
                continue;
            }
            let call = if js_mode {
                json!({ "name": "js", "arguments": { "code": crate::computer_use_js::PROBE_CODE } })
            } else {
                json!({ "name": "list_apps", "arguments": {} })
            };
            let _ = send_json(
                &mut stdin,
                &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": call }),
            );
            request_sent = true;
            continue;
        }
        if id == 3 {
            let text = message
                .get("result")
                .and_then(|result| result.get("content"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let is_error = message
                .get("result")
                .and_then(|result| result.get("isError"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            // JS 档的首次输出还附带 cua_repl 的使用说明，应用列表在标记之间。
            let apps = if js_mode {
                crate::computer_use_js::probe_apps(&text).map(str::to_string)
            } else {
                Some(text.clone())
            };
            if !is_error && let Some(apps) = apps.filter(|apps| !apps.trim().is_empty()) {
                let (count, samples) = summarize_apps(&apps);
                kill_tree(&mut child);
                return HealthReport {
                    ok: true,
                    target: "mac".into(),
                    app_count: Some(count),
                    // 原生 CU server 没有浏览器面，浏览器能力由独立 server 提供。
                    browser_count: None,
                    sample_apps: samples,
                    duration_ms: 0,
                    failure_kind: None,
                    failure_detail: None,
                };
            }
            failure = Some(probe_failure(
                classify(&text, is_error),
                &trim_detail(&text),
            ));
        }
    }

    let stderr_text = stderr_buffer
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    kill_tree(&mut child);
    // 读取线程在管道关闭后会自行结束；这里不 join，避免孙进程占着管道把自检拖死。
    drop(stdout_thread);
    let mut report = failure.unwrap_or_else(|| {
        probe_failure(
            "service_unavailable",
            if handshake_done && !request_sent {
                "握手完成但没有发出工具调用"
            } else {
                "自检没有拿到结果"
            },
        )
    });
    if report
        .failure_detail
        .as_deref()
        .unwrap_or_default()
        .is_empty()
        && !stderr_text.trim().is_empty()
    {
        report.failure_detail = Some(trim_detail(&stderr_text));
    }
    report
}

fn probe_failure(kind: &str, detail: &str) -> HealthReport {
    HealthReport {
        ok: false,
        target: "mac".into(),
        app_count: None,
        browser_count: None,
        sample_apps: vec![],
        duration_ms: 0,
        failure_kind: Some(kind.to_string()),
        failure_detail: Some(detail.to_string()),
    }
}

/// 从 `tools/list` 的响应里取工具名。
fn tool_names(message: &Value) -> Vec<String> {
    message
        .get("result")
        .and_then(|result| result.get("tools"))
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `list_apps` 的返回是一行一个 app：`名称 — 路径 — bundleID [标记]`。
/// 这里只数行数与取前 8 个显示名，用于自检摘要。
fn summarize_apps(text: &str) -> (usize, Vec<String>) {
    let mut count = 0usize;
    let mut samples = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        count += 1;
        if samples.len() < 8 {
            let name = line.split(" — ").next().unwrap_or(line).trim();
            if !name.is_empty() {
                samples.push(name.to_string());
            }
        }
    }
    (count, samples)
}

fn classify(text: &str, is_error: bool) -> &'static str {
    let lower = text.to_ascii_lowercase();
    // 直通路径特有的失败：父进程不是 OpenAI 签名时服务端直接拒绝。
    if lower.contains("sender process is not authenticated") || lower.contains("-10000") {
        return "untrusted_parent";
    }
    if lower.contains("createelicitation is unavailable")
        || lower.contains("does not support form elicitation")
    {
        return "client_unsupported";
    }
    if lower.contains("accessibility")
        || lower.contains("screen recording")
        || lower.contains("permissions are still pending")
        || lower.contains("privacy_accessibility")
    {
        return "permission_required";
    }
    if lower.contains("native pipe is unavailable")
        || lower.contains("service startup")
        || lower.contains("econnrefused")
        || lower.contains("socket")
    {
        return "service_unavailable";
    }
    if is_error {
        "error"
    } else {
        "service_unavailable"
    }
}

fn trim_detail(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "自检失败，未返回可读信息".into();
    }
    let limit = 400;
    let mut out: String = trimmed.chars().take(limit).collect();
    if trimmed.chars().count() > limit {
        out.push('…');
    }
    out
}

fn send_json(stdin: &mut Option<ChildStdin>, value: &Value) -> anyhow::Result<()> {
    let stdin = stdin.as_mut().context("自检进程的标准输入不可用")?;
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    stdin.write_all(line.as_bytes())?;
    stdin.flush()?;
    Ok(())
}

/// 结束整个进程组：cua-repl 的孙进程（node_repl）不会随父进程退出。
fn kill_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        let _ = Command::new("/bin/kill")
            .arg("-9")
            .arg(format!("-{pid}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

// ---------------------------------------------------------------------------
// 配置读写
// ---------------------------------------------------------------------------

fn with_lock<T>(run: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let path = lock_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("无法创建目录 {}", parent.display()))?;
    }
    let file = open_private(&path)?;
    FileExt::lock_exclusive(&file).context("无法锁定 Computer Use 接入操作")?;
    let result = run();
    let _ = FileExt::unlock(&file);
    result
}

fn open_private(path: &Path) -> anyhow::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("无法打开 {}", path.display()))
}

pub(crate) fn read_text(path: &Path) -> anyhow::Result<Option<String>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!("{} 超过 8 MB，未修改", path.display());
    }
    let text = fs::read_to_string(path).with_context(|| format!("无法读取 {}", path.display()))?;
    Ok(Some(text))
}

pub(crate) fn parse_json_object(text: &str, path: &Path) -> anyhow::Result<Map<String, Value>> {
    // Windows 上的工具写出的 JSON 可能带 UTF-8 BOM（#2728），JSON 解析器不认，先去掉。
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    let value: Value = serde_json::from_str(text)
        .with_context(|| format!("{} 不是合法 JSON，未修改", path.display()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => bail!("{} 的顶层不是对象，未修改", path.display()),
    }
}

pub(crate) fn write_with_backup(
    path: &Path,
    before: Option<&str>,
    contents: &str,
) -> anyhow::Result<Option<PathBuf>> {
    let mut backup_path = None;
    if let Some(previous) = before {
        // 其他程序不一定遵守本锁：读后变化就拒绝提交，不能覆盖用户的新配置。
        let current = read_text(path)?;
        if current.as_deref() != Some(previous) {
            bail!("{} 在写入期间被其他程序修改，未覆盖", path.display());
        }
        backup_path = Some(backup(path, previous)?);
    }
    crate::settings::atomic_write_with(path, |file| file.write_all(contents.as_bytes()))
        .with_context(|| format!("无法保存 {}", path.display()))?;
    Ok(backup_path)
}

fn backup(path: &Path, contents: &str) -> anyhow::Result<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0);
    // 用「原文件名 + 后缀」而不是换扩展名：`.claude.json` 换扩展会变成 `.claude.backup-…`，
    // 用户按文件名找不到备份。
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    let backup_path = path.with_file_name(format!("{file_name}.codex-plus-backup-{stamp}"));
    // 同一秒里连着保存两次：留第一份（改动前的原样）。直接覆盖写又不截断，会留下旧内容的尾巴。
    if backup_path.exists() {
        return Ok(backup_path);
    }
    let mut file = open_private(&backup_path)?;
    file.write_all(contents.as_bytes())?;
    prune_backups(path);
    Ok(backup_path)
}

fn prune_backups(path: &Path) {
    let Some(parent) = path.parent() else { return };
    let Some(stem) = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return;
    };
    let pattern = format!("{stem}.codex-plus-backup-");
    let mut items: Vec<(u64, PathBuf)> = Vec::new();
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(suffix) = name.strip_prefix(&pattern) {
            let stamp = suffix.parse::<u64>().unwrap_or(0);
            items.push((stamp, entry.path()));
        }
    }
    items.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, stale) in items.into_iter().skip(MAX_BACKUPS) {
        let _ = fs::remove_file(stale);
    }
}

fn read_json_entry(
    path: &Path,
    key_path: &[&str],
    server_id: &str,
) -> anyhow::Result<Option<Value>> {
    let Some(text) = read_text(path)? else {
        return Ok(None);
    };
    let root = parse_json_object(&text, path)?;
    let mut node = Some(&root);
    for key in key_path {
        node = node.and_then(|map| map.get(*key)).and_then(Value::as_object);
    }
    Ok(node.and_then(|servers| servers.get(server_id)).cloned())
}

/// 客户端会重写配置（Claude Desktop 会把 `type` 之类规范化掉），
/// 所以「是不是同一条」只比较真正决定行为的 command 与 args。
fn entry_matches(existing: &Value, expected: &Value) -> bool {
    existing.get("command") == expected.get("command")
        && existing.get("args") == expected.get("args")
        && env_of(existing) == env_of(expected)
}

/// 只比较我们会写的那个变量：客户端可能往 `env` 里加别的东西，不能因此判成过期。
fn env_of(entry: &Value) -> Option<&Value> {
    entry
        .get("env")
        .and_then(|env| env.get(crate::paths::STATE_DIR_ENV))
}

fn looks_like_ours(entry: &Value, runtime: &RuntimeLocation) -> bool {
    let command = entry
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let first_arg = entry
        .get("args")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_str)
        .unwrap_or_default();
    // CU 与浏览器是两个条目，都算本功能写入的。
    if first_arg == "--codex-plus-cu-mcp" || first_arg == "--codex-plus-browser-mcp" {
        return command == runtime.self_command() || is_codex_plus_binary(command);
    }
    // 兼容早期直连 cua-repl 的写法：升级后仍能识别成自己的条目并改写。
    command == runtime.node && first_arg == runtime.cua_repl
}

/// 同一份配置可能在 debug / release / 安装位置之间被复用：只认「是不是
/// Codex++ 自己的二进制」，而不是某一条绝对路径，否则换个构建就会被判成他人条目。
/// bundle 里叫 CodexPlusPlus，cargo 产物叫 codex-plus-plus，两种都认。
fn is_codex_plus_binary(command: &str) -> bool {
    std::path::Path::new(command)
        .file_name()
        .map(|name| {
            let lower = name.to_string_lossy().to_lowercase();
            let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
            stem.starts_with("codexplusplus") || stem.starts_with("codex-plus-plus")
        })
        .unwrap_or(false)
}

fn read_codex_entry(path: &Path, server_id: &str) -> anyhow::Result<Option<toml_edit::Item>> {
    let Some(text) = read_text(path)? else {
        return Ok(None);
    };
    let document: toml_edit::DocumentMut = text
        .parse()
        .with_context(|| format!("{} 不是合法 TOML", path.display()))?;
    Ok(document
        .get("mcp_servers")
        .and_then(toml_edit::Item::as_table_like)
        .and_then(|servers| servers.get(server_id))
        .cloned())
}

/// Codex 配置里的 `codex_plus_cu` 是不是 Codex++ 写入的残留：早期直连 cua-repl 的写法
/// （`node …/cua-repl.mjs`），或指向 Codex++ 自己的代理。
///
/// 不依赖 `RuntimeLocation`：ChatGPT.app 升级或卸载后路径都会变，残留仍要能认出来并移除。
fn codex_entry_looks_like_ours(entry: &toml_edit::Item) -> bool {
    let Some(table) = entry.as_table_like() else {
        return false;
    };
    let command = table
        .get("command")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default();
    let first_arg = table
        .get("args")
        .and_then(toml_edit::Item::as_array)
        .and_then(|args| args.get(0))
        .and_then(toml_edit::Value::as_str)
        .unwrap_or_default();
    if first_arg == "--codex-plus-cu-mcp" {
        return is_codex_plus_binary(command)
            || std::env::current_exe().is_ok_and(|exe| exe.to_string_lossy() == command);
    }
    let node = std::path::Path::new(command)
        .file_name()
        .is_some_and(|name| name == "node" || name == "node.exe");
    node && first_arg.replace('\\', "/").contains("cua-repl/bin/cua-repl.mjs")
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn non_macos_reports_unsupported_instead_of_missing_runtime() {
        assert_eq!(platform_supported(), cfg!(target_os = "macos"));
        let report = unsupported_platform_report(0);
        assert!(!report.ok);
        assert_eq!(report.failure_kind.as_deref(), Some("unsupported_platform"));
        assert!(UNSUPPORTED_PLATFORM.contains("暂不支持 Windows"));
        if !platform_supported() {
            // 非 macOS：接入直接给出「不支持」，不再落到「未找到 ChatGPT.app」。
            let error = install(Target::ClaudeCode).unwrap_err().to_string();
            assert_eq!(error, UNSUPPORTED_PLATFORM);
            assert_eq!(check().failure_kind.as_deref(), Some("unsupported_platform"));
        }
    }

    fn sample_runtime(root: &Path) -> RuntimeLocation {
        RuntimeLocation {
            app_path: "/Applications/ChatGPT.app".into(),
            node: root.join("node").to_string_lossy().into_owned(),
            client: root.join("SkyComputerUseClient").to_string_lossy().into_owned(),
            cua_repl: root.join("cua-repl.mjs").to_string_lossy().into_owned(),
            node_repl: root.join("node_repl").to_string_lossy().into_owned(),
            node_module_dirs: root.join("node_modules").to_string_lossy().into_owned(),
            codex_home: root.join("codex").to_string_lossy().into_owned(),
            service_path: None,
            codex_cli: None,
            app_version: Some("26.1016.61520".into()),
            package_version: Some("0.1.2".into()),
        }
    }

    #[test]
    fn json_object_with_utf8_bom_parses() {
        let map = parse_json_object("\u{feff}{\"mcpServers\":{}}", Path::new("claude_desktop_config.json")).unwrap();
        assert!(map.contains_key("mcpServers"));
    }

    #[test]
    fn proxy_vars_cover_both_cases_and_bypass_loopback() {
        let env = proxy_vars("http://127.0.0.1:7897");
        for key in ["http_proxy", "HTTP_PROXY", "https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"] {
            assert_eq!(env[key], "http://127.0.0.1:7897");
        }
        assert_eq!(env["no_proxy"], "localhost,127.0.0.1,::1");
        assert!(env.keys().all(|key| PROXY_ENV_KEYS.contains(&key.as_str())));
    }

    #[test]
    fn js_env_enables_browser_and_computer_like_codex() {
        let temp = tempfile::tempdir().unwrap();
        let env = sample_runtime(temp.path()).js_env();
        assert_eq!(env["CUA_REPL_ENABLED_SURFACES"], "browser,computer");
        assert_eq!(env["BROWSER_USE_AVAILABLE_BACKENDS"], "chrome");
        assert_eq!(env["BROWSER_USE_TINYSKY_ENABLED"], "1");
        assert!(env["NODE_REPL_INSTRUCTIONS_USE_CASE_CHROME"].contains("Prefer this method"));
        assert!(env.contains_key("NODE_REPL_INSTRUCTIONS_USE_CASE_COMPUTER_USE"));
    }

    #[test]
    fn native_client_env_only_carries_whitelisted_variables() {
        let temp = tempfile::tempdir().unwrap();
        let mut runtime = sample_runtime(temp.path());
        runtime.codex_cli = Some("/Applications/ChatGPT.app/codex".into());
        runtime.service_path = Some("/x/Codex Computer Use.app".into());
        let env = runtime.native_client_env();

        assert_eq!(env["CODEX_CLI_PATH"], "/Applications/ChatGPT.app/codex");
        assert_eq!(env["CODEX_HOME"], runtime.codex_home);
        // 实测没用，不再传。
        assert!(!env.contains_key("SKY_CUA_SERVICE_PATH"));
        // 测试进程自己的环境（cargo 的 CARGO_* 等）不能漏进去。
        assert!(std::env::vars().any(|(key, _)| key.starts_with("CARGO_")));
        for key in env.keys() {
            assert!(
                NATIVE_ENV_PASSTHROUGH.contains(&key.as_str())
                    || CUSTOM_CA_ENV_KEYS.contains(&key.as_str())
                    || PROXY_ENV_KEYS.contains(&key.as_str())
                    || key == "CODEX_HOME"
                    || key == "CODEX_CLI_PATH",
                "unexpected env var {key}"
            );
        }
        if std::env::var_os("PATH").is_some() {
            assert!(env.contains_key("PATH"));
        }
    }

    #[test]
    fn state_dir_override_travels_with_the_entry() {
        let base = json!({ "command": "/x/CodexPlusPlus", "args": ["--codex-plus-cu-mcp"] });
        // 没有覆盖：条目不带 env（正式安装的形状）。
        assert!(entry_with_state_dir(base.clone(), None).get("env").is_none());

        let with_dir = entry_with_state_dir(base.clone(), Some(PathBuf::from("/dev/state")));
        assert_eq!(with_dir["env"][crate::paths::STATE_DIR_ENV], "/dev/state");

        // 状态目录变了就要重写；客户端往 env 里塞的其它变量不影响判定。
        assert!(!entry_matches(&base, &with_dir));
        let mut decorated = with_dir.clone();
        decorated["env"]["EXTRA"] = json!("1");
        assert!(entry_matches(&decorated, &with_dir));
    }

    #[test]
    fn server_entry_is_stable() {
        let temp = TempDir::new().unwrap();
        let runtime = sample_runtime(temp.path());
        let left = runtime.server_entry();
        let right = runtime.server_entry();
        assert_eq!(left, right);
        assert_eq!(left["args"][0], "--codex-plus-cu-mcp");
        assert_eq!(left["command"], runtime.self_command());
        assert!(left.get("env").is_none());
    }

    #[test]
    fn install_and_uninstall_claude_json() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".claude.json");
        fs::write(&path, "{\n  \"numStartups\": 3,\n  \"mcpServers\": {}\n}\n").unwrap();
        let runtime = sample_runtime(temp.path());
        let first = install_json_entry(&path, &["mcpServers"], &runtime, Target::ClaudeCode).unwrap();
        assert!(first.changed);
        let text = fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["numStartups"], 3);
        assert_eq!(
            value["mcpServers"][SERVER_ID]["command"],
            runtime.self_command()
        );

        let second = install_json_entry(&path, &["mcpServers"], &runtime, Target::ClaudeCode).unwrap();
        assert!(!second.changed, "重复安装不应该重写文件");

        let removed = uninstall_json_entry(&path, &["mcpServers"], Target::ClaudeCode).unwrap();
        assert!(removed.changed);
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["mcpServers"].get(SERVER_ID).is_none());
        assert_eq!(value["numStartups"], 3);
    }

    #[test]
    fn zcode_servers_live_under_nested_mcp_servers() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.json");
        // ZCode 自己的 config.json 已有 plugins，不能被冲掉。
        fs::write(&path, "{\n  \"plugins\": {\"a\": true}\n}\n").unwrap();
        let runtime = sample_runtime(temp.path());
        let key_path = Target::Zcode.json_key_path().unwrap();
        assert!(install_json_entry(&path, key_path, &runtime, Target::Zcode).unwrap().changed);
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["plugins"]["a"], true);
        assert_eq!(value["mcp"]["servers"][SERVER_ID]["args"][0], "--codex-plus-cu-mcp");
        assert_eq!(
            value["mcp"]["servers"][BROWSER_SERVER_ID]["args"][0],
            "--codex-plus-browser-mcp"
        );
        assert!(value.get("mcpServers").is_none(), "不能写到 Claude 的键上");
        assert!(read_json_entry(&path, key_path, SERVER_ID).unwrap().is_some());

        assert!(uninstall_json_entry(&path, key_path, Target::Zcode).unwrap().changed);
        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(value["mcp"]["servers"].get(SERVER_ID).is_none());
        assert_eq!(value["plugins"]["a"], true);
    }

    #[test]
    fn non_object_intermediate_key_is_left_alone() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.json");
        fs::write(&path, "{\"mcp\": \"disabled\"}").unwrap();
        let runtime = sample_runtime(temp.path());
        assert!(install_json_entry(&path, &["mcp", "servers"], &runtime, Target::Zcode).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"mcp\": \"disabled\"}");
    }

    #[test]
    fn target_ids_round_trip() {
        for target in Target::ALL {
            assert_eq!(Target::parse(target.id()).unwrap(), target);
        }
        assert!(Target::parse("nope").is_err());
    }

    #[test]
    fn foreign_entry_is_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".claude.json");
        fs::write(
            &path,
            format!("{{\"mcpServers\":{{\"{SERVER_ID}\":{{\"command\":\"/other/node\",\"args\":[\"x\"]}}}}}}"),
        )
        .unwrap();
        let runtime = sample_runtime(temp.path());
        let error = install_json_entry(&path, &["mcpServers"], &runtime, Target::ClaudeCode).unwrap_err();
        assert!(error.to_string().contains("同名"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("/other/node"));
    }

    #[test]
    fn invalid_json_is_left_alone() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".claude.json");
        fs::write(&path, "{not json").unwrap();
        let runtime = sample_runtime(temp.path());
        assert!(install_json_entry(&path, &["mcpServers"], &runtime, Target::ClaudeCode).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{not json");
    }

    #[test]
    fn codex_target_is_never_installed() {
        let error = install(Target::Codex).unwrap_err();
        assert!(error.to_string().contains("Codex 自带 Computer Use"));
    }

    #[test]
    fn codex_leftovers_are_recognized_and_removed() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        // 两种残留：早期直连 cua-repl，以及指向 Codex++ 代理的写法。
        for (command, arg) in [
            (
                "/Applications/ChatGPT.app/Contents/Resources/cua_node/bin/node",
                "/Applications/ChatGPT.app/Contents/Resources/cua_node/lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs",
            ),
            ("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus", "--codex-plus-cu-mcp"),
        ] {
            fs::write(
                &path,
                format!(
                    "model = \"gpt-6\"\n\n[mcp_servers.other]\ncommand = \"keep\"\n\n[mcp_servers.codex_plus_cu]\ncommand = \"{command}\"\nargs = [\"{arg}\"]\nenabled = true\n"
                ),
            )
            .unwrap();
            let entry = read_codex_entry(&path, CODEX_SERVER_ID).unwrap().unwrap();
            assert!(codex_entry_looks_like_ours(&entry), "{command}");

            let removed = uninstall_codex_entry(&path, Target::Codex).unwrap();
            assert!(removed.changed);
            let text = fs::read_to_string(&path).unwrap();
            assert!(!text.contains("codex_plus_cu"));
            assert!(text.contains("model = \"gpt-6\""));
            assert!(text.contains("[mcp_servers.other]"));
        }
    }

    #[test]
    fn codex_foreign_entry_is_left_alone() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        let before = "[mcp_servers.codex_plus_cu]\ncommand = \"/usr/local/bin/someone-else\"\n";
        fs::write(&path, before).unwrap();
        let entry = read_codex_entry(&path, CODEX_SERVER_ID).unwrap().unwrap();
        assert!(!codex_entry_looks_like_ours(&entry));
        assert!(uninstall_codex_entry(&path, Target::Codex).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn summarize_apps_counts_lines_and_takes_names() {
        let text = concat!(
            "Finder — /System/Library/CoreServices/Finder.app/ — com.apple.finder [running, uses=12]\n",
            "\n",
            "Google Chrome — /Applications/Google Chrome.app/ — com.google.Chrome [frontmost, running, uses=8814]\n",
        );
        let (count, samples) = summarize_apps(text);
        assert_eq!(count, 2, "空行不计数");
        assert_eq!(samples, vec!["Finder", "Google Chrome"]);
    }

    #[test]
    fn tool_names_reads_tools_list() {
        let message = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": { "tools": [
                { "name": "list_apps" },
                { "name": "get_app_state" },
            ] }
        });
        assert_eq!(tool_names(&message), vec!["list_apps", "get_app_state"]);
    }

    #[test]
    fn classify_reports_untrusted_parent() {
        assert_eq!(
            classify(
                "Computer Use server error -10000: Sender process is not authenticated",
                true
            ),
            "untrusted_parent"
        );
    }

    #[test]
    fn classify_permission_and_elicitation() {
        assert_eq!(
            classify("Computer Use permissions are still pending", true),
            "permission_required"
        );
        assert_eq!(
            classify(
                "nodeRepl.createElicitation is unavailable because the MCP client",
                true
            ),
            "client_unsupported"
        );
        assert_eq!(
            classify("Sky Computer Use native pipe is unavailable: refused", true),
            "service_unavailable"
        );
    }

    #[test]
    fn custom_path_appends_file_name_for_directories() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let dir = home.join("cfg");
        fs::create_dir_all(&dir).unwrap();

        // 已存在的目录：拼上该类型的配置文件名。
        assert_eq!(
            resolve_custom_path(Target::ClaudeCode, dir.to_str().unwrap(), home),
            dir.join(".claude.json")
        );
        assert_eq!(
            resolve_custom_path(Target::ClaudeDesktop, dir.to_str().unwrap(), home),
            dir.join("claude_desktop_config.json")
        );
        // `~` 展开，首尾空白忽略。
        assert_eq!(
            resolve_custom_path(Target::WorkBuddy, "  ~/cfg ", home),
            dir.join("mcp.json")
        );
        // 还不存在、但带配置扩展名的按文件原样使用。
        let file = dir.join("custom.json");
        assert_eq!(resolve_custom_path(Target::Zcode, file.to_str().unwrap(), home), file);
        // 还不存在、也没有扩展名的当目录。
        let later = home.join("later");
        assert_eq!(
            resolve_custom_path(Target::Dsh, later.to_str().unwrap(), home),
            later.join("cordis.patch.yml")
        );
    }

    #[test]
    fn add_custom_target_validates_and_persists() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let store = home.join("state").join(CUSTOM_STORE_FILE);
        let dir = home.join(".rec/config");
        fs::create_dir_all(&dir).unwrap();

        let first = add_custom_in(&store, Target::ClaudeCode, "~/.rec/config", "", home).unwrap();
        assert!(first.id.starts_with(CUSTOM_ID_PREFIX));
        assert_eq!(first.path, "~/.rec/config");
        assert_eq!(load_custom_from(&store).unwrap(), vec![first.clone()]);

        // 同类型同文件不能重复添加；同目录换个类型可以。
        let duplicate = add_custom_in(&store, Target::ClaudeCode, dir.to_str().unwrap(), "", home);
        assert!(duplicate.unwrap_err().to_string().contains("已经添加过"));
        let second = add_custom_in(&store, Target::ClaudeDesktop, "~/.rec/config", "", home).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(load_custom_from(&store).unwrap().len(), 2);

        // 目录不存在、相对路径、空路径、不支持的类型都拒绝，且不改存储。
        assert!(add_custom_in(&store, Target::ClaudeCode, "~/missing/dir", "", home)
            .unwrap_err()
            .to_string()
            .contains("目录不存在"));
        assert!(add_custom_in(&store, Target::ClaudeCode, "relative/dir", "", home)
            .unwrap_err()
            .to_string()
            .contains("绝对路径"));
        assert!(add_custom_in(&store, Target::ClaudeCode, "   ", "", home).is_err());
        assert!(add_custom_in(&store, Target::Codex, "~/.rec/config", "", home).is_err());
        assert_eq!(load_custom_from(&store).unwrap().len(), 2);
    }

    #[test]
    fn custom_target_names_are_optional_validated_and_renamable() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let store = home.join("state").join(CUSTOM_STORE_FILE);
        fs::create_dir_all(home.join(".rec/a")).unwrap();
        fs::create_dir_all(home.join(".rec/b")).unwrap();

        // 起了名就用名字显示，首尾空白去掉；不起名显示类型名，存储里也不写 name 字段。
        let named = add_custom_in(&store, Target::ClaudeCode, "~/.rec/a", "  工作账号  ", home).unwrap();
        assert_eq!(named.name, "工作账号");
        assert_eq!(named.display_name(), "工作账号");
        let plain = add_custom_in(&store, Target::ClaudeCode, "~/.rec/b", "", home).unwrap();
        assert_eq!(plain.display_name(), "Claude Code");
        let text = fs::read_to_string(&store).unwrap();
        assert_eq!(text.matches("\"name\"").count(), 1);

        // 控制字符、超长名字拒绝，且不改存储。
        assert!(add_custom_in(&store, Target::ClaudeCode, "~/.rec/b", "a\nb", home).is_err());
        let long = "名".repeat(CUSTOM_NAME_MAX_CHARS + 1);
        assert!(rename_custom_in(&store, &named.id, &long).unwrap_err().to_string().contains("最多"));
        assert_eq!(load_custom_from(&store).unwrap().len(), 2);

        // 改名只动名字；改成空字符串恢复默认显示；不存在的 id 报错。
        let renamed = rename_custom_in(&store, &plain.id, "个人账号").unwrap();
        assert_eq!(renamed.path, "~/.rec/b");
        assert_eq!(load_custom_from(&store).unwrap()[1].name, "个人账号");
        rename_custom_in(&store, &named.id, "   ").unwrap();
        assert_eq!(load_custom_from(&store).unwrap()[0].display_name(), "Claude Code");
        assert!(rename_custom_in(&store, "custom-missing", "x").is_err());
    }

    #[test]
    fn custom_store_without_names_still_loads() {
        // 旧版写的存储没有 name 字段。
        let temp = TempDir::new().unwrap();
        let store = temp.path().join(CUSTOM_STORE_FILE);
        fs::write(&store, r#"{"targets":[{"id":"custom-1","kind":"claudeCode","path":"/x"}]}"#).unwrap();
        let targets = load_custom_from(&store).unwrap();
        assert_eq!(targets[0].name, "");
        assert_eq!(targets[0].display_name(), "Claude Code");
    }

    #[test]
    fn corrupt_custom_store_is_reported_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let store = temp.path().join(CUSTOM_STORE_FILE);
        fs::write(&store, "{ not json").unwrap();
        assert!(load_custom_from(&store).is_err());
        let dir = temp.path().join("cfg");
        fs::create_dir_all(&dir).unwrap();
        assert!(add_custom_in(&store, Target::ClaudeCode, dir.to_str().unwrap(), "", temp.path()).is_err());
        assert_eq!(fs::read_to_string(&store).unwrap(), "{ not json");
    }

    #[test]
    fn custom_status_follows_the_custom_file() {
        let temp = TempDir::new().unwrap();
        let runtime = sample_runtime(temp.path());
        let dir = temp.path().join("rec-config");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".claude.json");

        let before = status_at(Target::ClaudeCode, &[file.clone()], Some(&runtime), None);
        assert_eq!(before.state, "missing");
        assert_eq!(before.config_path, file.to_string_lossy());

        install_json_entry(&file, &["mcpServers"], &runtime, Target::ClaudeCode).unwrap();
        let after = status_at(Target::ClaudeCode, &[file.clone()], Some(&runtime), None);
        assert_eq!(after.state, "registered");

        // 目录缺失时直接判为不支持，不去读文件。
        let gone = status_at(
            Target::ClaudeCode,
            &[file],
            Some(&runtime),
            Some("目录不存在：x".into()),
        );
        assert_eq!(gone.state, "unsupported");
    }

    #[test]
    fn custom_claude_code_hooks_live_next_to_the_config() {
        let target = CustomTarget {
            id: "custom-1".into(),
            kind: Target::ClaudeCode,
            path: "/tmp/rec/config/.claude.json".into(),
            name: String::new(),
        };
        assert_eq!(
            target.claude_settings_path(),
            Some(PathBuf::from("/tmp/rec/config/settings.json"))
        );
        let desktop = CustomTarget { kind: Target::ClaudeDesktop, ..target };
        assert_eq!(desktop.claude_settings_path(), None);
    }
}
