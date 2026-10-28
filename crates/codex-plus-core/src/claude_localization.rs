//! Claude Desktop 汉化：受管副本、启动注入与可恢复的用户语言设置。
//! 命令只由管理器调用，不开放 HTTP / 第三方扩展路由。
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

static OPERATION: Mutex<()> = Mutex::new(());

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const STATUS_TIMEOUT: Duration = Duration::from_secs(15);

/// 探测输出落在临时文件，避免子进程填满管道后阻塞；超时确实终止并回收子进程。
#[derive(Debug)]
struct ProbeOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
}

fn probe_output(command: &mut Command, deadline: Instant) -> Result<ProbeOutput> {
    ensure!(Instant::now() < deadline, "Claude 检测超时，请重试");
    let path = std::env::temp_dir().join(format!("codex-plus-claude-probe-{}.tmp", uuid::Uuid::new_v4()));
    struct Capture(PathBuf);
    impl Drop for Capture {
        fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
    }
    let mut capture = fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path)?;
    // 句柄必须先关闭再移除（Windows 不允许删除仍打开的文件）。
    let cleanup = Capture(path);
    let result = (|| {
        command.stdin(Stdio::null()).stdout(capture.try_clone()?).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_create_no_window());
        }
        let mut child = command.spawn().context("无法启动 Claude 检测程序")?;
        let deadline = deadline.min(Instant::now() + PROBE_TIMEOUT);
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                result => {
                    let _ = child.kill();
                    let _ = child.wait();
                    if let Err(error) = result { return Err(error.into()); }
                    bail!("Claude 检测超时，检测进程已结束，请重试");
                }
            }
        };
        capture.seek(SeekFrom::Start(0))?;
        let mut stdout = Vec::new();
        capture.by_ref().take(64 * 1024).read_to_end(&mut stdout)?;
        Ok(ProbeOutput { status, stdout })
    })();
    // Command 自身也持有重定向句柄，结束后释放它们才可清理临时文件。
    command.stdout(Stdio::null());
    drop(capture);
    drop(cleanup);
    result
}

#[derive(Clone, Copy)]
enum AppLayout { Bundle, Executable }

impl AppLayout {
    fn current() -> Self {
        if cfg!(windows) { Self::Executable } else { Self::Bundle }
    }

    fn exists(self, app: &Path) -> bool {
        match self { Self::Bundle => app.is_dir(), Self::Executable => app.is_file() }
    }

    fn root(self, app: &Path) -> Result<&Path> {
        match self {
            Self::Bundle => Ok(app),
            Self::Executable => app.parent().filter(|p| !p.as_os_str().is_empty()).context("Claude 可执行文件缺少目录"),
        }
    }
}

/// 替换完整应用目录（Windows 不能只移动 exe，resources 等文件必须一起移动）。
/// 校验过的暂存目录才会进入这里；替换失败时恢复旧副本，不删除任何副本。
fn replace_copy(layout: AppLayout, staged: &Path, copy: &Path, backup: &Path) -> Result<()> {
    replace_copy_with(layout, staged, copy, backup, |from, to| fs::rename(from, to))
}

fn replace_copy_with(
    layout: AppLayout, staged: &Path, copy: &Path, backup: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    ensure!(layout.exists(staged), "Claude 暂存副本不存在，保留旧副本");
    let staged = layout.root(staged)?;
    let copy = layout.root(copy)?;
    let backup = layout.root(backup)?;
    ensure!(staged != copy && copy != backup && staged != backup, "Claude 副本目录不能相同");
    ensure!(!backup.exists(), "Claude 备份目录已存在，保留旧副本");
    if let Some(parent) = copy.parent() { fs::create_dir_all(parent)?; }
    let had_copy = copy.exists();
    if had_copy { rename(copy, backup)?; }
    if let Err(error) = rename(staged, copy) {
        if had_copy { rename(backup, copy).context("替换失败且旧副本还原失败，旧副本保留在备份目录")?; }
        return Err(error.into());
    }
    Ok(())
}

const ASSETS: &[(&str, &[u8])] = &[
    (
        "build-copy.py",
        include_bytes!("../../../assets/claude-localization/build-copy.py"),
    ),
    (
        "sign-copy.py",
        include_bytes!("../../../assets/claude-localization/sign-copy.py"),
    ),
    (
        "build-copy.ps1",
        include_bytes!("../../../assets/claude-localization/build-copy.ps1"),
    ),
    (
        "inject-cn.ps1",
        include_bytes!("../../../assets/claude-localization/inject-cn.ps1"),
    ),
    (
        "inject-cn.js",
        include_bytes!("../../../assets/claude-localization/inject-cn.js"),
    ),
    (
        "hook.js",
        include_bytes!("../../../assets/claude-localization/hook.js"),
    ),
    (
        "page-shim.js",
        include_bytes!("../../../assets/claude-localization/page-shim.js"),
    ),
    (
        "page-dictation.js",
        include_bytes!("../../../assets/claude-localization/page-dictation.js"),
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
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalizationStatus {
    pub supported: bool,
    pub source_app: String,
    pub copy_app: String,
    pub source_version: String,
    pub copy_version: String,
    pub prepared: bool,
    pub needs_rebuild: bool,
    pub running: bool,
    pub node_available: bool,
    pub python_available: bool,
    pub shell_locale: Option<String>,
    pub catalog_entries: usize,
    /// 副本页面里是否带语音入口；关掉后已在运行的副本要重启才会消失。
    pub voice_input: bool,
    /// 是否注入中文界面；关掉后副本按原语言启动，与「准备副本」互不影响。
    pub localization: bool,
    /// 是否在副本里解锁 Computer Use（改写 asar + 关闭 asar 完整性校验）。
    pub computer_use: bool,
    /// 当前平台是否真的能应用 Computer Use；Windows 的构建脚本不改写 asar，
    /// 打开开关只会存设置，界面据此禁用该开关（#2673）。
    pub computer_use_supported: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLocalizationResult {
    pub status: ClaudeLocalizationStatus,
    pub message: String,
}

fn runtime_dir() -> PathBuf {
    crate::paths::default_app_state_dir().join("claude-localization")
}

#[cfg(target_os = "macos")]
fn source_app(_deadline: Instant) -> Result<PathBuf> {
    let system = PathBuf::from("/Applications/Claude.app");
    if system.is_dir() {
        return Ok(system);
    }
    Ok(home_dir().join("Applications/Claude.app"))
}

#[cfg(target_os = "windows")]
fn source_app(deadline: Instant) -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(home_dir);
    let program = std::env::var_os("ProgramFiles").map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let query = probe_output(Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; @(Get-AppxPackage -Name Claude; Get-AppxPackage -Name AnthropicPBC.Claude) | Select-Object Name,InstallLocation | ConvertTo-Json -Compress"]), deadline);
    if let Ok(output) = &query {
        if output.status.success() {
            for install in claude_package_locations(&output.stdout)? {
                let packaged = PathBuf::from(install).join(r"app\Claude.exe");
                if packaged.is_file() { return Ok(packaged); }
            }
        }
    }
    let candidates = [
        local.join(r"Microsoft\WindowsApps\claude-desktop.exe"),
        local.join(r"Microsoft\WindowsApps\Claude_pzs8sxrjxfjjc\claude-desktop.exe"),
        local.join(r"Programs\Claude\Claude.exe"),
        local.join(r"Programs\claude-desktop\Claude.exe"),
        local.join(r"Claude\Claude.exe"),
        program.join(r"Claude\Claude.exe"),
    ];
    if let Some(path) = candidates.into_iter().find(|path| path.is_file()) { return Ok(path); }
    // 查询失败不是「未安装」，明确返回错误，避免把超时误报成未安装。
    let output = query?;
    ensure!(output.status.success(), "无法检测 Claude 安装位置，请检查 Windows 应用包服务后重试");
    Ok(local.join(r"Programs\Claude\Claude.exe"))
}

#[cfg(any(target_os = "windows", test))]
fn claude_package_locations(bytes: &[u8]) -> Result<Vec<String>> {
    // PowerShell 对零、一、多条记录分别输出空白、对象、数组。
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("Claude 应用包查询返回了无效 JSON")?;
    let records = match &value {
        serde_json::Value::Array(records) => records.as_slice(),
        serde_json::Value::Null => &[],
        _ => std::slice::from_ref(&value),
    };
    Ok(records
        .iter()
        .filter_map(|record| {
            let name = record.get("Name")?.as_str()?;
            if !["Claude", "AnthropicPBC.Claude"]
                .iter()
                .any(|known| name.eq_ignore_ascii_case(known))
            {
                return None;
            }
            let path = record.get("InstallLocation")?.as_str()?.trim();
            (!path.is_empty()).then(|| path.to_string())
        })
        .collect())
}

#[cfg(any(target_os = "windows", test))]
fn is_claude_msix_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('/', "\\");
    let parts = text
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let Some(index) = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("WindowsApps"))
    else {
        return false;
    };
    let tail = &parts[index + 1..];
    if tail.len() == 1 && tail[0].eq_ignore_ascii_case("claude-desktop.exe") {
        return true; // 已注册的 App Execution Alias。
    }
    let Some(package) = tail.first() else {
        return false;
    };
    let known = ["Claude_", "AnthropicPBC.Claude_"].iter().any(|prefix| {
        package
            .get(..prefix.len())
            .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
    });
    known
        && ((tail.len() == 3
            && tail[1].eq_ignore_ascii_case("app")
            && tail[2].eq_ignore_ascii_case("Claude.exe"))
            || (tail.len() == 2 && tail[1].eq_ignore_ascii_case("claude-desktop.exe")))
}

#[cfg(target_os = "windows")]
fn windows_msix_source(path: &Path) -> bool {
    is_claude_msix_path(path)
}

#[cfg(not(target_os = "windows"))]
fn windows_msix_source(_path: &Path) -> bool { false }

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn source_app(_deadline: Instant) -> Result<PathBuf> {
    Ok(PathBuf::from("Claude.exe"))
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default()
}

/// 副本名不再带「CN」：汉化只是可选增强之一，装了 Computer Use 之类的功能时名字不该暗示语言。
const COPY_BUNDLE_NAME: &str = "Claude Enhanced.app";
const COPY_DIR_NAME: &str = "Claude Enhanced";
const LEGACY_BUNDLE_NAME: &str = "Claude CN.app";
const LEGACY_DIR_NAME: &str = "Claude CN";

#[cfg(target_os = "macos")]
fn copy_app(_source: &Path) -> PathBuf { runtime_dir().join(COPY_BUNDLE_NAME) }

#[cfg(target_os = "windows")]
fn copy_app(source: &Path) -> PathBuf {
    if windows_msix_source(source) { source.to_path_buf() } else { runtime_dir().join(COPY_DIR_NAME).join("Claude.exe") }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn copy_app(_source: &Path) -> PathBuf { runtime_dir().join(COPY_DIR_NAME) }

/// 老副本还叫「Claude CN」，改名后原地迁移一次，避免用户被迫重新准备。
fn migrate_legacy_copy() {
    let root = runtime_dir();
    #[cfg(target_os = "macos")]
    let (legacy, current) = (root.join(LEGACY_BUNDLE_NAME), root.join(COPY_BUNDLE_NAME));
    #[cfg(not(target_os = "macos"))]
    let (legacy, current) = (root.join(LEGACY_DIR_NAME), root.join(COPY_DIR_NAME));
    if legacy.exists() && !current.exists() {
        let _ = fs::rename(&legacy, &current);
    }
}

#[cfg(target_os = "windows")]
fn staged_copy_path(label: &str) -> PathBuf {
    runtime_dir().join(label).join("Claude.exe")
}

#[cfg(not(target_os = "windows"))]
fn staged_copy_path(label: &str) -> PathBuf {
    runtime_dir().join(format!("{label}.app"))
}

#[cfg(target_os = "macos")]
fn config_path() -> PathBuf {
    let support = home_dir().join("Library/Application Support");
    for name in ["Claude-3p", "Claude"] {
        let dir = support.join(name);
        if dir.is_dir() {
            return dir.join("config.json");
        }
    }
    // 无现有档案时按应用部署类型选择；不输出或解析其他配置字段。
    let local = source_app(Instant::now() + PROBE_TIMEOUT)
        .is_ok_and(|source| source.join("Contents/Resources/ion-dist").is_dir());
    support
        .join(if local { "Claude-3p" } else { "Claude" })
        .join("config.json")
}

/// Windows 上可能同时存在好几份 Claude 外壳配置，不能只挑第一个：官方版装过就会有
/// `%APPDATA%\Claude`，但商店版（MSIX）与增强副本实际读的是 `Claude-3p`
/// （见 issue #2616，用户机器上在 `%LOCALAPPDATA%\Claude-3p`）。只写 Roaming\Claude
/// 时界面仍是英文。这里返回所有已存在的候选，汉化与还原逐个处理。
#[cfg(target_os = "windows")]
fn config_paths() -> Vec<PathBuf> {
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData/Roaming"));
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    windows_config_paths(&roaming, local.as_deref())
}

/// Windows 候选的纯计算部分，目录由调用方注入，便于在任意平台上测试。
///
/// 顺序即读取优先级：`%LOCALAPPDATA%\Claude-3p`、`%APPDATA%\Claude-3p`、
/// MSIX 虚拟化目录（`Packages\<包>\LocalCache\Roaming\Claude-3p|Claude`）、
/// `%APPDATA%\Claude`。只收已存在的目录，绝不新建；一个都没有时沿用旧行为
/// 回退到 `%APPDATA%\Claude\config.json`。
#[cfg(any(target_os = "windows", test))]
fn windows_config_paths(roaming: &Path, local: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(local) = local {
        dirs.push(local.join("Claude-3p"));
    }
    dirs.push(roaming.join("Claude-3p"));
    if let Some(local) = local {
        let mut packages: Vec<PathBuf> = fs::read_dir(local.join("Packages"))
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().to_string();
                ["AnthropicPBC.Claude_", "Claude_"].iter().any(|prefix| {
                    name.get(..prefix.len())
                        .is_some_and(|value| value.eq_ignore_ascii_case(prefix))
                })
            })
            .map(|entry| entry.path().join("LocalCache").join("Roaming"))
            .collect();
        // read_dir 顺序不稳定，排序后结果可复现。
        packages.sort();
        for package in packages {
            dirs.push(package.join("Claude-3p"));
            dirs.push(package.join("Claude"));
        }
    }
    dirs.push(roaming.join("Claude"));
    let mut paths: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let path = dir.join("config.json");
        // 大小写不敏感的文件系统上同一目录可能被两种写法各收一次，去重防止重复写备份。
        if dir.is_dir() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        paths.push(roaming.join("Claude").join("config.json"));
    }
    paths
}

#[cfg(target_os = "macos")]
fn config_paths() -> Vec<PathBuf> { vec![config_path()] }

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn config_paths() -> Vec<PathBuf> { vec![home_dir().join(".claude/config.json")] }

/// 读外壳语言时取第一个已有 config.json 的候选；都没有就取第一个候选（读出来为空）。
fn primary_config(configs: &[PathBuf]) -> Option<&PathBuf> {
    configs.iter().find(|path| path.is_file()).or_else(|| configs.first())
}

/// 每份配置的语言备份都放在它自己的目录里，各自还原、互不覆盖。
fn locale_backup_path(config: &Path) -> PathBuf {
    config.with_extension("codex-plus-locale-backup.json")
}

/// 先把所有候选的当前语言读出来；任何一份格式异常就整体放弃，避免只改了一半。
fn current_locales(configs: &[PathBuf]) -> Result<Vec<Option<Value>>> {
    configs
        .iter()
        .map(|config| Ok(read_config(config)?.get("locale").cloned()))
        .collect()
}

/// 给每份配置写入同一个语言，并各自在同目录备份原语言（已有备份不覆盖）。
fn localize_configs(configs: &[PathBuf], locale: &str) -> Result<()> {
    current_locales(configs)?;
    for config in configs {
        set_locale_at(config, &locale_backup_path(config), Some(locale), false)?;
    }
    Ok(())
}

/// 把有备份的配置逐个还原成各自备份里的原语言。
///
/// 外层 None 表示一份都没还原（从没汉化过，或备份损坏）——那些配置是用户自己的设置，不碰；
/// 内层是第一个被还原的语言，汉化前本来没有语言时为 None（还原即删掉 locale 字段）。
fn restore_saved_locales(configs: &[PathBuf]) -> Result<Option<Option<String>>> {
    let mut first = None;
    for config in configs {
        let backup = locale_backup_path(config);
        if let Some(value) = read_locale_backup(&backup) {
            set_locale_at(config, &backup, value.as_deref(), true)?;
            first.get_or_insert(value);
        }
    }
    Ok(first)
}

fn find_runtime(name: &str, deadline: Instant) -> Result<Option<PathBuf>> {
    #[cfg(target_os = "windows")]
    let names: Vec<String> = if name == "python3" {
        vec!["python.exe".to_string(), "py.exe".to_string(), "python3.exe".to_string()]
    } else {
        vec![format!("{name}.exe"), name.to_string()]
    };
    #[cfg(target_os = "windows")]
    let candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)).collect::<Vec<_>>())
        .collect();
    #[cfg(not(target_os = "windows"))]
    let mut candidates = vec![home_dir().join(".local/bin").join(name)];
    #[cfg(not(target_os = "windows"))]
    candidates.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(|dir| Path::new(dir).join(name)),
    );
    #[cfg(not(target_os = "windows"))]
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|dir| dir.join(name)));
    }
    let mut checked = std::collections::BTreeSet::new();
    for path in candidates {
        if !checked.insert(path.clone()) || !path.is_file() { continue; }
        let mut command = Command::new(&path);
        if name == "node" {
            command.args(["-e", "process.exit(typeof WebSocket === 'function' && typeof fetch === 'function' ? 0 : 1)"]);
        } else if name == "powershell" {
            command.args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"]);
        } else {
            command.arg("--version");
        }
        match probe_output(&mut command, deadline) {
            Ok(output) if output.status.success() => return Ok(Some(path)),
            Ok(_) => {},
            // PATH 里不可执行/已经失效的候选沿用旧行为继续找；超时必须反馈给用户。
            Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied)
            }) => {},
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

#[cfg(target_os = "macos")]
fn app_version(app: &Path, key: &str, deadline: Instant) -> Result<String> {
    if !app.is_dir() { return Ok(String::new()); }
    let output = probe_output(Command::new("/usr/bin/defaults")
        .args(["read"]).arg(app.join("Contents/Info")).arg(key), deadline)?;
    Ok(if output.status.success() { String::from_utf8_lossy(&output.stdout).trim().to_string() } else { String::new() })
}

#[cfg(target_os = "windows")]
fn app_version(app: &Path, _key: &str, deadline: Instant) -> Result<String> {
    if !app.is_file() { return Ok(String::new()); }
    let escaped = app.to_string_lossy().replace('\'', "''");
    let output = probe_output(Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion")), deadline)?;
    Ok(if output.status.success() { String::from_utf8_lossy(&output.stdout).trim().to_string() } else { String::new() })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_version(_app: &Path, _key: &str, _deadline: Instant) -> Result<String> { Ok(String::new()) }

#[cfg(target_os = "macos")]
fn claude_running(deadline: Instant) -> Result<bool> {
    let output = probe_output(Command::new("/usr/bin/pgrep")
        .args(["-f", "/Contents/MacOS/Claude( |$)"]), deadline)?;
    ensure!(matches!(output.status.code(), Some(0 | 1)), "无法检测 Claude 运行状态");
    Ok(output.status.success())
}

#[cfg(any(windows, test))]
fn tasklist_contains_claude(output: &[u8]) -> bool {
    String::from_utf8_lossy(output).lines().any(|line| {
        let name = line.trim().strip_prefix('"').and_then(|line| line.split('"').next());
        name.is_some_and(|name| name.eq_ignore_ascii_case("Claude.exe") || name.eq_ignore_ascii_case("claude-desktop.exe"))
    })
}

#[cfg(target_os = "windows")]
fn claude_running(deadline: Instant) -> Result<bool> {
    let output = probe_output(Command::new("tasklist.exe").args(["/FO", "CSV", "/NH", "/FI", "IMAGENAME eq Claude*"]), deadline)?;
    ensure!(output.status.success(), "无法检测 Claude 运行状态");
    Ok(tasklist_contains_claude(&output.stdout))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn claude_running(_deadline: Instant) -> Result<bool> { Ok(false) }

fn catalog_entries() -> usize {
    ASSETS.iter().find(|(name, _)| *name == "catalogs/zh-Hans.json")
        .and_then(|(_, bytes)| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|value| value.as_object().map(|obj| obj.len())).unwrap_or(0)
}

fn initial_status(source: &Path, copy: &Path) -> ClaudeLocalizationStatus {
    let settings = crate::settings::SettingsStore::default().load().ok();
    ClaudeLocalizationStatus {
        supported: false,
        source_app: source.to_string_lossy().to_string(),
        copy_app: copy.to_string_lossy().to_string(),
        source_version: String::new(), copy_version: String::new(),
        prepared: false, needs_rebuild: false, running: false,
        node_available: false, python_available: false, shell_locale: None,
        catalog_entries: catalog_entries(),
        voice_input: settings.as_ref().is_none_or(|settings| settings.claude_app_voice_input),
        localization: settings.as_ref().is_none_or(|settings| settings.claude_app_localization),
        computer_use: computer_use_enabled(),
        computer_use_supported: cfg!(target_os = "macos"),
    }
}

/// 副本是否需要用当前源码与设置重新制作。
///
/// Computer Use 只在能改写 asar 的平台参与比较：Windows 的准备脚本不改 asar，
/// 回执里也就不会写 `computerUse`；若仍按「回执里没有 = 关」比较，开关一旦打开
/// 就会永远判定需要更新，点「更新副本」后依旧提示需要更新（#2673）。
fn copy_needs_rebuild(
    prepared: bool,
    receipt: Option<&Value>,
    current_build: &str,
    copy_version: &str,
    source_version: &str,
    computer_use: bool,
    computer_use_supported: bool,
) -> bool {
    if !prepared {
        return false;
    }
    let prepared_computer_use = receipt
        .and_then(|r| r.get("computerUse"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    receipt.and_then(|r| r.get("sourceBuild")).and_then(Value::as_str) != Some(current_build)
        || copy_version != source_version
        || (computer_use_supported && prepared_computer_use != computer_use)
}

/// 未安装时不能继续探测运行时、版本或读取官方配置；测试注入探测器守住早退边界。
fn inspect_installed_source(
    source: &Path,
    layout: AppLayout,
    mut state: ClaudeLocalizationStatus,
    inspect: impl FnOnce(&mut ClaudeLocalizationStatus) -> Result<()>,
) -> Result<ClaudeLocalizationStatus> {
    if !layout.exists(source) { return Ok(state); }
    state.supported = true;
    inspect(&mut state)?;
    Ok(state)
}

pub fn status() -> Result<ClaudeLocalizationStatus> {
    migrate_legacy_copy();
    let deadline = Instant::now() + STATUS_TIMEOUT;
    let source = source_app(deadline)?;
    let copy = copy_app(&source);
    inspect_installed_source(&source, AppLayout::current(), initial_status(&source, &copy), |state| {
        state.source_version = app_version(&source, "CFBundleShortVersionString", deadline)?;
        state.copy_version = app_version(&copy, "CFBundleShortVersionString", deadline)?;
        let current_build = if cfg!(windows) { state.source_version.clone() }
            else { app_version(&source, "CFBundleVersion", deadline)? };
        let receipt = fs::read(runtime_dir().join("prepared.json")).ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        state.prepared = AppLayout::current().exists(&copy) && receipt.is_some();
        state.needs_rebuild = copy_needs_rebuild(
            state.prepared,
            receipt.as_ref(),
            &current_build,
            &state.copy_version,
            &state.source_version,
            state.computer_use,
            state.computer_use_supported,
        );
        state.shell_locale = primary_config(&config_paths())
            .and_then(|config| read_config(config).ok())
            .and_then(|v| v.get("locale")?.as_str().map(str::to_string));
        state.running = claude_running(deadline)?;
        state.node_available = find_runtime("node", deadline)?.is_some();
        // Windows 的准备脚本用 PowerShell，不需遍历或执行 Python 安装别名。
        state.python_available = if cfg!(windows) { false } else { find_runtime("python3", deadline)?.is_some() };
        Ok(())
    })
}

/// 语音入口开关。读不到设置时按开启处理，与 settings 里的默认值保持一致。
fn voice_input_enabled() -> bool {
    crate::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.claude_app_voice_input)
        .unwrap_or(true)
}

/// 副本的语音入口靠 Codex++ helper 转写（hook.js 读 helper 落下的凭据）。helper 原本只随
/// Codex 会话或协议代理常驻，没在跑 Codex 时 Claude 里点麦克风只会提示「语音服务未运行」。
/// 所以开关打开且已经制作过副本时，管理器要把 helper 常驻起来。
///
/// 只看副本凭据，不探测版本与进程，启动链路可以廉价调用；没做过副本的用户不占端口。
pub fn voice_input_needs_helper(settings: &crate::settings::BackendSettings) -> bool {
    voice_input_needs_helper_at(&runtime_dir(), settings.claude_app_voice_input)
}

fn voice_input_needs_helper_at(root: &Path, voice_input: bool) -> bool {
    voice_input && root.join("prepared.json").is_file()
}

/// Computer Use 开关。读不到按关闭处理——这条会改 asar，不能默认开。
fn computer_use_enabled() -> bool {
    crate::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.claude_app_computer_use)
        .unwrap_or(false)
}

/// 界面汉化开关。同样是读不到就按开启处理。
fn localization_enabled() -> bool {
    crate::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.claude_app_localization)
        .unwrap_or(true)
}

/// 写入语音入口开关。只改 Codex++ 自己的设置，不触碰副本与官方应用。
pub fn set_voice_input(enabled: bool) -> Result<ClaudeLocalizationStatus> {
    let store = crate::settings::SettingsStore::default();
    let mut settings = store.load()?;
    settings.claude_app_voice_input = enabled;
    store.save(&settings)?;
    status()
}

/// 写入界面汉化开关。副本是否已准备、是否重建，与这个开关无关。
/// 写入 Computer Use 开关。只改设置；副本要重新准备才会带上/去掉 asar 改写。
pub fn set_computer_use(enabled: bool) -> Result<ClaudeLocalizationStatus> {
    let store = crate::settings::SettingsStore::default();
    let mut settings = store.load()?;
    settings.claude_app_computer_use = enabled;
    store.save(&settings)?;
    status()
}

pub fn set_localization(enabled: bool) -> Result<ClaudeLocalizationStatus> {
    let store = crate::settings::SettingsStore::default();
    let mut settings = store.load()?;
    settings.claude_app_localization = enabled;
    store.save(&settings)?;
    if !enabled {
        // 关掉汉化就顺手把外壳语言还原回去，不必等到下次启动副本——否则用户直接打开
        // 官方 Claude 时看到的还是被改过的语言，会觉得开关没生效。
        restore_shell_locale()?;
    }
    status()
}

/// 读汉化前备份里的原语言。没有备份、或备份损坏时返回 None——调用方据此判定
/// 「从来没汉化过」，从而不去碰用户自己的语言设置。
///
/// 内层 None 表示备份记录的是「汉化前没有语言」：还原时要删掉 locale，
/// 否则那份配置会一直停在 zh-Hans。
fn read_locale_backup(backup: &Path) -> Option<Option<String>> {
    let value = fs::read(backup)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(Value::is_object)?;
    Some(value.get("locale").and_then(Value::as_str).map(str::to_string))
}

/// 把各份 Claude 外壳配置的语言还原成各自汉化前备份里的值。
///
/// 没有备份的那份说明从来没汉化过，什么都不做：用户自己的语言设置不该被我们删掉。
fn restore_shell_locale() -> Result<()> {
    restore_saved_locales(&config_paths()).map(|_| ())
}

fn install_assets(root: &Path) -> Result<()> {
    for (relative, bytes) in ASSETS {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().context("缺少运行目录")?)?;
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn hidden_script_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    command
}

fn run_script(
    program: &Path,
    script: &str,
    source: &Path,
    copy: &Path,
    args: &[&str],
    progress: &dyn Fn(u8, &str),
) -> Result<()> {
    let root = runtime_dir();
    let log = root.join("operation.log");
    let output = fs::File::create(&log)?;
    // 只隐藏脚本宿主，不给 Claude 本身加 WindowStyle Hidden，否则会破坏注入。
    let mut command = hidden_script_command(program);
    if cfg!(target_os = "windows") && script.ends_with(".ps1") {
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
    }
    let mut child = command
        .arg(root.join(script))
        .args(args)
        .env("CODEX_PLUS_CLAUDE_SOURCE", source)
        .env("CODEX_PLUS_CLAUDE_COPY", copy)
        // 启动脚本据此决定要不要给页面注入语音入口；构建/校验脚本读到也无害。
        .env(
            "CODEX_PLUS_CLAUDE_VOICE_INPUT",
            if voice_input_enabled() { "1" } else { "0" },
        )
        // 只有勾选后才改写副本 asar（并关闭 asar 完整性校验）。
        .env(
            "CODEX_PLUS_CLAUDE_COMPUTER_USE",
            if computer_use_enabled() { "1" } else { "0" },
        )
        // 同理：注入器按这个决定要不要带中文界面那一段。
        .env(
            "CODEX_PLUS_CLAUDE_LOCALIZATION",
            if localization_enabled() { "1" } else { "0" },
        )
        .env(
            "CODEX_PLUS_CLAUDE_PROGRESS",
            root.join("progress.json"),
        )
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()
        .context("无法启动汉化组件")?;
    let deadline = Instant::now() + Duration::from_secs(300);
    let progress_file = root.join("progress.json");
    let mut last_percent = 0u8;
    loop {
        if let Ok(text) = fs::read_to_string(&progress_file) {
            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                let percent = value.get("percent").and_then(Value::as_u64).unwrap_or(0).min(100) as u8;
                let message = value.get("message").and_then(Value::as_str).unwrap_or("");
                if percent != last_percent {
                    last_percent = percent;
                    progress(percent, message);
                }
            }
        }
        if let Some(exit) = child.try_wait()? {
            if exit.success() {
                return Ok(());
            }
            let text = fs::read_to_string(&log).unwrap_or_default();
            let tail = text
                .lines()
                .rev()
                .take(6)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            bail!("汉化操作失败：{tail}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("汉化操作超时；保留旧副本。请检查 Claude 是否等待钥匙串授权。");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn read_config(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    // Windows 上 Claude 的 config.json 可能带 UTF-8 BOM（#2728），JSON 解析器不认，先去掉。
    let bytes = fs::read(path)?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let value: Value = serde_json::from_slice(bytes)
        .with_context(|| format!("{} 不是合法 JSON，未修改语言", path.display()))?;
    ensure!(value.is_object(), "Claude 配置格式异常，未修改语言");
    Ok(value)
}

/// 只保存原语言，不回写整个旧配置，避免还原时丢失后来的设置。
fn set_locale_at(config: &Path, backup: &Path, locale: Option<&str>, restore: bool) -> Result<()> {
    let mut value = read_config(config)?;
    if !restore && !backup.exists() {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            backup,
            serde_json::to_vec(
                &json!({"locale": value.get("locale").cloned().unwrap_or(Value::Null)}),
            )?,
        )?;
    }
    if let Some(locale) = locale {
        value["locale"] = Value::String(locale.to_string());
    } else {
        value
            .as_object_mut()
            .context("配置格式异常")?
            .remove("locale");
    }
    if let Some(parent) = config.parent() {
        fs::create_dir_all(parent)?;
    }
    // 同目录暂存后 rename，避免中断留下不完整的 config.json。
    let temp = config.with_file_name(format!("codex-plus-locale-{}.json", uuid::Uuid::new_v4()));
    fs::write(&temp, serde_json::to_vec_pretty(&value)?)?;
    fs::rename(temp, config)?;
    Ok(())
}

pub fn perform(action: &str) -> Result<ClaudeLocalizationResult> {
    perform_with_progress(action, &|_, _| {})
}

/// 带进度回调的版本：percent 0-100，message 是给用户看的中文短句。
pub fn perform_with_progress(
    action: &str,
    progress: &dyn Fn(u8, &str),
) -> Result<ClaudeLocalizationResult> {
    ensure!(cfg!(any(target_os = "macos", target_os = "windows")), "Claude 增强目前支持 macOS 和 Windows");
    ensure!(
        ["prepare", "check", "launch", "restore"].contains(&action),
        "未知汉化操作"
    );
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 增强操作正在进行"))?;
    let deadline = Instant::now() + STATUS_TIMEOUT;
    let source = source_app(deadline)?;
    ensure!(AppLayout::current().exists(&source), "未检测到 Claude，请先安装官方应用");
    let script_runtime = if cfg!(target_os = "windows") {
        find_runtime("powershell", deadline)?.context("未找到 Windows PowerShell")?
    } else {
        find_runtime("python3", deadline)?.context("未找到 Python 3，请安装后再准备副本")?
    };
    if action != "check" {
        ensure!(
            !claude_running(deadline)?,
            "Claude 正在运行，请先完全退出，再执行此操作"
        );
    }
    let root = runtime_dir();
    install_assets(&root)?;
    let copy = copy_app(&source);
    let message = match action {
        "prepare" => {
            ensure!(
                find_runtime("node", Instant::now() + STATUS_TIMEOUT)?.is_some(),
                "需要带有内置 WebSocket 的 Node.js 22 或更新版本才能中文启动"
            );
            if cfg!(target_os = "windows") && windows_msix_source(&source) {
                fs::write(root.join("prepared.json"), serde_json::to_vec(&json!({
                    "sourceBuild": app_version(&source, "ProductVersion", Instant::now() + STATUS_TIMEOUT)?,
                    "msix": true
                }))?)?;
                "已检测到 Windows MSIX 版 Claude；保留官方包不变，使用启动注入。".to_string()
            } else {
            let staged = staged_copy_path(&format!("Claude-staging-{}", uuid::Uuid::new_v4()));
            let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
            progress(2, "准备增强版目录");
            run_script(&script_runtime, script, &source, &staged, &[], progress)?;
            progress(92, "校验增强版");
            run_script(&script_runtime, script, &source, &staged, &["--check"], progress)?;
            // 校验通过后才替换；旧副本保留，构建失败时不会影响它。
            let old = staged_copy_path(&format!("Claude-backup-{}", uuid::Uuid::new_v4()));
            replace_copy(AppLayout::current(), &staged, &copy, &old)?;
            fs::write(
                root.join("prepared.json"),
                serde_json::to_vec(&json!({
                    "sourceBuild": app_version(&source, "CFBundleVersion", Instant::now() + STATUS_TIMEOUT)?,
                    "computerUse": computer_use_enabled()
                }))?,
            )?;
            "增强版已制作，签名与 app.asar 校验通过。".to_string()
            }
        }
        "check" => {
            if cfg!(target_os = "windows") && windows_msix_source(&source) {
                ensure!(source.exists(), "未找到 Claude MSIX 安装入口");
                "Windows MSIX 安装入口已检测到，采用原版包启动注入。".to_string()
            } else {
                let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
                run_script(&script_runtime, script, &source, &copy, &["--check"], &|_, _| {})?;
                "副本签名、中文词表、注入开关及 app.asar 一致性校验通过。".to_string()
            }
        }
        "launch" | "restore" => {
            let launcher = if cfg!(target_os = "windows") {
                script_runtime.clone()
            } else {
                find_runtime("node", Instant::now() + STATUS_TIMEOUT)?.context("未找到带内置 WebSocket 的 Node.js 22 或更新版本")?
            };
            if !(cfg!(target_os = "windows") && windows_msix_source(&source)) {
                let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
                run_script(&script_runtime, script, &source, &copy, &["--check"], &|_, _| {})?;
            }
            // 所有候选配置逐个处理（Windows 上官方版与商店版各有一份，见 #2616）。
            let configs = config_paths();
            let previous = current_locales(&configs)?;
            let localization = localization_enabled();
            let locale = if action == "restore" {
                ensure!(
                    configs.iter().any(|config| locale_backup_path(config).is_file()),
                    "没有 codex++ 保存的语言备份"
                );
                restore_saved_locales(&configs)?
                    .flatten()
                    .unwrap_or_else(|| "en-US".to_string())
            } else if localization {
                localize_configs(&configs, "zh-Hans")?;
                "zh-Hans".to_string()
            } else {
                // 汉化关掉：副本按原语言启动。有备份说明之前汉化过，还原它；
                // 从没汉化过就完全不碰用户自己的语言设置。
                match restore_saved_locales(&configs)? {
                    Some(value) => value.unwrap_or_else(|| "en-US".to_string()),
                    None => previous
                        .iter()
                        .flatten()
                        .find_map(Value::as_str)
                        .unwrap_or("en-US")
                        .to_string(),
                }
            };
            let script = if cfg!(target_os = "windows") { "inject-cn.ps1" } else { "inject-cn.js" };
            if let Err(error) = run_script(&launcher, script, &source, &copy, &[if cfg!(windows) { "-Locale" } else { "--locale" }, locale.as_str()], &|_, _| {}) {
                // 启动失败后只在应用未运行时还原本次外壳语言，避免与 Claude 写盘竞争。
                if !claude_running(Instant::now() + PROBE_TIMEOUT).unwrap_or(true) {
                    for (config, previous) in configs.iter().zip(&previous) {
                        let _ = set_locale_at(
                            config,
                            &locale_backup_path(config),
                            previous.as_ref().and_then(Value::as_str),
                            true,
                        );
                    }
                }
                return Err(error);
            }
            if action == "restore" {
                "已还原保存的语言并启动副本。".to_string()
            } else if !localization {
                "已按原语言启动副本，其他增强照常注入。".to_string()
            } else {
                "中文启动注入完成，主界面将在加载后应用中文。".to_string()
            }
        }
        _ => unreachable!(),
    };
    Ok(ClaudeLocalizationResult {
        status: status()?,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows 上 Claude 的 config.json 带 UTF-8 BOM 时也能读、能汉化（#2728）。
    #[test]
    fn config_with_utf8_bom_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        fs::write(&config, b"\xEF\xBB\xBF{\"locale\":\"ja-JP\",\"other\":1}").unwrap();
        assert_eq!(read_config(&config).unwrap()["locale"], "ja-JP");
        localize_configs(&[config.clone()], "zh-Hans").unwrap();
        assert_eq!(read_config(&config).unwrap()["locale"], "zh-Hans");
        assert_eq!(read_config(&config).unwrap()["other"], 1);
    }

    #[test]
    fn claude_msix_namespace_discovery_and_classification_agree() {
        let packages = br#"[{"Name":"AnthropicPBC.Claude","InstallLocation":"C:\\Program Files\\WindowsApps\\AnthropicPBC.Claude_2.42304.0.0_x64__fnn82j28hfe8t"},{"Name":"ClaudeFork","InstallLocation":"C:\\Other"}]"#;
        let locations = claude_package_locations(packages).unwrap();
        assert_eq!(locations.len(), 1);
        assert!(is_claude_msix_path(Path::new(&format!(
            "{}\\app\\Claude.exe",
            locations[0]
        ))));
        assert_eq!(
            claude_package_locations(br#"{"Name":"Claude","InstallLocation":"C:\\legacy"}"#)
                .unwrap(),
            vec![r"C:\legacy"]
        );
        assert!(claude_package_locations(b"").unwrap().is_empty());
        assert!(claude_package_locations(b"not JSON").is_err());
    }

    #[test]
    fn claude_msix_classification_preserves_alias_and_rejects_lookalikes() {
        for path in [
            r"C:\Users\test\AppData\Local\Microsoft\WindowsApps\claude-desktop.exe",
            r"C:\Program Files\WindowsApps\Claude_1.0_x64__publisher\app\Claude.exe",
        ] {
            assert!(is_claude_msix_path(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\Programs\Claude\Claude.exe",
            r"C:\WindowsApps-other\AnthropicPBC.Claude_1_x64__publisher\app\Claude.exe",
            r"C:\WindowsApps\ClaudeFork_1_x64__publisher\app\Claude.exe",
            r"C:\WindowsApps\AnthropicPBC.Claude_1_x64__publisher\other\Claude.exe",
        ] {
            assert!(!is_claude_msix_path(Path::new(path)), "{path}");
        }
    }


    #[test]
    fn catalog_entries_come_from_the_chinese_catalog() {
        // 曾经用 ASSETS[7] 取词表：往资产清单里插一个文件就让下标整体错位，
        // 界面上的「官方词条」直接变成 0。这里钉住它按名字取、且确实读到了词条。
        let entries = catalog_entries();
        assert!(entries > 1000, "官方中文词表应有上千条，实际 {entries}");
    }

    fn empty_test_status() -> ClaudeLocalizationStatus {
        ClaudeLocalizationStatus {
            supported: false, source_app: String::new(), copy_app: String::new(),
            source_version: String::new(), copy_version: String::new(),
            prepared: false, needs_rebuild: false, running: false,
            node_available: false, python_available: false, shell_locale: None,
            catalog_entries: 0, voice_input: true, localization: true, computer_use: false,
            computer_use_supported: cfg!(target_os = "macos"),
        }
    }

    #[test]
    fn windows_never_asks_to_rebuild_for_computer_use() {
        // #2673：Windows 的 MSIX 回执没有 computerUse；开关打开时不能因此永远要求更新。
        let receipt = json!({ "sourceBuild": "26.1", "msix": true });
        assert!(!copy_needs_rebuild(true, Some(&receipt), "26.1", "26.1", "26.1", true, false));
        // macOS 仍按回执比较：开关与回执不一致就该重建。
        assert!(copy_needs_rebuild(true, Some(&receipt), "26.1", "26.1", "26.1", true, true));
        assert!(!copy_needs_rebuild(true, Some(&receipt), "26.1", "26.1", "26.1", false, true));
        // 版本变化与未准备的情况对所有平台一致。
        assert!(copy_needs_rebuild(true, Some(&receipt), "26.2", "26.1", "26.1", false, false));
        assert!(copy_needs_rebuild(true, Some(&receipt), "26.1", "26.2", "26.1", false, false));
        assert!(!copy_needs_rebuild(false, None, "26.1", "26.1", "26.1", true, false));
    }

    #[test]
    fn missing_claude_returns_without_running_any_deep_probe() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Claude.exe");
        let result = inspect_installed_source(&missing, AppLayout::Executable, empty_test_status(), |_| {
            panic!("没有安装 Claude 时不应探测运行时、版本或读取官方配置");
        }).unwrap();
        assert!(!result.supported);
        assert!(!result.prepared);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn windows_executable_and_macos_bundle_use_their_own_layout() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Claude.exe");
        fs::write(&exe, b"test exe").unwrap();
        let bundle = dir.path().join("Claude.app");
        fs::create_dir(&bundle).unwrap();
        assert!(AppLayout::Executable.exists(&exe));
        assert!(!AppLayout::Bundle.exists(&exe));
        assert!(AppLayout::Bundle.exists(&bundle));
        assert!(!AppLayout::Executable.exists(&bundle));
        let result = inspect_installed_source(&exe, AppLayout::Executable, empty_test_status(), |state| {
            state.prepared = true;
            Ok(())
        }).unwrap();
        assert!(result.supported && result.prepared);
    }

    #[test]
    fn tasklist_success_without_matching_process_is_not_running() {
        assert!(!tasklist_contains_claude(b"INFO: No tasks are running which match the specified criteria.\r\n"));
        assert!(!tasklist_contains_claude(b"\"NotClaude.exe\",\"12\",\"Console\"\r\n"));
        assert!(tasklist_contains_claude(b"\"cLaUdE.exe\",\"123\",\"Console\"\r\n"));
        assert!(tasklist_contains_claude(b"\"claude-desktop.exe\",\"123\",\"Console\"\r\n"));
    }

    fn seed_windows_copy(root: &Path, version: &[u8]) -> PathBuf {
        fs::create_dir_all(root.join("resources")).unwrap();
        fs::write(root.join("Claude.exe"), version).unwrap();
        fs::write(root.join("resources/app.asar"), version).unwrap();
        root.join("Claude.exe")
    }

    #[test]
    fn windows_preparation_moves_resources_with_exe_and_keeps_previous_copy() {
        let dir = tempfile::tempdir().unwrap();
        let staged = seed_windows_copy(&dir.path().join("staging"), b"new");
        let copy = seed_windows_copy(&dir.path().join("copy"), b"old");
        let backup = dir.path().join("backup/Claude.exe");
        replace_copy(AppLayout::Executable, &staged, &copy, &backup).unwrap();
        assert_eq!(fs::read(copy.parent().unwrap().join("resources/app.asar")).unwrap(), b"new");
        assert_eq!(fs::read(&copy).unwrap(), b"new");
        assert_eq!(fs::read(backup.parent().unwrap().join("resources/app.asar")).unwrap(), b"old");
    }

    #[test]
    fn failed_windows_copy_replacement_restores_entire_previous_directory() {
        let dir = tempfile::tempdir().unwrap();
        let staged = seed_windows_copy(&dir.path().join("staging"), b"new");
        let copy = seed_windows_copy(&dir.path().join("copy"), b"old");
        let backup = dir.path().join("backup/Claude.exe");
        let mut calls = 0;
        let result = replace_copy_with(AppLayout::Executable, &staged, &copy, &backup, |from, to| {
            calls += 1;
            if calls == 2 { return Err(std::io::Error::other("模拟第二次 rename 失败")); }
            fs::rename(from, to)
        });
        assert!(result.is_err());
        assert_eq!(calls, 3);
        assert_eq!(fs::read(&copy).unwrap(), b"old");
        assert_eq!(fs::read(copy.parent().unwrap().join("resources/app.asar")).unwrap(), b"old");
        assert_eq!(fs::read(staged.parent().unwrap().join("resources/app.asar")).unwrap(), b"new");
    }

    #[test]
    fn powershell_assets_are_utf8_bom_and_survive_checkout_newline_conversion() {
        for name in ["build-copy.ps1", "inject-cn.ps1"] {
            let bytes = ASSETS.iter().find(|(asset, _)| *asset == name).unwrap().1;
            assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
            let text = std::str::from_utf8(bytes).unwrap();
            assert!(!text.contains('\0'));
            assert!(text.contains("CODEX_PLUS_CLAUDE_COPY"));
            assert!(text.replace('\n', "\r\n").contains("$ErrorActionPreference"));
        }
        let build = std::str::from_utf8(ASSETS.iter().find(|(name, _)| *name == "build-copy.ps1").unwrap().1).unwrap();
        assert!(build.contains("[Security.Cryptography.SHA256]::Create()"));
        assert!(!build.contains("Get-FileHash"));
        let inject = std::str::from_utf8(ASSETS.iter().find(|(name, _)| *name == "inject-cn.ps1").unwrap().1).unwrap();
        assert!(!inject.contains("-WindowStyle Hidden"));
    }

    /// 超时回归的子进程就是当前测试二进制，不运行 shell 或用户安装的脚本。
    #[test]
    fn probe_fixture_child() {
        let Some(marker) = std::env::var_os("CODEX_PLUS_CLAUDE_TEST_PROBE") else { return; };
        fs::write(&marker, b"started").unwrap();
        std::thread::sleep(Duration::from_millis(900));
        fs::write(marker, b"not killed").unwrap();
    }

    #[test]
    fn hung_probe_is_terminated_and_reaped_instead_of_timing_out_only_in_ui() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("probe-state");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "claude_localization::tests::probe_fixture_child", "--nocapture"])
            .env("CODEX_PLUS_CLAUDE_TEST_PROBE", &marker);
        let started = Instant::now();
        let result = probe_output(&mut command, started + Duration::from_millis(500));
        assert!(result.unwrap_err().to_string().contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(fs::read(&marker).unwrap(), b"started", "子进程应已启动才能验证回收");
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(fs::read(marker).unwrap(), b"started", "超时返回后子进程不能继续运行");
    }

    #[test]
    fn locale_backup_tolerates_missing_or_broken_backup() {
        // 关掉汉化时会按这个值还原官方语言；读不出来就必须什么都不做，
        // 否则会把用户自己的语言设置删掉。
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("locale-backup.json");
        assert_eq!(read_locale_backup(&backup), None, "没有备份时不能凭空给一个语言");
        fs::write(&backup, br#"{"locale":"ja-JP","other":1}"#).unwrap();
        assert_eq!(read_locale_backup(&backup), Some(Some("ja-JP".to_string())));
        fs::write(&backup, br#"{"locale":null}"#).unwrap();
        assert_eq!(read_locale_backup(&backup), Some(None), "汉化前没有语言也是有效备份");
        fs::write(&backup, b"not json").unwrap();
        assert_eq!(read_locale_backup(&backup), None, "备份损坏时不能乱动用户设置");
        fs::write(&backup, b"[]").unwrap();
        assert_eq!(read_locale_backup(&backup), None, "非对象备份视为损坏");
    }

    #[test]
    fn restore_preserves_later_settings_and_missing_locale() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        let backup = dir.path().join("locale-backup.json");
        fs::write(&config, br#"{"locale":"ja-JP","other":1}"#).unwrap();
        set_locale_at(&config, &backup, Some("zh-Hans"), false).unwrap();
        let mut newer = read_config(&config).unwrap();
        newer["other"] = json!(2);
        fs::write(&config, serde_json::to_vec(&newer).unwrap()).unwrap();
        set_locale_at(&config, &backup, Some("zh-Hans"), false).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&backup).unwrap()).unwrap();
        assert_eq!(saved["locale"], "ja-JP");
        set_locale_at(&config, &backup, saved["locale"].as_str(), true).unwrap();
        let restored = read_config(&config).unwrap();
        assert_eq!(restored["locale"], "ja-JP");
        assert_eq!(restored["other"], 2);
        set_locale_at(&config, &backup, None, true).unwrap();
        assert!(read_config(&config).unwrap().get("locale").is_none());
    }

    #[test]
    fn malformed_config_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        fs::write(&config, b"broken").unwrap();
        assert!(
            set_locale_at(&config, &dir.path().join("backup"), Some("zh-Hans"), false).is_err()
        );
        assert_eq!(fs::read(&config).unwrap(), b"broken");
        assert!(!dir.path().join("backup").exists());
    }

    #[test]
    fn windows_config_paths_cover_store_and_official_profiles_in_order() {
        // #2616：官方版的 Roaming\Claude 存在时，商店版读的 Claude-3p 也必须被写到。
        let dir = tempfile::tempdir().unwrap();
        let roaming = dir.path().join("Roaming");
        let local = dir.path().join("Local");
        // 都不存在：沿用旧回退，且不能顺手建目录。
        assert_eq!(
            windows_config_paths(&roaming, Some(&local)),
            vec![roaming.join("Claude/config.json")]
        );
        assert!(!roaming.exists() && !local.exists(), "计算候选不能新建目录");

        let package = local.join("Packages/AnthropicPBC.Claude_fnn82j28hfe8t/LocalCache/Roaming");
        for created in [
            roaming.join("Claude"),
            roaming.join("Claude-3p"),
            local.join("Claude-3p"),
            package.join("Claude"),
            package.join("Claude-3p"),
            // 名字相近的其他包不算。
            local.join("Packages/ClaudeFork_x/LocalCache/Roaming/Claude-3p"),
        ] {
            fs::create_dir_all(created).unwrap();
        }
        assert_eq!(
            windows_config_paths(&roaming, Some(&local)),
            vec![
                local.join("Claude-3p/config.json"),
                roaming.join("Claude-3p/config.json"),
                package.join("Claude-3p/config.json"),
                package.join("Claude/config.json"),
                roaming.join("Claude/config.json"),
            ]
        );
        // 只有部分存在时，缺的不出现、也不被创建。
        let dir = tempfile::tempdir().unwrap();
        let roaming = dir.path().join("Roaming");
        let local = dir.path().join("Local");
        fs::create_dir_all(roaming.join("Claude")).unwrap();
        fs::create_dir_all(local.join("Claude-3p")).unwrap();
        assert_eq!(
            windows_config_paths(&roaming, Some(&local)),
            vec![local.join("Claude-3p/config.json"), roaming.join("Claude/config.json")]
        );
        assert!(!roaming.join("Claude-3p").exists());
        assert!(!local.join("Packages").exists());
        // 拿不到 LOCALAPPDATA 时只看 Roaming。
        assert_eq!(windows_config_paths(&roaming, None), vec![roaming.join("Claude/config.json")]);
    }

    #[test]
    fn localize_and_restore_every_config_symmetrically() {
        let dir = tempfile::tempdir().unwrap();
        let official = dir.path().join("Roaming/Claude/config.json");
        let store = dir.path().join("Local/Claude-3p/config.json");
        fs::create_dir_all(official.parent().unwrap()).unwrap();
        fs::create_dir_all(store.parent().unwrap()).unwrap();
        fs::write(&official, br#"{"locale":"ja-JP","other":1}"#).unwrap();
        // 商店版这份没有 config.json（还没写过语言）。
        let configs = vec![store.clone(), official.clone()];
        assert_eq!(primary_config(&configs), Some(&official), "读取取第一个已有配置");

        localize_configs(&configs, "zh-Hans").unwrap();
        for config in &configs {
            assert_eq!(read_config(config).unwrap()["locale"], "zh-Hans", "{config:?}");
            assert!(locale_backup_path(config).is_file(), "每份配置各自备份：{config:?}");
        }
        assert_eq!(primary_config(&configs), Some(&store));
        // 再汉化一次不能用 zh-Hans 覆盖原备份。
        localize_configs(&configs, "zh-Hans").unwrap();

        // 第一个是商店版，汉化前没有语言。
        assert_eq!(restore_saved_locales(&configs).unwrap(), Some(None));
        assert_eq!(read_config(&official).unwrap()["locale"], "ja-JP");
        assert_eq!(read_config(&official).unwrap()["other"], 1);
        // 汉化前没有语言的那份要把 locale 删掉，不能停在 zh-Hans。
        assert!(read_config(&store).unwrap().get("locale").is_none());

        // 从没汉化过（没有备份）的配置不碰。
        let untouched = dir.path().join("Other/Claude/config.json");
        fs::create_dir_all(untouched.parent().unwrap()).unwrap();
        fs::write(&untouched, br#"{"locale":"fr-FR"}"#).unwrap();
        assert_eq!(restore_saved_locales(std::slice::from_ref(&untouched)).unwrap(), None);
        assert_eq!(read_config(&untouched).unwrap()["locale"], "fr-FR");
    }

    #[test]
    fn localize_configs_touches_nothing_when_any_config_is_malformed() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("a/config.json");
        let broken = dir.path().join("b/config.json");
        fs::create_dir_all(good.parent().unwrap()).unwrap();
        fs::create_dir_all(broken.parent().unwrap()).unwrap();
        fs::write(&good, br#"{"locale":"en-US"}"#).unwrap();
        fs::write(&broken, b"broken").unwrap();
        assert!(localize_configs(&[good.clone(), broken.clone()], "zh-Hans").is_err());
        assert_eq!(read_config(&good).unwrap()["locale"], "en-US");
        assert!(!locale_backup_path(&good).exists());
        assert_eq!(fs::read(&broken).unwrap(), b"broken");
    }

    #[test]
    fn bundled_catalogs_are_real_json_and_install_without_external_paths() {
        let dir = tempfile::tempdir().unwrap();
        install_assets(dir.path()).unwrap();
        let catalog: Value =
            serde_json::from_slice(&fs::read(dir.path().join("catalogs/zh-Hans.json")).unwrap())
                .unwrap();
        assert!(catalog.as_object().unwrap().len() > 30_000);
        let dynamic: Value = serde_json::from_slice(
            &fs::read(dir.path().join("catalogs/dynamic/zh-Hans.json")).unwrap(),
        )
        .unwrap();
        assert!(!dynamic.as_object().unwrap().is_empty());
    }

    #[test]
    fn voice_input_needs_helper_only_after_copy_is_prepared() {
        let dir = tempfile::tempdir().unwrap();
        // 没做过副本：语音开关默认开启也不能让管理器常驻 helper 占端口。
        assert!(!voice_input_needs_helper_at(dir.path(), true));
        fs::write(dir.path().join("prepared.json"), b"{}").unwrap();
        assert!(voice_input_needs_helper_at(dir.path(), true));
        assert!(!voice_input_needs_helper_at(dir.path(), false));
    }
}
