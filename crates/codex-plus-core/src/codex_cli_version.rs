//! 本机安装的 Codex CLI 版本（独立 CLI 与 Desktop 内置 CLI）。
//!
//! 官方 ChatGPT 后端按请求里的客户端版本（`client_version` 参数、`version` 头、UA）决定返回哪些模型；
//! `models_cache.json` 里的版本是上次刷新缓存时的客户端写的，用户升级 CLI / Desktop 后可能长期停在旧版本，
//! 官方目录就不返回新模型（#3178）。这里探测本机实际安装的版本，供版本选择取较新者。
//!
//! 探测不在请求路径上起进程：Desktop 内置 CLI 读 `codex-package.json`，npm 安装读包的 `package.json`，
//! 只有都读不到时才跑一次 `codex --version`，结果按「路径 + 大小 + 修改时间」缓存；整次扫描结果再缓存几分钟。

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// 整次扫描结果的有效期：Desktop 自动更新后几分钟内跟上即可。
const SCAN_TTL: Duration = Duration::from_secs(300);
/// `codex --version` 最多等这么久。
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// 预发布标识的一段：纯数字按数值比，其余按 ASCII 比，数字段低于字母段（semver 2.0 第 11 条）。
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreId {
    Num(u64),
    Alpha(String),
}

impl Ord for PreId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (PreId::Num(left), PreId::Num(right)) => left.cmp(right),
            (PreId::Num(_), PreId::Alpha(_)) => Ordering::Less,
            (PreId::Alpha(_), PreId::Num(_)) => Ordering::Greater,
            (PreId::Alpha(left), PreId::Alpha(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for PreId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 严格的 `X.Y.Z[-预发布][+构建]` 版本号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexVersion {
    core: (u64, u64, u64),
    pre: Vec<PreId>,
}

impl CodexVersion {
    /// 解析版本号；不接受前后空白、`v` 前缀、缺段或多段。构建元数据校验后忽略。
    pub fn parse(text: &str) -> Option<Self> {
        let (rest, build) = match text.split_once('+') {
            Some((rest, build)) => (rest, Some(build)),
            None => (text, None),
        };
        let identifier_ok = |part: &str| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        };
        if let Some(build) = build {
            if !build.split('.').all(identifier_ok) {
                return None;
            }
        }
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (rest, None),
        };
        let numbers = core
            .split('.')
            .map(|part| {
                (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
                    .then(|| part.parse::<u64>().ok())
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()?;
        let [major, minor, patch]: [u64; 3] = numbers.try_into().ok()?;
        let pre = match pre {
            None => Vec::new(),
            Some(pre) => pre
                .split('.')
                .map(|part| {
                    if !identifier_ok(part) {
                        None
                    } else if part.bytes().all(|byte| byte.is_ascii_digit()) {
                        part.parse::<u64>().ok().map(PreId::Num)
                    } else {
                        Some(PreId::Alpha(part.to_string()))
                    }
                })
                .collect::<Option<Vec<_>>>()?,
        };
        Some(Self {
            core: (major, minor, patch),
            pre,
        })
    }

    /// 只保留 `X.Y.Z`：Codex CLI 请求官方模型目录时的 `client_version` 就是这个形式，预发布版也一样。
    pub fn whole(&self) -> String {
        let (major, minor, patch) = self.core;
        format!("{major}.{minor}.{patch}")
    }
}

impl Ord for CodexVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                (true, true) => Ordering::Equal,
                // 预发布版低于同号正式版：0.169.0-alpha.11.1 < 0.169.0。
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            }
        })
    }
}

impl PartialOrd for CodexVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 按 semver 比较两个版本号；任一方不合法时为 None。
pub fn compare(left: &str, right: &str) -> Option<Ordering> {
    Some(CodexVersion::parse(left)?.cmp(&CodexVersion::parse(right)?))
}

struct ScanMemo {
    version: Option<String>,
    at: Instant,
}

fn scan_memo() -> &'static Mutex<Option<ScanMemo>> {
    static MEMO: OnceLock<Mutex<Option<ScanMemo>>> = OnceLock::new();
    MEMO.get_or_init(Mutex::default)
}

fn store_scan(version: Option<String>) -> Option<String> {
    *scan_memo().lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ScanMemo {
        version: version.clone(),
        at: Instant::now(),
    });
    version
}

/// 本机安装的 Codex CLI 中最新的版本（完整版本号，可能带预发布）。会阻塞：缓存过期时重新扫描，
/// 可能要跑一次 `codex --version`。异步代码里请放进 `spawn_blocking`。
pub fn installed_codex_version() -> Option<String> {
    {
        let memo = scan_memo().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(memo) = memo.as_ref().filter(|memo| memo.at.elapsed() < SCAN_TTL) {
            return memo.version.clone();
        }
    }
    store_scan(scan_installed_versions().into_iter().max().map(|(_, text)| text))
}

/// 不阻塞的版本：直接返回上次扫描结果（可能已过期或还没有）；缺失或过期时在后台线程重新扫描。
/// 用在请求路径上。
pub fn installed_codex_version_cached() -> Option<String> {
    static REFRESHING: AtomicBool = AtomicBool::new(false);
    let (value, fresh) = {
        let memo = scan_memo().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match memo.as_ref() {
            Some(memo) => (memo.version.clone(), memo.at.elapsed() < SCAN_TTL),
            None => (None, false),
        }
    };
    if !fresh && !REFRESHING.swap(true, AtomicOrdering::SeqCst) {
        let spawned = std::thread::Builder::new()
            .name("codex-cli-version".into())
            .spawn(|| {
                store_scan(scan_installed_versions().into_iter().max().map(|(_, text)| text));
                REFRESHING.store(false, AtomicOrdering::SeqCst);
            });
        if spawned.is_err() {
            REFRESHING.store(false, AtomicOrdering::SeqCst);
        }
    }
    value
}

/// 扫描所有候选 CLI，返回能读出版本的那些（版本对象 + 原文）。
fn scan_installed_versions() -> Vec<(CodexVersion, String)> {
    let mut seen = std::collections::HashSet::new();
    candidate_binaries()
        .into_iter()
        .filter(|path| path.is_file())
        .filter(|path| seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone())))
        .filter_map(|path| binary_version(&path))
        .filter_map(|text| CodexVersion::parse(&text).map(|version| (version, text)))
        .collect()
}

/// 候选 CLI：Desktop 内置的（含 Windows 用户目录托管的那份）、独立安装器的、PATH 和常见安装目录里的 `codex`。
fn candidate_binaries() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(windows) {
        candidates.extend(crate::app_paths::find_desktop_managed_codex_cli());
        candidates.extend(crate::app_paths::find_standalone_codex_cli());
    }
    let saved = crate::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.codex_app_path)
        .unwrap_or_default();
    if let Some(app_dir) = crate::app_paths::resolve_codex_app_dir_with_saved(None, Some(&saved)) {
        candidates.extend(desktop_cli_candidates(&app_dir));
    }
    candidates.extend(path_cli_candidates(
        std::env::var_os("PATH").as_deref(),
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .as_deref(),
    ));
    candidates
}

/// Desktop 应用包里的 CLI 位置（与 `remote_control::sidecar::resolve_codex_cli` 一致）。
fn desktop_cli_candidates(app_dir: &Path) -> Vec<PathBuf> {
    if app_dir.extension().is_some_and(|extension| extension == "app") {
        let resources = app_dir.join("Contents").join("Resources");
        vec![
            resources.join("codex-cli/bin/codex"),
            resources.join("codex-cli/CodexCLI.app/Contents/MacOS/codex"),
            resources.join("codex"),
        ]
    } else {
        crate::app_paths::find_bundled_codex_cli(app_dir)
            .into_iter()
            .collect()
    }
}

fn path_cli_candidates(path: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(path).collect())
        .unwrap_or_default();
    if let Some(home) = home {
        for extra in [".local/bin", ".npm-global/bin", ".bun/bin", ".volta/bin"] {
            dirs.push(home.join(extra));
        }
    }
    if !cfg!(windows) {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    let names: &[&str] = if cfg!(windows) {
        // npm 在 Windows 上装的是 codex.cmd 外壳，版本从旁边的 node_modules 读，不执行它。
        &["codex.exe", "codex.cmd"]
    } else {
        &["codex"]
    };
    dirs.into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .collect()
}

/// 某个 CLI 的版本：先读安装包里的版本清单，读不到再跑 `--version`（按文件身份缓存）。
fn binary_version(path: &Path) -> Option<String> {
    package_version(path).or_else(|| {
        let script = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["cmd", "bat", "ps1"].contains(&extension.to_ascii_lowercase().as_str())
            });
        (!script).then(|| spawned_version_cached(path)).flatten()
    })
}

/// 安装包自带的版本清单：Desktop 内置 CLI 的 `codex-package.json`，npm 安装的 `@openai/codex` 包。
fn package_version(path: &Path) -> Option<String> {
    let read_version = |file: PathBuf, require_npm_name: bool| -> Option<String> {
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
        if require_npm_name && value.get("name").and_then(|name| name.as_str()) != Some("@openai/codex") {
            return None;
        }
        value
            .get("version")
            .and_then(|version| version.as_str())
            .filter(|version| CodexVersion::parse(version).is_some())
            .map(str::to_string)
    };
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    for dir in real.ancestors().skip(1).take(5) {
        if let Some(version) = read_version(dir.join("codex-package.json"), false)
            // .app/Contents/Resources/codex 旁边是 codex-cli/ 目录。
            .or_else(|| read_version(dir.join("codex-cli").join("codex-package.json"), false))
            .or_else(|| read_version(dir.join("package.json"), true))
        {
            return Some(version);
        }
    }
    // npm 全局安装的外壳（Windows 的 codex.cmd、未解析的链接）：包在 <前缀>/node_modules 或 <前缀>/../lib/node_modules。
    let dir = path.parent()?;
    [
        dir.join("node_modules"),
        dir.parent()?.join("lib").join("node_modules"),
    ]
    .into_iter()
    .find_map(|modules| {
        read_version(modules.join("@openai").join("codex").join("package.json"), true)
    })
}

type FileIdentity = (u64, Option<SystemTime>);

fn spawned_version_cached(path: &Path) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (FileIdentity, Option<String>)>>> = OnceLock::new();
    let metadata = std::fs::metadata(path).ok()?;
    let identity = (metadata.len(), metadata.modified().ok());
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some((cached_identity, version)) = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(path)
    {
        if *cached_identity == identity {
            return version.clone();
        }
    }
    let version = run_version(path);
    cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(path.to_path_buf(), (identity, version.clone()));
    version
}

fn run_version(path: &Path) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut command = Command::new(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // CLI 是控制台程序：Windows 上不加 CREATE_NO_WINDOW 会闪黑色终端（#2778）。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    let mut child = command.spawn().ok()?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    version_from_output(&output)
}

/// `codex --version` 的输出形如 `codex-cli 0.169.1`：取第一个合法的版本号片段。
fn version_from_output(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .map(|token| token.trim_start_matches(['v', 'V']))
        .find(|token| CodexVersion::parse(token).is_some())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerelease_versions_order_like_semver() {
        let ordered = [
            "0.164.0",
            "0.169.0-alpha.2",
            "0.169.0-alpha.11",
            "0.169.0-alpha.11.1",
            "0.169.0-beta",
            "0.169.0",
            "0.169.1",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert_eq!(compare(pair[0], pair[1]), Some(Ordering::Less), "{pair:?}");
            assert_eq!(compare(pair[1], pair[0]), Some(Ordering::Greater), "{pair:?}");
        }
        assert_eq!(compare("0.169.1+build.7", "0.169.1"), Some(Ordering::Equal));
        for invalid in ["", "0.169", "0.169.1.2", "v0.169.1", " 0.169.1", "0.169.1-", "0.169.1-a..b", "0.169.1+"] {
            assert_eq!(CodexVersion::parse(invalid), None, "{invalid:?}");
        }
        assert_eq!(CodexVersion::parse("0.169.0-alpha.11.1").unwrap().whole(), "0.169.0");
    }

    #[test]
    fn version_is_read_from_cli_output() {
        assert_eq!(version_from_output("codex-cli 0.169.1\n").as_deref(), Some("0.169.1"));
        assert_eq!(
            version_from_output("codex-cli 0.169.0-alpha.11.1").as_deref(),
            Some("0.169.0-alpha.11.1")
        );
        assert_eq!(version_from_output("error: something"), None);
    }

    #[test]
    fn desktop_and_npm_packages_report_their_version_without_running_the_cli() {
        let temp = tempfile::tempdir().unwrap();
        // Desktop 内置 CLI：codex-cli/{codex-package.json, bin/codex, CodexCLI.app/Contents/MacOS/codex}
        let resources = temp.path().join("Codex.app/Contents/Resources");
        let cli = resources.join("codex-cli");
        std::fs::create_dir_all(cli.join("bin")).unwrap();
        std::fs::create_dir_all(cli.join("CodexCLI.app/Contents/MacOS")).unwrap();
        std::fs::write(
            cli.join("codex-package.json"),
            r#"{"layoutVersion":1,"version":"0.169.0-alpha.11.1","entrypoint":"bin/codex"}"#,
        )
        .unwrap();
        for binary in [
            cli.join("bin/codex"),
            cli.join("CodexCLI.app/Contents/MacOS/codex"),
            resources.join("codex"),
        ] {
            std::fs::write(&binary, "#!/bin/sh\nexit 1\n").unwrap();
            assert_eq!(package_version(&binary).as_deref(), Some("0.169.0-alpha.11.1"), "{binary:?}");
        }
        // npm 全局安装：<前缀>/bin/codex 与 <前缀>/lib/node_modules/@openai/codex/package.json
        let prefix = temp.path().join("npm");
        let package = prefix.join("lib/node_modules/@openai/codex");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(package.join("package.json"), r#"{"name":"@openai/codex","version":"0.169.1"}"#).unwrap();
        std::fs::write(prefix.join("bin/codex"), "").unwrap();
        assert_eq!(package_version(&prefix.join("bin/codex")).as_deref(), Some("0.169.1"));
        // 别的包的 package.json 不算。
        let other = temp.path().join("other");
        std::fs::create_dir_all(other.join("bin")).unwrap();
        std::fs::write(other.join("package.json"), r#"{"name":"not-codex","version":"9.9.9"}"#).unwrap();
        std::fs::write(other.join("bin/codex"), "").unwrap();
        assert_eq!(package_version(&other.join("bin/codex")), None);
    }

    #[cfg(unix)]
    #[test]
    fn cli_without_package_is_run_once_per_file_identity() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("codex");
        let counter = temp.path().join("runs");
        std::fs::write(
            &binary,
            format!("#!/bin/sh\necho x >> '{}'\necho 'codex-cli 0.170.0'\n", counter.display()),
        )
        .unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(binary_version(&binary).as_deref(), Some("0.170.0"));
        assert_eq!(binary_version(&binary).as_deref(), Some("0.170.0"));
        assert_eq!(std::fs::read_to_string(&counter).unwrap().lines().count(), 1);
    }

    #[test]
    fn path_candidates_include_common_install_dirs() {
        let temp = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([temp.path().join("bin")]).unwrap();
        let candidates = path_cli_candidates(Some(&path), Some(temp.path()));
        let name = if cfg!(windows) { "codex.exe" } else { "codex" };
        assert!(candidates.contains(&temp.path().join("bin").join(name)));
        assert!(candidates.contains(&temp.path().join(".local/bin").join(name)));
    }
}
