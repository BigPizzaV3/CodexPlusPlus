//! AI 应用缓存白名单。清理只接受后端扫描快照，逐文件删除，不递归删除目录。

use std::collections::BTreeSet;
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, ensure};
use serde::Serialize;

const MIN_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ENTRIES: usize = 100_000;
const CACHE_NAMES: &[&str] = &[
    "Cache",
    "Code Cache",
    "GPUCache",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "GraphiteDawnCache",
    "ShaderCache",
    "GrShaderCache",
];

#[derive(Debug, Clone)]
struct CacheRoot {
    app: String,
    kind: &'static str,
    path: PathBuf,
    cleanable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheGroup {
    pub id: String,
    pub app: String,
    pub kind: String,
    pub path: String,
    pub total_bytes: u64,
    pub eligible_bytes: u64,
    pub eligible_files: usize,
    pub skipped_files: usize,
    pub cleanable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheReport {
    pub scan_id: String,
    pub groups: Vec<CacheGroup>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
struct CacheFile {
    group: String,
    path: PathBuf,
    size: u64,
    modified: SystemTime,
    identity: FileIdentity,
}

#[derive(Debug, PartialEq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    created: Option<SystemTime>,
}

impl FileIdentity {
    fn of(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            created: metadata.created().ok(),
        }
    }
}

pub struct CachePlan {
    pub report: CacheReport,
    home: PathBuf,
    roots: Vec<CacheRoot>,
    files: Vec<CacheFile>,
    created: SystemTime,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanResult {
    pub removed_files: usize,
    // 按扫描的文件长度累计，不把逻辑大小宣传成实际磁盘释放量。
    pub removed_bytes: u64,
    pub skipped_files: usize,
    pub failures: Vec<String>,
}

pub fn scan_default() -> Result<CachePlan> {
    let home = directories::BaseDirs::new()
        .context("无法识别用户目录")?
        .home_dir()
        .canonicalize()?;
    let codex = crate::codex_home::default_codex_home_dir();
    scan(&home, discover_roots(&home, &codex)?, SystemTime::now())
}

fn discover_roots(home: &Path, codex: &Path) -> Result<Vec<CacheRoot>> {
    let mut roots = Vec::new();
    let mut add = |app: &str, kind, path: PathBuf, cleanable| {
        roots.push(CacheRoot {
            app: app.into(),
            kind,
            path,
            cleanable,
        });
    };
    // CODEX_HOME 在用户目录外或通过符号链接配置时，只忽略这部分。
    add("Codex", "cache", codex.join("cache"), true);
    add("Codex", "temporary", codex.join(".tmp"), false);
    add("Claude", "cache", home.join(".claude/cache"), true);

    #[cfg(target_os = "macos")]
    {
        for (app, name) in [
            ("Codex", "Codex"),
            ("Codex", "com.openai.codex"),
            ("Codex++", "com.bigpizzav3.codexplusplus.current-build"),
            ("Claude", "com.anthropic.claudefordesktop"),
            ("Claude", "local.claudecodecn.claude"),
            ("Claude", "claude-cli-nodejs"),
        ] {
            add(app, "cache", home.join("Library/Caches").join(name), true);
        }
        for name in [
            "com.anthropic.claudefordesktop.ShipIt",
            "local.claudecodecn.claude.ShipIt",
        ] {
            add(
                "Claude",
                "update",
                home.join("Library/Caches").join(name),
                true,
            );
        }
        for (app, name) in [
            ("Codex", "com.openai.codex"),
            ("Claude", "Claude"),
            ("Claude", "Claude-3p"),
        ] {
            add(app, "logs", home.join("Library/Logs").join(name), true);
        }
    }

    let mut app_data = Vec::new();
    #[cfg(target_os = "macos")]
    for (app, name) in [
        ("Codex", "Codex"),
        ("Claude", "Claude"),
        ("Claude", "Claude-3p"),
    ] {
        app_data.push((app, home.join("Library/Application Support").join(name)));
    }
    #[cfg(target_os = "windows")]
    if let Some(base) = std::env::var_os("APPDATA") {
        for app in ["Codex", "Claude"] {
            app_data.push((app, PathBuf::from(&base).join(app)));
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    for app in ["Codex", "Claude"] {
        app_data.push((app, home.join(".config").join(app)));
    }
    for (app, base) in app_data {
        add_electron_roots(&mut roots, app, &base, home, 0)?;
    }
    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);
    // 自定义 CODEX_HOME 可能落在另一白名单目录内，避免重复统计或重复清理。
    let mut distinct: Vec<CacheRoot> = Vec::new();
    for root in roots {
        if !distinct
            .iter()
            .any(|parent| root.path.starts_with(&parent.path))
        {
            distinct.push(root);
        }
    }
    Ok(distinct)
}

fn protected_path(home: &Path, path: &Path) -> bool {
    path.strip_prefix(home).is_ok_and(|relative| {
        relative.components().any(|part| {
            let name = part.as_os_str().to_string_lossy().to_lowercase();
            if name == ".env" || name.starts_with(".env.") {
                return true;
            }
            matches!(
                name.as_str(),
                "plugins"
                    | "skills"
                    | "sessions"
                    | "archived_sessions"
                    | "worktrees"
                    | "vm_bundles"
                    | ".tmp"
                    | "auth.json"
                    | "config.toml"
                    | "settings.json"
            )
        })
    })
}

fn add_electron_roots(
    roots: &mut Vec<CacheRoot>,
    app: &str,
    base: &Path,
    home: &Path,
    depth: usize,
) -> Result<()> {
    if depth > 3 || !safe_path(home, base) {
        return Ok(());
    }
    for name in CACHE_NAMES
        .iter()
        .copied()
        .chain(["Shared Dictionary/cache"])
    {
        roots.push(CacheRoot {
            app: app.into(),
            kind: "cache",
            path: base.join(name),
            cleanable: true,
        });
    }
    // 仅进入已知浏览器 profile/partition 层级，不搜索整个应用数据目录。
    for name in ["Default", "codex-browser-app", "Partitions"] {
        let path = base.join(name);
        if name != "Partitions" {
            add_electron_roots(roots, app, &path, home, depth + 1)?;
        } else if safe_path(home, &path) {
            if let Ok(entries) = fs::read_dir(&path) {
                for entry in entries.take(256).flatten() {
                    if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                        add_electron_roots(roots, app, &entry.path(), home, depth + 1)?;
                    }
                }
            }
        }
    }
    Ok(())
}

// 所有祖先都检查，避免缓存路径或中间目录被链接到会话、凭据或仓库。
fn safe_path(home: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(home) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return false;
    }
    let mut current = home.to_path_buf();
    for part in relative.components() {
        let std::path::Component::Normal(part) = part else {
            return false;
        };
        current.push(part);
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            return false;
        };
        if is_link(&metadata) {
            return false;
        }
    }
    true
}

fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn eligible(metadata: &Metadata, now: SystemTime) -> bool {
    if !metadata.is_file() || is_link(metadata) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return false;
        }
    }
    metadata
        .modified()
        .ok()
        .and_then(|time| now.duration_since(time).ok())
        .is_some_and(|age| age >= MIN_AGE)
}

fn scan(home: &Path, roots: Vec<CacheRoot>, now: SystemTime) -> Result<CachePlan> {
    let scan_id = uuid::Uuid::new_v4().to_string();
    let mut plan = CachePlan {
        report: CacheReport {
            scan_id,
            groups: Vec::new(),
            warnings: Vec::new(),
        },
        home: home.into(),
        roots: Vec::new(),
        files: Vec::new(),
        created: now,
    };
    let mut visited = 0;
    for root in roots {
        if !root.path.exists() {
            continue;
        }
        if !safe_path(home, &root.path) {
            plan.report
                .warnings
                .push(format!("已跳过不安全的目录：{}", root.path.display()));
            continue;
        }
        if root.cleanable && protected_path(home, &root.path) {
            plan.report
                .warnings
                .push(format!("已跳过受保护的目录：{}", root.path.display()));
            continue;
        }
        let id = plan.roots.len().to_string();
        let mut group = CacheGroup {
            id,
            app: root.app.clone(),
            kind: root.kind.into(),
            path: root.path.display().to_string(),
            total_bytes: 0,
            eligible_bytes: 0,
            eligible_files: 0,
            skipped_files: 0,
            cleanable: root.cleanable,
        };
        walk(
            home,
            &root.path,
            &mut group,
            &mut plan.files,
            &mut plan.report.warnings,
            now,
            &mut visited,
            0,
        );
        plan.report.groups.push(group);
        plan.roots.push(root);
        if visited >= MAX_ENTRIES {
            plan.report
                .warnings
                .push("已达到扫描上限，统计结果不完整。".into());
            break;
        }
    }
    Ok(plan)
}

fn walk(
    home: &Path,
    path: &Path,
    group: &mut CacheGroup,
    files: &mut Vec<CacheFile>,
    warnings: &mut Vec<String>,
    now: SystemTime,
    visited: &mut usize,
    depth: usize,
) {
    if group.cleanable && protected_path(home, path) {
        group.skipped_files += 1;
        return;
    }
    if *visited >= MAX_ENTRIES || depth > 32 {
        if depth > 32 {
            warnings.push(format!("目录层级过深，已跳过：{}", path.display()));
        }
        return;
    }
    *visited += 1;
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            warnings.push(format!("读取失败：{}：{error}", path.display()));
            return;
        }
    };
    if is_link(&metadata) {
        group.skipped_files += 1;
        return;
    }
    if metadata.is_dir() {
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    match entry {
                        Ok(entry) => walk(
                            home,
                            &entry.path(),
                            group,
                            files,
                            warnings,
                            now,
                            visited,
                            depth + 1,
                        ),
                        Err(error) => {
                            warnings.push(format!("目录读取失败：{}：{error}", path.display()))
                        }
                    }
                    if *visited >= MAX_ENTRIES {
                        break;
                    }
                }
            }
            Err(error) => warnings.push(format!("目录读取失败：{}：{error}", path.display())),
        }
    } else if metadata.is_file() {
        group.total_bytes = group.total_bytes.saturating_add(metadata.len());
        if group.cleanable && eligible(&metadata, now) {
            group.eligible_bytes = group.eligible_bytes.saturating_add(metadata.len());
            group.eligible_files += 1;
            files.push(CacheFile {
                group: group.id.clone(),
                path: path.into(),
                size: metadata.len(),
                modified: metadata.modified().unwrap(),
                identity: FileIdentity::of(&metadata),
            });
        } else {
            group.skipped_files += 1;
        }
    }
}

impl CachePlan {
    pub fn validate_selection(
        &self,
        scan_id: &str,
        selected: &[String],
        now: SystemTime,
    ) -> Result<BTreeSet<String>> {
        ensure!(self.report.scan_id == scan_id, "扫描已失效，请重新扫描。");
        ensure!(
            now.duration_since(self.created)
                .is_ok_and(|age| age < PLAN_TTL),
            "扫描已过期，请重新扫描。"
        );
        ensure!(!selected.is_empty(), "请选择缓存项目。");
        let ids: BTreeSet<_> = selected.iter().cloned().collect();
        ensure!(ids.len() == selected.len(), "选择包含重复项目。");
        for id in &ids {
            ensure!(
                self.report
                    .groups
                    .iter()
                    .any(|group| &group.id == id && group.cleanable && group.eligible_files > 0),
                "选择包含不可清理项目。"
            );
        }
        Ok(ids)
    }

    /// 一次性消费快照。UI 必须确认后调用，传来的值只有扫描 ID 和组 ID。
    pub fn clean(
        self,
        scan_id: &str,
        selected: &[String],
        confirmed: bool,
    ) -> Result<CacheCleanResult> {
        self.clean_with_processes(
            scan_id,
            selected,
            confirmed,
            SystemTime::now(),
            running_apps()?,
        )
    }

    fn clean_with_processes(
        self,
        scan_id: &str,
        selected: &[String],
        confirmed: bool,
        now: SystemTime,
        running: BTreeSet<String>,
    ) -> Result<CacheCleanResult> {
        ensure!(confirmed, "尚未确认清理。");
        let ids = self.validate_selection(scan_id, selected, now)?;
        for group in self
            .report
            .groups
            .iter()
            .filter(|group| ids.contains(&group.id))
        {
            let app = if group.app == "Codex++" {
                "Codex"
            } else {
                &group.app
            };
            ensure!(
                !running.contains(app),
                "请先退出 {app} 应用及其 CLI，再重新扫描清理。"
            );
        }
        let mut result = CacheCleanResult::default();
        for file in self.files.iter().filter(|file| ids.contains(&file.group)) {
            // 不删除目录，也不碰扫描后创建、变化或被替换的文件。
            let unchanged = safe_path(&self.home, &file.path)
                && fs::symlink_metadata(&file.path).is_ok_and(|metadata| {
                    eligible(&metadata, now)
                        && metadata.len() == file.size
                        && metadata.modified().ok() == Some(file.modified)
                        && FileIdentity::of(&metadata) == file.identity
                });
            if !unchanged {
                result.skipped_files += 1;
                continue;
            }
            match fs::remove_file(&file.path) {
                Ok(()) => {
                    result.removed_files += 1;
                    result.removed_bytes = result.removed_bytes.saturating_add(file.size);
                }
                Err(error) => result
                    .failures
                    .push(format!("{}：{error}", file.path.display())),
            }
        }
        Ok(result)
    }
}

fn running_apps() -> Result<BTreeSet<String>> {
    #[cfg(unix)]
    let names = {
        let output = std::process::Command::new("ps")
            .args(["-axo", "comm="])
            .output()
            .context("无法检查应用进程，已取消清理")?;
        ensure!(output.status.success(), "无法检查应用进程，已取消清理");
        String::from_utf8(output.stdout).context("无法读取进程名称，已取消清理")?
    };
    #[cfg(windows)]
    let names = {
        let processes = crate::windows_enumerate_processes();
        ensure!(!processes.is_empty(), "无法检查应用进程，已取消清理");
        processes
            .iter()
            .map(|process| process.exe_file.clone())
            .collect::<Vec<_>>()
            .join("\n")
    };
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("此平台暂不支持安全清理");
    #[cfg(any(unix, windows))]
    Ok(apps_from_process_names(&names))
}

fn apps_from_process_names(names: &str) -> BTreeSet<String> {
    let mut running = BTreeSet::new();
    for name in names.lines() {
        let name = name.trim().to_lowercase().replace('\\', "/");
        let executable = name
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".exe");
        // 管理工具自身需要保持运行；Codex++ 客户端与 Codex 共用缓存。
        if executable.contains("manager") || executable == "ps" {
            continue;
        }
        if name.contains("codex.app/")
            || name.contains("codex++.app/")
            || ["codex", "codex++", "codex-plus-launcher", "codex-plus-plus"].contains(&executable)
            || executable.starts_with("codex helper")
        {
            running.insert("Codex".into());
        }
        if name.contains("claude.app/")
            || name.contains("claude cn.app/")
            || ["claude", "claude-3p", "claude cn"].contains(&executable)
            || executable.starts_with("claude helper")
        {
            running.insert("Claude".into());
        }
    }
    running
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::FileTimes;

    fn old_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(SystemTime::now() - MIN_AGE * 2))
            .unwrap();
    }

    fn fixture() -> (tempfile::TempDir, CachePlan) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        old_file(&home.join(".codex/cache/old"), "cache");
        old_file(&home.join(".codex/cache/auth.json"), "credential");
        old_file(&home.join(".codex/.tmp/git/work"), "temp");
        old_file(&home.join(".codex/sessions/keep"), "history");
        old_file(&home.join(".codex/auth.json"), "credential");
        old_file(&home.join(".codex/plugins/cache/keep"), "plugin");
        fs::write(home.join(".codex/cache/recent"), "recent").unwrap();
        let roots = discover_roots(&home, &home.join(".codex")).unwrap();
        let plan = scan(&home, roots, SystemTime::now()).unwrap();
        (temp, plan)
    }

    fn selected(plan: &CachePlan) -> Vec<String> {
        plan.report
            .groups
            .iter()
            .filter(|group| group.cleanable && group.eligible_files > 0)
            .map(|group| group.id.clone())
            .collect()
    }

    #[test]
    fn removes_only_confirmed_old_cache_and_keeps_protected_data() {
        let (temp, plan) = fixture();
        let ids = selected(&plan);
        let scan_id = plan.report.scan_id.clone();
        assert_eq!(ids.len(), 1);
        let report = plan
            .clean_with_processes(&scan_id, &ids, true, SystemTime::now(), BTreeSet::new())
            .unwrap();
        assert_eq!(report.removed_files, 1);
        assert_eq!(report.removed_bytes, 5);
        assert!(!temp.path().join(".codex/cache/old").exists());
        for name in [
            "cache/recent",
            "cache/auth.json",
            ".tmp/git/work",
            "sessions/keep",
            "auth.json",
            "plugins/cache/keep",
        ] {
            assert!(temp.path().join(".codex").join(name).exists(), "{name}");
        }
    }

    #[test]
    fn requires_confirmation_and_valid_unexpired_selection() {
        let (temp, plan) = fixture();
        let now = SystemTime::now();
        assert!(
            plan.validate_selection("forged", &selected(&plan), now)
                .is_err()
        );
        assert!(
            plan.validate_selection(&plan.report.scan_id, &["../sessions".into()], now)
                .is_err()
        );
        assert!(
            plan.validate_selection(&plan.report.scan_id, &selected(&plan), now + PLAN_TTL)
                .is_err()
        );
        let temp_id = plan
            .report
            .groups
            .iter()
            .find(|group| group.kind == "temporary")
            .unwrap()
            .id
            .clone();
        assert!(
            plan.validate_selection(&plan.report.scan_id, &[temp_id], now)
                .is_err()
        );
        let ids = selected(&plan);
        let id = plan.report.scan_id.clone();
        assert!(
            plan.clean_with_processes(&id, &ids, false, now, BTreeSet::new())
                .is_err()
        );
        assert!(temp.path().join(".codex/cache/old").exists());
    }

    #[test]
    fn running_app_blocks_cleanup() {
        let (temp, plan) = fixture();
        let ids = selected(&plan);
        let id = plan.report.scan_id.clone();
        let running = apps_from_process_names(
            "/Applications/Codex.app/Contents/MacOS/Codex\nclaude\nCodexPlusPlusManager",
        );
        assert_eq!(running.len(), 2);
        assert!(
            plan.clean_with_processes(&id, &ids, true, SystemTime::now(), running)
                .is_err()
        );
        assert!(temp.path().join(".codex/cache/old").exists());
    }

    #[test]
    fn custom_codex_home_cannot_turn_plugins_into_cleanable_cache() {
        let (temp, _) = fixture();
        let home = temp.path().canonicalize().unwrap();
        let roots = discover_roots(&home, &home.join(".codex/plugins")).unwrap();
        let plan = scan(&home, roots, SystemTime::now()).unwrap();
        assert!(plan.files.is_empty());
        assert!(!plan.report.warnings.is_empty());
    }

    #[test]
    fn electron_profiles_include_only_known_cache_subdirectories() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let app = home.join("app");
        for path in [
            "Cache/data",
            "Default/Code Cache/data",
            "Default/Partitions/sandbox/GPUCache/data",
            "Partitions/preview/Shared Dictionary/cache/data",
        ] {
            old_file(&app.join(path), "cache");
        }
        for path in [
            "Local Storage/leveldb/data",
            "Default/Cookies",
            "vm_bundles/resource",
            "claude-code/runtime",
        ] {
            old_file(&app.join(path), "keep");
        }
        let mut roots = Vec::new();
        add_electron_roots(&mut roots, "Codex", &app, &home, 0).unwrap();
        let plan = scan(&home, roots, SystemTime::now()).unwrap();
        assert_eq!(plan.files.len(), 4);
        let ids = selected(&plan);
        let scan_id = plan.report.scan_id.clone();
        let report = plan
            .clean_with_processes(&scan_id, &ids, true, SystemTime::now(), BTreeSet::new())
            .unwrap();
        assert_eq!(report.removed_files, 4);
        assert!(app.join("Default/Cookies").exists());
        assert!(app.join("Local Storage/leveldb/data").exists());
        assert!(app.join("vm_bundles/resource").exists());
        assert!(app.join("claude-code/runtime").exists());
    }

    #[test]
    fn changed_or_new_files_are_not_removed() {
        let (temp, plan) = fixture();
        let ids = selected(&plan);
        let id = plan.report.scan_id.clone();
        fs::write(temp.path().join(".codex/cache/old"), "new contents").unwrap();
        old_file(&temp.path().join(".codex/cache/added"), "new");
        let report = plan
            .clean_with_processes(&id, &ids, true, SystemTime::now(), BTreeSet::new())
            .unwrap();
        assert_eq!(report.removed_files, 0);
        assert_eq!(report.skipped_files, 1);
        assert!(temp.path().join(".codex/cache/added").exists());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_hard_links_never_become_candidates() {
        use std::os::unix::fs::symlink;
        let (temp, _) = fixture();
        let home = temp.path().canonicalize().unwrap();
        symlink(
            home.join(".codex/sessions"),
            home.join(".codex/cache/linked"),
        )
        .unwrap();
        fs::hard_link(
            home.join(".codex/auth.json"),
            home.join(".codex/cache/hardlink"),
        )
        .unwrap();
        let plan = scan(
            &home,
            discover_roots(&home, &home.join(".codex")).unwrap(),
            SystemTime::now(),
        )
        .unwrap();
        assert_eq!(plan.files.len(), 1);
        fs::rename(home.join(".codex/cache"), home.join("original-cache")).unwrap();
        symlink(home.join(".codex/sessions"), home.join(".codex/cache")).unwrap();
        let ids = selected(&plan);
        let id = plan.report.scan_id.clone();
        let report = plan
            .clean_with_processes(&id, &ids, true, SystemTime::now(), BTreeSet::new())
            .unwrap();
        assert_eq!(report.removed_files, 0);
        assert_eq!(report.skipped_files, 1);
        assert!(home.join(".codex/sessions/keep").exists());
    }
}
