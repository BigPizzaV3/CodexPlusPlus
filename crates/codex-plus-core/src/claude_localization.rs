//! Claude Desktop 汉化：受管副本、启动注入与可恢复的用户语言设置。
//! 命令只由管理器调用，不开放 HTTP / 第三方扩展路由。
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

static OPERATION: Mutex<()> = Mutex::new(());
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
fn source_app() -> PathBuf {
    let system = PathBuf::from("/Applications/Claude.app");
    if system.is_dir() {
        return system;
    }
    home_dir().join("Applications/Claude.app")
}

#[cfg(target_os = "windows")]
fn source_app() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(home_dir);
    let program = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    // MSIX 的 shim 在 WindowsApps 下，而真正的 Electron 可执行文件位于
    // 包目录的 app\claude.exe；优先取后者，才能创建受管副本。
    if let Ok(output) = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg("(Get-AppxPackage -Name Claude).InstallLocation")
        .output()
    {
        let install = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let packaged = PathBuf::from(install).join(r"app\claude.exe");
        if packaged.is_file() { return packaged; }
    }
    let candidates = [
        local.join(r"Microsoft\WindowsApps\claude-desktop.exe"),
        local.join(r"Microsoft\WindowsApps\Claude_pzs8sxrjxfjjc\claude-desktop.exe"),
        local.join(r"Programs\Claude\Claude.exe"),
        local.join(r"Programs\claude-desktop\Claude.exe"),
        local.join(r"Claude\Claude.exe"),
        program.join(r"Claude\Claude.exe"),
    ];
    candidates.into_iter().find(|path| path.is_file()).unwrap_or_else(|| local.join(r"Programs\Claude\Claude.exe"))
}

#[cfg(target_os = "windows")]
fn windows_msix_source(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_ascii_lowercase();
    lower.ends_with("\\claude-desktop.exe") && lower.contains("\\windowsapps\\")
}

#[cfg(not(target_os = "windows"))]
fn windows_msix_source(_path: &Path) -> bool { false }

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn source_app() -> PathBuf {
    PathBuf::from("Claude.exe")
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn copy_app() -> PathBuf { runtime_dir().join("Claude CN.app") }

#[cfg(target_os = "windows")]
fn copy_app() -> PathBuf {
    let source = source_app();
    if windows_msix_source(&source) { source } else { runtime_dir().join("Claude CN").join("Claude.exe") }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn copy_app() -> PathBuf { runtime_dir().join("Claude CN") }

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
    let local = source_app().join("Contents/Resources/ion-dist").is_dir();
    support
        .join(if local { "Claude-3p" } else { "Claude" })
        .join("config.json")
}

#[cfg(target_os = "windows")]
fn config_path() -> PathBuf {
    let support = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_dir().join("AppData/Roaming"));
    for name in ["Claude", "Claude-3p"] {
        let dir = support.join(name);
        if dir.is_dir() { return dir.join("config.json"); }
    }
    support.join("Claude").join("config.json")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn config_path() -> PathBuf { home_dir().join(".claude/config.json") }

fn find_runtime(name: &str) -> Option<PathBuf> {
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
    candidates.into_iter().find(|path| {
        if !path.is_file() { return false; }
        let mut command = Command::new(path);
        if name == "node" {
            command.args(["-e", "process.exit(typeof WebSocket === 'function' && typeof fetch === 'function' ? 0 : 1)"]);
        } else if name == "powershell" {
            command.args(["-NoProfile", "-NonInteractive", "-Command", "exit 0"]);
        } else {
            command.arg("--version");
        }
        command.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
    })
}

#[cfg(target_os = "macos")]
fn app_version(app: &Path, key: &str) -> String {
    Command::new("/usr/bin/defaults")
        .args(["read"])
        .arg(app.join("Contents/Info"))
        .arg(key)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn app_version(app: &Path, _key: &str) -> String {
    let escaped = app.to_string_lossy().replace('\'', "''");
    Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.ProductVersion"))
        .output().ok().filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()).unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_version(_app: &Path, _key: &str) -> String { String::new() }

#[cfg(target_os = "macos")]
fn claude_running() -> bool {
    // 仅取 PID，不读取/输出进程参数或用户档案。
    Command::new("/usr/bin/pgrep")
        .args(["-f", "/Contents/MacOS/Claude( |$)"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "windows")]
fn claude_running() -> bool {
    Command::new("tasklist.exe")
        .args(["/FI", "IMAGENAME eq Claude.exe", "/NH"])
        .stdout(Stdio::null()).stderr(Stdio::null()).status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn claude_running() -> bool { false }

pub fn status() -> ClaudeLocalizationStatus {
    let source = source_app();
    let copy = copy_app();
    let source_version = app_version(&source, "CFBundleShortVersionString");
    let copy_version = app_version(&copy, "CFBundleShortVersionString");
    let current_build = app_version(&source, "CFBundleVersion");
    let prepared = copy.is_dir() && runtime_dir().join("prepared.json").is_file();
    let receipt = fs::read(runtime_dir().join("prepared.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .unwrap_or(Value::Null);
    let shell_locale = read_config(&config_path())
        .ok()
        .and_then(|v| v.get("locale")?.as_str().map(str::to_string));
    // 按名字取词表，不用下标：往 ASSETS 里插一个文件就会让下标整体错位，
    // 上一版就是这样把「官方词条」读成 0 的。
    let catalog_entries = ASSETS
        .iter()
        .find(|(name, _)| *name == "catalogs/zh-Hans.json")
        .and_then(|(_, bytes)| serde_json::from_slice::<Value>(bytes).ok())
        .and_then(|value| value.as_object().map(|obj| obj.len()))
        .unwrap_or(0);
    ClaudeLocalizationStatus {
        supported: cfg!(any(target_os = "macos", target_os = "windows")) && source.exists(),
        source_app: source.to_string_lossy().to_string(),
        copy_app: copy.to_string_lossy().to_string(),
        needs_rebuild: prepared
            && (receipt.get("sourceBuild").and_then(Value::as_str) != Some(current_build.as_str())
                || copy_version != source_version),
        source_version,
        copy_version,
        prepared,
        running: claude_running(),
        node_available: find_runtime("node").is_some(),
        python_available: find_runtime("python3").is_some(),
        shell_locale,
        catalog_entries,
        voice_input: voice_input_enabled(),
        localization: localization_enabled(),
    }
}

/// 语音入口开关。读不到设置时按开启处理，与 settings 里的默认值保持一致。
fn voice_input_enabled() -> bool {
    crate::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.claude_app_voice_input)
        .unwrap_or(true)
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
    Ok(status())
}

/// 写入界面汉化开关。副本是否已准备、是否重建，与这个开关无关。
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
    Ok(status())
}

/// 读汉化前备份里的原语言。没有备份、或备份损坏时返回 None——调用方据此判定
/// 「从来没汉化过」，从而不去碰用户自己的语言设置。
fn saved_shell_locale(backup: &Path) -> Option<String> {
    fs::read(backup)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("locale").and_then(Value::as_str).map(str::to_string))
}

/// 把官方 Claude 的外壳语言还原成汉化前备份里的值。
///
/// 没有备份说明从来没汉化过，这时什么都不做：用户自己的语言设置不该被我们删掉。
fn restore_shell_locale() -> Result<()> {
    let config = config_path();
    let backup = config.with_extension("codex-plus-locale-backup.json");
    match saved_shell_locale(&backup) {
        Some(value) => set_locale_at(&config, &backup, Some(value.as_str()), true),
        None => Ok(()),
    }
}

fn install_assets(root: &Path) -> Result<()> {
    for (relative, bytes) in ASSETS {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().context("缺少运行目录")?)?;
        fs::write(path, bytes)?;
    }
    Ok(())
}

fn run_script(program: &Path, script: &str, copy: &Path, args: &[&str]) -> Result<()> {
    let root = runtime_dir();
    let log = root.join("operation.log");
    let output = fs::File::create(&log)?;
    let mut command = Command::new(program);
    if cfg!(target_os = "windows") && script.ends_with(".ps1") {
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
    }
    let mut child = command
        .arg(root.join(script))
        .args(args)
        .env("CODEX_PLUS_CLAUDE_SOURCE", source_app())
        .env("CODEX_PLUS_CLAUDE_COPY", copy)
        // 启动脚本据此决定要不要给页面注入语音入口；构建/校验脚本读到也无害。
        .env(
            "CODEX_PLUS_CLAUDE_VOICE_INPUT",
            if voice_input_enabled() { "1" } else { "0" },
        )
        // 同理：注入器按这个决定要不要带中文界面那一段。
        .env(
            "CODEX_PLUS_CLAUDE_LOCALIZATION",
            if localization_enabled() { "1" } else { "0" },
        )
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()
        .context("无法启动汉化组件")?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
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
    let value: Value = serde_json::from_slice(&fs::read(path)?)?;
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
    ensure!(cfg!(any(target_os = "macos", target_os = "windows")), "Claude 增强目前支持 macOS 和 Windows");
    ensure!(
        ["prepare", "check", "launch", "restore"].contains(&action),
        "未知汉化操作"
    );
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 增强操作正在进行"))?;
    let source = source_app();
    ensure!(source.is_dir(), "未找到 Claude.app，请先安装官方应用");
    let script_runtime = if cfg!(target_os = "windows") {
        find_runtime("powershell").context("未找到 Windows PowerShell")?
    } else {
        find_runtime("python3").context("未找到 Python 3，请安装后再准备副本")?
    };
    if action != "check" {
        ensure!(
            !claude_running(),
            "Claude 正在运行，请先用 ⌘Q 完全退出，再执行此操作"
        );
    }
    let root = runtime_dir();
    install_assets(&root)?;
    let copy = copy_app();
    let message = match action {
        "prepare" => {
            ensure!(
                find_runtime("node").is_some(),
                "需要带有内置 WebSocket 的 Node.js 22 或更新版本才能中文启动"
            );
            if cfg!(target_os = "windows") && windows_msix_source(&source) {
                fs::write(root.join("prepared.json"), serde_json::to_vec(&json!({
                    "sourceBuild": app_version(&source, "ProductVersion"),
                    "msix": true
                }))?)?;
                "已检测到 Windows MSIX 版 Claude；保留官方包不变，使用启动注入。".to_string()
            } else {
            let staged = staged_copy_path(&format!("Claude-staging-{}", uuid::Uuid::new_v4()));
            let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
            run_script(&script_runtime, script, &staged, &[])?;
            // 校验通过后才替换；旧副本保留，构建失败时不会影响它。
            let old = staged_copy_path(&format!("Claude-backup-{}", uuid::Uuid::new_v4()));
            if copy.exists() {
                if let Some(parent) = old.parent() { fs::create_dir_all(parent)?; }
                fs::rename(&copy, &old)?;
            }
            if let Err(error) = fs::rename(&staged, &copy) {
                if old.exists() {
                    let _ = fs::rename(&old, &copy);
                }
                return Err(error.into());
            }
            fs::write(
                root.join("prepared.json"),
                serde_json::to_vec(&json!({
                    "sourceBuild": app_version(&source, "CFBundleVersion")
                }))?,
            )?;
            "副本已准备并通过签名及 app.asar 一致性校验。".to_string()
            }
        }
        "check" => {
            if cfg!(target_os = "windows") && windows_msix_source(&source) {
                ensure!(source.exists(), "未找到 Claude MSIX 安装入口");
                "Windows MSIX 安装入口已检测到，采用原版包启动注入。".to_string()
            } else {
                let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
                run_script(&script_runtime, script, &copy, &["--check"])?;
                "副本签名、中文词表、注入开关及 app.asar 一致性校验通过。".to_string()
            }
        }
        "launch" | "restore" => {
            let launcher = if cfg!(target_os = "windows") {
                script_runtime.clone()
            } else {
                find_runtime("node").context("未找到带内置 WebSocket 的 Node.js 22 或更新版本")?
            };
            if !(cfg!(target_os = "windows") && windows_msix_source(&source)) {
                let script = if cfg!(target_os = "windows") { "build-copy.ps1" } else { "build-copy.py" };
                run_script(&script_runtime, script, &copy, &["--check"])?;
            }
            let config = config_path();
            let backup = config.with_extension("codex-plus-locale-backup.json");
            let previous = read_config(&config)?.get("locale").cloned();
            let saved_locale = || saved_shell_locale(&backup);
            let localization = localization_enabled();
            let locale = if action == "restore" {
                ensure!(backup.is_file(), "没有 codex++ 保存的语言备份");
                let value = saved_locale();
                set_locale_at(&config, &backup, value.as_deref(), true)?;
                value.unwrap_or_else(|| "en-US".to_string())
            } else if localization {
                set_locale_at(&config, &backup, Some("zh-Hans"), false)?;
                "zh-Hans".to_string()
            } else {
                // 汉化关掉：副本按原语言启动。有备份说明之前汉化过，还原它；
                // 从没汉化过就完全不碰用户自己的语言设置。
                match saved_locale() {
                    Some(value) => {
                        set_locale_at(&config, &backup, Some(value.as_str()), true)?;
                        value
                    }
                    None => previous
                        .as_ref()
                        .and_then(Value::as_str)
                        .unwrap_or("en-US")
                        .to_string(),
                }
            };
            let script = if cfg!(target_os = "windows") { "inject-cn.ps1" } else { "inject-cn.js" };
            if let Err(error) = run_script(&launcher, script, &copy, &["--locale", locale.as_str()]) {
                // 启动失败后只在应用未运行时还原本次外壳语言，避免与 Claude 写盘竞争。
                if !claude_running() {
                    let _ = set_locale_at(
                        &config,
                        &backup,
                        previous.as_ref().and_then(Value::as_str),
                        true,
                    );
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
        status: status(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_entries_come_from_the_chinese_catalog() {
        // 曾经用 ASSETS[7] 取词表：往资产清单里插一个文件就让下标整体错位，
        // 界面上的「官方词条」直接变成 0。这里钉住它按名字取、且确实读到了词条。
        let entries = status().catalog_entries;
        assert!(entries > 1000, "官方中文词表应有上千条，实际 {entries}");
    }

    #[test]
    fn saved_shell_locale_tolerates_missing_or_broken_backup() {
        // 关掉汉化时会按这个值还原官方语言；读不出来就必须什么都不做，
        // 否则会把用户自己的语言设置删掉。
        let dir = tempfile::tempdir().unwrap();
        let backup = dir.path().join("locale-backup.json");
        assert_eq!(saved_shell_locale(&backup), None, "没有备份时不能凭空给一个语言");
        fs::write(&backup, br#"{"locale":"ja-JP","other":1}"#).unwrap();
        assert_eq!(saved_shell_locale(&backup).as_deref(), Some("ja-JP"));
        fs::write(&backup, b"not json").unwrap();
        assert_eq!(saved_shell_locale(&backup), None, "备份损坏时不能乱动用户设置");
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
}
