use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use anyhow::{Context, bail};

#[derive(Debug, Clone, Copy)]
struct AppPackageSpec {
    identity: &'static str,
    app_id: &'static str,
    executable_names: &'static [&'static str],
    priority: u8,
}

const CODEX_PACKAGE_EXECUTABLES: &[&str] = &["ChatGPT.exe", "Codex.exe", "codex.exe"];
#[cfg(not(target_os = "linux"))]
const STANDALONE_CODEX_EXECUTABLES: &[&str] = &["ChatGPT.exe", "Codex.exe", "codex.exe"];

/// Linux 可执行文件名（原生优先，兼容便携包）
#[cfg(target_os = "linux")]
const LINUX_CODEX_EXECUTABLES: &[&str] = &[
    "ChatGPT",
    "chatgpt",
    "Codex",
    "codex",
    "ChatGPT.exe",
    "Codex.exe",
    "codex.exe",
];


#[cfg(windows)]
const OPENAI_PACKAGE_FAMILY_NAMES: &[&str] = &[
    "OpenAI.Codex_2p2nqsd0c76g0",
    "OpenAI.CodexBeta_2p2nqsd0c76g0",
    "OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0",
];

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RegisteredWindowsPackage {
    pub full_name: String,
    pub install_location: PathBuf,
}

const APP_PACKAGE_SPECS: &[AppPackageSpec] = &[
    AppPackageSpec {
        identity: "OpenAI.Codex",
        app_id: "App",
        executable_names: CODEX_PACKAGE_EXECUTABLES,
        priority: 1,
    },
    AppPackageSpec {
        identity: "OpenAI.CodexBeta",
        app_id: "App",
        executable_names: CODEX_PACKAGE_EXECUTABLES,
        priority: 1,
    },
    AppPackageSpec {
        identity: "OpenAI.ChatGPT-Desktop",
        app_id: "App",
        executable_names: CODEX_PACKAGE_EXECUTABLES,
        // Codex 已迁移为新的 ChatGPT Desktop；同机并存时优先新宿主。
        priority: 2,
    },
];

pub fn find_latest_codex_app_dir(root: &Path) -> Option<PathBuf> {
    let mut matches = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let spec = package_spec_from_path(&path)?;
            let version = version_tuple(&path)?;
            let app_dir = package_entry_dir(&path, spec)?;
            Some((spec.priority, version, app_dir))
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let (_, _, latest) = matches.pop()?;
    Some(latest)
}

pub fn find_latest_codex_app_dir_from_roots(roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter_map(|root| find_latest_codex_app_dir(root))
        .max_by(compare_app_dir_candidates)
}

pub fn find_latest_codex_app_dir_default() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        // 注册信息是 Store 当前状态的权威来源；查询失败时再退回目录扫描。
        if let Ok(Some(appx)) = find_latest_codex_app_dir_from_appx_package() {
            return Some(appx);
        }
        windows_app_package_roots()
            .iter()
            .filter_map(|root| find_latest_codex_app_dir(root))
            .max_by(compare_app_dir_candidates)
    }

    #[cfg(target_os = "macos")]
    {
        find_macos_codex_app_default()
    }

    #[cfg(target_os = "linux")]
    {
        find_linux_codex_app_default()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }


}
#[cfg(windows)]
fn find_latest_codex_app_dir_from_appx_package() -> anyhow::Result<Option<PathBuf>> {
    Ok(registered_windows_packages()?
        .into_iter()
        .filter(|package| is_supported_windows_app_package_name(&package.full_name))
        .filter_map(|package| normalize_codex_app_path(&package.install_location))
        .max_by(compare_app_dir_candidates))
}

#[cfg(windows)]
pub(crate) fn registered_windows_packages() -> anyhow::Result<Vec<RegisteredWindowsPackage>> {
    // AppX 包在 Codex 更新后会改变 full name 和安装目录，不能跨启动周期缓存。
    query_registered_windows_packages()
}

#[cfg(windows)]
fn query_registered_windows_packages() -> anyhow::Result<Vec<RegisteredWindowsPackage>> {
    let mut packages = Vec::new();
    for family_name in OPENAI_PACKAGE_FAMILY_NAMES {
        for full_name in package_full_names_for_family(family_name)? {
            let install_location = package_path_by_full_name(&full_name)
                .with_context(|| format!("failed to resolve registered package {full_name}"))?;
            packages.push(RegisteredWindowsPackage {
                full_name,
                install_location,
            });
        }
    }
    Ok(packages)
}

#[cfg(windows)]
fn package_full_names_for_family(family_name: &str) -> anyhow::Result<Vec<String>> {
    use windows::Win32::Foundation::{
        APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
    };
    use windows::Win32::Storage::Packaging::Appx::GetPackagesByPackageFamily;
    use windows::core::{PCWSTR, PWSTR};

    let family = family_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut count = 0u32;
    let mut buffer_length = 0u32;
    let first = unsafe {
        GetPackagesByPackageFamily(
            PCWSTR(family.as_ptr()),
            &mut count,
            None,
            &mut buffer_length,
            PWSTR(std::ptr::null_mut()),
        )
    };
    if first == APPMODEL_ERROR_NO_PACKAGE || (first == ERROR_SUCCESS && count == 0) {
        return Ok(Vec::new());
    }
    if first != ERROR_INSUFFICIENT_BUFFER {
        bail!("GetPackagesByPackageFamily failed with {}", first.0);
    }

    let mut pointers = vec![PWSTR(std::ptr::null_mut()); count as usize];
    let mut buffer = vec![0u16; buffer_length as usize];
    let status = unsafe {
        GetPackagesByPackageFamily(
            PCWSTR(family.as_ptr()),
            &mut count,
            Some(pointers.as_mut_ptr()),
            &mut buffer_length,
            PWSTR(buffer.as_mut_ptr()),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("GetPackagesByPackageFamily failed with {}", status.0);
    }
    buffer.truncate(buffer_length as usize);
    buffer
        .split(|value| *value == 0)
        .filter(|value| !value.is_empty())
        .map(|value| String::from_utf16(value).context("invalid package full name"))
        .collect()
}

#[cfg(windows)]
fn package_path_by_full_name(full_name: &str) -> anyhow::Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
    use windows::Win32::Storage::Packaging::Appx::GetPackagePathByFullName;
    use windows::core::{PCWSTR, PWSTR};

    let full_name = full_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut path_length = 0u32;
    let first = unsafe {
        GetPackagePathByFullName(
            PCWSTR(full_name.as_ptr()),
            &mut path_length,
            PWSTR(std::ptr::null_mut()),
        )
    };
    if first != ERROR_INSUFFICIENT_BUFFER {
        bail!("GetPackagePathByFullName failed with {}", first.0);
    }
    let mut path = vec![0u16; path_length as usize];
    let status = unsafe {
        GetPackagePathByFullName(
            PCWSTR(full_name.as_ptr()),
            &mut path_length,
            PWSTR(path.as_mut_ptr()),
        )
    };
    if status != ERROR_SUCCESS {
        bail!("GetPackagePathByFullName failed with {}", status.0);
    }
    let end = path
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(path.len());
    Ok(PathBuf::from(OsString::from_wide(&path[..end])))
}

#[cfg(windows)]
fn windows_app_package_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(program_files).join("WindowsApps"));
    }
    if let Some(program_files) = std::env::var_os("ProgramW6432") {
        roots.push(PathBuf::from(program_files).join("WindowsApps"));
    }
    roots.push(PathBuf::from(r"C:\Program Files\WindowsApps"));
    roots.sort();
    roots.dedup();
    roots
}

pub fn user_data_candidates() -> Vec<PathBuf> {
    user_data_candidates_from(
        std::env::var_os("LOCALAPPDATA").as_deref().map(Path::new),
        std::env::var_os("APPDATA").as_deref().map(Path::new),
    )
}

pub fn user_data_candidates_from(local: Option<&Path>, roaming: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(local) = local {
        append_user_data_variants(&mut candidates, local);
    }
    if let Some(roaming) = roaming {
        append_user_data_variants(&mut candidates, roaming);
    }
    candidates
}

pub fn find_macos_codex_app(search_roots: &[PathBuf]) -> Option<PathBuf> {
    for root in search_roots {
        for candidate in macos_app_candidates(root) {
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    // 固定名字都没命中时按 bundle id 兜底：应用被改名（`Codex 2.app`）或
    // 装在子目录（`/Applications/OpenAI/Codex.app`）时名字判据失效，但
    // Info.plist 里的 CFBundleIdentifier 仍然是权威身份（issue #2204）。
    #[cfg(not(windows))]
    for root in search_roots {
        if let Some(app) = find_macos_codex_app_by_bundle_id(root, 0) {
            return Some(app);
        }
    }
    None
}

/// 递归深度上限：一层子目录足够覆盖 `/Applications/OpenAI/Codex.app` 这类
/// 分组安装，又不会在大目录树上耗时。
#[cfg(not(windows))]
const MACOS_BUNDLE_SEARCH_MAX_DEPTH: usize = 1;

#[cfg(not(windows))]
fn find_macos_codex_app_by_bundle_id(root: &Path, depth: usize) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut directories: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // 目录顺序由文件系统给出，不保证稳定；排序让同一棵树上的结果可复现。
    directories.sort();
    for path in &directories {
        if path.extension() == Some(OsStr::new("app")) && is_codex_bundle_identifier(path) {
            return Some(path.clone());
        }
    }
    if depth >= MACOS_BUNDLE_SEARCH_MAX_DEPTH {
        return None;
    }
    for path in &directories {
        if path.extension() == Some(OsStr::new("app")) {
            continue;
        }
        if let Some(app) = find_macos_codex_app_by_bundle_id(path, depth + 1) {
            return Some(app);
        }
    }
    None
}

/// 官方桌面应用的 bundle id。Codex 已并入 ChatGPT Desktop，两个都认
/// （与 APP_PACKAGE_SPECS 里的 identity 列表保持一致）。
#[cfg(not(windows))]
const MACOS_CODEX_BUNDLE_IDENTIFIERS: &[&str] =
    &["com.openai.codex", "com.openai.chat", "com.openai.chatgpt"];

#[cfg(not(windows))]
fn is_codex_bundle_identifier(app_dir: &Path) -> bool {
    let Some(bundle_id) = macos_app_plist_value(app_dir, "CFBundleIdentifier") else {
        return false;
    };
    MACOS_CODEX_BUNDLE_IDENTIFIERS
        .iter()
        .any(|known| bundle_id.trim().eq_ignore_ascii_case(known))
}

pub fn find_macos_codex_app_default() -> Option<PathBuf> {
    find_macos_codex_app(&macos_default_search_roots())
}

/// 默认探测根，也用于失败文案里「都找过哪些地方」的自述。
pub fn macos_default_search_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        roots.push(home.join("Applications"));
    }
    roots
}


pub fn find_linux_codex_app(search_roots: &[PathBuf]) -> Option<PathBuf> {
    for root in search_roots {
        for candidate in linux_app_candidates(root) {
            if let Some(app_dir) = normalize_codex_app_path(&candidate) {
                return Some(app_dir);
            }
        }
    }
    None
}

fn linux_app_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // 直接匹配应用目录（仅在目录实际存在时推入）
    for name in &["ChatGPT", "chatgpt", "Codex", "codex"] {
        let app_dir = root.join(name);
        if app_dir.is_dir() {
            candidates.push(app_dir.clone());
            let app_sub = app_dir.join("app");
            if app_sub.is_dir() {
                candidates.push(app_sub);
            }
        }
    }
    // 扫描根目录下的一级子目录，查找包含可执行文件的 app 目录
    // 例如：/usr/lib/chatgpt/ChatGPT（可执行文件直接在子目录中）
    #[cfg(target_os = "linux")]
    {
        if root.is_dir() {
            if let Ok(entries) = std::fs::read_dir(root) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let name = path.file_name().and_then(OsStr::to_str);
                    if let Some(name) = name {
                        let lower = name.to_ascii_lowercase();
                        if lower == "chatgpt" || lower == "codex" || lower == "codex-beta" {
                            candidates.push(path.clone());
                            // 也检查该子目录的 app 子目录（AppDir 结构）
                            let app_sub = path.join("app");
                            if app_sub.is_dir() {
                                candidates.push(app_sub);
                            }
                        }
                    }
                }
            }
        }
    }
    candidates
}

pub fn find_linux_codex_app_default() -> Option<PathBuf> {
    let home = directories::BaseDirs::new();
    let mut roots = Vec::new();
    // 系统级（官方 deb 默认安装在 /usr/lib/chatgpt）
    roots.push(PathBuf::from("/usr/lib"));
    roots.push(PathBuf::from("/opt"));
    // 用户级
    if let Some(h) = home {
        roots.push(h.home_dir().join("Applications"));
        roots.push(h.home_dir().join(".local").join("share"));
    }
    find_linux_codex_app(&roots)
}

pub fn resolve_codex_app_dir(app_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(app_dir) = app_dir {
        return normalize_codex_app_path(app_dir);
    }
    if cfg!(target_os = "macos") {
        return find_macos_codex_app_default();
    }
    #[cfg(target_os = "linux")]
    {
        return find_linux_codex_app_default();
    }
    // Windows: try MS Store version first, then standalone install
    find_latest_codex_app_dir_default().or_else(|| find_standalone_codex_app_dir())
}

/// Search for standalone Codex installations (non-MS Store).
///
/// Common paths:
/// - %LOCALAPPDATA%\OpenAI\Codex\bin\  (standalone installer)
/// - %LOCALAPPDATA%\OpenAI\Codex\      (user data root)
/// - %LOCALAPPDATA%\Programs\OpenAI\Codex\ (alternative)
pub fn find_standalone_codex_app_dir() -> Option<PathBuf> {
    let local_appdata = std::env::var_os("LOCALAPPDATA")?;

    let candidates: &[PathBuf] = &[
        PathBuf::from(&local_appdata)
            .join("OpenAI")
            .join("Codex")
            .join("bin"),
        PathBuf::from(&local_appdata).join("OpenAI").join("Codex"),
        PathBuf::from(&local_appdata)
            .join("Programs")
            .join("OpenAI")
            .join("Codex"),
    ];

    for candidate in candidates {
        if let Some(path) = normalize_codex_app_path(candidate) {
            if build_codex_executable(&path).exists() {
                return Some(path);
            }
        }
    }
    None
}

/// Finds the CLI shipped by the standalone Codex installer.
pub fn find_standalone_codex_cli() -> Option<PathBuf> {
    let local_appdata = std::env::var_os("LOCALAPPDATA")?;
    find_standalone_codex_cli_in(&PathBuf::from(local_appdata).join("OpenAI").join("Codex").join("bin"))
}

fn find_standalone_codex_cli_in(bin_dir: &Path) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = standalone_cli_in_dir(bin_dir) { candidates.push(path); }
    if let Ok(entries) = std::fs::read_dir(bin_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(path) = standalone_cli_in_dir(&entry.path()) { candidates.push(path); }
            }
        }
    }
    candidates.sort_by_key(|path| std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok());
    candidates.pop()
}

fn standalone_cli_in_dir(dir: &Path) -> Option<PathBuf> {
    ["codex.exe", "codex", "Codex.exe", "Codex"].iter().map(|name| dir.join(name)).find(|path| path.is_file())
}

/// `normalize_codex_app_path` 拒绝一个已存在路径时，说明具体卡在哪一条判据。
///
/// issue #2204：失败时四个平台四种原因共用一句「Codex App directory not found」，
/// 用户与维护者都无法判断是「路径不存在」「误选了 Codex++」还是「目录里没有可执行
/// 文件」。这里只做归因，不改变接受/拒绝结果。
pub fn describe_codex_app_dir_rejection(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return "路径为空".to_string();
    }
    if is_codex_plus_plus_path(path) {
        return "该路径是 Codex++ 自己的安装目录，不是 Codex 桌面应用".to_string();
    }
    if !path.exists() {
        return "路径不存在".to_string();
    }
    if path.is_file() {
        return "这是一个文件；请选择 Codex 应用目录或目录内的可执行文件".to_string();
    }
    if path.join("app").is_dir() {
        return "目录中没有找到 Codex 可执行文件（Codex.exe / ChatGPT.exe），且目录名也不像 MSIX 包".to_string();
    }
    "目录中没有找到 Codex 可执行文件（Codex.exe / ChatGPT.exe）".to_string()
}

/// 自动探测全部落空时的失败文案，按「显式传入 / 已保存 / 纯自动」三种来源分别给出
/// 可操作提示，并列出实际搜索过的根目录（issue #2204）。
pub fn describe_codex_app_dir_not_found(
    explicit_app_dir: Option<&Path>,
    saved_app_path: Option<&str>,
) -> String {
    if let Some(app_dir) = explicit_app_dir {
        return format!(
            "Codex App directory not found: 传入的 --app-path {} 不可用（{}），\
             且显式路径无效时不会回退自动探测。",
            app_dir.display(),
            describe_codex_app_dir_rejection(app_dir)
        );
    }
    let saved = saved_app_path
        .map(str::trim)
        .filter(|saved| !saved.is_empty());
    if let Some(saved) = saved {
        let path = Path::new(saved);
        return format!(
            "Codex App directory not found: 已保存的路径 {} 不可用（{}），\
             自动探测也未找到 Codex 桌面应用（{}）。\
             请在管理工具的「Codex 应用路径」里重新选择。",
            path.display(),
            describe_codex_app_dir_rejection(path),
            describe_default_search_roots()
        );
    }
    format!(
        "Codex App directory not found: 没有配置应用路径，自动探测也未找到 Codex 桌面应用\
         （已搜索：{}）。请在管理工具里手动选择 Codex 应用目录。",
        describe_default_search_roots()
    )
}

/// 各平台默认探测面，用于失败文案自述「找过哪里」。
fn describe_default_search_roots() -> String {
    #[cfg(target_os = "macos")]
    {
        return macos_default_search_roots()
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
    }
    #[cfg(target_os = "linux")]
    {
        return ["/usr/lib", "/opt", "~/Applications", "~/.local/share"].join(", ");
    }
    #[cfg(windows)]
    {
        let mut roots: Vec<String> = windows_app_package_roots()
            .iter()
            .map(|root| root.display().to_string())
            .collect();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            roots.push(
                PathBuf::from(local)
                    .join("OpenAI")
                    .join("Codex")
                    .display()
                    .to_string(),
            );
        }
        return roots.join(", ");
    }
    #[allow(unreachable_code)]
    String::new()
}

#[cfg(test)]
mod standalone_cli_tests {
    use super::find_standalone_codex_cli_in;

    #[test]
    fn standalone_cli_finds_latest_versioned_binary() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old");
        let new = temp.path().join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("codex.exe"), "old").unwrap();
        std::fs::write(new.join("codex.exe"), "new").unwrap();
        std::fs::File::options().write(true).open(old.join("codex.exe")).unwrap().set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000))).unwrap();
        assert_eq!(find_standalone_codex_cli_in(temp.path()), Some(new.join("codex.exe")));
    }

    #[test]
    fn standalone_cli_returns_none_without_binary() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(find_standalone_codex_cli_in(temp.path()), None);
    }

    #[test]
    fn standalone_cli_finds_unversioned_binary() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("codex.exe");
        std::fs::write(&binary, "cli").unwrap();
        assert_eq!(find_standalone_codex_cli_in(temp.path()), Some(binary));
    }
}

pub fn resolve_codex_app_dir_with_saved(
    app_dir: Option<&Path>,
    saved_app_path: Option<&str>,
) -> Option<PathBuf> {
    if let Some(app_dir) = app_dir {
        // 显式 --app-path 仅接受有效 Codex 应用；无效时不回退，避免静默启动错误目录
        return normalize_codex_app_path(app_dir);
    }
    if let Some(saved) = saved_app_path
        .map(str::trim)
        .filter(|saved| !saved.is_empty())
    {
        // 用户显式保存的路径优先（issue #2394）。此前 Store 包目录会被注册表查询
        // 结果无条件覆盖，表现为「保存了但重启后没用」——用户特意选中的目录又被
        // 换回自动探测到的那个。
        //
        // 这里仍然只在保存路径**有效**时优先：normalize_codex_app_path 对已不存在的
        // 目录返回 None（Store 更新会删掉旧版本目录），此时自然落到下面的自动探测，
        // 由注册表给出当前版本，不会把用户卡在失效路径上。
        if let Some(path) = normalize_codex_app_path(Path::new(saved)) {
            return Some(path);
        }
    }
    resolve_codex_app_dir(None)
}

pub fn normalize_codex_app_path(path: &Path) -> Option<PathBuf> {
    if path.as_os_str().is_empty() {
        return None;
    }

    // 拒绝把 Codex++ 管理工具安装目录误当成 Codex 桌面应用
    if is_codex_plus_plus_path(path) {
        return None;
    }

    if !path.exists() {
        return None;
    }

    if path.extension() == Some(OsStr::new("app")) {
        return Some(path.to_path_buf());
    }

    if path.is_file() {
        // 任意普通文件或可执行文件自身不再视为应用根；仅当父目录已是合法 Codex 目录时取父路径
        let parent = path.parent()?;
        return normalize_codex_app_path(parent);
    }

    if executable_in_dir(path).is_some() {
        return Some(path.to_path_buf());
    }

    let nested_app = path.join("app");
    if nested_app.is_dir() {
        if executable_in_dir(&nested_app).is_some() {
            return Some(nested_app);
        }
        // WindowsApps 常因 ACL 无法枚举 exe；只要包名像 OpenAI.Codex_* 仍接受 app\
        if is_codex_store_package_dir(path) {
            return Some(nested_app);
        }
    }

    // 接受 Store 包目录本身（含 …\OpenAI.Codex_*\app）
    if path.is_dir() && is_codex_store_package_dir(path) {
        return Some(path.to_path_buf());
    }

    None
}

/// Codex++ 管理控制台/安装根，绝不能当作 OpenAI Codex 桌面应用。
///
/// 目录名只是线索，不能单独作为判据：用户完全可能把真正的 Codex 放在
/// `C:\codex++` 这类目录下（issue #1036），此时按名字一刀切会连真 Codex 一起拒掉，
/// 表现为「能找到 codex_app 但版本为 null、启动报 failed to launch」。
/// 因此只有目录里确实带着本产品自身的文件（管理工具可执行文件 / 卸载器）时才排除。
fn is_codex_plus_plus_path(path: &Path) -> bool {
    if !codex_plus_plus_name_path(path) {
        return false;
    }
    has_codex_plus_plus_marker(path)
}

/// 路径中是否存在「像 Codex++ 安装目录」的目录名。
fn codex_plus_plus_name_path(path: &Path) -> bool {
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        // macOS 下安装根是 `Codex++.app`，去掉 bundle 扩展名再比对，
        // 否则 `codex++` 这一档永远命不中。
        let lower = name
            .strip_suffix(".app")
            .unwrap_or(name)
            .to_ascii_lowercase();
        if lower == "codex++"
            || lower == "codexplusplus"
            || lower == "codex-plus-plus"
            || lower == "codex++ manager"
            || lower.contains("codex-plus-manager")
        {
            return true;
        }
    }
    let normalized = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    normalized.contains("\\programs\\codex++")
        || normalized.contains("\\codex++\\")
        || normalized.ends_with("\\codex++")
}

/// 目录里是否存在本产品自己铺设的文件。用于确认「这个名字像 Codex++ 的目录」
/// 确实是 Codex++ 的安装位置，而不是用户恰好这样命名的普通 Codex 目录。
fn has_codex_plus_plus_marker(path: &Path) -> bool {
    // 沿祖先链向上看：调用方可能传入安装根，也可能传入根里的可执行文件，还可能
    // 传入 .app 包内的 Contents/MacOS/<exe>。安装器把 MANAGER_BINARY /
    // uninstall.exe 放在安装根，macOS 则把 bundle 可执行文件放在 Contents/MacOS。
    let mut roots: Vec<&Path> = Vec::new();
    let mut cursor = Some(path);
    while let Some(current) = cursor {
        roots.push(current);
        cursor = current.parent();
    }
    // macOS 的 .app 包把可执行文件放在 Contents/MacOS 下，名字取自 bundle 的
    // CFBundleExecutable（CodexPlusPlus / CodexPlusPlusManager），与安装根里的
    // 二进制名不同，因此两条都列。
    const MARKERS: &[&str] = &[
        "codex-plus-plus-manager.exe",
        "uninstall.exe",
        "codex-plus-plus-manager",
        "codex-plus-plus",
        "CodexPlusPlusManager",
        "CodexPlusPlus",
    ];
    for root in roots {
        for marker in MARKERS {
            if root.join(marker).is_file() {
                return true;
            }
        }
        let macos_dir = root.join("Contents").join("MacOS");
        if macos_dir.is_dir()
            && MARKERS
                .iter()
                .any(|marker| macos_dir.join(marker).is_file())
        {
            return true;
        }
    }
    false
}

fn is_codex_store_package_dir(path: &Path) -> bool {
    package_spec_from_path(path).is_some()
}

pub fn build_codex_executable(app_dir: &Path) -> PathBuf {
    if app_dir.extension() == Some(OsStr::new("app")) {
        let macos_dir = app_dir.join("Contents").join("MacOS");
        if let Some(executable) = macos_app_plist_value(app_dir, "CFBundleExecutable")
            .filter(|value| !value.contains('/') && !value.contains('\\'))
        {
            return macos_dir.join(executable);
        }
        return macos_dir.join("Codex");
    }
    if let Some(executable) = executable_in_dir(app_dir) {
        return executable;
    }
    if let Some(spec) = package_spec_from_path(app_dir) {
        return app_dir.join(spec.executable_names[0]);
    }
    #[cfg(target_os = "linux")]
    {
        app_dir.join("ChatGPT")
    }
    #[cfg(not(target_os = "linux"))]
    {
        app_dir.join("Codex.exe")
    }
}

pub fn find_bundled_codex_cli(app_dir: &Path) -> Option<PathBuf> {
    let candidates = if app_dir.extension() == Some(OsStr::new("app")) {
        vec![app_dir.join("Contents").join("Resources").join("codex")]
    } else {
        vec![
            app_dir.join("resources").join("codex.exe"),
            app_dir.join("Resources").join("codex.exe"),
            app_dir.join("resources").join("codex"),
            app_dir.join("Resources").join("codex"),
        ]
    };
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// 是否指向 Store 版安装目录（WindowsApps）里的可执行文件。
///
/// MSIX 包目录受系统保护，第三方进程不能直接执行其中的 exe，
/// 改文件夹权限也不会生效（#2028：微信连接填「桌面版内置 CLI」必然 os error 5）。
/// macOS 的 .app/Contents/Resources/codex 是普通可执行文件，不受此限制。
pub fn is_windows_store_cli_path(executable: &str) -> bool {
    executable
        .replace('/', "\\")
        .to_ascii_lowercase()
        .contains("\\windowsapps\\")
}

/// 桌面版在用户目录维护的 Codex CLI——Windows 上的标准路径。
///
/// Store 版桌面应用会把可独立执行的 CLI 放在
/// `%LOCALAPPDATA%\OpenAI\Codex\bin\<哈希>\codex.exe` 并随桌面版一起更新；
/// 该目录在系统保护目录之外，第三方进程可以直接运行。目录名是内容哈希，
/// 每次更新都会变，所以不能缓存、只按「含 codex.exe 的最新子目录」解析；
/// 兼容旧的平铺布局 `bin\codex.exe`（#2028/#1879 的根治路径）。
pub fn find_desktop_managed_codex_cli() -> Option<PathBuf> {
    find_desktop_managed_codex_cli_from_var(
        std::env::var_os("LOCALAPPDATA").as_deref().map(Path::new),
    )
}

fn find_desktop_managed_codex_cli_from_var(local_appdata: Option<&Path>) -> Option<PathBuf> {
    let bin = local_appdata?.join("OpenAI").join("Codex").join("bin");
    let mut candidates = vec![bin.join("codex.exe")];
    if let Ok(entries) = std::fs::read_dir(&bin) {
        for entry in entries.filter_map(Result::ok) {
            candidates.push(entry.path().join("codex.exe"));
        }
    }
    candidates
        .into_iter()
        .filter(|exe| exe.is_file())
        .filter_map(|exe| {
            let modified = std::fs::metadata(&exe)
                .and_then(|metadata| metadata.modified())
                .ok()?;
            Some((modified, exe))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, exe)| exe)
}

pub fn codex_app_version(app_dir: &Path) -> Option<String> {
    if app_dir.extension() == Some(OsStr::new("app")) {
        return macos_app_version(app_dir);
    }
    let package_dir = if app_dir
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("app"))
    {
        app_dir.parent()?
    } else {
        app_dir
    };
    codex_package_version(package_dir)
        .or_else(|| codex_directory_version(package_dir))
        .or_else(|| codex_directory_version(app_dir))
        .or_else(|| codex_version_file(package_dir))
        .or_else(|| codex_version_file(app_dir))
        // 免安装/自解包布局（如 …\Codex\app）既没有 MSIX 包目录名，也没有 version
        // 文件，只在目录里留一个以版本号命名的清单，如 `149.0.7827.115.manifest`。
        // 少了这一档，版本解析为空 → 健康检查显示「未检测到」、插件解锁策略退化成
        // unknown（issue #1160）。
        .or_else(|| codex_manifest_version(package_dir))
        .or_else(|| codex_manifest_version(app_dir))
}

/// 从解包目录里形如 `149.0.7827.115.manifest` 的清单文件名取版本号。
///
/// 按 is_version_like 校验文件名主干，避免把无关的 `*.manifest` 误当成版本；
/// 同时命中多个时取版本最高的一个。
fn codex_manifest_version(app_dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(app_dir).ok()?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let stem = name.strip_suffix(".manifest")?;
            is_version_like(stem).then(|| stem.to_string())
        })
        .max_by(|left, right| compare_version_strings(left, right))
}

/// 按点分数字段逐个比较版本号；字段数不同时先比公共前缀，
/// 完全相同再按字段数多者为大（`149.0` < `149.0.0`）。
///
/// 逐字段按数值比较，不能按字符串比较：`"115"` 在字典序上小于 `"20"`，
/// 会把 `…7827.115` 判成低于 `…7827.20`。
fn compare_version_strings(left: &str, right: &str) -> std::cmp::Ordering {
    let parts = |version: &str| {
        version
            .split('.')
            .map(|part| part.parse::<u64>().ok())
            .collect::<Vec<_>>()
    };
    let (left, right) = (parts(left), parts(right));
    for (left_part, right_part) in left.iter().zip(right.iter()) {
        let ordering = left_part.cmp(right_part);
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

pub fn packaged_app_user_model_id(app_dir: &Path) -> Option<String> {
    packaged_app_user_model_id_when_registered(app_dir, app_dir_is_registered_package)
}

/// `packaged_app_user_model_id` 的核心，注册判定以闭包注入，便于测试。
///
/// 目录名像 MSIX 包（`OpenAI.Codex_<版本>_x64__<publisher>`）不等于该包已注册：
/// 用户把 Store 包目录整体复制/搬移到 `D:\aitool\...` 后，路径形状依旧成立，于是
/// 反推出的 AUMID 指向一个未注册的包，`ActivateApplication` 报 0x80073CF1
/// （issue #2140）。未注册时返回 None，调用方回落到按路径启动。
fn packaged_app_user_model_id_when_registered(
    app_dir: &Path,
    mut is_registered: impl FnMut(&Path) -> bool,
) -> Option<String> {
    let package_name = package_name_from_app_dir(app_dir)?;
    let (spec, _, publisher_id) = codex_package_parts(&package_name)?;
    if publisher_id.is_empty() {
        return None;
    }
    // 注册判定针对包目录本身（`…\OpenAI.Codex_<版本>_x64__<publisher>`），
    // 而不是它下面的 `app` 子目录——MSIX 注册记录的是安装位置。
    let package_dir = packaged_package_dir(app_dir)?;
    if !is_registered(package_dir) {
        return None;
    }
    // 新版 ChatGPT-Desktop 可能改变包内 Application Id（参见 #2148 的 0x80270254 报错），
    // 这里优先读取真实 manifest，读取失败再回退到历史硬编码值。
    //
    // 注意：这段逻辑曾被 #2202 的 799ef0c9（一个纯 Linux 修复）基于旧基线静默回退掉，
    // 导致 #2308/#2310 的「该进程没有程序包标识符」。恢复时连同下面的测试一起。
    let app_id = packaged_manifest_app_id(app_dir).unwrap_or_else(|| spec.app_id.to_string());
    Some(format!("{}_{publisher_id}!{app_id}", spec.identity))
}

/// 包目录（`…/OpenAI.Codex_<版本>_x64__<publisher>`，可含 `app` 尾段）。
fn packaged_package_dir(app_dir: &Path) -> Option<&Path> {
    if app_dir
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("app"))
    {
        app_dir.parent()
    } else {
        Some(app_dir)
    }
}

/// 非 Windows 平台没有 MSIX 注册概念，包身份判定交给平台自身的启动方式。
#[cfg(not(windows))]
fn app_dir_is_registered_package(_package_dir: &Path) -> bool {
    true
}

/// 该包目录是否确实是当前用户下已注册的 Store 包安装位置。
///
/// 判据取 `GetPackagesByPackageFamily` 返回的 install location（系统权威来源），
/// 而不是路径末段的名字：搬移过的同名目录必须被判为未注册。
/// 查询本身失败时保守返回 true——离线/受限环境下不能因为探测失败就
/// 让正常的 Store 安装丢掉 AUMID 激活（#2308/#2310）。
#[cfg(windows)]
fn app_dir_is_registered_package(package_dir: &Path) -> bool {
    let Ok(packages) = registered_windows_packages() else {
        return true;
    };
    packages
        .iter()
        .any(|package| same_windows_path(&package.install_location, package_dir))
}

#[cfg(windows)]
fn same_windows_path(left: &Path, right: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        // 先用 canonicalize 归一化 `..`、短名与符号链接；失败（WindowsApps 的 ACL
        // 常让第三方进程读不到）时退回字符串比较，去掉分隔符差异与尾部分隔符。
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    }
    normalize(left) == normalize(right)
}

fn packaged_manifest_app_id(app_dir: &Path) -> Option<String> {
    let package_dir = packaged_package_dir(app_dir)?;
    let manifest = std::fs::read_to_string(package_dir.join("AppxManifest.xml")).ok()?;
    manifest_first_application_id(&manifest)
}

// AppxManifest.xml 中第一个 <Application> 节点的 Id，即 AUMID 感叹号后的部分。
fn manifest_first_application_id(manifest: &str) -> Option<String> {
    let mut rest = manifest;
    while let Some(pos) = rest.find("<Application") {
        rest = &rest[pos + "<Application".len()..];
        // 跳过 <Applications> 等容器节点，只处理 <Application ...>。
        if !rest.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        let tag_end = rest.find('>')?;
        if let Some(id) = xml_attribute_value(&rest[..tag_end], "Id") {
            return Some(id);
        }
        rest = &rest[tag_end..];
    }
    None
}

fn xml_attribute_value(tag: &str, name: &str) -> Option<String> {
    for segment in tag.split_whitespace() {
        let Some((attr, value)) = segment.split_once('=') else {
            continue;
        };
        if attr != name {
            continue;
        }
        return Some(value.trim_matches('"').trim_matches('\'').to_string());
    }
    None
}

fn package_name_from_app_dir(app_dir: &Path) -> Option<String> {
    let path = app_dir.to_string_lossy().replace('\\', "/");
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let mut package_name = parts.next_back()?;
    if package_name.eq_ignore_ascii_case("app") {
        package_name = parts.next_back()?;
    }
    Some(package_name.to_string())
}

fn codex_package_version(package_dir: &Path) -> Option<String> {
    let path = package_dir.to_string_lossy().replace('\\', "/");
    let name = path
        .split('/')
        .rev()
        .find(|part| codex_package_parts(part).is_some())?;
    let (_, version, _) = codex_package_parts(name)?;
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

fn codex_directory_version(app_dir: &Path) -> Option<String> {
    directory_version(app_dir).or_else(|| {
        app_dir
            .canonicalize()
            .ok()
            .and_then(|path| directory_version(&path))
    })
}

fn directory_version(path: &Path) -> Option<String> {
    let version = path.file_name()?.to_str()?;
    if is_version_like(version) {
        Some(version.to_string())
    } else {
        None
    }
}

fn is_version_like(version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(first) = parts.next() else {
        return false;
    };
    if first.is_empty() || !first.chars().all(|ch| ch.is_ascii_digit()) {
        return false;
    }
    let mut count = 1;
    for part in parts {
        if part.is_empty() || !part.chars().all(|ch| ch.is_ascii_digit()) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn codex_version_file(app_dir: &Path) -> Option<String> {
    let version = std::fs::read_to_string(app_dir.join("version")).ok()?;
    let version = version.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

fn macos_app_version(app_dir: &Path) -> Option<String> {
    macos_app_plist_value(app_dir, "CFBundleShortVersionString")
        .or_else(|| macos_app_plist_value(app_dir, "CFBundleVersion"))
}

fn macos_app_plist_value(app_dir: &Path, key: &str) -> Option<String> {
    let plist = std::fs::read_to_string(app_dir.join("Contents").join("Info.plist")).ok()?;
    plist_string_value(&plist, key)
}

fn plist_string_value(plist: &str, key: &str) -> Option<String> {
    let (_, after_key) = plist.split_once(&format!("<key>{key}</key>"))?;
    let (_, after_string_open) = after_key.split_once("<string>")?;
    let (value, _) = after_string_open.split_once("</string>")?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn append_user_data_variants(candidates: &mut Vec<PathBuf>, base: &Path) {
    candidates.push(base.join("OpenAI").join("ChatGPT"));
    candidates.push(base.join("OpenAI.ChatGPT-Desktop"));
    candidates.push(base.join("ChatGPT"));
    candidates.push(base.join("OpenAI").join("Codex"));
    candidates.push(base.join("OpenAI.Codex"));
    candidates.push(base.join("Codex"));
}

fn macos_app_candidates(root: &Path) -> Vec<PathBuf> {
    if root.extension() == Some(OsStr::new("app")) {
        return vec![root.to_path_buf()];
    }
    const NAMES: &[&str] = &[
        "Codex.app",
        "OpenAI Codex.app",
        "OpenAI.Codex.app",
        "ChatGPT.app",
    ];
    let mut candidates: Vec<PathBuf> = NAMES.iter().map(|name| root.join(name)).collect();
    // 一级子目录：用户常把应用归到 `/Applications/OpenAI/Codex.app` 这类分组目录下，
    // 只拼四个固定文件名会漏掉（issue #2204）。
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut subdirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        subdirs.sort();
        for subdir in subdirs {
            for name in NAMES {
                candidates.push(subdir.join(name));
            }
        }
    }
    candidates
}

fn version_tuple(path: &Path) -> Option<Vec<u32>> {
    let name = path.file_name()?.to_str()?;
    let (_, version, _) = codex_package_parts(name)?;
    let parts = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parts.is_empty() { None } else { Some(parts) }
}

pub(crate) fn is_supported_windows_app_package_name(package_name: &str) -> bool {
    codex_package_parts(package_name).is_some()
}

pub(crate) fn is_supported_app_executable_name(name: &str) -> bool {
    // Windows
    if name.eq_ignore_ascii_case("Codex.exe") || name.eq_ignore_ascii_case("ChatGPT.exe") {
        return true;
    }
    // Linux（无扩展名）
    #[cfg(target_os = "linux")]
    {
        if name.eq_ignore_ascii_case("Codex") || name.eq_ignore_ascii_case("ChatGPT") {
            return true;
        }
    }
    false
}

fn package_spec_from_path(path: &Path) -> Option<AppPackageSpec> {
    let package_name = package_name_from_app_dir(path)?;
    let (spec, _, _) = codex_package_parts(&package_name)?;
    Some(spec)
}

fn compare_app_dir_candidates(left: &PathBuf, right: &PathBuf) -> std::cmp::Ordering {
    app_dir_sort_key(left).cmp(&app_dir_sort_key(right))
}

fn app_dir_sort_key(app_dir: &Path) -> Option<(u8, Vec<u32>)> {
    let spec = package_spec_from_path(app_dir)?;
    let package_dir = if app_dir
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case("app"))
    {
        app_dir.parent().unwrap_or(app_dir)
    } else {
        app_dir
    };
    Some((spec.priority, version_tuple(package_dir)?))
}

fn package_entry_dir(package_dir: &Path, spec: AppPackageSpec) -> Option<PathBuf> {
    let app = package_dir.join("app");
    if app.is_dir() {
        return Some(app);
    }
    for name in spec.executable_names {
        if package_dir.join(name).is_file() {
            return Some(package_dir.to_path_buf());
        }
    }
    None
}

fn executable_in_dir(dir: &Path) -> Option<PathBuf> {
    let names: &[&str] = if let Some(spec) = package_spec_from_path(dir) {
        spec.executable_names
    } else {
        #[cfg(target_os = "linux")]
        { LINUX_CODEX_EXECUTABLES }
        #[cfg(not(target_os = "linux"))]
        { STANDALONE_CODEX_EXECUTABLES }
    };
    for name in names {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn codex_package_parts(package_name: &str) -> Option<(AppPackageSpec, &str, &str)> {
    for spec in APP_PACKAGE_SPECS {
        let Some(rest) = strip_prefix_ignore_ascii_case(package_name, spec.identity) else {
            continue;
        };
        let Some(rest) = rest.strip_prefix('_') else {
            continue;
        };
        let Some((version, rest)) = rest.split_once('_') else {
            continue;
        };
        // MSIX full name 的 resource id 可能为 `~`，例如
        // `Name_1.2.3.0_neutral_~_publisher`; 也兼容无 resource id 时的 `x64__publisher`。
        let Some((_, publisher_id)) = rest.rsplit_once('_') else {
            continue;
        };
        if publisher_id.is_empty() {
            continue;
        }
        return Some((*spec, version, publisher_id));
    }
    None
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }
    let (head, rest) = value.split_at(prefix.len());
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

#[cfg(test)]
mod registered_package_tests {
    use super::packaged_app_user_model_id_when_registered;
    use std::path::{Path, PathBuf};

    fn store_package_app_dir(root: &Path) -> PathBuf {
        root.join("OpenAI.Codex_26.901.6511.0_x64__2p2nqsd0c76g0")
            .join("app")
    }

    /// issue #2140：目录名像 MSIX 包但系统未注册时必须返回 None，
    /// 否则 launcher 会拿这个 AUMID 去激活一个不存在的包（0x80073CF1）。
    #[test]
    fn unregistered_package_dir_yields_no_aumid() {
        let temp = tempfile::tempdir().unwrap();
        let app = store_package_app_dir(&temp.path().join("aitool"));
        std::fs::create_dir_all(&app).unwrap();

        assert_eq!(
            packaged_app_user_model_id_when_registered(&app, |_| false),
            None
        );
    }

    /// 已注册时仍按 manifest / 硬编码给出 AUMID，守住 #2308/#2310 的修复。
    #[test]
    fn registered_package_dir_keeps_manifest_application_id() {
        let temp = tempfile::tempdir().unwrap();
        let app = store_package_app_dir(temp.path());
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.parent().unwrap().join("AppxManifest.xml"),
            r#"<Package><Applications><Application Id="AppX" Executable="ChatGPT.exe"/></Applications></Package>"#,
        )
        .unwrap();

        assert_eq!(
            packaged_app_user_model_id_when_registered(&app, |_| true).as_deref(),
            Some("OpenAI.Codex_2p2nqsd0c76g0!AppX")
        );
    }

    /// 注册判定收到的是包目录而不是 `app` 子目录——比较的是安装位置本身。
    #[test]
    fn registration_probe_receives_package_directory() {
        let temp = tempfile::tempdir().unwrap();
        let app = store_package_app_dir(temp.path());
        std::fs::create_dir_all(&app).unwrap();

        let mut probed = None;
        let _ = packaged_app_user_model_id_when_registered(&app, |dir| {
            probed = Some(dir.to_path_buf());
            true
        });
        assert_eq!(probed.as_deref(), app.parent());
    }
}

#[cfg(test)]
mod saved_path_tests {
    use super::resolve_codex_app_dir_with_saved;
    use std::path::PathBuf;

    fn store_package_app_dir(root: &std::path::Path) -> PathBuf {
        root.join("OpenAI.Codex_26.901.6511.0_x64__2p2nqsd0c76g0")
            .join("app")
    }

    /// issue #2394：保存的 Store 包目录仍然存在时必须原样返回，
    /// 不能被注册表查询结果覆盖（否则「保存了但重启后没用」）。
    #[test]
    fn saved_store_package_dir_wins_over_registry_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let app = store_package_app_dir(temp.path());
        std::fs::create_dir_all(&app).unwrap();

        assert_eq!(
            resolve_codex_app_dir_with_saved(None, Some(&app.to_string_lossy())).as_deref(),
            Some(app.as_path())
        );
    }

    /// 保存路径已被删除（Store 更新删掉旧版本目录）时不再返回它，
    /// 由自动探测接手——这条保证「优先」不会把用户卡在失效路径上。
    #[test]
    fn removed_saved_store_package_dir_falls_back_to_detection() {
        let temp = tempfile::tempdir().unwrap();
        let missing = store_package_app_dir(temp.path());

        assert_ne!(
            resolve_codex_app_dir_with_saved(None, Some(&missing.to_string_lossy())).as_deref(),
            Some(missing.as_path())
        );
    }
}

#[cfg(test)]
mod cli_path_tests {
    use super::{find_desktop_managed_codex_cli_from_var, is_windows_store_cli_path};
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    fn touch(path: &Path, ago_secs: u64) {
        let file = std::fs::File::options().append(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(ago_secs))
            .unwrap();
    }

    /// #2028：桌面版把 CLI 维护在 bin\<哈希>\ 下，哈希目录随更新变化，
    /// 解析必须挑含 codex.exe 的最新子目录，跳过只放辅助工具的目录。
    #[test]
    fn picks_newest_hash_dir_containing_codex_exe() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("OpenAI").join("Codex").join("bin");
        let old = bin.join("aaa111");
        let new = bin.join("bbb222");
        let rg_only = bin.join("ccc333");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::create_dir_all(&rg_only).unwrap();
        std::fs::write(old.join("codex.exe"), "old").unwrap();
        std::fs::write(new.join("codex.exe"), "new").unwrap();
        std::fs::write(rg_only.join("rg.exe"), "rg").unwrap();
        touch(&old.join("codex.exe"), 10_000);
        touch(&new.join("codex.exe"), 60);
        touch(&rg_only.join("rg.exe"), 1);

        let found = find_desktop_managed_codex_cli_from_var(Some(temp.path()));
        assert_eq!(found.as_deref(), Some(new.join("codex.exe").as_path()));
    }

    #[test]
    fn flat_bin_layout_is_still_accepted() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("OpenAI").join("Codex").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("codex.exe"), "flat").unwrap();

        let found = find_desktop_managed_codex_cli_from_var(Some(temp.path()));
        assert_eq!(found.as_deref(), Some(bin.join("codex.exe").as_path()));
    }

    #[test]
    fn missing_bin_directory_returns_none() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            find_desktop_managed_codex_cli_from_var(Some(temp.path())),
            None
        );
        assert_eq!(find_desktop_managed_codex_cli_from_var(None), None);
    }

    #[test]
    fn detects_windows_store_paths_across_separators_and_cases() {
        assert!(is_windows_store_cli_path(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0_x64__p\app\resources\codex.exe"
        ));
        assert!(is_windows_store_cli_path(
            "c:/program files/windowsapps/someapp/app/codex.exe"
        ));
        // 桌面版维护的 bin 目录 / macOS 内置路径 / 裸命令名都不能误伤
        assert!(!is_windows_store_cli_path(
            r"C:\Users\a\AppData\Local\OpenAI\Codex\bin\abc123\codex.exe"
        ));
        assert!(!is_windows_store_cli_path(
            "/Applications/ChatGPT.app/Contents/Resources/codex"
        ));
        assert!(!is_windows_store_cli_path("codex"));
    }
}

#[cfg(all(test, not(windows)))]
mod macos_discovery_tests {
    use super::{
        describe_codex_app_dir_not_found, describe_codex_app_dir_rejection, find_macos_codex_app,
    };
    use std::path::Path;

    fn write_bundle(app: &Path, bundle_id: &str) {
        let contents = app.join("Contents");
        std::fs::create_dir_all(&contents).unwrap();
        std::fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>{bundle_id}</string>
</dict>
</plist>
"#
            ),
        )
        .unwrap();
    }

    /// issue #2204：应用装在根目录下的一层子目录里（如
    /// `/Applications/OpenAI/Codex.app`）时也要能找到。
    #[test]
    fn finds_app_one_level_below_search_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Applications");
        let app = root.join("OpenAI").join("Codex.app");
        write_bundle(&app, "com.openai.codex");

        assert_eq!(
            find_macos_codex_app(&[root]).as_deref(),
            Some(app.as_path())
        );
    }

    /// bundle 被改名（`Codex 2.app`）时按 CFBundleIdentifier 兜底。
    #[test]
    fn finds_renamed_bundle_by_identifier() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Applications");
        let app = root.join("OpenAI").join("Codex 2.app");
        write_bundle(&app, "com.openai.codex");

        assert_eq!(
            find_macos_codex_app(&[root]).as_deref(),
            Some(app.as_path())
        );
    }

    /// 无关 bundle 不能被误认——bundle id 判据必须真的比对。
    #[test]
    fn ignores_unrelated_bundles() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Applications");
        write_bundle(
            &root.join("OpenAI").join("Some Tool.app"),
            "com.example.tool",
        );

        assert_eq!(find_macos_codex_app(&[root]), None);
    }

    /// 固定名字仍然优先于 bundle id 兜底，避免改动探测顺序影响既有选择。
    #[test]
    fn fixed_names_still_win_over_bundle_id_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Applications");
        let named = root.join("Codex.app");
        std::fs::create_dir_all(&named).unwrap();
        // 同根下还有一个 bundle id 匹配但名字陌生的候选。
        write_bundle(&root.join("OpenAI").join("Mystery.app"), "com.openai.codex");

        assert_eq!(
            find_macos_codex_app(&[root]).as_deref(),
            Some(named.as_path())
        );
    }

    /// 四种失败原因要能分辨，且都保留既有英文前缀便于日志检索（issue #2204）。
    #[test]
    fn rejection_reasons_are_distinguishable() {
        let temp = tempfile::tempdir().unwrap();

        let missing = temp.path().join("nope");
        assert_eq!(describe_codex_app_dir_rejection(&missing), "路径不存在");

        let empty_dir = temp.path().join("empty");
        std::fs::create_dir_all(&empty_dir).unwrap();
        assert!(describe_codex_app_dir_rejection(&empty_dir).contains("可执行文件"));

        let manager = temp.path().join("Programs").join("Codex++");
        std::fs::create_dir_all(&manager).unwrap();
        std::fs::write(manager.join("codex-plus-plus-manager.exe"), "").unwrap();
        assert!(describe_codex_app_dir_rejection(&manager).contains("Codex++"));
    }

    #[test]
    fn not_found_message_names_the_source_of_the_path() {
        let explicit = Path::new("/nope/Codex.app");
        let message = describe_codex_app_dir_not_found(Some(explicit), None);
        assert!(message.starts_with("Codex App directory not found"));
        assert!(message.contains("--app-path"));

        let saved = describe_codex_app_dir_not_found(None, Some("/nope/Codex.app"));
        assert!(saved.contains("已保存的路径"));

        let none = describe_codex_app_dir_not_found(None, None);
        assert!(none.contains("没有配置应用路径"));
        // 纯自动探测的文案要自述搜索面，用户才知道该把应用放到哪里。
        assert!(none.contains("已搜索"));
    }
}
