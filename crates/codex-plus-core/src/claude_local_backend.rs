//! 原版 Claude Desktop 的本地兼容服务：只管理自己的进程、资源和独立用户目录。

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::gateway::agents::connect::wiring;
use crate::gateway::store::{AgentBinding, GatewaySettings, GatewayStore, new_client_token};
use crate::settings::{ClaudeLocalSettings, SettingsStore};
use crate::tools::ToolId;

pub const AGENT_ID: &str = "claude-desktop-local";
const HEALTH_PATH: &str = "/__codex_claude_local/health";
const CA_NAME: &str = "Codex++ Claude Local CA";
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const START_TIMEOUT: Duration = Duration::from_secs(12);
static OPERATION: Mutex<()> = Mutex::new(());

const ASSETS: &[(&str, &[u8])] = &[
    (
        "proxy.mjs",
        include_bytes!("../../../tools/claude-local/proxy.mjs"),
    ),
    (
        "page-shim.js",
        include_bytes!("../../../assets/claude-localization/page-shim.js"),
    ),
    (
        "runtime.mjs",
        include_bytes!("../../../tools/claude-local/runtime.mjs"),
    ),
    (
        "official-assets.mjs",
        include_bytes!("../../../tools/claude-local/official-assets.mjs"),
    ),
    (
        "conversation-service.mjs",
        include_bytes!("../../../tools/claude-local/conversation-service.mjs"),
    ),
    (
        "rpc.mjs",
        include_bytes!("../../../tools/claude-local/rpc.mjs"),
    ),
    (
        "demo/index.html",
        include_bytes!("../../../tools/claude-local/demo/index.html"),
    ),
    (
        "scripts/patch-dictation-start.mjs",
        include_bytes!("../../../tools/claude-local/scripts/patch-dictation-start.mjs"),
    ),
    (
        "scripts/patch-dictation-drain.mjs",
        include_bytes!("../../../tools/claude-local/scripts/patch-dictation-drain.mjs"),
    ),
    (
        "catalogs/zh-Hans.json",
        include_bytes!("../../../assets/claude-localization/catalogs/zh-Hans.json"),
    ),
    (
        "catalogs/dynamic/zh-Hans.json",
        include_bytes!("../../../assets/claude-localization/catalogs/dynamic/zh-Hans.json"),
    ),
    (
        "catalogs/zh-Hans.overrides.json",
        include_bytes!("../../../assets/claude-localization/catalogs/zh-Hans.overrides.json"),
    ),
    // 由 tools/claude-local/sanitize-mock.mjs 从本机录制数据生成的脱敏模板，
    // 不含任何个人字段，供完整功能模式作为 PoC bootstrap/api 数据。
    (
        "mock/bootstrap.json",
        include_bytes!("../../../tools/claude-local/mock-template/bootstrap.json"),
    ),
    (
        "mock/api/account_profile.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/account_profile.json"),
    ),
    (
        "mock/api/claude_code_user_settings.json",
        include_bytes!(
            "../../../tools/claude-local/mock-template/api/claude_code_user_settings.json"
        ),
    ),
    (
        "mock/api/conversations.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/conversations.json"),
    ),
    (
        "mock/api/experiences.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/experiences.json"),
    ),
    (
        "mock/api/mcp_servers.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/mcp_servers.json"),
    ),
    (
        "mock/api/memory_settings.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/memory_settings.json"),
    ),
    (
        "mock/api/reflections_settings.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/reflections_settings.json"),
    ),
    (
        "mock/api/skills.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/skills.json"),
    ),
    (
        "mock/api/trial_status.json",
        include_bytes!("../../../tools/claude-local/mock-template/api/trial_status.json"),
    ),
];

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalStatus {
    pub supported: bool,
    pub source_app: String,
    pub version: String,
    pub claude_running: bool,
    pub running: bool,
    /// Claude 还挂在本机代理上（哪怕代理进程已经退了）：这种「半开」状态要能按停止收尾。
    pub local_process: bool,
    pub connected: bool,
    pub port: Option<u16>,
    pub ca_ready: bool,
    pub ca_trusted: bool,
    pub node_available: bool,
    pub dictation_ready: bool,
    pub config: ClaudeLocalSettings,
    pub models: Vec<String>,
    /// 当前默认模型实际下发的窗口（供应商声明值）；`None` 表示未声明，Claude 按 200k 处理。
    pub context_window: Option<u64>,
    /// 因为窗口比默认模型小、没有列给 Claude 的模型。
    pub hidden_models: Vec<String>,
    pub warning: Option<String>,
    pub needs_restart: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalResult {
    pub status: ClaudeLocalStatus,
    pub message: String,
}

/// 直连上游：不经 Codex++ 网关，本地兼容服务把请求直接发到这组 Base URL + Key
/// （`{base}/v1/messages`，与 `ANTHROPIC_BASE_URL` 同一语义）。
///
/// 只有 ClaudeMom 会设置；没设置时一切照旧走网关绑定。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectUpstream {
    pub base_url: String,
    pub api_key: String,
    /// 默认模型；Claude 里选 Opus / Sonnet 这类自带条目时也落到它。
    pub model: String,
    /// Haiku 档（标题生成等小任务）用的模型；空则用默认模型。
    pub small_model: String,
    /// 列给 Claude 模型选择器的模型（默认模型总在第一个）。
    pub models: Vec<String>,
    /// 上下文窗口；`None` 表示不声明，Claude 按 200k 处理。
    pub context_window: Option<u64>,
    /// 模型显示名（上游 `/v1/models` 的 `display_name`）；没有的按 id 生成。
    pub model_names: BTreeMap<String, String>,
}

impl DirectUpstream {
    /// 去掉首尾空白与结尾斜杠，默认模型放第一位并去重，小模型缺省用默认模型。
    pub fn normalized(mut self) -> Self {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.api_key = self.api_key.trim().to_string();
        self.model = self.model.trim().to_string();
        self.small_model = self.small_model.trim().to_string();
        if self.small_model.is_empty() {
            self.small_model = self.model.clone();
        }
        let mut models = Vec::new();
        for model in std::iter::once(&self.model).chain(self.models.iter()) {
            let model = model.trim();
            if !model.is_empty() && !models.iter().any(|known: &String| known == model) {
                models.push(model.to_string());
            }
        }
        self.models = models;
        self.context_window = self.context_window.filter(|window| *window > 0);
        self
    }

    fn ready(&self) -> bool {
        !self.base_url.is_empty() && !self.model.is_empty()
    }

    fn model_windows(&self) -> BTreeMap<String, u64> {
        self.context_window
            .map(|window| self.models.iter().map(|model| (model.clone(), window)).collect())
            .unwrap_or_default()
    }

    fn model_names(&self) -> BTreeMap<String, Value> {
        self.models
            .iter()
            .map(|id| {
                let name = self
                    .model_names
                    .get(id)
                    .map(|name| name.trim())
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| friendly_model_name(id.rsplit('/').next().unwrap_or(id)));
                (id.clone(), json!({"name": name, "description": ""}))
            })
            .collect()
    }
}

static DIRECT_UPSTREAM: Mutex<Option<DirectUpstream>> = Mutex::new(None);

/// 切到直连上游（`Some`）或回到网关（`None`）。改了之后下次启动 / 状态读取即按新上游计算；
/// 正在运行的本地服务配置指纹会变，界面会提示需要重启。
pub fn set_direct_upstream(upstream: Option<DirectUpstream>) {
    *DIRECT_UPSTREAM
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = upstream.map(DirectUpstream::normalized);
}

fn direct_upstream() -> Option<DirectUpstream> {
    DIRECT_UPSTREAM
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

/// 交给本地兼容服务的上游，来自网关绑定或直连配置。
struct LocalUpstream {
    root: String,
    token: String,
    default_model: String,
    small_model: String,
    models: Vec<String>,
    model_names: BTreeMap<String, Value>,
    model_windows: BTreeMap<String, u64>,
    default_window: u64,
}

impl LocalUpstream {
    fn direct(upstream: &DirectUpstream) -> Self {
        Self {
            root: upstream.base_url.clone(),
            token: upstream.api_key.clone(),
            default_model: upstream.model.clone(),
            small_model: upstream.small_model.clone(),
            models: upstream.models.clone(),
            model_names: upstream.model_names(),
            model_windows: upstream.model_windows(),
            default_window: upstream.context_window.unwrap_or_default(),
        }
    }
}

struct BackendRuntime {
    child: Child,
    stdin: Option<ChildStdin>,
    port: u16,
    instance_id: String,
    config_hash: String,
}

fn runtime_slot() -> &'static Mutex<Option<BackendRuntime>> {
    static SLOT: OnceLock<Mutex<Option<BackendRuntime>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn root_dir() -> PathBuf {
    crate::paths::default_app_state_dir().join("claude-local")
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|base| base.home_dir().to_path_buf())
        .unwrap_or_default()
}

/// 原版 Claude 本地模式支持的平台：macOS 与 Windows。
fn platform_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// Windows 上解析 Claude.exe 要走 PowerShell 查 Appx 包，冷启动很慢（status 每 3 秒
/// 轮询），结果做进程内缓存：缓存路径已不存在且不是 MSIX 路径时才重新解析。
#[cfg(target_os = "windows")]
fn source_app() -> PathBuf {
    static CACHE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut slot = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(path) = slot.as_ref()
        && (path.is_file() || crate::claude_localization::is_claude_msix_path(path))
    {
        return path.clone();
    }
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData/Local"));
    let fallback = local.join(r"Programs\Claude\Claude.exe");
    let resolved = crate::claude_localization::source_app(
        Instant::now() + Duration::from_secs(12),
    )
    .unwrap_or(fallback);
    *slot = Some(resolved.clone());
    resolved
}

#[cfg(not(target_os = "windows"))]
fn source_app() -> PathBuf {
    let system = PathBuf::from("/Applications/Claude.app");
    if system.is_dir() {
        system
    } else {
        home_dir().join("Applications/Claude.app")
    }
}

fn executable(source: &Path) -> PathBuf {
    if cfg!(windows) {
        source.to_path_buf()
    } else {
        source.join("Contents/MacOS/Claude")
    }
}

/// 官方应用自带的页面资源目录：mac 在 bundle 里，Windows 在 exe 旁边的 `resources/`。
fn desktop_resources(source: &Path) -> PathBuf {
    if cfg!(windows) {
        source
            .parent()
            .map(|dir| dir.join("resources"))
            .unwrap_or_default()
    } else {
        source.join("Contents/Resources")
    }
}

/// 状态栏的「已安装」判断：Windows 商店版（MSIX）在 WindowsApps 里有 ACL，
/// `is_file` 可能误报，路径形状匹配也算安装。
fn source_supported(source: &Path) -> bool {
    #[cfg(windows)]
    {
        return executable(source).is_file()
            || crate::claude_localization::is_claude_msix_path(source);
    }
    #[cfg(not(windows))]
    {
        cfg!(target_os = "macos") && executable(source).is_file()
    }
}

fn private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// 完整功能模式里内置 Claude Code 的配置目录（`CLAUDE_CONFIG_DIR`，会话记录在它的 `projects/` 下）。
///
/// Desktop 左侧的会话列表存在真实 Claude profile 里，Codex++ 与 ClaudeMom 共用同一份；会话记录也必须共用，
/// 否则在一个程序里建的会话到另一个里打不开（一直显示「此页面加载时间比平时更长」）。
/// 所以放在两个程序的状态目录之外；拿不到家目录时退回各自状态目录里的旧位置。
fn full_code_dir(root: &Path) -> PathBuf {
    shared_full_code_dir().unwrap_or_else(|| root.join("claude-code"))
}

fn shared_full_code_dir() -> Option<PathBuf> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".claude-desktop-local").join("claude-code"))
}

/// 共享目录之前，会话记录在各自状态目录的 `claude-code/` 里：启动前把缺的文件补进共享目录。
/// 只复制共享目录里还没有的文件，不覆盖、不删除原文件；复制失败只记日志，不挡启动。
fn merge_legacy_full_code_dir(legacy: &Path, shared: &Path) {
    if legacy == shared || !legacy.is_dir() {
        return;
    }
    if let Err(error) = copy_missing(legacy, shared) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "claude_local.merge_code_dir_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
}

fn copy_missing(from: &Path, to: &Path) -> Result<()> {
    private_dir(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_missing(&entry.path(), &target)?;
        } else if kind.is_file() && !target.exists() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    private_file(path)?.write_all(bytes)?;
    Ok(())
}

// ── 真实 profile 的 1p/3p 切换 ─────────────────────────────────
// 完整功能模式沿用 PoC 做法：复用真实 Claude profile，需要把
// claude_desktop_config.json 的 deploymentMode 改为 1p 并把
// Claude-3p/configLibrary 挪开。改动记录到 <state>/real-profile-switch.json，
// 停止/切回 3p 时按记录恢复，保证幂等可回滚。
const SWITCH_STATE_FILE: &str = "real-profile-switch.json";
const CONFIG_LIBRARY_BACKUP: &str = "configLibrary.codex-plus-backup";

/// Claude Desktop 的一组 profile 目录：data 放 1p 的 userData（claude_desktop_config.json、Cache、claude-code-sessions），
/// mode 是写 deploymentMode 的 Claude 目录，three_p 是 3p 模式的 userData。macOS 上 data 与 mode 相同。
struct ProfileSet {
    data: PathBuf,
    mode: PathBuf,
    three_p: PathBuf,
}

fn profile_sets() -> Vec<ProfileSet> {
    #[cfg(target_os = "windows")]
    {
        let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        return profile_sets_for(true, &home_dir(), roaming.as_deref(), local.as_deref());
    }
    #[cfg(not(target_os = "windows"))]
    profile_sets_for(false, &home_dir(), None, None)
}

/// 纯计算部分：目录由调用方注入，可在 mac 上测 Windows 布局。
///
/// Windows 下第一组是 classic 安装（data 在 `%APPDATA%`，mode/3p 在 `%LOCALAPPDATA%`）；
/// 之后 `%LOCALAPPDATA%\Packages` 里每个 `Claude_*` / `AnthropicPBC.Claude_*` 包
/// （大小写不敏感、排序后）各一组 MSIX 虚拟化目录。APPDATA/LOCALAPPDATA 缺失时
/// 回退 `home\AppData\Roaming|Local`。返回的第一组是主集合。
fn profile_sets_for(
    windows: bool,
    home: &Path,
    roaming: Option<&Path>,
    local: Option<&Path>,
) -> Vec<ProfileSet> {
    if !windows {
        let support = home.join("Library/Application Support");
        return vec![ProfileSet {
            data: support.join("Claude"),
            mode: support.join("Claude"),
            three_p: support.join("Claude-3p"),
        }];
    }
    let roaming = roaming
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join("AppData").join("Roaming"));
    let local = local
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join("AppData").join("Local"));
    let mut sets = vec![ProfileSet {
        data: roaming.join("Claude"),
        mode: local.join("Claude"),
        three_p: local.join("Claude-3p"),
    }];
    let mut packages: Vec<PathBuf> = fs::read_dir(local.join("Packages"))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            ["Claude_", "AnthropicPBC.Claude_"].iter().any(|prefix| {
                name.get(..prefix.len())
                    .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
            }) && entry.path().is_dir()
        })
        .map(|entry| entry.path())
        .collect();
    // read_dir 顺序不稳定，排序后结果可复现。
    packages.sort();
    for package in packages {
        let cache = package.join("LocalCache");
        sets.push(ProfileSet {
            data: cache.join("Roaming").join("Claude"),
            mode: cache.join("Local").join("Claude"),
            three_p: cache.join("Local").join("Claude-3p"),
        });
    }
    sets
}

/// 各组的 deploymentMode 配置：按 data、mode、three_p 的顺序取，去重。
fn desktop_config_paths(sets: &[ProfileSet]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for set in sets {
        for dir in [&set.data, &set.mode, &set.three_p] {
            let path = dir.join("claude_desktop_config.json");
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

fn switch_state_path(root: &Path) -> PathBuf {
    root.join(SWITCH_STATE_FILE)
}

/// 处理「上次切换留下、状态文件却已经丢了」的 `configLibrary` 备份。
///
/// 正常流程里备份由 `restore_real_profile` 按状态文件还原。状态文件丢了（崩溃、
/// 中途被删）就会卡在这里：备份一直在，`apply_real_profile_one_p` 每次启动都拒绝。
/// 于是按现场情况收尾：目标不在就把备份放回去；目标还在（Claude 自己重建过）
/// 就保留目标，把备份归档到 state 的 `backups/` 里，谁也不丢。
fn recover_orphan_config_library(sets: &[ProfileSet], root: &Path) -> Result<Option<String>> {
    // 状态文件还认这份备份：属于正常切换流程，交给 restore_real_profile。
    // 新旧两种格式都认：新格式逐个列在 configLibraryBackups，旧格式是单条
    // configLibraryMoved + configLibraryBackup。
    let tracked = |backup: &Path| {
        let backup = backup.to_string_lossy().into_owned();
        let Some(state) = fs::read(switch_state_path(root))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        else {
            return false;
        };
        if let Some(list) = state["configLibraryBackups"].as_array() {
            return list
                .iter()
                .any(|entry| entry["backup"].as_str() == Some(backup.as_str()));
        }
        state["configLibraryMoved"].as_bool() == Some(true)
            && state["configLibraryBackup"].as_str() == Some(backup.as_str())
    };
    let mut notes = Vec::new();
    for set in sets {
        let target = set.three_p.join("configLibrary");
        let backup = target.with_file_name(CONFIG_LIBRARY_BACKUP);
        if !backup.exists() || tracked(&backup) {
            continue;
        }
        if !target.exists() {
            fs::rename(&backup, &target)
                .with_context(|| format!("无法恢复 {}", target.display()))?;
            notes.push(format!(
                "已恢复上次遗留的 configLibrary 备份：{}",
                target.display()
            ));
            continue;
        }
        let archive = root.join("backups");
        private_dir(&archive)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or_default();
        let mut archived = archive.join(format!("configLibrary-orphan-{stamp}"));
        for index in 1.. {
            if !archived.exists() {
                break;
            }
            archived = archive.join(format!("configLibrary-orphan-{stamp}-{index}"));
        }
        fs::rename(&backup, &archived)
            .with_context(|| format!("无法归档遗留的 {}", backup.display()))?;
        notes.push(format!(
            "已把上次遗留的 configLibrary 备份归档到 {}",
            archived.display()
        ));
    }
    Ok((!notes.is_empty()).then(|| notes.join("\n")))
}

fn apply_real_profile_one_p(sets: &[ProfileSet], root: &Path) -> Result<()> {
    recover_orphan_config_library(sets, root)?;
    let state_path = switch_state_path(root);
    if state_path.exists() {
        // 已按记录切换过：校验仍在 1p 状态即可（幂等）。
        let state: Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        ensure!(
            state["applied"].as_bool() == Some(true),
            "Claude 本地模式切换状态文件不一致，请检查 {SWITCH_STATE_FILE}"
        );
        return Ok(());
    }
    private_dir(root)?;
    let mut original = serde_json::Map::new();
    for config_path in desktop_config_paths(sets) {
        let key = config_path.to_string_lossy().into_owned();
        if fs::symlink_metadata(&config_path).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("Claude 配置不能是符号链接: {}", config_path.display());
        }
        match fs::read(&config_path) {
            Ok(bytes) => {
                let mut config: Value = serde_json::from_slice(&bytes)
                    .with_context(|| format!("{} 不是有效 JSON", config_path.display()))?;
                let obj = config
                    .as_object_mut()
                    .context("Claude 配置必须是 JSON 对象")?;
                original.insert(
                    key,
                    obj.get("deploymentMode").cloned().unwrap_or(Value::Null),
                );
                if obj.get("deploymentMode").and_then(Value::as_str) != Some("1p") {
                    obj.insert("deploymentMode".into(), json!("1p"));
                    crate::settings::atomic_write(
                        &config_path,
                        &serde_json::to_vec_pretty(&config)?,
                    )?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                original.insert(key, Value::Null);
            }
            Err(error) => return Err(error.into()),
        }
    }
    // 每组 profile 各自的 configLibrary 都要挪开（Windows 可能 classic + MSIX 多组）。
    let mut libraries = Vec::new();
    for set in sets {
        let library = set.three_p.join("configLibrary");
        let backup = library.with_file_name(CONFIG_LIBRARY_BACKUP);
        if backup.exists() {
            bail!(
                "Claude-3p 已存在未恢复的 configLibrary 备份，请先处理 {}",
                backup.display()
            );
        }
        if library.exists() {
            libraries.push((library, backup));
        }
    }
    let mut moved = Vec::new();
    for (library, backup) in libraries {
        fs::rename(&library, &backup)
            .with_context(|| format!("无法挪开 {}", library.display()))?;
        moved.push(json!({
            "target": library.to_string_lossy(),
            "backup": backup.to_string_lossy(),
        }));
    }
    let state = json!({
        "applied": true,
        "deploymentMode": Value::Object(original),
        "configLibraryBackups": Value::Array(moved),
    });
    crate::settings::atomic_write(&state_path, &serde_json::to_vec_pretty(&state)?)?;
    Ok(())
}

fn restore_real_profile(sets: &[ProfileSet], root: &Path) -> Result<()> {
    // 状态文件丢了也得先把遗留备份收尾，否则切回 3p 之后它还会一直卡着下次启动。
    recover_orphan_config_library(sets, root)?;
    let state_path = switch_state_path(root);
    if !state_path.exists() {
        return Ok(());
    }
    let state: Value = serde_json::from_slice(&fs::read(&state_path)?)?;
    if let Some(map) = state["deploymentMode"].as_object() {
        for (path, original) in map {
            let config_path = PathBuf::from(path);
            if !config_path.exists() {
                continue;
            }
            let mut config: Value = serde_json::from_slice(&fs::read(&config_path)?)?;
            let obj = match config.as_object_mut() {
                Some(obj) => obj,
                None => continue,
            };
            match original {
                Value::Null => {
                    obj.remove("deploymentMode");
                }
                value => {
                    obj.insert("deploymentMode".into(), value.clone());
                }
            }
            crate::settings::atomic_write(&config_path, &serde_json::to_vec_pretty(&config)?)?;
        }
    }
    if let Some(list) = state["configLibraryBackups"].as_array() {
        for entry in list {
            let backup = PathBuf::from(entry["backup"].as_str().unwrap_or_default());
            let target = PathBuf::from(entry["target"].as_str().unwrap_or_default());
            if backup.exists() && !target.exists() {
                fs::rename(&backup, &target)
                    .with_context(|| format!("无法恢复 {}", target.display()))?;
            }
        }
    } else if state["configLibraryMoved"].as_bool() == Some(true) {
        // 旧格式（只记了主集合的一份备份）也要能读回。
        let backup = PathBuf::from(state["configLibraryBackup"].as_str().unwrap_or(""));
        if let Some(target) = sets.first().map(|set| set.three_p.join("configLibrary"))
            && backup.exists()
            && !target.exists()
        {
            fs::rename(&backup, &target)
                .with_context(|| format!("无法恢复 {}", target.display()))?;
        }
    }
    fs::remove_file(&state_path).ok();
    Ok(())
}

// 完整功能模式的 PoC 数据路径：仅支持显式 dev 覆盖 CLAUDE_LOCAL_RECORDED_DIR
// （指向含 .cache/ + mock/bootstrap.json 的录制目录）。正式环境不做祖先搜索，
// 数据来自安装到 state 的 mock 模板 + 在线 SPA 写穿缓存。
fn recorded_data_dir() -> Option<PathBuf> {
    let valid = |dir: &Path| {
        dir.join(".cache").is_dir() && dir.join("mock").join("bootstrap.json").is_file()
    };
    let dir = std::env::var("CLAUDE_LOCAL_RECORDED_DIR")
        .ok()
        .map(PathBuf::from)?;
    valid(&dir).then_some(dir)
}

fn install_assets(root: &Path) -> Result<()> {
    private_dir(root)?;
    for (name, contents) in ASSETS {
        let path = root.join(name);
        if fs::read(&path).is_ok_and(|existing| existing == *contents) {
            continue;
        }
        if let Some(parent) = path.parent() {
            private_dir(parent)?;
        }
        write_private(&path, contents)?;
    }
    // 信任标记：代理据此确认旁边的 mock/ 是我们安装的内嵌模板（而非来路不明的数据）。
    if root.join("mock/bootstrap.json").is_file() {
        write_private(
            &root.join("mock/.codex-plus-template"),
            b"codex-plus mock template\n",
        )?;
    }
    Ok(())
}

// 这些探测命令只输出短状态；所有等待均有上限，避免系统工具卡住管理器。
fn probe(command: &mut Command, timeout: Duration) -> Result<(bool, Vec<u8>)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("无法运行 Claude 本地服务检查")?;
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            let mut bytes = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                stdout.take(64 * 1024).read_to_end(&mut bytes)?;
            }
            return Ok((status.success(), bytes));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Claude 本地服务检查超时");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn checked(command: &mut Command, timeout: Duration, message: &str) -> Result<()> {
    ensure!(probe(command, timeout)?.0, "{message}");
    Ok(())
}

/// 统一建 Command：Windows 上不为控制台程序（node、openssl 等）弹黑色终端窗口。
fn hidden_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command
}

fn node_runtime() -> Option<PathBuf> {
    let node = crate::dsh_plugins::locate_node()?;
    probe(
        hidden_command(&node).args([
            "-e",
            "process.exit(typeof fetch==='function'&&typeof WebSocket==='function'?0:1)",
        ]),
        PROBE_TIMEOUT,
    )
    .ok()
    .filter(|(success, _)| *success)
    .map(|_| node)
}

/// `--proxy-server` 完整标志：整词匹配，5351 不会误伤 53514。
fn proxy_flag(port: u16) -> String {
    format!("--proxy-server=http://127.0.0.1:{port}")
}

/// 进程列表里有没有进程把 `--proxy-server` 指到这个端口：
/// 逐行、按空白切分，整词等于完整标志才算。
fn processes_use_proxy(processes: &str, port: u16) -> bool {
    let flag = proxy_flag(port);
    processes
        .lines()
        .any(|line| line.split_whitespace().any(|token| token == flag))
}

/// 从 `pgrep -fl`（行首是 pid）里挑出挂在指定代理端口上的进程 pid。
fn claude_pids_on_proxy(listing: &str, port: u16) -> Vec<u32> {
    let flag = proxy_flag(port);
    listing
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim().split_once(' ')?;
            if command.split_whitespace().any(|token| token == flag) {
                pid.trim().parse().ok()
            } else {
                None
            }
        })
        .collect()
}

/// 关掉「经本机代理启动」的 Claude：`--proxy-server=http://127.0.0.1:<port>` 是它独有的标记，
/// 按端口整词匹配就不会误伤用户自己开的 Claude。返回是否真的动过进程。
fn quit_claude_with_proxy(port: u16) -> Result<bool> {
    if !platform_supported() {
        return Ok(false);
    }
    let pids = claude_pids_on_proxy(&claude_processes()?, port);
    if pids.is_empty() {
        return Ok(false);
    }
    #[cfg(target_os = "macos")]
    {
        for pid in &pids {
            let _ = Command::new("/bin/kill")
                .args(["-TERM", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(windows)]
    {
        for pid in &pids {
            crate::windows_integration::request_process_close(*pid);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && pids.iter().any(|pid| process_alive(*pid)) {
        std::thread::sleep(Duration::from_millis(50));
    }
    #[cfg(target_os = "macos")]
    for pid in pids.iter().filter(|pid| process_alive(**pid)) {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(windows)]
    for pid in pids.iter().filter(|pid| process_alive(**pid)) {
        crate::windows_integration::close_process_tree(*pid, true);
    }
    Ok(true)
}

fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        return matches!(
            crate::windows_integration::process_lifetime(pid),
            Ok(Some((_, exited))) if !exited
        );
    }
    #[cfg(not(windows))]
    Command::new("/bin/ps")
        .args(["-p", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 该进程是不是 Claude Desktop（而不是 Claude Code CLI 的 claude.exe）：
/// 路径读不到算，WindowsApps 包路径算，旁边有 `resources\app.asar` 的算；
/// 其余（`~/.local/bin/claude.exe`、`claude-code\<ver>\claude.exe` 这类 CLI）不算。
#[cfg(any(windows, test))]
fn is_desktop_claude_image(exe_file: &str, path: Option<&Path>) -> bool {
    if !exe_file.eq_ignore_ascii_case("Claude.exe") {
        return false;
    }
    let Some(path) = path else {
        return true;
    };
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if text.contains(r"\windowsapps\") {
        return true;
    }
    path.parent()
        .is_some_and(|dir| dir.join("resources").join("app.asar").is_file())
}

fn claude_processes() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        let (success, bytes) = probe(
            Command::new("/usr/bin/pgrep").args(["-fl", "/Contents/MacOS/Claude( |$)"]),
            PROBE_TIMEOUT,
        )?;
        return Ok(if success {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            String::new()
        });
    }
    #[cfg(windows)]
    {
        // 与 pgrep -fl 同一形状：`{pid} {command_line}`，pids_on_proxy/use_proxy 原样可用。
        let mut listing = String::new();
        for info in crate::windows_integration::enumerate_processes() {
            if !is_desktop_claude_image(&info.exe_file, info.executable_path.as_deref()) {
                continue;
            }
            let command_line = crate::windows_integration::process_command_line(info.process_id)
                .unwrap_or_default();
            listing.push_str(&format!("{} {}\n", info.process_id, command_line));
        }
        return Ok(listing);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    Ok(String::new())
}

#[cfg(target_os = "macos")]
fn version(source: &Path) -> String {
    if !source.is_dir() {
        return String::new();
    }
    probe(
        Command::new("/usr/bin/defaults")
            .arg("read")
            .arg(source.join("Contents/Info"))
            .arg("CFBundleShortVersionString"),
        PROBE_TIMEOUT,
    )
    .ok()
    .filter(|(success, _)| *success)
    .map(|(_, bytes)| String::from_utf8_lossy(&bytes).trim().to_string())
    .unwrap_or_default()
}

/// Windows 版本号走 PowerShell 读 ProductVersion（冷启动慢），
/// 按 （路径， 修改时间） 缓存，避免每 3 秒一次的状态轮询反复起 PowerShell。
#[cfg(target_os = "windows")]
fn version(source: &Path) -> String {
    static CACHE: OnceLock<Mutex<(PathBuf, Option<std::time::SystemTime>, String)>> =
        OnceLock::new();
    let modified = fs::metadata(source).and_then(|meta| meta.modified()).ok();
    let cache = CACHE.get_or_init(|| Mutex::new((PathBuf::new(), None, String::new())));
    let mut slot = cache.lock().unwrap_or_else(|error| error.into_inner());
    if slot.0 == source && slot.1 == modified {
        return slot.2.clone();
    }
    let resolved = crate::claude_localization::app_version(
        source,
        "",
        Instant::now() + Duration::from_secs(12),
    )
    .unwrap_or_default();
    // 读失败（超时等）不缓存空值，下次轮询再试。
    if !resolved.is_empty() {
        *slot = (source.to_path_buf(), modified, resolved.clone());
    }
    resolved
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn version(_source: &Path) -> String {
    String::new()
}

fn ca_ready(root: &Path) -> bool {
    ["ca.pem", "ca.key", "server.crt", "server.key"]
        .iter()
        .all(|name| {
            fs::metadata(root.join("ca").join(name)).is_ok_and(|m| m.is_file() && m.len() > 0)
        })
}

/// CA 证书的 SHA-1 指纹（大写十六进制、无冒号）：certutil 按它定位信任条目。
#[cfg(any(windows, test))]
fn ca_thumbprint(pem: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(pem).ok()?;
    let begin = "-----BEGIN CERTIFICATE-----";
    let end = "-----END CERTIFICATE-----";
    let start = text.find(begin)? + begin.len();
    let stop = text[start..].find(end)? + start;
    let encoded: String = text[start..stop]
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    use base64::Engine;
    let der = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
    let digest = <sha1::Sha1 as sha1::Digest>::digest(&der);
    Some(digest.iter().map(|byte| format!("{byte:02X}")).collect())
}

fn ca_trusted(root: &Path) -> bool {
    if !ca_ready(root) {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        return probe(
            Command::new("/usr/bin/security")
                .args(["verify-cert", "-q", "-L", "-c"])
                .arg(root.join("ca/server.crt"))
                .args(["-p", "ssl", "-n", "claude.ai"]),
            PROBE_TIMEOUT,
        )
        .is_ok_and(|(success, _)| success);
    }
    #[cfg(windows)]
    {
        let Some(thumbprint) = fs::read(root.join("ca/ca.pem"))
            .ok()
            .and_then(|pem| ca_thumbprint(&pem))
        else {
            return false;
        };
        return probe(
            hidden_command("certutil").args(["-user", "-store", "Root", &thumbprint]),
            Duration::from_secs(10),
        )
        .is_ok_and(|(success, _)| success);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    false
}

fn prepare_ca(root: &Path) -> Result<()> {
    if ca_ready(root) {
        return Ok(());
    }
    let ca = root.join("ca");
    private_dir(&ca)?;
    generate_ca(&ca)
}

/// 用 rcgen 在进程内生成整套本地 CA 材料，不再调用外部 openssl
/// （Windows 系统不自带 openssl，之前还得去找 Git for Windows 附带的那枚）。
/// 只在 `ca_ready` 为假时调用：磁盘上已有的证书（含旧版 RSA 证书）继续沿用。
fn generate_ca(ca: &Path) -> Result<()> {
    use rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType,
        ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SerialNumber,
    };
    let now = time::OffsetDateTime::now_utc();
    // 随机序列号：uuid v4 的 16 字节，最高位清零避免被当成负数。
    let serial = || {
        let mut bytes = *uuid::Uuid::new_v4().as_bytes();
        bytes[0] &= 0x7f;
        SerialNumber::from_slice(&bytes)
    };
    let distinguished_name = |name: &str| {
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, name);
        dn
    };

    // 本地 CA：默认 ECDSA P-256 自签，只用来签本机这一枚服务证书。
    let ca_key = KeyPair::generate().context("生成本地 CA 私钥失败")?;
    let mut ca_params = CertificateParams::default();
    ca_params.distinguished_name = distinguished_name(CA_NAME);
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.serial_number = Some(serial());
    ca_params.not_before = now - time::Duration::days(1);
    ca_params.not_after = now + time::Duration::days(3650);
    let ca_cert = ca_params
        .self_signed(&ca_key)
        .context("生成本地 CA 证书失败")?;

    // 服务端证书：本地代理拦截 claude.ai 系 TLS 用。SAN 覆盖官方域名与 localhost；
    // "127.0.0.1" 会被解析成 IpAddress 类型的 SAN（不是 DNS）。有效期不超过 825 天
    // （Apple 对 TLS 服务端证书的要求），取 824 天留一天余量。
    let server_key = KeyPair::generate().context("生成本地服务私钥失败")?;
    let mut server_params = CertificateParams::new(vec![
        "claude.ai".into(),
        "*.claude.ai".into(),
        "claude.com".into(),
        "*.claude.com".into(),
        "anthropic.com".into(),
        "*.anthropic.com".into(),
        "claudeusercontent.com".into(),
        "*.claudeusercontent.com".into(),
        "localhost".into(),
        "127.0.0.1".into(),
    ])
    .context("本地服务证书参数无效")?;
    server_params.distinguished_name = distinguished_name("claude.ai");
    server_params.is_ca = IsCa::NoCa;
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server_params.use_authority_key_identifier_extension = true;
    server_params.serial_number = Some(serial());
    server_params.not_before = now - time::Duration::days(1);
    server_params.not_after = now + time::Duration::days(824);
    let issuer = Issuer::new(ca_params, &ca_key);
    let server_cert = server_params
        .signed_by(&server_key, &issuer)
        .context("签发本地服务证书失败")?;

    // 私钥在前证书在后；PEM 私钥是 PKCS#8，Node 的 tls.createSecureContext 能直接读。
    write_private(&ca.join("ca.key"), ca_key.serialize_pem().as_bytes())?;
    write_private(&ca.join("server.key"), server_key.serialize_pem().as_bytes())?;
    write_private(&ca.join("ca.pem"), ca_cert.pem().as_bytes())?;
    write_private(&ca.join("server.crt"), server_cert.pem().as_bytes())?;
    Ok(())
}

fn trust_ca(root: &Path) -> Result<()> {
    ensure!(
        platform_supported(),
        "本地证书信任目前仅支持 macOS 和 Windows"
    );
    if ca_trusted(root) {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    checked(
        Command::new("/usr/bin/security")
            .args(["add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k"])
            .arg(home_dir().join("Library/Keychains/login.keychain-db"))
            .arg(root.join("ca/ca.pem")),
        Duration::from_secs(120),
        "本地证书尚未信任，请在系统授权窗口中允许后重试",
    )?;
    // certutil -addstore 会弹 Windows 安全警告，用户点「是」才会返回成功。
    #[cfg(windows)]
    checked(
        hidden_command("certutil")
            .args(["-user", "-addstore", "Root"])
            .arg(root.join("ca/ca.pem")),
        Duration::from_secs(120),
        "本地证书尚未信任，请在 Windows 弹出的安全警告中点击“是”后重试",
    )?;
    ensure!(ca_trusted(root), "本地证书信任未生效，请重新检查系统授权");
    Ok(())
}

/// 内嵌的运行时代理资产指纹。
///
/// 代理进程是常驻的，`start_backend` 靠 `config_hash` 判断能不能复用。运行时
/// 资产（`runtime.mjs` / `proxy.mjs` …）换了而哈希不变时，旧进程会带着上一版
/// 环境变量契约继续跑（例如不知道 `CLAUDE_LOCAL_GATEWAY_DEFAULT_WINDOW`），
/// 界面上看起来就是"改了没生效"。把资产内容算进哈希，升级后必然重启。
fn assets_digest() -> &'static str {
    static DIGEST: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hasher = Sha256::new();
        for (name, contents) in ASSETS {
            hasher.update(name.as_bytes());
            hasher.update((contents.len() as u64).to_le_bytes());
            hasher.update(contents);
        }
        format!("{:x}", hasher.finalize())
    })
}

/// 取消本地 CA 的信任。
///
/// 完整功能模式靠这枚 CA 拦截 claude.ai 的 TLS；不再使用时它不该继续留在钥匙串里：
/// 私钥就在应用状态目录（0600），一条残留的信任条目等于给本机留了个长期中间人。
fn untrust_ca(root: &Path) -> Result<()> {
    ensure!(
        platform_supported(),
        "本地证书信任目前仅支持 macOS 和 Windows"
    );
    let cert = root.join("ca/ca.pem");
    if !cert.is_file() {
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("/usr/bin/security")
            .args(["remove-trusted-cert", "-d"])
            .arg(&cert)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        ensure!(
            !ca_trusted(root),
            "本地证书仍在信任列表中，请在“钥匙串访问”里删除 Codex++ Local CA"
        );
    }
    // certutil -delstore 同样会弹确认框，给足用户点确认的时间。
    #[cfg(windows)]
    {
        let Some(thumbprint) = fs::read(&cert)
            .ok()
            .and_then(|pem| ca_thumbprint(&pem))
        else {
            bail!("无法读取本地证书指纹，无法取消信任");
        };
        let _ = probe(
            hidden_command("certutil").args(["-user", "-delstore", "Root", &thumbprint]),
            Duration::from_secs(120),
        );
        ensure!(
            !ca_trusted(root),
            "本地证书仍在信任列表中，请运行 certmgr.msc，在“受信任的根证书颁发机构”里删除 Codex++ Claude Local CA"
        );
    }
    Ok(())
}

fn config_hash(config: &ClaudeLocalSettings) -> String {
    if let Some(upstream) = direct_upstream() {
        // 直连：换了 Base URL、Key、模型都算配置变了（指纹是摘要，不会把 Key 原样落盘）。
        return format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    config,
                    "direct",
                    &upstream,
                    assets_digest(),
                    desktop_fingerprint()
                ))
                .unwrap_or_default()
            )
        );
    }
    let gateway_state = GatewayStore::default().load().ok().map(|gateway| {
        let gateway = gateway.with_extension_providers();
        let models = available_models(&gateway);
        let binding = local_binding(&gateway, config).ok();
        let windows = binding
            .as_ref()
            .map(|binding| available_model_windows(&gateway, binding, config.list_all_models))
            .unwrap_or_default();
        // 「Agent 接入」里换了模型也算配置变了：代理要重启，默认模型才会跟着换。
        let model = binding.map(|binding| binding.model);
        (models, windows, model)
    });
    config_hash_with_gateway_state(config, gateway_state)
}

fn config_hash_with_gateway_state(
    config: &ClaudeLocalSettings,
    gateway_state: Option<(Vec<String>, BTreeMap<String, u64>, Option<String>)>,
) -> String {
    hash_with_desktop(config, gateway_state, desktop_fingerprint())
}

fn hash_with_desktop(
    config: &ClaudeLocalSettings,
    gateway_state: Option<(Vec<String>, BTreeMap<String, u64>, Option<String>)>,
    desktop: Option<(u64, u128)>,
) -> String {
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(config, gateway_state, assets_digest(), desktop))
                .unwrap_or_default()
        )
    )
}

/// 安装包里 `app.asar` 的大小与修改时间。受管路径的官方页面和资源都从它读，
/// Desktop 更新后它会变：算进配置指纹，本地服务就会重启换上新页面，状态里也会提示重启。
/// 只读文件元数据，不起进程（状态每 3 秒刷新一次）。
fn desktop_fingerprint() -> Option<(u64, u128)> {
    let meta = fs::metadata(desktop_resources(&source_app()).join("app.asar")).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), modified))
}

fn available_models(gateway: &GatewaySettings) -> Vec<String> {
    let mut models = Vec::new();
    for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
        for model in crate::gateway::route::chat_models(&provider.relay) {
            if is_internal_review_model(&model) {
                continue;
            }
            let model = format!("{}/{model}", provider.relay.id);
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    for group in gateway
        .groups
        .iter()
        .filter(|group| !group.members.is_empty())
    {
        models.push(group.model_id());
    }
    models
}

/// 把模型 slug 拼成给人看的样子：`gpt-6.1-sol` → `GPT-6.1 Sol`、
/// `deepseek-v4-flash` → `DeepSeek V4 Flash`、`kimi-k3` → `Kimi K3`、
/// `claude-opus-4-6` → `Claude Opus 4.6`。
/// 只用于界面显示，发给上游的 model id 一个字都不改。
fn friendly_model_name(slug: &str) -> String {
    // Claude 系 slug 用连字符分隔版本号（`opus-4-6` 即 4.6）：相邻的 1–2 位纯数字段用点连起来；
    // 日期（`20251001`）、`4k` 这类不算版本号，照旧空格分隔。
    let version_digit = |part: &str| (1..=2).contains(&part.len()) && part.chars().all(|c| c.is_ascii_digit());
    let parts: Vec<String> = slug
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .enumerate()
        .map(|(index, part)| friendly_model_word(part, index == 0))
        .collect();
    let mut out = String::new();
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            // `GPT-6.1` / `GLM-5.3`：前缀后面紧跟版本号时用连字符，其余用空格。
            let previous = parts[index - 1].as_str();
            let digit = part.chars().next().is_some_and(|c| c.is_ascii_digit());
            out.push(if (previous == "GPT" || previous == "GLM") && digit {
                '-'
            } else if version_digit(previous) && version_digit(part) {
                '.'
            } else {
                ' '
            });
        }
        out.push_str(part);
    }
    if out.is_empty() { slug.to_string() } else { out }
}

fn friendly_model_word(raw: &str, first: bool) -> String {
    let lower = raw.to_ascii_lowercase();
    let capitalized = || {
        let mut chars = lower.chars();
        match chars.next() {
            Some(head) => head.to_ascii_uppercase().to_string() + chars.as_str(),
            None => String::new(),
        }
    };
    match lower.as_str() {
        "gpt" => "GPT".to_string(),
        "glm" => "GLM".to_string(),
        "llm" | "api" => lower.to_ascii_uppercase(),
        "deepseek" => "DeepSeek".to_string(),
        "kimi" => "Kimi".to_string(),
        "qwen" => "Qwen".to_string(),
        "claude" => "Claude".to_string(),
        "gemini" => "Gemini".to_string(),
        "grok" => "Grok".to_string(),
        "mimo" => "MiMo".to_string(),
        "mini" | "pro" | "flash" | "sol" | "terra" | "luna" | "astra" | "fast" | "codex"
        | "opus" | "sonnet" | "haiku" | "fable" | "turbo" | "lite" | "reasoner" | "thinking" | "image" => {
            capitalized()
        }
        // `v4` / `v2.5`：大写 V
        _ if lower
            .strip_prefix('v')
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '.')) =>
        {
            format!("V{}", &lower[1..])
        }
        // 非首段的「字母+数字」短词（`k3`）：首字母大写
        _ if !first
            && lower.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && lower.chars().any(|c| c.is_ascii_digit())
            && lower.len() <= 8 =>
        {
            capitalized()
        }
        // 版本号、编号、不认识的词原样保留
        _ => raw.to_string(),
    }
}

/// Claude 模型选择器的显示名：有组显示名用组名，否则把 slug 拼得像样点（不再是 `jojocodemax/gpt-6.1-sol`）；
/// 副标题放供应商显示名（取不到就用前缀原文）。发给网关的 model id 不变。
fn model_names(
    gateway: &GatewaySettings,
    models: &[String],
) -> std::collections::BTreeMap<String, Value> {
    models
        .iter()
        .map(|id| {
            let friendly = crate::gateway::route::model_display_name(gateway, id);
            let slug = id.rsplit('/').next().unwrap_or(id);
            let name = if friendly != id.as_str() {
                friendly.to_string()
            } else {
                friendly_model_name(slug)
            };
            let vendor = id.split('/').next().unwrap_or_default();
            let provider = gateway
                .providers
                .iter()
                .find(|provider| provider.relay.id == vendor)
                .map(|provider| provider.relay.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| vendor.to_string());
            (id.clone(), json!({"name": name, "description": provider}))
        })
        .collect()
}

/// 自动评审模型（`codex-auto-review`）是 Codex 内部子代理用的，不该出现在
/// 人的模型选择器里：选中它当聊天模型没有意义，只会白烧额度。
fn is_internal_review_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("auto-review")
}

/// 交给 Claude 的模型：`(可见, 因窗口更小而隐藏)`。
///
/// Claude Code 的上下文窗口是一份**全局环境变量**（`CLAUDE_CODE_MAX_CONTEXT_TOKENS`），
/// 启动时按默认模型算一次；在 Claude 里切到窗口更小的模型时它不会跟着变小，
/// auto-compact 会一直等到大窗口才触发，请求先被上游顶回来。所以只暴露窗口
/// 不小于默认模型的模型，其余的仍留在 Codex++ 的默认模型选择器里。
fn exposed_models(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    list_all: bool,
) -> (Vec<String>, Vec<String>) {
    let models = available_models(gateway);
    if list_all {
        // 用户显式要求全列：不按窗口过滤，界面里自己留意切到小窗口模型的风险。
        return (models, Vec::new());
    }
    let Some(selected) = selected_model_window(gateway, binding, &binding.model) else {
        return (models, Vec::new());
    };
    models
        .into_iter()
        .partition(|model| {
            selected_model_window(gateway, binding, model).is_some_and(|window| window >= selected)
        })
}

fn available_model_windows(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    list_all: bool,
) -> BTreeMap<String, u64> {
    exposed_models(gateway, binding, list_all)
        .0
        .into_iter()
        .filter_map(|model| {
            selected_model_window(gateway, binding, &model).map(|window| (model, window))
        })
        .collect()
}

fn selected_model_window(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<u64> {
    crate::gateway::route::model_entry_window(gateway, binding, model)
        .or(binding.context_window)
        .filter(|window| *window > 0)
}

/// 用户自己在环境里给的窗口（手动 `CLAUDE_CODE_MAX_CONTEXT_TOKENS=1000000` 那种）。
fn inherited_context_tokens() -> Option<u64> {
    std::env::var("CLAUDE_CODE_MAX_CONTEXT_TOKENS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
}

/// 网关声明优先，其次是用户显式设的值；两者都没有时不设，交给 Claude Code 自己的模型表。
fn pick_context_tokens(resolved: Option<u64>, inherited: Option<u64>) -> Option<u64> {
    let usable = |window: Option<u64>| window.filter(|window| *window > 0);
    usable(resolved).or_else(|| usable(inherited))
}

fn context_tokens(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<u64> {
    pick_context_tokens(
        selected_model_window(gateway, binding, model),
        inherited_context_tokens(),
    )
}

fn validate_config(mut config: ClaudeLocalSettings) -> Result<ClaudeLocalSettings> {
    config.model = config.model.trim().to_string();
    ensure!(
        config.model.len() <= 512 && !config.model.chars().any(char::is_control),
        "模型名称包含无效字符或过长"
    );
    if config.background_mode {
        config.computer_use = true;
    }
    // 显示名与套餐按白名单落盘：界面改哪一格都不影响其它字段的规范化。
    config.display_name = config.effective_display_name();
    ensure!(config.email_is_valid(), "邮箱格式不正确");
    config.email = config.effective_email();
    config.plan = config.normalized_plan().to_string();
    Ok(config)
}

fn bind_gateway(
    store: &GatewayStore,
    config: &ClaudeLocalSettings,
) -> Result<(GatewaySettings, AgentBinding)> {
    let (_, binding) = store.update(|gateway| {
        let view = gateway.clone().with_extension_providers();
        let binding = local_binding(&view, config)?;
        gateway.bindings.insert(AGENT_ID.into(), binding.clone());
        Ok(binding)
    })?;
    // 不走 enable_binding：它会改 Desktop 的部署模式，原版本地服务只需要网关路由。
    Ok((store.load()?.with_extension_providers(), binding))
}

/// Claude 用哪个模型：`Agent 接入` 里那份绑定选过就用它；否则退回配置里的旧值；
/// 两者都没有才默认第一个可用模型。**选过但已经不可用**时留空，交给调用方报错，
/// 不静默换成别的模型。
fn chosen_model(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    config: &ClaudeLocalSettings,
) -> String {
    let routable = |model: &str| {
        !model.is_empty() && crate::gateway::route::model_is_routable(gateway, binding, model)
    };
    // 「Agent 接入」页里 Claude Desktop 那一行写的是 `claude-desktop` 绑定；本地模式用的是
    // `claude-desktop-local`。用户只看得见前者，所以以它为准，本地绑定自己的值退到第二位。
    let sibling = gateway
        .binding(&ToolId::Unknown("claude-desktop".into()))
        .map(|binding| binding.model.trim().to_string())
        .unwrap_or_default();
    let bound = binding.model.trim();
    let configured = config.model.trim();
    if routable(&sibling) {
        sibling
    } else if routable(bound) {
        bound.to_string()
    } else if routable(configured) {
        configured.to_string()
    } else if !bound.is_empty() {
        // 选过但上游已经不提供它了：原样留着，让下面的 ensure 明确报错，不静默换模型。
        bound.to_string()
    } else if !configured.is_empty() {
        configured.to_string()
    } else {
        binding.extra_models.first().cloned().unwrap_or_default()
    }
}

fn local_binding(gateway: &GatewaySettings, config: &ClaudeLocalSettings) -> Result<AgentBinding> {
    let agent = ToolId::Unknown(AGENT_ID.into());
    let mut binding = gateway.binding(&agent).cloned().unwrap_or_default();
    binding.pool_exclude.clear();
    binding.extra_models = available_models(gateway);
    // 模型以「Agent 接入」里这份绑定的选择为准：那边一处选好，Codex / Claude / Grok 都按它走。
    // 配置里的 `model` 只是老版本的遗留值，绑定还没选过模型时才拿来兜底。
    binding.model = chosen_model(gateway, &binding, &config);
    let model = crate::gateway::route::binding_default_model(gateway, &binding);
    ensure!(
        !model.is_empty() && crate::gateway::route::model_is_routable(gateway, &binding, &model),
        "请先在供应商池中添加可用模型，并为 Claude 本地模式选择模型"
    );
    binding.model = model;
    binding.enabled = true;
    if binding.token.is_empty() {
        binding.token = new_client_token(&agent);
    }
    Ok(binding)
}

/// Codex++ 重启后要不要在原端口恢复代理：runtime 不在、ready 有端口、
/// Claude 还挂在这个端口上、且完整功能模式开着。
fn resume_port(
    runtime_alive: bool,
    ready_port: Option<u16>,
    processes: &str,
    enabled: bool,
) -> Option<u16> {
    if runtime_alive || !enabled {
        return None;
    }
    ready_port.filter(|port| processes_use_proxy(processes, *port))
}

/// 管理器重启后代理随之退出，而 Claude 还指着那个端口：在原端口把代理拉起来，
/// Claude 的重试就能自己连上（换端口它永远连不上）。
pub fn resume_attached_proxy() -> Result<Option<u16>> {
    if !platform_supported() {
        return Ok(None);
    }
    let config = SettingsStore::default().load()?.claude_desktop_local;
    let root = root_dir();
    let Some(port) = resume_port(
        runtime_status().is_some(),
        ready_port(&root),
        &claude_processes()?,
        config.enabled,
    ) else {
        return Ok(None);
    };
    let node = node_runtime().context("未找到支持 fetch 和 WebSocket 的 Node.js")?;
    // 升级 Codex++ 后恢复的代理也要用新版脚本，不能沿用状态目录里的旧副本。
    install_assets(&root)?;
    sync_model_compact_windows(&root, &config);
    start_backend(&root, &node, &config, Some(port)).map(Some)
}

/// `ready.json` 里记的端口：代理进程已经没了也还能用来认出「还挂着旧代理的 Claude」。
fn ready_port(root: &Path) -> Option<u16> {
    let bytes = fs::read(root.join("ready.json")).ok()?;
    let value = serde_json::from_slice::<Value>(&bytes).ok()?;
    value.get("port")?.as_u64()?.try_into().ok()
}

fn parse_ready(value: &Value, instance: &str, pid: u32) -> Option<u16> {
    if value.get("instanceId")?.as_str()? != instance
        || value.get("pid")?.as_u64()? != u64::from(pid)
        || value.get("mode")?.as_str()? != "mock"
    {
        return None;
    }
    value
        .get("port")?
        .as_u64()?
        .try_into()
        .ok()
        .filter(|port| *port != 0)
}

fn read_health(port: u16) -> Result<Value> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_millis(700)))?;
    stream.set_write_timeout(Some(Duration::from_millis(700)))?;
    write!(
        stream,
        "GET {HEALTH_PATH} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;
    let mut bytes = Vec::new();
    stream.take(16 * 1024).read_to_end(&mut bytes)?;
    let header_end = bytes
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .context("本地服务状态响应不完整")?;
    ensure!(
        bytes.starts_with(b"HTTP/1.1 200 ") || bytes.starts_with(b"HTTP/1.0 200 "),
        "本地服务状态异常"
    );
    Ok(serde_json::from_slice(&bytes[header_end + 4..])?)
}

fn runtime_status() -> Option<(u16, String, String, Value)> {
    let mut slot = runtime_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let runtime = slot.as_mut()?;
    if runtime.child.try_wait().ok().flatten().is_some() {
        slot.take();
        return None;
    }
    let port = runtime.port;
    let instance = runtime.instance_id.clone();
    let hash = runtime.config_hash.clone();
    let pid = runtime.child.id();
    drop(slot);
    let health = read_health(port).ok()?;
    (parse_ready(&health, &instance, pid) == Some(port)).then_some((port, instance, hash, health))
}

pub fn status() -> Result<ClaudeLocalStatus> {
    let settings = SettingsStore::default().load()?;
    let config = settings.claude_desktop_local.clone();
    let root = root_dir();
    let source = source_app();
    let processes = claude_processes()?;
    let claude_running = !processes.is_empty();
    let runtime = runtime_status();
    // 完整功能模式直接用真实 profile，无 --user-data-dir；按代理端口识别本地进程。
    let port_hint = runtime
        .as_ref()
        .map(|(port, _, _, _)| *port)
        .or_else(|| ready_port(&root));
    let local_process =
        port_hint.is_some_and(|port| processes_use_proxy(&processes, port));
    let running = runtime.is_some();
    let connected = local_process
        && runtime.as_ref().is_some_and(|(_, _, _, health)| {
            health
                .get("lastClientAt")
                .is_some_and(|last| !last.is_null())
        });
    let previous_launch = fs::read(root.join("launch.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    let changed_since_launch = previous_launch
        .as_ref()
        .and_then(|value| value.get("configHash"))
        .and_then(Value::as_str)
        .is_some_and(|hash| hash != config_hash(&config));
    let needs_restart = (config.enabled && claude_running && (!local_process || !running))
        || runtime
            .as_ref()
            .is_some_and(|(_, _, hash, _)| hash != &config_hash(&config))
        || (!config.enabled && local_process)
        || (claude_running && changed_since_launch);
    let direct = direct_upstream();
    let (models, hidden_models, context_window, no_models) = match direct.as_ref() {
        Some(upstream) => (
            upstream.models.clone(),
            Vec::new(),
            upstream.context_window,
            !upstream.ready(),
        ),
        None => {
            let gateway = GatewayStore::default().load()?.with_extension_providers();
            // 状态里报的是「交给 Claude 的那份列表」，不是网关全量：窗口更小的模型不会出现。
            let binding = local_binding(&gateway, &config).ok();
            let (models, hidden_models) = match binding.as_ref() {
                Some(binding) => exposed_models(&gateway, binding, config.list_all_models),
                None => (available_models(&gateway), Vec::new()),
            };
            let context_window = binding
                .as_ref()
                .and_then(|binding| context_tokens(&gateway, binding, &binding.model));
            let no_models = available_models(&gateway).is_empty();
            (models, hidden_models, context_window, no_models)
        }
    };
    let ready = ca_ready(&root);
    let trusted = ready && ca_trusted(&root);
    let effective_dictation = crate::dictation::effective_settings(&settings);
    let dictation_ready =
        crate::dictation::ensure_local_sensevoice_available(&effective_dictation).is_ok();
    let warning = if let Some(warning) = runtime
        .as_ref()
        .and_then(|(_, _, _, health)| health.get("warning"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        Some(warning.to_string())
    } else if no_models {
        Some(if direct.is_some() {
            "还没有选好上游：先添加 Base URL 和 Key，并选一个默认模型".into()
        } else {
            "供应商池中还没有可用的聊天模型".into()
        })
    } else if config.enabled && !trusted {
        Some("首次使用需要准备并信任本地证书".into())
    } else if config.enabled && config.voice_input && !dictation_ready {
        Some("本地听写尚未就绪，请在语音设置中启用 SenseVoice 并准备模型".into())
    } else {
        fs::read_to_string(root.join("compatibility-warning.txt"))
            .ok()
            .filter(|text| !text.is_empty())
    };
    Ok(ClaudeLocalStatus {
        supported: source_supported(&source),
        version: version(&source),
        source_app: source.to_string_lossy().into_owned(),
        claude_running,
        running,
        local_process,
        connected,
        port: runtime.map(|(port, _, _, _)| port),
        ca_ready: ready,
        ca_trusted: trusted,
        node_available: node_runtime().is_some(),
        dictation_ready,
        config,
        models,
        context_window,
        hidden_models,
        warning,
        needs_restart,
    })
}

pub fn configure(config: ClaudeLocalSettings) -> Result<ClaudeLocalStatus> {
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 本地服务操作正在进行"))?;
    let config = validate_config(config)?;
    save_config(&SettingsStore::default(), config.clone())?;
    status()
}

fn save_config(store: &SettingsStore, config: ClaudeLocalSettings) -> Result<()> {
    let mut settings = store.load()?;
    settings.claude_desktop_local = config;
    store.save(&settings)?;
    Ok(())
}

fn wait_backend_ready(
    child: &mut Child,
    ready_file: &Path,
    instance: &str,
    timeout: Duration,
) -> Result<u16> {
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(
            child.try_wait()?.is_none(),
            "Claude 本地服务提前退出，请查看本地服务日志"
        );
        if let Ok(bytes) = fs::read(ready_file)
            && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            && let Some(port) = parse_ready(&value, instance, child.id())
            && read_health(port)
                .is_ok_and(|health| parse_ready(&health, instance, child.id()) == Some(port))
        {
            return Ok(port);
        }
        ensure!(Instant::now() < deadline, "Claude 本地服务启动超时");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn prepare_dictation(root: &Path, node: &Path) -> Result<()> {
    let cache = root.join(".cache");
    private_dir(&cache)?;
    // 各 profile 组的 electron 缓存目录都要试（Windows 可能 classic + MSIX 多组）；
    // 只取已存在的，都不存在就用主集合的那份保持原语义。
    let sets = profile_sets();
    let mut electron_caches: Vec<PathBuf> = sets
        .iter()
        .map(|set| set.data.join("Cache").join("Cache_Data"))
        .filter(|dir| dir.is_dir())
        .collect();
    if electron_caches.is_empty()
        && let Some(main) = sets.first()
    {
        electron_caches.push(main.data.join("Cache").join("Cache_Data"));
    }
    let mut last_error = None;
    for electron_cache in electron_caches {
        let drain = probe(
            hidden_command(node)
                .arg(root.join("scripts/patch-dictation-drain.mjs"))
                .args(["--apply", "--cache-dir"])
                .arg(&cache)
                .arg("--electron-cache-dir")
                .arg(&electron_cache)
                .env("CLAUDE_LOCAL_MODE", "mock"),
            Duration::from_secs(20),
        );
        let code = "const m=await import(process.argv[1]);m.applyStartupPatches({cacheDir:process.argv[2],electronCacheDir:process.argv[3]});";
        let start = probe(
            hidden_command(node)
                .args(["--input-type=module", "-e", code])
                .arg(
                    reqwest::Url::from_file_path(root.join("scripts/patch-dictation-start.mjs"))
                        .map_err(|_| anyhow::anyhow!("听写资源路径无效"))?
                        .as_str(),
                )
                .arg(&cache)
                .arg(&electron_cache)
                .env("CLAUDE_LOCAL_MODE", "mock"),
            Duration::from_secs(20),
        );
        if drain.is_ok_and(|(success, _)| success) && start.is_ok_and(|(success, _)| success) {
            write_private(&root.join("compatibility-warning.txt"), b"")?;
            return Ok(());
        }
        last_error = Some("当前版本的听写兼容资源尚未准备完成；首次加载或客户端更新后请重新准备。");
    }
    write_private(
        &root.join("compatibility-warning.txt"),
        last_error.unwrap_or_default().as_bytes(),
    )?;
    Ok(())
}

fn start_backend(
    root: &Path,
    node: &Path,
    config: &ClaudeLocalSettings,
    preferred_port: Option<u16>,
) -> Result<u16> {
    if let Some((port, _, hash, _)) = runtime_status()
        && hash == config_hash(config)
    {
        return Ok(port);
    }
    stop_backend();
    let upstream = match direct_upstream() {
        Some(upstream) => {
            ensure!(upstream.ready(), "还没有选好上游：先添加 Base URL 和 Key，并选一个默认模型");
            // 直连不需要网关，但本地听写的转写走常驻 helper（它写 dictation-helper.json 凭据），
            // 开着原生听写时照样把它起起来。
            if config.voice_input {
                let helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
                crate::resident_helper::ensure_resident_helper(helper_port)?;
            }
            LocalUpstream::direct(&upstream)
        }
        None => {
            let helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
            crate::resident_helper::ensure_resident_helper(helper_port)?;
            let (gateway, binding) = bind_gateway(&GatewayStore::default(), config)?;
            let wiring = wiring(&gateway, &binding, AGENT_ID, helper_port);
            let (models, _) = exposed_models(&gateway, &binding, config.list_all_models);
            LocalUpstream {
                model_names: model_names(&gateway, &models),
                model_windows: available_model_windows(&gateway, &binding, config.list_all_models),
                // 选中模型的窗口也单独给一份：逐模型表里没有它时，界面还知道该显示多长，
                // 与启动 Claude Code 时写进 CLAUDE_CODE_MAX_CONTEXT_TOKENS 的值同源。
                default_window: context_tokens(&gateway, &binding, &binding.model)
                    .unwrap_or_default(),
                root: wiring.root,
                token: wiring.token,
                default_model: wiring.default_model,
                small_model: wiring.small_model,
                models,
            }
        }
    };
    let instance = uuid::Uuid::new_v4().to_string();
    let ready_file = root.join("ready.json");
    let log = private_file(&root.join("proxy.log"))?;
    let mut command = hidden_command(node);
    command
        .arg(root.join("proxy.mjs"))
        // 恢复挂接时优先在原端口重启：Claude 只认启动时的 --proxy-server 端口。
        // 请求日志默认关（会记下访问的路径）；排障时在管理器环境里设 CODEX_PLUS_CLAUDE_LOCAL_LOG=1 打开，写进 proxy.log。
        .args([
            "--port",
            &preferred_port.unwrap_or(0).to_string(),
            "--mode",
            "mock",
            "--log",
            if std::env::var("CODEX_PLUS_CLAUDE_LOCAL_LOG").is_ok_and(|value| value == "1") { "1" } else { "0" },
            "--ca",
        ])
        .arg(root.join("ca"))
        .current_dir(root)
        .env("CLAUDE_LOCAL_MANAGED", "1")
        .env(
            "CODEX_PLUS_STATE_DIR",
            crate::paths::default_app_state_dir(),
        )
        .env("CLAUDE_LOCAL_CACHE_DIR", root.join(".cache"))
        .env("CLAUDE_LOCAL_WEB_I18N_DIR", root.join("catalogs"))
        .env("CLAUDE_LOCAL_UPSTREAM_PROXY", upstream_web_proxy().unwrap_or_default())
        .env(
            "CLAUDE_LOCAL_DESKTOP_RESOURCES",
            desktop_resources(&source_app()),
        )
        .env("CLAUDE_LOCAL_GATEWAY_URL", &upstream.root)
        .env("CLAUDE_LOCAL_GATEWAY_KEY", &upstream.token)
        .env("CLAUDE_LOCAL_GATEWAY_MODEL", &upstream.default_model)
        .env("CLAUDE_LOCAL_GATEWAY_SMALL_MODEL", &upstream.small_model)
        .env(
            "CLAUDE_LOCAL_GATEWAY_MODELS",
            serde_json::to_string(&upstream.models)?,
        )
        .env(
            "CLAUDE_LOCAL_GATEWAY_MODEL_NAMES",
            serde_json::to_string(&upstream.model_names)?,
        )
        .env(
            "CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS",
            serde_json::to_string(&upstream.model_windows)?,
        )
        .env(
            "CLAUDE_LOCAL_GATEWAY_DEFAULT_WINDOW",
            upstream.default_window.to_string(),
        )
        .env("CLAUDE_LOCAL_STATE_FILE", root.join("conversations.json"))
        .env("CLAUDE_LOCAL_READY_FILE", &ready_file)
        .env("CLAUDE_LOCAL_INSTANCE_ID", &instance)
        .env("CLAUDE_LOCAL_LOCALIZATION", flag(config.localization))
        .env("CLAUDE_LOCAL_VOICE_INPUT", flag(config.voice_input))
        .env("CLAUDE_LOCAL_COMPUTER_USE", flag(config.computer_use))
        .env("CLAUDE_LOCAL_BACKGROUND_MODE", flag(config.background_mode))
        .env("CLAUDE_LOCAL_HIDE_UNROUTED_MODELS", flag(config.hide_unrouted_models))
        .env("CLAUDE_LOCAL_DISPLAY_NAME", config.effective_display_name())
        .env("CLAUDE_LOCAL_EMAIL", config.effective_email())
        .env("CLAUDE_LOCAL_PLAN", config.normalized_plan());
    if let Some(dir) = recorded_data_dir() {
        command.env("CLAUDE_LOCAL_RECORDED_DIR", dir);
    }
    command
        .stdin(Stdio::piped())
        .stdout(log.try_clone()?)
        .stderr(log);
    let mut child = command.spawn().context("无法启动 Claude 本地兼容服务")?;
    let stdin = child.stdin.take();
    let started = wait_backend_ready(&mut child, &ready_file, &instance, START_TIMEOUT);
    match started {
        Ok(port) => {
            *runtime_slot()
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(BackendRuntime {
                child,
                stdin,
                port,
                instance_id: instance,
                config_hash: config_hash(config),
            });
            Ok(port)
        }
        Err(error) => {
            drop(stdin);
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

/// 本地服务抓 claude.ai 网页（首次写穿缓存的外壳与资源）用的出站代理。
/// 受限地区直连会被 302 到 app-unavailable-in-region；Node 不读系统代理，要显式交给它。
/// 继承的代理变量优先，其次系统代理；只给 `http://` 的 CONNECT 代理（Node 端只会这一种）。
fn upstream_web_proxy() -> Option<String> {
    let inherited = INHERITED_PROXY_VARS
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.trim().is_empty()));
    pick_upstream_web_proxy(
        inherited,
        crate::proxy::detect_system_proxy_for_url("https://claude.ai"),
    )
}

fn pick_upstream_web_proxy(inherited: Option<String>, system: Option<String>) -> Option<String> {
    let usable = |value: String| {
        let value = value.trim().to_string();
        value.to_ascii_lowercase().starts_with("http://").then_some(value)
    };
    inherited.and_then(usable).or_else(|| system.and_then(usable))
}

fn flag(enabled: bool) -> &'static str {
    if enabled { "1" } else { "0" }
}

fn local_launch_args(_root: &Path, port: u16, localization: bool) -> Vec<String> {
    let mut args = vec![
        format!("--proxy-server=http://127.0.0.1:{port}"),
        "--disable-http-cache".into(),
    ];
    if localization {
        args.push("--lang=zh-CN".into());
    }
    args
}


/// 把每个模型的压缩窗口写进 Claude Code 的 `settings.json`。
///
/// Claude Code 支持**逐模型**的压缩设置：`modelSettings.<canonical model>.autoCompactWindow`
/// （100000–1000000，或 `auto`）。它按当前模型解析，所以切模型时会跟着变——正好补上
/// `CLAUDE_CODE_MAX_CONTEXT_TOKENS` 只有一份、进程级的缺口，而且不需要重启。
/// `CLAUDE_CODE_AUTO_COMPACT_WINDOW` 环境变量会盖过它，所以我们不设那个变量。
fn write_model_compact_windows(code_dir: &Path, windows: &BTreeMap<String, u64>) -> Result<()> {
    if windows.is_empty() {
        return Ok(());
    }
    let path = code_dir.join("settings.json");
    let mut settings: Value = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let Some(object) = settings.as_object_mut() else {
        return Ok(());
    };
    let entry = object.entry("modelSettings").or_insert_with(|| json!({}));
    // 用户自己写成了别的形状就不动它。
    let Some(models) = entry.as_object_mut() else {
        return Ok(());
    };
    for (model, window) in windows {
        let clamped = (*window).clamp(100_000, 1_000_000);
        let slot = models.entry(model.clone()).or_insert_with(|| json!({}));
        if let Some(fields) = slot.as_object_mut() {
            fields.insert("autoCompactWindow".into(), json!(clamped));
        }
    }
    crate::settings::atomic_write(&path, &serde_json::to_vec_pretty(&settings)?)?;
    Ok(())
}

/// 按当前网关声明同步一次逐模型窗口；失败不影响启动。
fn sync_model_compact_windows(root: &Path, config: &ClaudeLocalSettings) {
    if let Some(upstream) = direct_upstream() {
        let _ = write_model_compact_windows(&full_code_dir(root), &upstream.model_windows());
        return;
    }
    let Ok(gateway) = GatewayStore::default()
        .load()
        .map(|gateway| gateway.with_extension_providers())
    else {
        return;
    };
    let Ok(binding) = local_binding(&gateway, config) else {
        return;
    };
    let windows = available_model_windows(&gateway, &binding, config.list_all_models);
    let _ = write_model_compact_windows(&full_code_dir(root), &windows);
}

/// 把一个已拼好参数/环境变量的 Command 换成另一个可执行文件（MSIX 别名重试用）。
#[cfg(windows)]
fn clone_command_to(command: &Command, program: &Path) -> Command {
    let mut copy = hidden_command(program);
    copy.args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => {
                copy.env(key, value);
            }
            None => {
                copy.env_remove(key);
            }
        }
    }
    if let Some(dir) = command.get_current_dir() {
        copy.current_dir(dir);
    }
    copy
}

/// spawn 一个 detached 的 Claude 进程并交给等待线程。
///
/// Windows 的 MSIX 包内路径直接 spawn 可能因重解析点权限失败：若 source 是 MSIX
/// 路径且 `%LOCALAPPDATA%\Microsoft\WindowsApps\claude-desktop.exe` 别名存在，
/// 用别名原样重试一次（不用包激活，环境变量才能传进去）。
fn spawn_detached(command: &mut Command, source: &Path, failure: &'static str) -> Result<()> {
    let _ = source;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        Err(error) => {
            #[cfg(windows)]
            {
                let local = std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home_dir().join("AppData").join("Local"));
                let alias = local.join(r"Microsoft\WindowsApps\claude-desktop.exe");
                if crate::claude_localization::is_claude_msix_path(source)
                    && alias != executable(source)
                    && alias.is_file()
                {
                    let mut retry = clone_command_to(command, &alias);
                    retry
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    if let Ok(mut child) = retry.spawn() {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        return Ok(());
                    }
                }
            }
            Err(error).context(failure)
        }
    }
}

/// 完整功能模式启动 Claude 前要清掉的代理变量（大小写两套都清，Windows 下环境变量不分大小写）。
const INHERITED_PROXY_VARS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Desktop 只在环境里没有代理变量时，才把 --proxy-server（本地服务）交给内置 Claude Code；
/// 继承来的 HTTPS_PROXY（系统代理、终端里 export 的）会被原样沿用，
/// Claude Code 就绕过本地服务直连官方，拿本地假令牌被拒 401。
fn strip_inherited_proxy(command: &mut Command) {
    for name in INHERITED_PROXY_VARS {
        command.env_remove(name);
    }
}

fn launch_original(
    source: &Path,
    root: &Path,
    port: Option<u16>,
    localization: bool,
) -> Result<()> {
    let mut command = hidden_command(executable(source));
    for name in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ASAR",
        "NODE_OPTIONS",
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    ] {
        command.env_remove(name);
    }
    if let Some(port) = port
        && let Some(upstream) = direct_upstream()
    {
        // 直连上游：内置 Claude Code 也走本地服务，令牌就是上游的 Key（本地服务按它校验）。
        strip_inherited_proxy(&mut command);
        if let Some(window) = upstream.context_window {
            command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", window.to_string());
        }
        apply_real_profile_one_p(&profile_sets(), root)?;
        let code_dir = full_code_dir(root);
        private_dir(&code_dir)?;
        merge_legacy_full_code_dir(&root.join("claude-code"), &code_dir);
        command
            .args(local_launch_args(root, port, localization))
            .env("CLAUDE_CONFIG_DIR", &code_dir)
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("ANTHROPIC_API_KEY", &upstream.api_key)
            .env("ANTHROPIC_AUTH_TOKEN", &upstream.api_key)
            .env("NO_PROXY", "localhost,127.0.0.1")
            .env("no_proxy", "localhost,127.0.0.1");
    } else if let Some(port) = port {
        strip_inherited_proxy(&mut command);
        let gateway = GatewayStore::default().load()?;
        let binding = gateway
            .active_binding(&ToolId::Unknown(AGENT_ID.into()))
            .context("Claude 本地模式的网关绑定尚未就绪")?
            .clone();
        if let Some(window) = context_tokens(&gateway, &binding, &binding.model) {
            command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", window.to_string());
        }
        // 完整功能模式与 PoC 一致：直接用真实 Claude profile（无 --user-data-dir），
        // 启动前把真实配置切到 1p 并记录回滚信息。
        apply_real_profile_one_p(&profile_sets(), root)?;
        let code_dir = full_code_dir(root);
        private_dir(&code_dir)?;
        merge_legacy_full_code_dir(&root.join("claude-code"), &code_dir);
        // 与 3p 路径一致：把 Codex++ 配置的每模型窗口/推理档传给内置 Claude Code。
        let helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
        let wiring = wiring(&gateway, &binding, AGENT_ID, helper_port);
        let token = wiring.token.clone();
        command
            .args(local_launch_args(root, port, localization))
            .env("CLAUDE_CONFIG_DIR", &code_dir)
            .env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{port}"))
            .env("ANTHROPIC_API_KEY", &token)
            .env("ANTHROPIC_AUTH_TOKEN", &token)
            .env("NO_PROXY", "localhost,127.0.0.1")
            .env("no_proxy", "localhost,127.0.0.1");
        if let Some(capabilities) =
            crate::gateway::agents::catalog_apps::desktop_capabilities(&wiring)
        {
            command.env("CLAUDE_CODE_MODEL_CAPABILITIES", capabilities);
        }
    }
    spawn_detached(&mut command, source, "无法启动原版 Claude Desktop")?;
    Ok(())
}

fn launch_three_p(source: &Path, root: &Path, config: &ClaudeLocalSettings) -> Result<()> {
    stop_backend();
    // 切回 3p 前先恢复真实 profile（enable_binding 会重写 configLibrary，顺序不能反）。
    restore_real_profile(&profile_sets(), root)?;
    GatewayStore::default().update(|gateway| {
        if let Some(binding) = gateway.bindings.get_mut(AGENT_ID) {
            binding.enabled = false;
        }
        Ok(())
    })?;
    let port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
    crate::resident_helper::ensure_resident_helper(port)?;
    let context = crate::gateway::apply::ApplyContext::default_paths(port);
    let agent = ToolId::Unknown("claude-desktop".into());
    let gateway = context.gateway.load()?.with_extension_providers();
    let mut binding = gateway.binding(&agent).cloned().unwrap_or_default();
    binding.pool_exclude.clear();
    binding.extra_models = available_models(&gateway);
    // 与完整功能模式同一条规则：绑定里选过就用它，配置里的旧值只作兜底。
    binding.model = chosen_model(&gateway, &binding, config);
    let default_model = crate::gateway::route::binding_default_model(&gateway, &binding);
    ensure!(
        !default_model.is_empty()
            && crate::gateway::route::model_is_routable(&gateway, &binding, &default_model),
        "请先在供应商池中添加可用模型，并为 Claude 选择模型"
    );
    binding.model = default_model;
    let gateway = crate::gateway::apply::enable_binding(&context, &agent, binding)?;
    let binding = gateway
        .binding(&agent)
        .context("Claude Desktop 网关配置缺失")?;
    let wiring = wiring(&gateway, binding, "claude-desktop", port);
    private_dir(root)?;
    private_dir(&root.join("claude-code-3p"))?;
    let mut command = hidden_command(executable(source));
    for name in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ASAR",
        "NODE_OPTIONS",
        "CLAUDE_USER_DATA_DIR",
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    ] {
        command.env_remove(name);
    }
    if let Some(window) = context_tokens(&gateway, binding, &binding.model) {
        command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", window.to_string());
    }
    command
        .env("CLAUDE_CONFIG_DIR", root.join("claude-code-3p"))
        .env("ANTHROPIC_BASE_URL", &wiring.root)
        .env("ANTHROPIC_API_KEY", &wiring.token)
        .env("ANTHROPIC_AUTH_TOKEN", &wiring.token)
        .env("NO_PROXY", "localhost,127.0.0.1")
        .env("no_proxy", "localhost,127.0.0.1");
    if let Some(capabilities) = crate::gateway::agents::catalog_apps::desktop_capabilities(&wiring)
    {
        command.env("CLAUDE_CODE_MODEL_CAPABILITIES", capabilities);
    }
    spawn_detached(&mut command, source, "无法启动原版 Claude Desktop 的 3p 模式")?;
    Ok(())
}

/// 只停止 Codex++ 自己持有的代理进程，不关闭 Claude 或其它实例。
pub fn stop_backend() {
    let runtime = runtime_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(mut runtime) = runtime {
        runtime.stdin.take();
        terminate_child(&mut runtime.child, Duration::from_secs(2));
    }
    let root = root_dir();
    reap_stale_proxy(&root);
    reap_orphan_proxies(&root);
}

/// 先礼后兵地结束一个子进程。
fn terminate_child(child: &mut Child, grace: Duration) {
    let deadline = Instant::now() + grace;
    while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// 回收上一次运行留下的代理进程。
///
/// 应用崩溃或重启后 `runtime_slot` 是空的，只有 `ready.json` 还记着它的 pid：
/// 不回收就会一直占着端口和内存，还带着旧的 token 与同一个本地 CA 在跑。
pub fn reap_stale_proxy(root: &Path) {
    let Ok(bytes) = fs::read(root.join("ready.json")) else {
        return;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return;
    };
    let Some(pid) = value.get("pid").and_then(Value::as_u64) else {
        return;
    };
    let Ok(pid) = u32::try_from(pid) else { return };
    if pid == 0 || pid == std::process::id() {
        return;
    }
    if !is_our_proxy(pid, root) {
        return;
    }
    terminate_pid(pid, root);
}

/// TERM → 最多等 2 秒（用 `is_our_proxy` 探活）→ 仍活着就 KILL。
/// Windows 没有 TERM/KILL 之分，直接 TerminateProcess 后最多等 2 秒确认。
fn terminate_pid(pid: u32, root: &Path) {
    #[cfg(windows)]
    {
        let _ = root;
        crate::windows_integration::terminate_process(pid);
        let deadline = Instant::now() + Duration::from_secs(2);
        while process_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        return;
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("/bin/kill")
            .args(["-TERM", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let deadline = Instant::now() + Duration::from_secs(2);
        while is_our_proxy(pid, root) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if is_our_proxy(pid, root) {
            let _ = Command::new("/bin/kill")
                .args(["-KILL", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// 回收 `ready.json` 已经不记的遗留代理：重启前没回收成功的会一直占着端口和内存。
/// 只收本目录（root）下的 `proxy.mjs`，排除自己和当前正在跑的 runtime 代理。
#[cfg(target_os = "macos")]
fn reap_orphan_proxies(root: &Path) {
    let output = Command::new("/usr/bin/pgrep")
        .args(["-f", "proxy.mjs"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output();
    let Ok(output) = output else { return };
    let current_child = runtime_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|runtime| runtime.child.id());
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(pid) = line.trim().parse::<u32>() else {
            continue;
        };
        if pid == 0 || pid == std::process::id() || Some(pid) == current_child {
            continue;
        }
        if is_our_proxy(pid, root) {
            terminate_pid(pid, root);
        }
    }
}

#[cfg(windows)]
fn reap_orphan_proxies(root: &Path) {
    let current_child = runtime_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|runtime| runtime.child.id());
    for info in crate::windows_integration::enumerate_processes() {
        if !info.exe_file.eq_ignore_ascii_case("node.exe") {
            continue;
        }
        let pid = info.process_id;
        if pid == 0 || pid == std::process::id() || Some(pid) == current_child {
            continue;
        }
        if is_our_proxy(pid, root) {
            terminate_pid(pid, root);
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn reap_orphan_proxies(_root: &Path) {}

/// pid 还活着，且命令行就是本目录的 `proxy.mjs`（与 `ready.json` 里的旧记录核对）。
fn is_our_proxy(pid: u32, root: &Path) -> bool {
    #[cfg(windows)]
    {
        return crate::windows_integration::process_command_line(pid).is_some_and(|command| {
            command.contains("proxy.mjs")
                && command.contains(&root.to_string_lossy().to_string())
        });
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "command="])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output();
        let Ok(output) = output else { return false };
        let command = String::from_utf8_lossy(&output.stdout);
        command.contains("proxy.mjs") && command.contains(&root.to_string_lossy().to_string())
    }
}

pub fn perform(action: &str) -> Result<ClaudeLocalResult> {
    ensure!(
        platform_supported(),
        "原版 Claude 本地模式目前仅支持 macOS 和 Windows"
    );
    ensure!(
        ["launch", "stop", "trust", "untrust", "prepare", "sync_sessions"].contains(&action),
        "未知的 Claude 本地服务操作"
    );
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 本地服务操作正在进行"))?;
    let config = SettingsStore::default().load()?.claude_desktop_local;
    let root = root_dir();
    let source = source_app();
    let mut sync_summary = String::new();
    let mut message = match action {
        "stop" => {
            let port = runtime_status()
                .map(|(port, _, _, _)| port)
                .or_else(|| ready_port(&root));
            stop_backend();
            // 先让 Claude 退出，再还原 profile：Claude 退出时还会回写自己的配置。
            let quit = port.map(quit_claude_with_proxy).transpose()?.unwrap_or(false);
            restore_real_profile(&profile_sets(), &root)?;
            if quit {
                "Claude 本地服务已停止，经它启动的 Claude 已退出。".to_string()
            } else {
                "Claude 本地服务已停止。".to_string()
            }
        }
        "trust" => {
            install_assets(&root)?;
            prepare_ca(&root)?;
            trust_ca(&root)?;
            "本地证书已信任。".to_string()
        }
        "untrust" => {
            untrust_ca(&root)?;
            "本地证书已取消信任。".to_string()
        }
        "prepare" => {
            install_assets(&root)?;
            prepare_ca(&root)?;
            if config.voice_input {
                let node = node_runtime().context("未找到支持 fetch 和 WebSocket 的 Node.js")?;
                prepare_dictation(&root, &node)?;
            }
            "Claude 本地兼容资源已准备，原版应用未修改。".to_string()
        }
        "sync_sessions" => {
            ensure!(
                claude_processes()?.is_empty(),
                "请先完全退出 Claude 再同步会话"
            );
            let sets = profile_sets();
            let main = sets.first().context("无法定位 Claude 的 profile 目录")?;
            let report = crate::claude_session_sync::sync_sessions(
                &main.three_p.join("claude-code-sessions"),
                &[root.join("claude-code-3p"), home_dir().join(".claude")],
                &main.data.join("claude-code-sessions"),
                &[full_code_dir(&root), home_dir().join(".claude")],
                &root.join("backups/session-sync"),
            )?;
            if !report.note.is_empty() {
                report.note.clone()
            } else {
                format!(
                    "已同步会话：3p→完整 {} 个，完整→3p {} 个，更新 {} 个，跳过已删除 {} 个。",
                    report.to_full, report.to_three_p, report.updated, report.skipped_deleted
                )
            }
        }
        "launch" => {
            ensure!(
                source_supported(&source),
                "未安装 Claude Desktop，请先安装官方应用"
            );
            ensure!(
                claude_processes()?.is_empty(),
                "Claude 正在运行，请先完全退出，再从 Codex++ 启动以应用本地模式"
            );
            if config.session_sync {
                let sets = profile_sets();
                let main = sets.first().context("无法定位 Claude 的 profile 目录")?;
                match crate::claude_session_sync::sync_sessions(
                    &main.three_p.join("claude-code-sessions"),
                    &[root.join("claude-code-3p"), home_dir().join(".claude")],
                    &main.data.join("claude-code-sessions"),
                    &[full_code_dir(&root), home_dir().join(".claude")],
                    &root.join("backups/session-sync"),
                ) {
                    Ok(report) if report.to_full + report.to_three_p + report.updated > 0 => {
                        sync_summary = format!(
                            "已同步会话：3p→完整 {} 个，完整→3p {} 个，更新 {} 个。",
                            report.to_full, report.to_three_p, report.updated
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "claude_session_sync.launch_failed",
                            serde_json::json!({"error": format!("{error:#}")}),
                        );
                    }
                }
            }
            ensure!(
                config.enabled || direct_upstream().is_none(),
                "直连上游只支持完整功能模式"
            );
            if config.enabled {
                install_assets(&root)?;
                prepare_ca(&root)?;
                ensure!(
                    ca_trusted(&root),
                    "请先点击“信任本地证书”，完成系统授权后再启动"
                );
                let node = node_runtime().context("未找到支持 fetch 和 WebSocket 的 Node.js")?;
                if config.voice_input {
                    prepare_dictation(&root, &node)?;
                }
                sync_model_compact_windows(&root, &config);
                let port = start_backend(&root, &node, &config, None)?;
                if let Err(error) = launch_original(&source, &root, Some(port), config.localization)
                {
                    stop_backend();
                    return Err(error);
                }
                "已通过本地兼容服务启动原版 Claude Desktop。".to_string()
            } else {
                launch_three_p(&source, &root, &config)?;
                "已通过 Codex++ 网关启动原版 Claude Desktop 的 3p 模式。".to_string()
            }
        }
        _ => unreachable!(),
    };
    if action == "launch" {
        write_private(
            &root.join("launch.json"),
            &serde_json::to_vec(
                &json!({"configHash": config_hash(&config), "localMode": config.enabled}),
            )?,
        )?;
    }
    if !sync_summary.is_empty() {
        message = format!("{sync_summary}{message}");
    }
    Ok(ClaudeLocalResult {
        status: status()?,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_web_proxy_prefers_inherited_then_system_and_only_http() {
        let some = |value: &str| Some(value.to_string());
        assert_eq!(pick_upstream_web_proxy(some("http://10.0.0.1:8080"), some("http://127.0.0.1:7897")), some("http://10.0.0.1:8080"));
        assert_eq!(pick_upstream_web_proxy(None, some(" http://127.0.0.1:7897 ")), some("http://127.0.0.1:7897"));
        // socks 代理 Node 端不支持：跳过继承的，退回系统代理；都不行就直连。
        assert_eq!(pick_upstream_web_proxy(some("socks5://127.0.0.1:7898"), some("http://127.0.0.1:7897")), some("http://127.0.0.1:7897"));
        assert_eq!(pick_upstream_web_proxy(some("socks5://127.0.0.1:7898"), None), None);
        assert_eq!(pick_upstream_web_proxy(None, None), None);
    }

    /// Desktop 更新（app.asar 变了）算配置变化：本地服务要重启才能换上新的官方页面。
    #[test]
    fn desktop_update_changes_the_config_hash() {
        let config = ClaudeLocalSettings::default();
        let before = hash_with_desktop(&config, None, Some((100, 1)));
        assert_eq!(before, hash_with_desktop(&config, None, Some((100, 1))));
        assert_ne!(before, hash_with_desktop(&config, None, Some((100, 2))), "修改时间变了");
        assert_ne!(before, hash_with_desktop(&config, None, Some((101, 1))), "大小变了");
    }

    /// 直连上游：Base URL 与 ANTHROPIC_BASE_URL 同语义（去掉结尾斜杠），默认模型排第一且去重，
    /// 小模型缺省用默认模型；没有窗口就不声明。（不碰全局开关，免得影响并行用例。）
    #[test]
    fn direct_upstream_normalizes_models_and_windows() {
        let upstream = DirectUpstream {
            base_url: " https://api.example.com/ ".into(),
            api_key: " sk-test ".into(),
            model: "claude-opus-4-6".into(),
            small_model: String::new(),
            models: vec!["glm-5".into(), "claude-opus-4-6".into(), " ".into(), "glm-5".into()],
            context_window: Some(0),
            model_names: BTreeMap::from([("glm-5".to_string(), "GLM 5 · 智谱".to_string())]),
        }
        .normalized();
        assert_eq!(upstream.base_url, "https://api.example.com");
        assert_eq!(upstream.api_key, "sk-test");
        assert_eq!(upstream.small_model, "claude-opus-4-6");
        assert_eq!(upstream.models, ["claude-opus-4-6", "glm-5"]);
        assert_eq!(upstream.context_window, None);
        assert!(upstream.model_windows().is_empty());
        assert!(upstream.ready());

        let wide = DirectUpstream { context_window: Some(1_000_000), ..upstream.clone() };
        let local = LocalUpstream::direct(&wide);
        assert_eq!(local.root, "https://api.example.com");
        assert_eq!(local.token, "sk-test");
        assert_eq!(local.default_window, 1_000_000);
        assert_eq!(local.model_windows.get("glm-5"), Some(&1_000_000));
        assert_eq!(local.model_names["claude-opus-4-6"]["name"], friendly_model_name("claude-opus-4-6"));
        assert_eq!(local.model_names["glm-5"]["name"], "GLM 5 · 智谱", "上游给了显示名就用它");

        assert!(!DirectUpstream::default().normalized().ready(), "没有 Base URL 和模型不能启动");
    }

    fn provider(id: &str, models: &str) -> crate::gateway::store::GatewayProvider {
        serde_json::from_value(json!({
            "relay": {
                "id": id,
                "name": id,
                "modelList": models,
            },
        }))
        .unwrap()
    }

    /// 共享会话目录：旧位置的会话记录补进共享目录，已有的文件不覆盖，原文件保留。
    #[test]
    fn legacy_full_code_dir_merges_missing_files_only() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("state/claude-local/claude-code");
        let shared = temp.path().join("shared/claude-code");
        fs::create_dir_all(legacy.join("projects/p1")).unwrap();
        fs::write(legacy.join("projects/p1/a.jsonl"), "from-codex").unwrap();
        fs::write(legacy.join("projects/p1/b.jsonl"), "old-b").unwrap();
        fs::create_dir_all(shared.join("projects/p1")).unwrap();
        fs::write(shared.join("projects/p1/b.jsonl"), "newer-b").unwrap();

        merge_legacy_full_code_dir(&legacy, &shared);
        assert_eq!(fs::read_to_string(shared.join("projects/p1/a.jsonl")).unwrap(), "from-codex");
        assert_eq!(fs::read_to_string(shared.join("projects/p1/b.jsonl")).unwrap(), "newer-b", "不覆盖");
        assert!(legacy.join("projects/p1/a.jsonl").exists(), "原文件保留");
        // 不存在的旧目录、同一目录都直接跳过。
        merge_legacy_full_code_dir(&temp.path().join("missing"), &shared);
        merge_legacy_full_code_dir(&shared, &shared);
    }

    #[test]
    fn model_compact_windows_merge_into_settings_and_clamp() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("claude-code");
        fs::create_dir_all(&dir).unwrap();
        // 已有设置（含用户自己的键）必须保留
        fs::write(dir.join("settings.json"), br#"{"theme":"dark","modelSettings":{"other":{"autoCompactWindow":300000}}}"#).unwrap();

        let windows = BTreeMap::from([
            ("relay/gpt-6.1-sol".to_string(), 1_100_000),
            ("relay/small".to_string(), 128_000),
        ]);
        write_model_compact_windows(&dir, &windows).unwrap();

        let settings: Value =
            serde_json::from_slice(&fs::read(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["modelSettings"]["other"]["autoCompactWindow"], 300_000);
        assert_eq!(settings["modelSettings"]["relay/gpt-6.1-sol"]["autoCompactWindow"], 1_000_000);
        assert_eq!(settings["modelSettings"]["relay/small"]["autoCompactWindow"], 128_000);
    }

    #[test]
    fn friendly_model_names_read_like_product_names() {
        assert_eq!(friendly_model_name("gpt-6.1-sol"), "GPT-6.1 Sol");
        assert_eq!(friendly_model_name("gpt-5.6-terra"), "GPT-5.6 Terra");
        assert_eq!(friendly_model_name("deepseek-v4-flash"), "DeepSeek V4 Flash");
        assert_eq!(friendly_model_name("kimi-k3"), "Kimi K3");
        assert_eq!(friendly_model_name("glm-5.3"), "GLM-5.3");
        // Claude 系用连字符分版本号：还原成点；日期段照旧空格分隔
        assert_eq!(friendly_model_name("claude-opus-4-6"), "Claude Opus 4.6");
        assert_eq!(friendly_model_name("claude-opus-5-5"), "Claude Opus 5.5");
        assert_eq!(friendly_model_name("claude-fable-5"), "Claude Fable 5");
        assert_eq!(friendly_model_name("claude-fable-5-1"), "Claude Fable 5.1");
        assert_eq!(friendly_model_name("claude-haiku-4-5-20251001"), "Claude Haiku 4.5 20251001");
        assert_eq!(friendly_model_name("claude-3-5-sonnet-20241022"), "Claude 3.5 Sonnet 20241022");
        // 不认识的词保持原样，不会凭空变形
        assert_eq!(friendly_model_name("gpt-image-2-4k"), "GPT Image 2 4k");
    }

    #[test]
    fn internal_review_models_are_not_offered_as_chat_models() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![provider("openai", "gpt-6\ncodex-auto-review")];
        assert_eq!(available_models(&gateway), vec!["openai/gpt-6"]);
    }

    /// mac 形状的单组 sets，与 `profile_sets_for(false, …)` 一致。
    fn mac_sets_at(support: &Path) -> Vec<ProfileSet> {
        vec![ProfileSet {
            data: support.join("Claude"),
            mode: support.join("Claude"),
            three_p: support.join("Claude-3p"),
        }]
    }

    fn orphan_fixture(temp: &tempfile::TempDir) -> (Vec<ProfileSet>, PathBuf, PathBuf, PathBuf) {
        let support = temp.path().join("support");
        let root = temp.path().join("state");
        let sets = mac_sets_at(&support);
        let target = support.join("Claude-3p/configLibrary");
        let backup = target.with_file_name(CONFIG_LIBRARY_BACKUP);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&support).unwrap();
        (sets, root, target, backup)
    }

    #[test]
    fn orphan_config_library_backup_is_put_back_when_target_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, target, backup) = orphan_fixture(&temp);
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("00000000.json"), b"{}").unwrap();

        let note = recover_orphan_config_library(&sets, &root).unwrap();
        assert!(note.is_some(), "收尾之后要给用户一句说明");
        assert!(target.join("00000000.json").is_file(), "目标不在就把备份放回去");
        assert!(!backup.exists());
    }

    #[test]
    fn orphan_config_library_backup_is_archived_when_target_exists() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, target, backup) = orphan_fixture(&temp);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("fresh.json"), b"{}").unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("old.json"), b"{}").unwrap();

        recover_orphan_config_library(&sets, &root).unwrap();
        assert!(target.join("fresh.json").is_file(), "Claude 重建的那份保持不动");
        assert!(!backup.exists(), "遗留备份不再卡住启动");
        let archived: Vec<_> = fs::read_dir(root.join("backups")).unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(archived.len(), 1);
        assert!(archived[0].starts_with("configLibrary-orphan-"), "{archived:?}");
    }

    #[test]
    fn tracked_config_library_backup_is_left_to_the_restore_step() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, target, backup) = orphan_fixture(&temp);
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&backup).unwrap();
        crate::settings::atomic_write(
            &switch_state_path(&root),
            &serde_json::to_vec_pretty(&json!({
                "applied": true,
                "deploymentMode": Value::Object(serde_json::Map::new()),
                "configLibraryMoved": true,
                "configLibraryBackup": backup.to_string_lossy(),
            }))
            .unwrap(),
        )
        .unwrap();

        assert_eq!(recover_orphan_config_library(&sets, &root).unwrap(), None);
        assert!(backup.exists(), "正常流程里的备份交给 restore_real_profile");
    }

    #[test]
    fn one_p_switch_no_longer_fails_on_a_leftover_backup() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, target, backup) = orphan_fixture(&temp);
        for app in ["Claude", "Claude-3p"] {
            let dir = temp.path().join("support").join(app);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("claude_desktop_config.json"), b"{\"mcpServers\":{}}").unwrap();
        }
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&backup).unwrap();

        apply_real_profile_one_p(&sets, &root).unwrap();
        assert!(switch_state_path(&root).is_file(), "这次要正常记下切换状态");
        assert!(backup.exists() && !target.exists(), "configLibrary 已挪开");
        let config: Value = serde_json::from_slice(
            &fs::read(temp.path().join("support/Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["deploymentMode"], json!("1p"));
    }

    #[test]
    fn local_mode_follows_the_claude_desktop_binding_model() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("relay", "gpt-6-astra\ngpt-6.1-sol"),
        ];
        gateway.bindings.insert(
            "claude-desktop".into(),
            AgentBinding {
                model: "relay/gpt-6.1-sol".into(),
                ..AgentBinding::default()
            },
        );
        gateway.bindings.insert(
            AGENT_ID.into(),
            AgentBinding {
                model: "relay/gpt-6-astra".into(),
                ..AgentBinding::default()
            },
        );
        let binding = local_binding(&gateway, &ClaudeLocalSettings::default()).unwrap();
        assert_eq!(
            binding.model, "relay/gpt-6.1-sol",
            "用户在「Agent 接入」的 Claude Desktop 行选的模型说了算"
        );
    }

    #[test]
    fn agent_binding_model_wins_over_legacy_local_model() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("xai", "grok-4"),
        ];
        // 「Agent 接入」里已经给 Claude 选过模型：本地模式跟着它走。
        gateway.bindings.insert(
            AGENT_ID.into(),
            AgentBinding {
                model: "anthropic/claude-sonnet-4-6".into(),
                ..AgentBinding::default()
            },
        );
        let binding = local_binding(
            &gateway,
            &ClaudeLocalSettings {
                model: "xai/grok-4".into(),
                ..ClaudeLocalSettings::default()
            },
        )
        .unwrap();
        assert_eq!(binding.model, "anthropic/claude-sonnet-4-6");
    }

    #[test]
    fn exposed_models_hide_windows_smaller_than_the_selected_one() {
        let mut gateway = GatewaySettings::default();
        let mut relay = provider("relay", "gpt-6-astra\nsmall-model");
        relay.model_windows.insert("gpt-6-astra".into(), 1_100_000);
        relay.model_windows.insert("small-model".into(), 128_000);
        gateway.providers.push(relay);

        let binding = AgentBinding {
            model: "relay/gpt-6-astra".into(),
            ..AgentBinding::default()
        };
        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert_eq!(visible, vec!["relay/gpt-6-astra"]);
        assert_eq!(hidden, vec!["relay/small-model"], "小窗口模型不交给 Claude");

        // 默认模型自己没声明窗口时不过滤：Claude 那边按 200k 兜底，也不会更差。
        let binding = AgentBinding {
            model: "relay/unknown-model".into(),
            ..AgentBinding::default()
        };
        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert!(hidden.is_empty());
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn list_all_models_skips_the_window_filter() {
        let mut gateway = GatewaySettings::default();
        let mut relay = provider("relay", "gpt-6-astra\nsmall-model");
        relay.model_windows.insert("gpt-6-astra".into(), 1_100_000);
        relay.model_windows.insert("small-model".into(), 128_000);
        gateway.providers.push(relay);
        let binding = AgentBinding {
            model: "relay/gpt-6-astra".into(),
            ..AgentBinding::default()
        };

        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert_eq!(visible, vec!["relay/gpt-6-astra"]);
        assert_eq!(hidden, vec!["relay/small-model"]);

        // 显式要求全列：小窗口模型也交给 Claude（界面自己提示风险）
        let (visible, hidden) = exposed_models(&gateway, &binding, true);
        assert_eq!(visible.len(), 2);
        assert!(hidden.is_empty());
    }

    #[test]
    fn all_pool_models_keep_provider_identity_for_each_family() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("xai", "grok-4"),
            provider("openai", "gpt-6\nshared-model"),
            provider("relay", "shared-model"),
        ];
        assert_eq!(
            available_models(&gateway),
            vec![
                "anthropic/claude-sonnet-4-6",
                "xai/grok-4",
                "openai/gpt-6",
                "openai/shared-model",
                "relay/shared-model",
            ]
        );
        let snapshot = gateway.clone();
        let binding = local_binding(
            &gateway,
            &ClaudeLocalSettings {
                model: "xai/grok-4".into(),
                ..ClaudeLocalSettings::default()
            },
        )
        .unwrap();
        assert_eq!(binding.model, "xai/grok-4");
        assert_eq!(binding.extra_models, available_models(&gateway));
        assert!(binding.enabled && !binding.token.is_empty());
        assert_eq!(
            gateway, snapshot,
            "合成本地绑定不应写应用配置或修改其它网关绑定"
        );
        gateway.providers[1].enabled = false;
        assert!(
            local_binding(
                &gateway,
                &ClaudeLocalSettings {
                    model: "xai/grok-4".into(),
                    ..ClaudeLocalSettings::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn selected_model_window_prefers_provider_metadata_and_hash_tracks_changes() {
        let mut gateway = GatewaySettings::default();
        let mut provider = provider("openai", "gpt-6\nsmall-model");
        provider.model_windows.insert("gpt-6".into(), 1_100_000);
        gateway.providers.push(provider);
        let binding = AgentBinding {
            model: "openai/gpt-6".into(),
            context_window: Some(200_000),
            ..AgentBinding::default()
        };

        assert_eq!(
            selected_model_window(&gateway, &binding, &binding.model),
            Some(1_100_000)
        );
        assert_eq!(
            selected_model_window(&gateway, &binding, "openai/small-model"),
            Some(200_000)
        );

        let config = ClaudeLocalSettings::default();
        let models = vec!["openai/gpt-6".into()];
        let bound = Some("openai/gpt-6".to_string());
        let initial = config_hash_with_gateway_state(
            &config,
            Some((
                models.clone(),
                BTreeMap::from([("openai/gpt-6".into(), 1_100_000)]),
                bound.clone(),
            )),
        );
        let changed_window = config_hash_with_gateway_state(
            &config,
            Some((
                models.clone(),
                BTreeMap::from([("openai/gpt-6".into(), 300_000)]),
                bound.clone(),
            )),
        );
        assert_ne!(initial, changed_window, "窗口变了要重启代理");
        let changed_model = config_hash_with_gateway_state(
            &config,
            Some((
                models,
                BTreeMap::from([("openai/gpt-6".into(), 1_100_000)]),
                Some("openai/small-model".to_string()),
            )),
        );
        assert_ne!(initial, changed_model, "「Agent 接入」换了模型也要重启代理");
    }

    #[test]
    fn context_tokens_prefers_gateway_window_then_inherited_value() {
        assert_eq!(
            pick_context_tokens(Some(1_100_000), Some(200_000)),
            Some(1_100_000),
            "网关声明压过环境里的旧值"
        );
        assert_eq!(
            pick_context_tokens(None, Some(1_000_000)),
            Some(1_000_000),
            "网关没声明时沿用用户自己设的窗口"
        );
        assert_eq!(pick_context_tokens(None, None), None, "都没有就不设，交给 Claude Code");
        assert_eq!(pick_context_tokens(Some(0), Some(0)), None, "0 不是窗口");
        assert_eq!(pick_context_tokens(Some(0), Some(272_000)), Some(272_000));
    }

    #[test]
    fn assets_digest_is_stable_and_part_of_the_config_hash() {
        let digest = assets_digest();
        assert_eq!(digest.len(), 64, "sha256 十六进制");
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(digest, assets_digest(), "同一份资产必须给出同一个指纹");

        let config = ClaudeLocalSettings::default();
        let hash = config_hash_with_gateway_state(&config, None);
        let without_assets = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(
                    &config,
                    Option::<(Vec<String>, BTreeMap<String, u64>, Option<String>)>::None
                ))
                .unwrap_or_default()
            )
        );
        assert_ne!(hash, without_assets, "资产指纹要参与代理复用判断");
    }

    #[test]
    fn saving_mode_changes_preserves_enhanced_copy_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(directory.path().join("settings.json"));
        let mut settings = crate::settings::BackendSettings::default();
        settings.claude_app_localization = false;
        settings.claude_app_voice_input = false;
        settings.claude_app_computer_use = true;
        settings.claude_app_extensions = true;
        store.save(&settings).unwrap();
        for enabled in [true, false] {
            let config = ClaudeLocalSettings {
                enabled,
                model: "relay/model".into(),
                ..ClaudeLocalSettings::default()
            };
            save_config(&store, config.clone()).unwrap();
            let saved = store.load().unwrap();
            assert_eq!(saved.claude_desktop_local, config);
            assert!(!saved.claude_app_localization && !saved.claude_app_voice_input);
            assert!(saved.claude_app_computer_use && saved.claude_app_extensions);
        }
    }

    #[test]
    fn backend_readiness_checks_http_identity_and_timeout() {
        let Some(node) = crate::dsh_plugins::locate_node() else {
            return;
        };
        let directory = tempfile::tempdir().unwrap();
        let ready_file = directory.path().join("ready.json");
        let fixture = r#"
const fs=require('node:fs'),http=require('node:http');
const receipt={instanceId:'fixture',pid:process.pid,mode:'mock'};
const server=http.createServer((req,res)=>{
  const body=JSON.stringify({...receipt,instanceId:process.argv[2]});
  res.writeHead(200,{'content-type':'application/json','content-length':Buffer.byteLength(body)});
  res.end(body);
});
server.listen(0,'127.0.0.1',()=>{
  receipt.port=server.address().port;
  fs.writeFileSync(process.argv[1],JSON.stringify(receipt));
});
"#;
        for identity in ["fixture", "stale-instance"] {
            let mut child = Command::new(&node)
                .args(["-e", fixture])
                .arg(&ready_file)
                .arg(identity)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let outcome = wait_backend_ready(
                &mut child,
                &ready_file,
                "fixture",
                if identity == "fixture" {
                    Duration::from_secs(3)
                } else {
                    Duration::from_millis(250)
                },
            );
            let _ = child.kill();
            let _ = child.wait();
            if identity == "fixture" {
                assert!(outcome.is_ok(), "当前进程和 HTTP 身份均匹配时应就绪");
            } else {
                assert!(outcome.unwrap_err().to_string().contains("超时"));
            }
        }
    }

    #[test]
    fn local_mode_defaults_preserve_original_launch() {
        let config: ClaudeLocalSettings = serde_json::from_value(json!({})).unwrap();
        assert!(!config.enabled);
        assert!(config.localization && config.voice_input);
        assert!(!config.computer_use && !config.background_mode);
        let settings: crate::settings::BackendSettings = serde_json::from_value(json!({})).unwrap();
        assert_eq!(settings.claude_desktop_local, config);
    }

    #[test]
    fn local_settings_serialize_independently_from_enhanced_copy() {
        let config = validate_config(ClaudeLocalSettings {
            enabled: true,
            model: "  relay/model  ".into(),
            background_mode: true,
            ..ClaudeLocalSettings::default()
        })
        .unwrap();
        assert_eq!(config.model, "relay/model");
        assert!(config.computer_use);
        let value = serde_json::to_value(config).unwrap();
        assert_eq!(value["voiceInput"], true);
        assert_eq!(value["backgroundMode"], true);
        assert!(
            validate_config(ClaudeLocalSettings {
                model: "model\ninjected".into(),
                ..ClaudeLocalSettings::default()
            })
            .is_err()
        );
    }

    #[test]
    fn ready_receipt_requires_current_process_instance_and_port() {
        let valid = json!({"instanceId":"current","pid":123,"mode":"mock","port":49123});
        assert_eq!(parse_ready(&valid, "current", 123), Some(49123));
        assert_eq!(parse_ready(&valid, "old", 123), None);
        assert_eq!(parse_ready(&valid, "current", 999), None);
        for port in [0, 65536] {
            let mut value = valid.clone();
            value["port"] = json!(port);
            assert_eq!(parse_ready(&value, "current", 123), None);
        }
    }

    #[test]
    fn local_launch_is_isolated_without_copy_or_integrity_flags() {
        let args = local_launch_args(Path::new("/state/claude local"), 45123, true);
        assert!(args.contains(&"--proxy-server=http://127.0.0.1:45123".to_string()));
        // 完整功能模式直接使用真实 Claude profile，不传 --user-data-dir
        assert!(!args.iter().any(|arg| arg.starts_with("--user-data-dir")));
        assert!(args.contains(&"--lang=zh-CN".to_string()));
        let no_localization = local_launch_args(Path::new("/state/claude local"), 45123, false);
        assert!(!no_localization.iter().any(|arg| arg.starts_with("--lang=")));
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("inspect") || arg.contains("disable-web-security"))
        );
    }

    #[test]
    fn real_profile_switch_round_trips_deployment_mode_and_config_library() {
        let dir = tempfile::tempdir().unwrap();
        let app_support = dir.path().join("support");
        let state_root = dir.path().join("state");
        fs::create_dir_all(app_support.join("Claude")).unwrap();
        fs::create_dir_all(app_support.join("Claude-3p/configLibrary")).unwrap();
        fs::write(
            app_support.join("Claude/claude_desktop_config.json"),
            r#"{"deploymentMode":"3p","preferences":{"theme":"dark"}}"#,
        )
        .unwrap();
        fs::write(
            app_support.join("Claude-3p/claude_desktop_config.json"),
            r#"{"deploymentMode":"3p"}"#,
        )
        .unwrap();
        fs::write(app_support.join("Claude-3p/configLibrary/lib"), b"lib").unwrap();

        let sets = mac_sets_at(&app_support);
        apply_real_profile_one_p(&sets, &state_root).unwrap();
        // 幂等：第二次 apply 不重复写状态
        apply_real_profile_one_p(&sets, &state_root).unwrap();

        let one_p: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(one_p["deploymentMode"], json!("1p"));
        assert_eq!(one_p["preferences"]["theme"], json!("dark"));
        let one_p_3p: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude-3p/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(one_p_3p["deploymentMode"], json!("1p"));
        assert!(!app_support.join("Claude-3p/configLibrary").exists());
        assert!(
            app_support
                .join("Claude-3p")
                .join(CONFIG_LIBRARY_BACKUP)
                .is_dir()
        );
        assert!(state_root.join(SWITCH_STATE_FILE).is_file());

        restore_real_profile(&sets, &state_root).unwrap();
        let restored: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(restored["deploymentMode"], json!("3p"));
        let restored_3p: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude-3p/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(restored_3p["deploymentMode"], json!("3p"));
        assert!(app_support.join("Claude-3p/configLibrary/lib").is_file());
        assert!(!state_root.join(SWITCH_STATE_FILE).exists());
        // 恢复后再次 restore 是空操作
        restore_real_profile(&sets, &state_root).unwrap();
    }

    #[test]
    fn real_profile_switch_preserves_missing_deployment_mode() {
        let dir = tempfile::tempdir().unwrap();
        let app_support = dir.path().join("support");
        let state_root = dir.path().join("state");
        fs::create_dir_all(app_support.join("Claude")).unwrap();
        fs::create_dir_all(app_support.join("Claude-3p")).unwrap();
        fs::write(
            app_support.join("Claude/claude_desktop_config.json"),
            r#"{"preferences":{"theme":"dark"}}"#,
        )
        .unwrap();
        let sets = mac_sets_at(&app_support);
        apply_real_profile_one_p(&sets, &state_root).unwrap();
        let applied: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(applied["deploymentMode"], json!("1p"));
        restore_real_profile(&sets, &state_root).unwrap();
        let restored: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert!(restored.get("deploymentMode").is_none());
        assert_eq!(restored["preferences"]["theme"], json!("dark"));
    }

    #[test]
    fn embedded_assets_do_not_contain_recorded_accounts_or_certificates() {
        let dir = tempfile::tempdir().unwrap();
        install_assets(dir.path()).unwrap();
        assert!(dir.path().join("proxy.mjs").is_file());
        assert!(dir.path().join("page-shim.js").is_file());
        assert!(dir.path().join("official-assets.mjs").is_file());
        assert!(dir.path().join("catalogs/dynamic/zh-Hans.json").is_file());
        assert!(dir.path().join("mock/bootstrap.json").is_file());
        assert!(!dir.path().join("ca").exists());
        assert!(!dir.path().join("vendor").exists());
        assert!(
            !ASSETS
                .iter()
                .any(|(name, _)| name.contains("ion-dist.asar"))
        );
    }

    #[test]
    fn embedded_mock_template_is_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        install_assets(dir.path()).unwrap();
        let text = fs::read_to_string(dir.path().join("mock/bootstrap.json")).unwrap();
        let bootstrap: Value = serde_json::from_str(&text).unwrap();
        // 保留 GrowthBook / 模型选择器 / 计划形状
        assert!(
            bootstrap["org_growthbook"]["features"]
                .as_object()
                .is_some_and(|f| !f.is_empty())
        );
        assert!(
            bootstrap["activeOrganization"]["claude_ai_bootstrap_models_config"]
                .as_array()
                .is_some_and(|m| !m.is_empty())
        );
        // 只允许合成账号字段；不得残留真实邮箱/其它 uuid
        assert_eq!(
            bootstrap["account"]["email_address"],
            json!("local@claude.local")
        );
        assert_eq!(bootstrap["account"]["full_name"], json!("Local User"));
        assert_eq!(bootstrap["activeOrganization"]["name"], json!("Local"));
        // 允许公共支持邮箱（@anthropic.com 出现在 GrowthBook 文案里）与合成域；
        // 其它邮箱一律不得残留。
        for cap in text.split('"').filter(|s| s.contains('@')) {
            let emails: Vec<&str> = cap
                .split(|c: char| {
                    !c.is_alphanumeric() && c != '@' && c != '.' && c != '-' && c != '_'
                })
                .filter(|s| {
                    s.contains('@') && s.rsplit('@').next().is_some_and(|d| d.contains('.'))
                })
                .collect();
            for email in emails {
                let email = email.trim_end_matches('.');
                assert!(
                    email.ends_with("@claude.local")
                        || email.ends_with("@anthropic.com")
                        || email.ends_with("@example.com")
                        || email.ends_with(".iam.gserviceaccount.com"),
                    "unexpected personal email in template: {email}"
                );
            }
        }
        let convos = fs::read_to_string(dir.path().join("mock/api/conversations.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&convos).unwrap()["data"],
            json!([])
        );
    }

    #[test]
    fn local_agent_never_writes_desktop_deployment_configuration() {
        let env = crate::gateway::agents::AgentEnv::at(PathBuf::from("/synthetic-home"))
            .with_os(crate::gateway::agents::Os::Mac);
        let mut files = crate::gateway::agents::FileSet::default();
        let gateway = GatewaySettings::default();
        let binding = AgentBinding::default();
        let wiring = wiring(&gateway, &binding, AGENT_ID, 57321);
        (crate::gateway::agents::catalog_apps::CLAUDE_DESKTOP_LOCAL.write)(
            &env, &wiring, &mut files,
        )
        .unwrap();
        assert!(files.changes().unwrap().is_empty());
        assert!(crate::gateway::agents::spec(AGENT_ID).is_some());
        assert_eq!(
            wiring.root,
            "http://127.0.0.1:57321/gw/claude-desktop-local"
        );
    }

    /// 选择器显示名：组模型用组名、普通模型拼成产品名、副标题用供应商显示名。
    #[test]
    fn model_names_prefers_friendly_names_and_vendor_label() {
        let mut gateway = crate::gateway::store::GatewaySettings::default();
        let mut provider = provider("jojo", "gpt-6.1-sol");
        provider.relay.name = "Jojo Codemax".into();
        gateway.providers.push(provider);
        let names = model_names(&gateway, &["jojo/gpt-6.1-sol".to_string()]);
        let entry = &names["jojo/gpt-6.1-sol"];
        assert_eq!(entry["name"], "GPT-6.1 Sol");
        assert_eq!(entry["description"], "Jojo Codemax");
    }

    #[test]
    fn resume_port_only_when_claude_is_still_attached() {
        let attached = format!("4509 /Applications/Claude.app/Contents/MacOS/Claude {}", proxy_flag(53514));
        assert_eq!(resume_port(false, Some(53514), &attached, true), Some(53514));
        assert_eq!(resume_port(false, Some(53514), "", true), None, "没挂接不恢复");
        assert_eq!(resume_port(false, Some(5351), &attached, true), None, "端口不匹配不恢复");
        assert_eq!(resume_port(false, Some(53514), &attached, false), None, "未开完整功能不恢复");
        assert_eq!(resume_port(true, Some(53514), &attached, true), None, "runtime 在时不恢复");
        assert_eq!(resume_port(false, None, &attached, true), None);
    }

    /// 显示名与套餐：缺字段走默认（ultra / 空名→Local User），非法值被规范化。
    #[test]
    fn display_name_and_plan_defaults_and_normalization() {
        let config: crate::settings::ClaudeLocalSettings =
            serde_json::from_str("{}").unwrap();
        assert_eq!(config.plan, "ultra");
        assert_eq!(config.display_name, "");
        assert_eq!(config.normalized_plan(), "ultra");
        assert_eq!(config.effective_display_name(), "Local User");

        let mut config = crate::settings::ClaudeLocalSettings::default();
        for (raw, expected) in [("FREE", "free"), ("Pro", "pro"), ("max", "max"), (" vip ", "ultra")] {
            config.plan = raw.into();
            assert_eq!(config.normalized_plan(), expected, "{raw}");
        }
        config.display_name = "  张三\u{1f600}  ".into();
        assert_eq!(config.effective_display_name(), "张三\u{1f600}");
        config.display_name = "a\nb\t".into();
        assert_eq!(config.effective_display_name(), "ab");
        config.display_name = "x".repeat(80);
        assert_eq!(config.effective_display_name().chars().count(), 64);
        config.display_name = " \t\n ".into();
        assert_eq!(config.effective_display_name(), "Local User");
    }

    /// 「隐藏无供应商的模型」默认开：老配置里没有这个键也按开处理，显式关掉能存住。
    #[test]
    fn hide_unrouted_models_defaults_on_for_old_configs() {
        let old: ClaudeLocalSettings = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert!(old.hide_unrouted_models);
        let off: ClaudeLocalSettings =
            serde_json::from_value(json!({"hideUnroutedModels": false})).unwrap();
        assert!(!off.hide_unrouted_models);
        assert_eq!(flag(off.hide_unrouted_models), "0");
    }

    /// 完整功能模式不把继承来的代理变量交给 Claude：大小写两套都显式删除。
    #[test]
    fn full_mode_launch_strips_inherited_proxy_vars() {
        let mut command = Command::new("claude");
        command.env("HTTPS_PROXY", "http://127.0.0.1:7897");
        strip_inherited_proxy(&mut command);
        let removed: Vec<String> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect();
        for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
            assert!(
                removed.iter().any(|key| {
                    key == name || (cfg!(windows) && key.eq_ignore_ascii_case(name))
                }),
                "{name} 未被删除：{removed:?}"
            );
        }
    }

    /// 邮箱：缺字段/空串走默认，合法值 trim 后落盘，格式不对拒绝保存。
    #[test]
    fn email_defaults_normalization_and_validation() {
        let config: crate::settings::ClaudeLocalSettings =
            serde_json::from_str("{}").unwrap();
        assert_eq!(config.email, "");
        assert_eq!(config.effective_email(), "local@claude.local");

        let saved = validate_config(crate::settings::ClaudeLocalSettings {
            email: "  me@example.com  ".into(),
            ..crate::settings::ClaudeLocalSettings::default()
        })
        .unwrap();
        assert_eq!(saved.email, "me@example.com");

        let saved = validate_config(crate::settings::ClaudeLocalSettings {
            email: " \t ".into(),
            ..crate::settings::ClaudeLocalSettings::default()
        })
        .unwrap();
        assert_eq!(saved.email, "local@claude.local");

        let too_long = format!("{}@b.c", "a".repeat(260));
        for bad in ["no-at", "@b.c", "a@", "a b@c.d", "a@b\nc", too_long.as_str()] {
            let config = crate::settings::ClaudeLocalSettings {
                email: bad.into(),
                ..crate::settings::ClaudeLocalSettings::default()
            };
            assert!(validate_config(config.clone()).is_err(), "{bad:?}");
            // 手改设置文件写坏时，启动仍按默认邮箱。
            assert_eq!(config.effective_email(), "local@claude.local", "{bad:?}");
        }
    }

    /// 代理端口按整词匹配：5351 不能误伤 53514、535140，行尾无空格也算。
    #[test]
    fn processes_use_proxy_matches_the_whole_flag() {
        let main = format!("4509 /Applications/Claude.app/Contents/MacOS/Claude {} --disable-http-cache --lang=zh-CN", proxy_flag(53514));
        for (processes, port, expected) in [
            (main.as_str(), 53514, true),
            (main.as_str(), 5351, false),
            (main.as_str(), 53515, false),
        ] {
            assert_eq!(processes_use_proxy(processes, port), expected, "{port}");
        }
        // 标志在行尾（后面没有空白）也要算。
        let tail = format!("4509 /Applications/Claude.app/Contents/MacOS/Claude {}", proxy_flag(53514));
        assert!(processes_use_proxy(&tail, 53514));
        // 不带该标志的用户自己开的 Claude 不算。
        assert!(!processes_use_proxy("4509 /Applications/Claude.app/Contents/MacOS/Claude", 53514));
    }

    /// pgrep -fl 输出里只取端口精确匹配的 Claude 主进程：helper 行和别的端口行都不算。
    #[test]
    fn claude_pids_on_proxy_picks_only_the_main_process_on_that_port() {
        let listing = format!(
            "4509 /Applications/Claude.app/Contents/MacOS/Claude {} --disable-http-cache
             4512 /Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer)
             77248 /Applications/Claude.app/Contents/MacOS/Claude {}",
            proxy_flag(53514),
            proxy_flag(63917),
        );
        assert_eq!(claude_pids_on_proxy(&listing, 53514), vec![4509]);
        assert_eq!(claude_pids_on_proxy(&listing, 63917), vec![77248]);
        assert!(claude_pids_on_proxy(&listing, 5351).is_empty());
        assert!(claude_pids_on_proxy("", 53514).is_empty());
    }

    #[test]
    fn profile_sets_cover_mac_and_windows_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let sets = profile_sets_for(false, &home, None, None);
        assert_eq!(sets.len(), 1);
        let support = home.join("Library/Application Support");
        assert_eq!(sets[0].data, support.join("Claude"));
        assert_eq!(sets[0].mode, support.join("Claude"), "mac 上 data 与 mode 相同");
        assert_eq!(sets[0].three_p, support.join("Claude-3p"));

        let roaming = temp.path().join("roaming");
        let local = temp.path().join("local");
        let packages = local.join("Packages");
        for name in [
            "Claude_pzs8sxrjxfjjc",
            "AnthropicPBC.Claude_fnn82j28hfe8t",
            "Other_x",
        ] {
            fs::create_dir_all(packages.join(name)).unwrap();
        }
        let sets = profile_sets_for(true, &home, Some(&roaming), Some(&local));
        assert_eq!(sets.len(), 3, "classic + 两组 packaged，Other 被忽略");
        assert_eq!(sets[0].data, roaming.join("Claude"));
        assert_eq!(sets[0].mode, local.join("Claude"));
        assert_eq!(sets[0].three_p, local.join("Claude-3p"));
        // 排序后 AnthropicPBC.Claude_… 在 Claude_… 之前。
        for (set, package) in sets[1..].iter().zip([
            "AnthropicPBC.Claude_fnn82j28hfe8t",
            "Claude_pzs8sxrjxfjjc",
        ]) {
            let cache = packages.join(package).join("LocalCache");
            assert_eq!(set.data, cache.join("Roaming").join("Claude"));
            assert_eq!(set.mode, cache.join("Local").join("Claude"));
            assert_eq!(set.three_p, cache.join("Local").join("Claude-3p"));
        }
    }

    #[test]
    fn desktop_config_paths_preserve_mac_order_and_dedupe() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let sets = profile_sets_for(false, &home, None, None);
        let support = home.join("Library/Application Support");
        assert_eq!(
            desktop_config_paths(&sets),
            vec![
                support.join("Claude/claude_desktop_config.json"),
                support.join("Claude-3p/claude_desktop_config.json"),
            ],
            "macOS 的结果与顺序必须保持不变"
        );

        let roaming = temp.path().join("roaming");
        let local = temp.path().join("local");
        fs::create_dir_all(local.join("Packages").join("Claude_abc")).unwrap();
        let sets = profile_sets_for(true, &home, Some(&roaming), Some(&local));
        let paths = desktop_config_paths(&sets);
        assert_eq!(paths.len(), sets.len() * 3, "每组 data/mode/three_p 各一份");
        let mut sorted = paths.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(paths.len(), sorted.len(), "不能有重复路径");
    }

    #[test]
    fn real_profile_switch_handles_multiple_windows_profile_sets() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let roaming = temp.path().join("roaming");
        let local = temp.path().join("local");
        fs::create_dir_all(local.join("Packages").join("Claude_abc")).unwrap();
        let sets = profile_sets_for(true, &home, Some(&roaming), Some(&local));
        assert_eq!(sets.len(), 2, "classic + 一组 packaged");
        let root = temp.path().join("state");
        for set in &sets {
            for dir in [&set.data, &set.mode, &set.three_p] {
                fs::create_dir_all(dir).unwrap();
                fs::write(
                    dir.join("claude_desktop_config.json"),
                    r#"{"deploymentMode":"3p"}"#,
                )
                .unwrap();
            }
            let library = set.three_p.join("configLibrary");
            fs::create_dir_all(&library).unwrap();
            fs::write(library.join("lib"), b"lib").unwrap();
        }

        apply_real_profile_one_p(&sets, &root).unwrap();
        for set in &sets {
            for dir in [&set.data, &set.mode, &set.three_p] {
                let config: Value = serde_json::from_slice(
                    &fs::read(dir.join("claude_desktop_config.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(config["deploymentMode"], json!("1p"), "{}", dir.display());
            }
            let backup = set
                .three_p
                .join("configLibrary")
                .with_file_name(CONFIG_LIBRARY_BACKUP);
            assert!(!set.three_p.join("configLibrary").exists());
            assert!(backup.join("lib").is_file(), "{}", backup.display());
        }

        restore_real_profile(&sets, &root).unwrap();
        for set in &sets {
            for dir in [&set.data, &set.mode, &set.three_p] {
                let config: Value = serde_json::from_slice(
                    &fs::read(dir.join("claude_desktop_config.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(config["deploymentMode"], json!("3p"), "{}", dir.display());
            }
            assert!(set.three_p.join("configLibrary/lib").is_file());
            assert!(
                !set
                    .three_p
                    .join("configLibrary")
                    .with_file_name(CONFIG_LIBRARY_BACKUP)
                    .exists()
            );
        }
        assert!(!root.join(SWITCH_STATE_FILE).exists());
    }

    #[test]
    fn legacy_switch_state_still_restores_config_library() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, target, backup) = orphan_fixture(&temp);
        fs::create_dir_all(&target).unwrap();
        let support = temp.path().join("support");
        fs::create_dir_all(support.join("Claude")).unwrap();
        fs::write(
            support.join("Claude/claude_desktop_config.json"),
            r#"{"deploymentMode":"1p"}"#,
        )
        .unwrap();
        // 旧格式状态文件：单条 configLibraryMoved + configLibraryBackup。
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("lib"), b"lib").unwrap();
        fs::remove_dir_all(&target).unwrap();
        crate::settings::atomic_write(
            &switch_state_path(&root),
            &serde_json::to_vec_pretty(&json!({
                "applied": true,
                "deploymentMode": {
                    support.join("Claude/claude_desktop_config.json").to_string_lossy(): "3p",
                },
                "configLibraryMoved": true,
                "configLibraryBackup": backup.to_string_lossy(),
            }))
            .unwrap(),
        )
        .unwrap();

        restore_real_profile(&sets, &root).unwrap();
        assert!(target.join("lib").is_file(), "旧格式记录的备份要能放回");
        assert!(!backup.exists());
        let config: Value = serde_json::from_slice(
            &fs::read(support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(config["deploymentMode"], json!("3p"));
        assert!(!root.join(SWITCH_STATE_FILE).exists());
    }

    #[test]
    fn orphan_backup_tracked_by_legacy_state_is_left_alone() {
        let temp = tempfile::tempdir().unwrap();
        let (sets, root, _target, backup) = orphan_fixture(&temp);
        fs::create_dir_all(&backup).unwrap();
        // 旧格式只记主集合那份备份：路径一致才算被跟踪。
        crate::settings::atomic_write(
            &switch_state_path(&root),
            &serde_json::to_vec_pretty(&json!({
                "applied": true,
                "configLibraryMoved": true,
                "configLibraryBackup": backup.to_string_lossy(),
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(recover_orphan_config_library(&sets, &root).unwrap(), None);
        assert!(backup.exists(), "被旧格式状态文件跟踪的备份交给 restore");
    }

    #[test]
    fn orphan_recovery_handles_each_profile_set_independently() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let roaming = temp.path().join("roaming");
        let local = temp.path().join("local");
        fs::create_dir_all(local.join("Packages").join("Claude_abc")).unwrap();
        let sets = profile_sets_for(true, &home, Some(&roaming), Some(&local));
        assert_eq!(sets.len(), 2);
        let root = temp.path().join("state");
        fs::create_dir_all(&root).unwrap();
        // 第一组：目标不在 → 备份放回。
        let backup0 = sets[0]
            .three_p
            .join("configLibrary")
            .with_file_name(CONFIG_LIBRARY_BACKUP);
        fs::create_dir_all(&backup0).unwrap();
        // 第二组：目标也在 → 备份归档。
        let target1 = sets[1].three_p.join("configLibrary");
        let backup1 = target1.with_file_name(CONFIG_LIBRARY_BACKUP);
        fs::create_dir_all(&target1).unwrap();
        fs::create_dir_all(&backup1).unwrap();

        let note = recover_orphan_config_library(&sets, &root).unwrap().unwrap();
        assert!(sets[0].three_p.join("configLibrary").is_dir(), "目标不在的放回");
        assert!(target1.is_dir() && !backup1.exists(), "目标还在的归档");
        let archived: Vec<_> = fs::read_dir(root.join("backups"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(archived.len(), 1);
        assert_eq!(note.lines().count(), 2, "两组各一句说明");
    }

    #[test]
    fn ca_thumbprint_matches_openssl_sha1_fingerprint() {
        // 测试证书由 `openssl req -x509 -newkey rsa:2048 -nodes -subj /CN=test` 生成，
        // 期望值是 `openssl x509 -noout -fingerprint -sha1` 去掉冒号。
        let pem = b"-----BEGIN CERTIFICATE-----\n\
MIICmjCCAYICCQCpYaxH1PIjyzANBgkqhkiG9w0BAQsFADAPMQ0wCwYDVQQDDAR0\n\
ZXN0MB4XDTI2MTExMDE0MjQwNFoXDTI2MTExMTE0MjQwNFowDzENMAsGA1UEAwwE\n\
dGVzdDCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBALUXi/kw+Wb9kU9x\n\
dQfgaKAMqWFo92iPmOrkE7w3TuS4GbToKOLe6hr0CNPsgmbiNusef+n1mQGHFsZL\n\
BofAtXU93+gvpQbFmW0oKxNpfuoLdCMYZlZ17DdSffQIMwK8YR11dNafjks3eaIX\n\
8gffHZKkWYLZ5YdhT1fpJK90vdylyHHtPW2SlMpGmdkojACxzGKkzHyFZdtXB2lp\n\
VaHcXTyvoTYMLrF19UuQHi394h0/0XYqZINks3M+BgMOUDcXjKwJxW58N73id2JF\n\
EdQhIOM7Mok8Wfwf+x9/EqNiANKL4RG0RU3SRbh022AT0tpabaEF8Bbcnkn+W4vU\n\
mm6LprECAwEAATANBgkqhkiG9w0BAQsFAAOCAQEANiM68FroLYEdyxX++l8rS521\n\
a1zbJqt/22R/T4FV+4/28wLNBlI0zsm0ajW6MF+F0EGQXkiP7wtp4/x8BZkp6LU8\n\
30hdyepra0mf/qc35JGuOnH3qwRQuXdDYsLFpTuP+wHHZlw6XV3PY/Pg0hNmMViP\n\
E2ZwvQab9tv86m7gd4KrWAq6rwYCeVy7CA/Y/tTZgaDZrmAtfKAYuMtw5mK7DT6n\n\
RGvMS5OzZFvnFlekIHE6c+6QK9QtWD7P+PkIpRa1D01cb9SwsRHqGfs7o2UbaZxs\n\
9soF6shq4Ugxrbr27r195accXSekHve+EcgH3gw2mOkeb6MbidqYBUYFcf+Suw==\n\
-----END CERTIFICATE-----\n";
        assert_eq!(
            ca_thumbprint(pem).as_deref(),
            Some("60772CEDC0B873CCC34DAA9C0D371AB3D338398E")
        );
        assert_eq!(ca_thumbprint(b"not a pem"), None);
    }

    #[test]
    fn is_desktop_claude_image_distinguishes_desktop_from_cli() {
        let temp = tempfile::tempdir().unwrap();
        // 父目录下有 resources/app.asar → 桌面版
        let app_dir = temp.path().join("Claude");
        fs::create_dir_all(app_dir.join("resources")).unwrap();
        fs::write(app_dir.join("resources").join("app.asar"), b"asar").unwrap();
        let exe = app_dir.join("Claude.exe");
        assert!(is_desktop_claude_image("Claude.exe", Some(&exe)));
        // 只有 claude.exe 没有 resources → Claude Code CLI
        let cli_dir = temp.path().join("cli");
        fs::create_dir_all(&cli_dir).unwrap();
        assert!(!is_desktop_claude_image("claude.exe", Some(&cli_dir.join("claude.exe"))));
        // WindowsApps 包路径 → 桌面版
        let msix = Path::new(r"C:\Program Files\WindowsApps\AnthropicPBC.Claude_1_x64__p\app\Claude.exe");
        assert!(is_desktop_claude_image("Claude.exe", Some(msix)));
        // 读不到路径（权限不足）也算，由后续命令行判断兜底
        assert!(is_desktop_claude_image("Claude.exe", None));
        // 名字不是 Claude.exe 一律不算
        assert!(!is_desktop_claude_image("node.exe", None));
    }

    #[test]
    fn claude_pids_on_proxy_reads_windows_listing() {
        let listing = format!(
            "1234 \"C:\\Program Files\\WindowsApps\\AnthropicPBC.Claude_1_x64__p\\app\\Claude.exe\" {} --lang=zh-CN",
            proxy_flag(5000)
        );
        assert_eq!(claude_pids_on_proxy(&listing, 5000), vec![1234]);
        assert!(claude_pids_on_proxy(&listing, 500).is_empty(), "端口不整词匹配");
    }

    /// 进程内生成的 CA 材料：四件齐全、PEM 头正确（私钥是 PKCS#8）、
    /// ca.pem 能算出 SHA-1 指纹。
    #[test]
    fn generate_ca_writes_complete_pem_material() {
        let temp = tempfile::tempdir().unwrap();
        let ca = temp.path().join("ca");
        fs::create_dir_all(&ca).unwrap();
        generate_ca(&ca).unwrap();

        assert!(ca_ready(temp.path()), "四个文件都要有内容");
        for name in ["ca.key", "server.key"] {
            let pem = fs::read(ca.join(name)).unwrap();
            assert!(
                pem.starts_with(b"-----BEGIN PRIVATE KEY-----"),
                "{name} 应是 PKCS#8 PEM 私钥"
            );
        }
        for name in ["ca.pem", "server.crt"] {
            let pem = fs::read(ca.join(name)).unwrap();
            assert!(
                pem.starts_with(b"-----BEGIN CERTIFICATE-----"),
                "{name} 应是 PEM 证书"
            );
        }
        let thumbprint = ca_thumbprint(&fs::read(ca.join("ca.pem")).unwrap()).unwrap();
        assert_eq!(thumbprint.len(), 40, "SHA-1 指纹 40 位十六进制");
        assert!(thumbprint.chars().all(|ch| ch.is_ascii_digit() || ('A'..='F').contains(&ch)));

        // 已有证书不重复生成：再调 prepare_ca 不动磁盘上的材料。
        let ca_pem = fs::read(ca.join("ca.pem")).unwrap();
        prepare_ca(temp.path()).unwrap();
        assert_eq!(fs::read(ca.join("ca.pem")).unwrap(), ca_pem);
    }
}
