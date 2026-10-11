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

fn source_app() -> PathBuf {
    let system = PathBuf::from("/Applications/Claude.app");
    if system.is_dir() {
        return system;
    }
    home_dir().join("Applications/Claude.app")
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .unwrap_or_default()
}

fn copy_app() -> PathBuf {
    runtime_dir().join("Claude CN.app")
}

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

fn find_runtime(name: &str) -> Option<PathBuf> {
    let mut candidates = vec![home_dir().join(".local/bin").join(name)];
    candidates.extend(
        ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(|dir| Path::new(dir).join(name)),
    );
    if let Some(paths) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&paths).map(|dir| dir.join(name)));
    }
    candidates.into_iter().find(|path| {
        if !path.is_file() { return false; }
        let mut command = Command::new(path);
        if name == "node" {
            command.args(["-e", "process.exit(typeof WebSocket === 'function' && typeof fetch === 'function' ? 0 : 1)"]);
        } else {
            command.arg("--version");
        }
        command.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|status| status.success())
    })
}

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

fn claude_running() -> bool {
    // 仅取 PID，不读取/输出进程参数或用户档案。
    Command::new("/usr/bin/pgrep")
        .args(["-f", "/Contents/MacOS/Claude( |$)"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

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
    let catalog_entries = serde_json::from_slice::<Value>(ASSETS[5].1)
        .ok()
        .and_then(|value| value.as_object().map(|obj| obj.len()))
        .unwrap_or(0);
    ClaudeLocalizationStatus {
        supported: cfg!(target_os = "macos"),
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
    let mut child = Command::new(program)
        .arg(root.join(script))
        .args(args)
        .env("CODEX_PLUS_CLAUDE_SOURCE", source_app())
        .env("CODEX_PLUS_CLAUDE_COPY", copy)
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
    ensure!(cfg!(target_os = "macos"), "Claude 汉化目前仅支持 macOS");
    ensure!(
        ["prepare", "check", "launch", "restore"].contains(&action),
        "未知汉化操作"
    );
    let _guard = OPERATION
        .try_lock()
        .map_err(|_| anyhow::anyhow!("另一项 Claude 汉化操作正在进行"))?;
    let source = source_app();
    ensure!(source.is_dir(), "未找到 Claude.app，请先安装官方应用");
    let python = find_runtime("python3").context("未找到 Python 3，请安装后再准备副本")?;
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
            let staged = root.join(format!("Claude-staging-{}.app", uuid::Uuid::new_v4()));
            run_script(&python, "build-copy.py", &staged, &[])?;
            // 校验通过后才替换；旧副本保留，构建失败时不会影响它。
            let old = root.join(format!("Claude-backup-{}.app", uuid::Uuid::new_v4()));
            if copy.exists() {
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
        "check" => {
            run_script(&python, "build-copy.py", &copy, &["--check"])?;
            "副本签名、中文词表、注入开关及 app.asar 一致性校验通过。".to_string()
        }
        "launch" | "restore" => {
            let node =
                find_runtime("node").context("未找到带内置 WebSocket 的 Node.js 22 或更新版本")?;
            run_script(&python, "build-copy.py", &copy, &["--check"])?;
            let config = config_path();
            let backup = config.with_extension("codex-plus-locale-backup.json");
            let previous = read_config(&config)?.get("locale").cloned();
            let original = if action == "restore" {
                ensure!(backup.is_file(), "没有 codex++ 保存的语言备份");
                let saved: Value = serde_json::from_slice(&fs::read(&backup)?)?;
                saved
                    .get("locale")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            } else {
                Some("zh-Hans".to_string())
            };
            set_locale_at(&config, &backup, original.as_deref(), action == "restore")?;
            let locale = original.as_deref().unwrap_or("en-US");
            if let Err(error) = run_script(&node, "inject-cn.js", &copy, &["--locale", locale]) {
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
