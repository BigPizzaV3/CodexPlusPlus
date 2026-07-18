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
const STANDALONE_CODEX_EXECUTABLES: &[&str] = &["ChatGPT.exe", "Codex.exe", "codex.exe"];

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
    matches.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .reverse()
            .then_with(|| left.1.cmp(&right.1))
    });
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
        find_latest_codex_app_dir_from_roots(&windows_app_package_roots())
            .or_else(find_latest_codex_app_dir_from_appx_package)
    }

    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn find_latest_codex_app_dir_from_appx_package() -> Option<PathBuf> {
    registered_windows_packages()
        .ok()?
        .into_iter()
        .filter(|package| is_supported_windows_app_package_name(&package.full_name))
        .filter_map(|package| normalize_codex_app_path(&package.install_location))
        .max_by(compare_app_dir_candidates)
}

#[cfg(windows)]
pub(crate) fn registered_windows_packages() -> anyhow::Result<Vec<RegisteredWindowsPackage>> {
    use std::sync::OnceLock;

    static PACKAGES: OnceLock<Result<Vec<RegisteredWindowsPackage>, String>> = OnceLock::new();
    PACKAGES
        .get_or_init(|| query_registered_windows_packages().map_err(|error| error.to_string()))
        .clone()
        .map_err(anyhow::Error::msg)
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
    None
}

pub fn find_macos_codex_app_default() -> Option<PathBuf> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()) {
        roots.push(home.join("Applications"));
    }
    find_macos_codex_app(&roots)
}

pub fn resolve_codex_app_dir(app_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(app_dir) = app_dir {
        return normalize_codex_app_path(app_dir);
    }
    if cfg!(target_os = "macos") {
        return find_macos_codex_app_default();
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
        // 已保存路径无效（例如误选 Codex++）时回退自动探测
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

    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if is_supported_app_executable_name(file_name) {
        return path.parent().map(Path::to_path_buf);
    }

    if path.extension() == Some(OsStr::new("app")) {
        return Some(path.to_path_buf());
    }

    if path.is_file() {
        // 任意普通文件不再视为应用根；仅当父目录已是合法 Codex 目录时取父路径
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
fn is_codex_plus_plus_path(path: &Path) -> bool {
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower == "codex++"
            || lower == "codexplusplus"
            || lower == "codex-plus-plus"
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
    app_dir.join("Codex.exe")
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
}

pub fn packaged_app_user_model_id(app_dir: &Path) -> Option<String> {
    let package_name = package_name_from_app_dir(app_dir)?;
    let (spec, _, publisher_id) = codex_package_parts(&package_name)?;
    if publisher_id.is_empty() {
        return None;
    }
    Some(format!("{}_{publisher_id}!{}", spec.identity, spec.app_id))
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
    [
        "Codex.app",
        "OpenAI Codex.app",
        "OpenAI.Codex.app",
        "ChatGPT.app",
    ]
    .into_iter()
    .map(|name| root.join(name))
    .collect()
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
    name.eq_ignore_ascii_case("Codex.exe") || name.eq_ignore_ascii_case("ChatGPT.exe")
}

fn package_spec_from_path(path: &Path) -> Option<AppPackageSpec> {
    let package_name = package_name_from_app_dir(path)?;
    let (spec, _, _) = codex_package_parts(&package_name)?;
    Some(spec)
}

fn compare_app_dir_candidates(left: &PathBuf, right: &PathBuf) -> std::cmp::Ordering {
    app_dir_sort_key(left).cmp(&app_dir_sort_key(right))
}

fn app_dir_sort_key(app_dir: &Path) -> Option<(std::cmp::Reverse<u8>, Vec<u32>)> {
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
    Some((
        std::cmp::Reverse(spec.priority),
        version_tuple(package_dir)?,
    ))
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
    let names = package_spec_from_path(dir)
        .map(|spec| spec.executable_names)
        .unwrap_or(STANDALONE_CODEX_EXECUTABLES);
    for name in names {
        let candidate = dir.join(name);
        if candidate.exists() {
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
        let Some((_, publisher_id)) = rest.rsplit_once("__") else {
            continue;
        };
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
