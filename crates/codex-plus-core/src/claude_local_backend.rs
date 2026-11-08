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
    pub connected: bool,
    pub port: Option<u16>,
    pub ca_ready: bool,
    pub ca_trusted: bool,
    pub node_available: bool,
    pub dictation_ready: bool,
    pub config: ClaudeLocalSettings,
    pub models: Vec<String>,
    pub warning: Option<String>,
    pub needs_restart: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalResult {
    pub status: ClaudeLocalStatus,
    pub message: String,
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

fn source_app() -> PathBuf {
    let system = PathBuf::from("/Applications/Claude.app");
    if system.is_dir() {
        system
    } else {
        home_dir().join("Applications/Claude.app")
    }
}

fn executable(source: &Path) -> PathBuf {
    source.join("Contents/MacOS/Claude")
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

fn app_support_dir() -> PathBuf {
    home_dir().join("Library/Application Support")
}

fn desktop_config_paths(app_support: &Path) -> [PathBuf; 2] {
    [
        app_support.join("Claude/claude_desktop_config.json"),
        app_support.join("Claude-3p/claude_desktop_config.json"),
    ]
}

fn switch_state_path(root: &Path) -> PathBuf {
    root.join(SWITCH_STATE_FILE)
}

fn apply_real_profile_one_p(app_support: &Path, root: &Path) -> Result<()> {
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
    for config_path in desktop_config_paths(app_support) {
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
    let mut config_library_moved = false;
    let config_library = app_support.join("Claude-3p/configLibrary");
    let backup = config_library.with_file_name(CONFIG_LIBRARY_BACKUP);
    if backup.exists() {
        bail!(
            "Claude-3p 已存在未恢复的 configLibrary 备份，请先处理 {}",
            backup.display()
        );
    }
    if config_library.exists() {
        fs::rename(&config_library, &backup)
            .with_context(|| format!("无法挪开 {}", config_library.display()))?;
        config_library_moved = true;
    }
    let state = json!({
        "applied": true,
        "deploymentMode": Value::Object(original),
        "configLibraryMoved": config_library_moved,
        "configLibraryBackup": backup.to_string_lossy(),
    });
    crate::settings::atomic_write(&state_path, &serde_json::to_vec_pretty(&state)?)?;
    Ok(())
}

fn restore_real_profile(app_support: &Path, root: &Path) -> Result<()> {
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
    if state["configLibraryMoved"].as_bool() == Some(true) {
        let backup = PathBuf::from(state["configLibraryBackup"].as_str().unwrap_or(""));
        let target = app_support.join("Claude-3p/configLibrary");
        if backup.exists() && !target.exists() {
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

// 首次运行且没有可回放的页面缓存时提示：依赖在线抓取，离线会降级。
fn replay_cache_populated(root: &Path) -> bool {
    let cache = root.join(".cache");
    cache.is_dir()
        && fs::read_dir(cache)
            .map(|mut entries| entries.any(|entry| entry.is_ok()))
            .unwrap_or(false)
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

fn node_runtime() -> Option<PathBuf> {
    let node = crate::dsh_plugins::locate_node()?;
    probe(
        Command::new(&node).args([
            "-e",
            "process.exit(typeof fetch==='function'&&typeof WebSocket==='function'?0:1)",
        ]),
        PROBE_TIMEOUT,
    )
    .ok()
    .filter(|(success, _)| *success)
    .map(|_| node)
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
    #[cfg(not(target_os = "macos"))]
    Ok(String::new())
}

fn version(source: &Path) -> String {
    if !source.is_dir() || !cfg!(target_os = "macos") {
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

fn ca_ready(root: &Path) -> bool {
    ["ca.pem", "ca.key", "server.crt", "server.key"]
        .iter()
        .all(|name| {
            fs::metadata(root.join("ca").join(name)).is_ok_and(|m| m.is_file() && m.len() > 0)
        })
}

fn ca_trusted(root: &Path) -> bool {
    if !cfg!(target_os = "macos") || !ca_ready(root) {
        return false;
    }
    probe(
        Command::new("/usr/bin/security")
            .args(["verify-cert", "-q", "-L", "-c"])
            .arg(root.join("ca/server.crt"))
            .args(["-p", "ssl", "-n", "claude.ai"]),
        PROBE_TIMEOUT,
    )
    .is_ok_and(|(success, _)| success)
}

fn prepare_ca(root: &Path) -> Result<()> {
    if ca_ready(root) {
        return Ok(());
    }
    let ca = root.join("ca");
    private_dir(&ca)?;
    let config = ca.join("openssl.cnf");
    write_private(
        &config,
        br#"[req]
distinguished_name=dn
prompt=no
[dn]
CN=Codex++ Claude Local
[v3_ca]
basicConstraints=critical,CA:TRUE
keyUsage=critical,keyCertSign,cRLSign
subjectKeyIdentifier=hash
[v3_leaf]
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth
subjectAltName=@alt
[alt]
DNS.1=claude.ai
DNS.2=*.claude.ai
DNS.3=claude.com
DNS.4=*.claude.com
DNS.5=anthropic.com
DNS.6=*.anthropic.com
DNS.7=claudeusercontent.com
DNS.8=*.claudeusercontent.com
DNS.9=localhost
IP.1=127.0.0.1
"#,
    )?;
    let openssl = crate::dsh_plugins::locate_program("openssl")
        .context("未找到 OpenSSL，无法准备本地证书")?;
    let mut command = Command::new(&openssl);
    command
        .args(["genrsa", "-out"])
        .arg(ca.join("ca.key"))
        .arg("2048");
    checked(
        &mut command,
        Duration::from_secs(20),
        "生成本地 CA 私钥失败",
    )?;
    let mut command = Command::new(&openssl);
    command
        .args(["req", "-x509", "-new", "-key"])
        .arg(ca.join("ca.key"))
        .args([
            "-sha256",
            "-days",
            "3650",
            "-subj",
            &format!("/CN={CA_NAME}"),
            "-out",
        ])
        .arg(ca.join("ca.pem"))
        .args(["-extensions", "v3_ca", "-config"])
        .arg(&config);
    checked(
        &mut command,
        Duration::from_secs(20),
        "生成本地 CA 证书失败",
    )?;
    let mut command = Command::new(&openssl);
    command
        .args(["genrsa", "-out"])
        .arg(ca.join("server.key"))
        .arg("2048");
    checked(
        &mut command,
        Duration::from_secs(20),
        "生成本地服务私钥失败",
    )?;
    let mut command = Command::new(&openssl);
    command
        .args(["req", "-new", "-key"])
        .arg(ca.join("server.key"))
        .args(["-subj", "/CN=claude.ai", "-out"])
        .arg(ca.join("server.csr"))
        .arg("-config")
        .arg(&config);
    checked(
        &mut command,
        Duration::from_secs(20),
        "生成本地服务证书请求失败",
    )?;
    let mut command = Command::new(&openssl);
    command
        .args(["x509", "-req", "-in"])
        .arg(ca.join("server.csr"))
        .arg("-CA")
        .arg(ca.join("ca.pem"))
        .arg("-CAkey")
        .arg(ca.join("ca.key"))
        .args(["-CAcreateserial", "-days", "825", "-sha256", "-extfile"])
        .arg(&config)
        .args(["-extensions", "v3_leaf", "-out"])
        .arg(ca.join("server.crt"));
    checked(
        &mut command,
        Duration::from_secs(20),
        "签发本地服务证书失败",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for name in ["ca.key", "server.key"] {
            fs::set_permissions(ca.join(name), fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn trust_ca(root: &Path) -> Result<()> {
    ensure!(cfg!(target_os = "macos"), "本地证书信任目前仅支持 macOS");
    if ca_trusted(root) {
        return Ok(());
    }
    checked(
        Command::new("/usr/bin/security")
            .args(["add-trusted-cert", "-r", "trustRoot", "-p", "ssl", "-k"])
            .arg(home_dir().join("Library/Keychains/login.keychain-db"))
            .arg(root.join("ca/ca.pem")),
        Duration::from_secs(120),
        "本地证书尚未信任，请在系统授权窗口中允许后重试",
    )?;
    ensure!(ca_trusted(root), "本地证书信任未生效，请重新检查系统授权");
    Ok(())
}

fn config_hash(config: &ClaudeLocalSettings) -> String {
    let models = GatewayStore::default()
        .load()
        .map(|gateway| available_models(&gateway.with_extension_providers()))
        .unwrap_or_default();
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(config, models)).unwrap_or_default())
    )
}

fn available_models(gateway: &GatewaySettings) -> Vec<String> {
    let mut models = Vec::new();
    for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
        for model in crate::gateway::route::chat_models(&provider.relay) {
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

fn available_model_windows(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
) -> BTreeMap<String, u64> {
    available_models(gateway)
        .into_iter()
        .filter_map(|model| {
            crate::gateway::route::model_entry_window(gateway, binding, &model)
                .or(binding.context_window)
                .filter(|window| *window > 0)
                .map(|window| (model, window))
        })
        .collect()
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

fn local_binding(gateway: &GatewaySettings, config: &ClaudeLocalSettings) -> Result<AgentBinding> {
    let agent = ToolId::Unknown(AGENT_ID.into());
    let mut binding = gateway.binding(&agent).cloned().unwrap_or_default();
    binding.pool_exclude.clear();
    binding.extra_models = available_models(gateway);
    binding.model = if config.model.is_empty() {
        binding.extra_models.first().cloned().unwrap_or_default()
    } else {
        config.model.clone()
    };
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
    let local_process = runtime.as_ref().is_some_and(|(port, _, _, _)| {
        processes.contains(&format!("--proxy-server=http://127.0.0.1:{port}"))
    });
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
    let gateway = GatewayStore::default().load()?.with_extension_providers();
    let models = available_models(&gateway);
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
    } else if models.is_empty() {
        Some("供应商池中还没有可用的聊天模型".into())
    } else if config.enabled && !trusted {
        Some("首次使用需要准备并信任本地证书".into())
    } else if config.enabled && config.voice_input && !dictation_ready {
        Some("本地听写尚未就绪，请在语音设置中启用 SenseVoice 并准备模型".into())
    } else if config.enabled && recorded_data_dir().is_none() && !replay_cache_populated(&root) {
        Some("首次运行需要从 claude.ai 在线抓取页面资源；离线环境下完整功能模式可能无法加载".into())
    } else {
        fs::read_to_string(root.join("compatibility-warning.txt"))
            .ok()
            .filter(|text| !text.is_empty())
    };
    Ok(ClaudeLocalStatus {
        supported: cfg!(target_os = "macos") && executable(&source).is_file(),
        version: version(&source),
        source_app: source.to_string_lossy().into_owned(),
        claude_running,
        running,
        connected,
        port: runtime.map(|(port, _, _, _)| port),
        ca_ready: ready,
        ca_trusted: trusted,
        node_available: node_runtime().is_some(),
        dictation_ready,
        config,
        models,
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
    let original_cache = home_dir().join("Library/Application Support/Claude/Cache/Cache_Data");
    let mut last_error = None;
    for electron_cache in [&original_cache] {
        let drain = probe(
            Command::new(node)
                .arg(root.join("scripts/patch-dictation-drain.mjs"))
                .args(["--apply", "--cache-dir"])
                .arg(&cache)
                .arg("--electron-cache-dir")
                .arg(electron_cache)
                .env("CLAUDE_LOCAL_MODE", "mock"),
            Duration::from_secs(20),
        );
        let code = "const m=await import(process.argv[1]);m.applyStartupPatches({cacheDir:process.argv[2],electronCacheDir:process.argv[3]});";
        let start = probe(
            Command::new(node)
                .args(["--input-type=module", "-e", code])
                .arg(
                    reqwest::Url::from_file_path(root.join("scripts/patch-dictation-start.mjs"))
                        .map_err(|_| anyhow::anyhow!("听写资源路径无效"))?
                        .as_str(),
                )
                .arg(&cache)
                .arg(electron_cache)
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

fn start_backend(root: &Path, node: &Path, config: &ClaudeLocalSettings) -> Result<u16> {
    if let Some((port, _, hash, _)) = runtime_status()
        && hash == config_hash(config)
    {
        return Ok(port);
    }
    stop_backend();
    let helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
    crate::resident_helper::ensure_resident_helper(helper_port)?;
    let (gateway, binding) = bind_gateway(&GatewayStore::default(), config)?;
    let wiring = wiring(&gateway, &binding, AGENT_ID, helper_port);
    let instance = uuid::Uuid::new_v4().to_string();
    let ready_file = root.join("ready.json");
    let log = private_file(&root.join("proxy.log"))?;
    let mut command = Command::new(node);
    command
        .arg(root.join("proxy.mjs"))
        .args(["--port", "0", "--mode", "mock", "--log", "0", "--ca"])
        .arg(root.join("ca"))
        .current_dir(root)
        .env("CLAUDE_LOCAL_MANAGED", "1")
        .env(
            "CODEX_PLUS_STATE_DIR",
            crate::paths::default_app_state_dir(),
        )
        .env("CLAUDE_LOCAL_CACHE_DIR", root.join(".cache"))
        .env("CLAUDE_LOCAL_WEB_I18N_DIR", root.join("catalogs"))
        .env(
            "CLAUDE_LOCAL_DESKTOP_RESOURCES",
            source_app().join("Contents/Resources"),
        )
        .env("CLAUDE_LOCAL_GATEWAY_URL", wiring.root)
        .env("CLAUDE_LOCAL_GATEWAY_KEY", wiring.token)
        .env("CLAUDE_LOCAL_GATEWAY_MODEL", wiring.default_model)
        .env("CLAUDE_LOCAL_GATEWAY_SMALL_MODEL", wiring.small_model)
        .env(
            "CLAUDE_LOCAL_GATEWAY_MODELS",
            serde_json::to_string(&available_models(&gateway))?,
        )
        .env(
            "CLAUDE_LOCAL_GATEWAY_MODEL_WINDOWS",
            serde_json::to_string(&available_model_windows(&gateway, &binding))?,
        )
        .env("CLAUDE_LOCAL_STATE_FILE", root.join("conversations.json"))
        .env("CLAUDE_LOCAL_READY_FILE", &ready_file)
        .env("CLAUDE_LOCAL_INSTANCE_ID", &instance)
        .env("CLAUDE_LOCAL_LOCALIZATION", flag(config.localization))
        .env("CLAUDE_LOCAL_VOICE_INPUT", flag(config.voice_input))
        .env("CLAUDE_LOCAL_COMPUTER_USE", flag(config.computer_use))
        .env("CLAUDE_LOCAL_BACKGROUND_MODE", flag(config.background_mode));
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

fn launch_original(
    source: &Path,
    root: &Path,
    port: Option<u16>,
    localization: bool,
) -> Result<()> {
    let mut command = Command::new(executable(source));
    for name in ["ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ASAR", "NODE_OPTIONS"] {
        command.env_remove(name);
    }
    if let Some(port) = port {
        let gateway = GatewayStore::default().load()?;
        let binding = gateway
            .active_binding(&ToolId::Unknown(AGENT_ID.into()))
            .context("Claude 本地模式的网关绑定尚未就绪")?
            .clone();
        // 完整功能模式与 PoC 一致：直接用真实 Claude profile（无 --user-data-dir），
        // 启动前把真实配置切到 1p 并记录回滚信息。
        apply_real_profile_one_p(&app_support_dir(), root)?;
        private_dir(&root.join("claude-code"))?;
        // 与 3p 路径一致：把 Codex++ 配置的每模型窗口/推理档传给内置 Claude Code。
        let helper_port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
        let wiring = wiring(&gateway, &binding, AGENT_ID, helper_port);
        let token = wiring.token.clone();
        command
            .args(local_launch_args(root, port, localization))
            .env("CLAUDE_CONFIG_DIR", root.join("claude-code"))
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
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动原版 Claude Desktop")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn launch_three_p(source: &Path, root: &Path, config: &ClaudeLocalSettings) -> Result<()> {
    stop_backend();
    // 切回 3p 前先恢复真实 profile（enable_binding 会重写 configLibrary，顺序不能反）。
    restore_real_profile(&app_support_dir(), root)?;
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
    binding.model = if config.model.is_empty() {
        binding.extra_models.first().cloned().unwrap_or_default()
    } else {
        config.model.clone()
    };
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
    let mut command = Command::new(executable(source));
    for name in [
        "ELECTRON_RUN_AS_NODE",
        "ELECTRON_NO_ASAR",
        "NODE_OPTIONS",
        "CLAUDE_USER_DATA_DIR",
    ] {
        command.env_remove(name);
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
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("无法启动原版 Claude Desktop 的 3p 模式")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
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
        let deadline = Instant::now() + Duration::from_secs(2);
        while runtime.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if runtime.child.try_wait().ok().flatten().is_none() {
            let _ = runtime.child.kill();
        }
        let _ = runtime.child.wait();
    }
}

pub fn perform(action: &str) -> Result<ClaudeLocalResult> {
    ensure!(
        cfg!(target_os = "macos"),
        "原版 Claude 本地模式目前仅支持 macOS"
    );
    ensure!(
        ["launch", "stop", "trust", "prepare"].contains(&action),
        "未知的 Claude 本地服务操作"
    );
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 本地服务操作正在进行"))?;
    let config = SettingsStore::default().load()?.claude_desktop_local;
    let root = root_dir();
    let source = source_app();
    let message = match action {
        "stop" => {
            stop_backend();
            // 完整功能模式停止时把真实 profile 切回原始状态。
            restore_real_profile(&app_support_dir(), &root)?;
            "Claude 本地服务已停止，Claude 应用保持打开。".to_string()
        }
        "trust" => {
            install_assets(&root)?;
            prepare_ca(&root)?;
            trust_ca(&root)?;
            "本地证书已信任。".to_string()
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
        "launch" => {
            ensure!(
                executable(&source).is_file(),
                "未安装 Claude Desktop，请先安装官方应用"
            );
            ensure!(
                claude_processes()?.is_empty(),
                "Claude 正在运行，请先完全退出，再从 Codex++ 启动以应用本地模式"
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
                let port = start_backend(&root, &node, &config)?;
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
    Ok(ClaudeLocalResult {
        status: status()?,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

        apply_real_profile_one_p(&app_support, &state_root).unwrap();
        // 幂等：第二次 apply 不重复写状态
        apply_real_profile_one_p(&app_support, &state_root).unwrap();

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

        restore_real_profile(&app_support, &state_root).unwrap();
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
        restore_real_profile(&app_support, &state_root).unwrap();
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
        apply_real_profile_one_p(&app_support, &state_root).unwrap();
        let applied: Value = serde_json::from_slice(
            &fs::read(app_support.join("Claude/claude_desktop_config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(applied["deploymentMode"], json!("1p"));
        restore_real_profile(&app_support, &state_root).unwrap();
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
}
