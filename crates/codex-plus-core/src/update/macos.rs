//! macOS 更新在同卷私有目录中准备完整 app，再用 rename 事务替换；不修改包内文件。
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy)]
struct BundleSpec {
    name: &'static str,
    id: &'static str,
    executable: &'static str,
}

const BUNDLES: [BundleSpec; 1] = [BundleSpec {
    name: "Codex++.app",
    id: "com.bigpizzav3.codexplusplus",
    executable: "CodexPlusPlus",
}];

const LEGACY_MIGRATION_MESSAGE: &str = "旧双应用布局需手动迁移：下载新版安装包，将完整 Codex++.app 安装到 Applications 后打开；旧版自动更新器不支持单应用安装包，旧入口请确认后自行移除。";

fn installed_root_from_executable(executable: &Path) -> anyhow::Result<PathBuf> {
    let root = executable
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("请从已安装的 Codex++.app 内更新。"))?;
    let expected = root
        .join(BUNDLES[0].name)
        .join("Contents/MacOS")
        .join(BUNDLES[0].executable);
    // Finder 可能保留目录大小写；是否为新版界面由 CodexPlusUnifiedApp 标记校验。
    let normalize_path = |path: &Path| {
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_ascii_lowercase()
    };
    if normalize_path(&expected) != normalize_path(executable) {
        let app_name = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());
        if app_name.is_some_and(|name| name.eq_ignore_ascii_case("Codex++ 管理工具.app")) {
            bail!("{LEGACY_MIGRATION_MESSAGE}");
        }
        bail!(
            "当前应用目录名称不符合自动更新布局；请将完整新版 Codex++.app 手动安装到标准应用目录后更新。"
        );
    }
    Ok(root.to_path_buf())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BundleInfo {
    name: String,
    unified: bool,
    id: String,
    executable: String,
    version: String,
    team: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdatePlan {
    schema: u8,
    install_root: PathBuf,
    transaction: PathBuf,
    installer: PathBuf,
    version: String,
    manager_pid: u32,
    old_bundles: [BundleInfo; 1],
}

trait BundleOps {
    fn inspect(&self, bundle: &Path) -> anyhow::Result<BundleInfo>;
    fn verify_signature(&self, bundle: &Path) -> anyhow::Result<()>;
    fn copy_bundle(&self, source: &Path, target: &Path) -> anyhow::Result<()>;
    fn rename(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
        fs::rename(source, target)
            .with_context(|| format!("移动 app 失败：{} → {}", source.display(), target.display()))
    }
}

fn validate_target_environment(
    architectures: &[&str],
    host_arm64: bool,
    rosetta_running: bool,
    minimum_os: Option<&str>,
    current_os: &str,
) -> anyhow::Result<()> {
    let compatible = if host_arm64 {
        architectures
            .iter()
            .any(|arch| matches!(*arch, "arm64" | "arm64e"))
            || (rosetta_running && architectures.contains(&"x86_64"))
    } else {
        architectures.contains(&"x86_64")
    };
    if !compatible {
        bail!("更新 app 不支持当前 Mac 的 CPU 架构，请下载匹配本机的安装包。");
    }
    if let Some(minimum) = minimum_os {
        if super::is_newer_version(minimum, current_os)? {
            bail!("更新 app 至少需要 macOS {minimum}，当前系统为 {current_os}。");
        }
    }
    Ok(())
}

fn approval_allows_replacement(decision: Option<&str>) -> anyhow::Result<bool> {
    match decision {
        Some("approve") => Ok(true),
        Some("cancel") => bail!("更新已取消，旧 app 未替换。"),
        None => Ok(false),
        _ => bail!("更新批准信息无效，旧 app 未替换。"),
    }
}

fn validate_bundle(
    ops: &impl BundleOps,
    bundle: &Path,
    spec: BundleSpec,
) -> anyhow::Result<BundleInfo> {
    if fs::symlink_metadata(bundle)?.file_type().is_symlink() {
        bail!("更新 app 不允许是符号链接：{}", bundle.display());
    }
    let info = ops.inspect(bundle)?;
    if info.id != spec.id
        || info.executable != spec.executable
        || info.name != crate::install::APP_NAME
    {
        bail!("更新 app 身份或可执行文件不匹配：{}", bundle.display());
    }
    if !info.unified {
        bail!("{LEGACY_MIGRATION_MESSAGE}");
    }
    super::parse_version_tag(&info.version)?;
    let executable = bundle.join("Contents/MacOS").join(spec.executable);
    if !executable.is_file()
        || !fs::canonicalize(&executable)?.starts_with(fs::canonicalize(bundle)?)
    {
        bail!("更新 app 可执行文件缺失或越出包目录：{}", bundle.display());
    }
    Ok(info)
}

fn prepare_bundles(ops: &impl BundleOps, plan: &UpdatePlan, mounted: &Path) -> anyhow::Result<()> {
    let staged = plan.transaction.join("staged");
    fs::create_dir(&staged)?;
    for (index, spec) in BUNDLES.iter().copied().enumerate() {
        let source = mounted.join(spec.name);
        let info = validate_bundle(ops, &source, spec)?;
        if super::parse_version_tag(&info.version)? != super::parse_version_tag(&plan.version)?
            || !super::is_newer_version(&info.version, &plan.old_bundles[index].version)?
        {
            bail!("安装包版本不符合更新计划：{}", source.display());
        }
        if let Some(team) = &plan.old_bundles[index].team {
            if info.team.as_ref() != Some(team) {
                bail!("安装包签名 TeamIdentifier 不匹配：{}", source.display());
            }
        }
        ops.verify_signature(&source)?;
        let target = staged.join(spec.name);
        ops.copy_bundle(&source, &target)?;
        let copied = validate_bundle(ops, &target, spec)?;
        if copied.id != info.id || copied.version != info.version || copied.team != info.team {
            bail!("复制后 app 元数据变化：{}", target.display());
        }
        ops.verify_signature(&target)?;
    }
    Ok(())
}

fn replace_bundles(ops: &impl BundleOps, plan: &UpdatePlan) -> anyhow::Result<()> {
    // 等待进程退出后重新核对旧版本，防止准备期间其它更新改了目标。
    for (index, spec) in BUNDLES.iter().copied().enumerate() {
        let old = validate_bundle(ops, &plan.install_root.join(spec.name), spec)?;
        if old.version != plan.old_bundles[index].version
            || old.team != plan.old_bundles[index].team
        {
            bail!("安装目录在更新准备期间已变化，未替换任何 app。");
        }
    }
    let backups = plan.transaction.join("backups");
    let rejected = plan.transaction.join("rejected");
    fs::create_dir(&backups)?;
    fs::create_dir(&rejected)?;
    let mut backed_up = Vec::new();
    let mut installed = Vec::new();
    let result = (|| -> anyhow::Result<()> {
        for (index, spec) in BUNDLES.iter().copied().enumerate() {
            let target = plan.install_root.join(spec.name);
            ops.rename(&target, &backups.join(spec.name))?;
            backed_up.push(index);
            ops.rename(&plan.transaction.join("staged").join(spec.name), &target)?;
            installed.push(index);
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        for index in installed.into_iter().rev() {
            let spec = BUNDLES[index];
            if let Err(error) = ops.rename(
                &plan.install_root.join(spec.name),
                &rejected.join(spec.name),
            ) {
                rollback_errors.push(error.to_string());
            }
        }
        for index in backed_up.into_iter().rev() {
            let spec = BUNDLES[index];
            let target = plan.install_root.join(spec.name);
            if target.exists() {
                rollback_errors.push(format!("目标仍存在，拒绝覆盖：{}", target.display()));
                continue;
            }
            if let Err(error) = ops.rename(&backups.join(spec.name), &target) {
                rollback_errors.push(error.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            bail!(
                "更新失败：{error}；回滚未完整完成：{}；旧 app 保留在 {}",
                rollback_errors.join("；"),
                backups.display()
            );
        }
        bail!("更新失败，旧 Codex++ app 已恢复：{error}");
    }
    Ok(())
}

/// 更新事务目录建在目标 .app 同级（同卷才能 rename 原子替换），名字固定为前缀 + 小写 UUID。
const TRANSACTION_PREFIX: &str = ".codex-plus-update-";
/// 历史事务目录至少闲置这么久才清理，避免碰到别的实例刚建好、helper 还没拿锁的事务。
const STALE_TRANSACTION_MIN_AGE: Duration = Duration::from_secs(15 * 60);
/// 事务目录顶层只会出现这些文件（含 atomic_write 的临时文件）；另有 `decision-<uuid>`。
/// 2.1.2 及更早版本的 helper 留下的也是同一组文件。`.DS_Store` 是用户在访达里点开过残留
/// 目录时由访达写入的显示元数据，不是用户数据；不放行的话，看过一眼的残留就永远清不掉。
const TRANSACTION_FILES: [&str; 7] = [
    "plan.json",
    "manager-update-helper",
    "helper-stderr.log",
    "result.json",
    "result.json.tmp",
    "decision",
    ".DS_Store",
];
/// helper 结束时写进 `result.json` 的终态；写完后 helper 只剩重新打开 app、退出。
const TRANSACTION_FINISHED_STATES: [&str; 3] = ["installed", "installed_restart_failed", "failed"];
/// 启动清理之后的补跑间隔（累计）：上一轮有留待下次的事务目录时才继续。
/// 第一次补跑等刚完成更新的 helper 退出、放开更新锁；第二次补跑在闲置期过后，
/// 收掉没写终态（如中途被杀）的事务。
const STARTUP_CLEANUP_RETRY_DELAYS: [Duration; 2] = [
    Duration::from_secs(60),
    Duration::from_secs(STALE_TRANSACTION_MIN_AGE.as_secs() + 60),
];
/// 事务目录顶层只会出现这些子目录。
const TRANSACTION_DIRS: [&str; 4] = ["mount", "staged", "backups", "rejected"];

fn transaction_dir_name(id: uuid::Uuid) -> String {
    format!("{TRANSACTION_PREFIX}{}", id.hyphenated())
}

/// `<前缀><uuid>` 的严格匹配：UUID 必须是 `Uuid::new_v4().to_string()` 的原样写法。
fn is_uuid_suffixed(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|rest| {
        uuid::Uuid::try_parse(rest).is_ok_and(|id| id.hyphenated().to_string() == rest)
    })
}

fn is_transaction_dir_name(name: &str) -> bool {
    is_uuid_suffixed(name, TRANSACTION_PREFIX)
}

/// 校验 `transaction` 确实是 `install_root` 下由更新流程创建、可以整个删除的事务目录。
/// 任何一项不符都拒绝：宁可留下残留，也不能删到别的目录。
fn ensure_removable_transaction(install_root: &Path, transaction: &Path) -> anyhow::Result<()> {
    if transaction.parent() != Some(install_root) {
        bail!("事务目录不在目标 app 所在目录：{}", transaction.display());
    }
    if !transaction
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_transaction_dir_name)
    {
        bail!("不是更新事务目录：{}", transaction.display());
    }
    // symlink_metadata 不跟随链接：同名符号链接一律不碰。
    let meta = fs::symlink_metadata(transaction)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("更新事务路径不是普通目录：{}", transaction.display());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // 更新流程建目录后立即设为 0700；别人的或权限放开过的目录不是我们的。
        if meta.mode() & 0o077 != 0 {
            bail!("更新事务目录不是私有目录：{}", transaction.display());
        }
    }
    for entry in fs::read_dir(transaction)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().unwrap_or_default();
        let expected_dir = if TRANSACTION_DIRS.contains(&name) {
            true
        } else if TRANSACTION_FILES.contains(&name) || is_uuid_suffixed(name, "decision-") {
            false
        } else {
            bail!("更新事务目录含有非更新暂存内容：{}", entry.path().display());
        };
        // DirEntry::file_type 不跟随符号链接。
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || file_type.is_dir() != expected_dir {
            bail!("更新事务目录条目类型异常：{}", entry.path().display());
        }
    }
    let mount = transaction.join("mount");
    if let Ok(mount_meta) = fs::symlink_metadata(&mount) {
        // 安装包仍挂载着时挂载点非空 / 设备号不同；绝不递归进挂载的卷。
        if fs::read_dir(&mount)?.next().is_some() {
            bail!("安装包挂载点非空，可能仍在挂载：{}", mount.display());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if mount_meta.dev() != meta.dev() {
                bail!("安装包挂载点仍在挂载：{}", mount.display());
            }
        }
        #[cfg(not(unix))]
        let _ = mount_meta;
    }
    // 回滚没完成时 backups 里可能是唯一一份旧 app：目标位置没有 app 就整个保留。
    for spec in BUNDLES {
        if fs::symlink_metadata(transaction.join("backups").join(spec.name)).is_ok()
            && fs::symlink_metadata(install_root.join(spec.name)).is_err()
        {
            bail!(
                "目标位置缺少 {}，事务里的旧 app 备份需保留：{}",
                spec.name,
                transaction.display()
            );
        }
    }
    Ok(())
}

/// `result.json` 是否已写成终态（安装完成 / 失败）。只认事务目录里的普通小文件，
/// 符号链接、超大、解析失败一律当作「未结束」，回落到闲置时间判断。
fn transaction_finished(transaction: &Path) -> bool {
    let path = transaction.join("result.json");
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return false;
    };
    if !meta.is_file() || meta.len() > 64 * 1024 {
        return false;
    }
    fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|state| state["status"].as_str().map(str::to_string))
        .is_some_and(|status| TRANSACTION_FINISHED_STATES.contains(&status.as_str()))
}

/// 事务目录能否现在清理（不含 [`ensure_removable_transaction`] 的结构校验）。
///
/// **调用方必须已拿到该安装目录的更新锁**：helper 从开始到退出一直持有这把锁，只在拿到锁
/// 之后写 `result.json`。所以「拿到锁 + 终态」说明写终态的 helper 已经退出，刚完成的更新
/// 也可以马上清（#2774：2.1.2 → 2.1.3 由旧 helper 替换、不删目录，新版第一次启动时目录
/// 才改动过几秒，按闲置时间一律跳过，用户就一直看得到）。
/// 没有终态的（另一个实例刚建好、helper 还没拿锁，或中途被杀）仍要闲置够久才清。
fn ensure_transaction_idle(transaction: &Path, now: SystemTime) -> anyhow::Result<()> {
    if transaction_finished(transaction) {
        return Ok(());
    }
    let modified = fs::symlink_metadata(transaction)?.modified()?;
    if now.duration_since(modified).unwrap_or_default() < STALE_TRANSACTION_MIN_AGE {
        bail!("更新事务目录最近还有改动，可能仍在使用，留待下次清理");
    }
    Ok(())
}

/// 启动清理：先跑一次，之后只要上一轮还有留待下次的事务目录，就按 `delays` 依次补跑；
/// `cleanup` 返回是否仍有留待下次的。补跑次数有上限，不常驻轮询。
fn run_cleanup_with_retries(
    delays: &[Duration],
    mut sleep: impl FnMut(Duration),
    mut cleanup: impl FnMut() -> bool,
) {
    if !cleanup() {
        return;
    }
    for delay in delays {
        sleep(*delay);
        if !cleanup() {
            return;
        }
    }
}

/// 严格校验后删除一个更新事务目录（暂存副本、旧 app 备份、helper 副本等）。
fn remove_transaction(install_root: &Path, transaction: &Path) -> anyhow::Result<()> {
    ensure_removable_transaction(install_root, transaction)?;
    fs::remove_dir_all(transaction)
        .with_context(|| format!("删除更新事务目录失败：{}", transaction.display()))
}

/// 清理 `install_root` 下以前的更新留下的事务目录（issue #2774）。
/// 只看名字是「前缀 + UUID」的直接子项，逐个校验后删除；单个失败不影响其它，
/// 每一项的结果交给调用方写诊断日志。调用方须持有该安装目录的更新锁。
fn cleanup_stale_transactions(
    install_root: &Path,
    now: SystemTime,
) -> anyhow::Result<Vec<(PathBuf, Result<(), String>)>> {
    let mut outcomes = Vec::new();
    for entry in fs::read_dir(install_root)? {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry
            .file_name()
            .to_str()
            .is_some_and(is_transaction_dir_name)
        {
            continue;
        }
        let path = entry.path();
        let result = (|| -> anyhow::Result<()> {
            ensure_transaction_idle(&path, now)?;
            remove_transaction(install_root, &path)
        })();
        outcomes.push((path, result.map_err(|error| format!("{error:#}"))));
    }
    Ok(outcomes)
}

/// 记录本机更新建过的事务目录（JSON 字符串数组），放在 Codex++ 的 state 目录里（#2774 后续）。
///
/// 启动 / 下次更新时只清理记下来的这些路径，不再列出 app 所在目录：app 放在「下载 / 桌面 /
/// 文稿」等受保护目录时，每次启动列目录都可能触发隐私授权提示。
const TRANSACTION_RECORD_FILE: &str = "macos-update-transactions.json";

/// 同一进程里启动清理（后台线程）与更新开始时的记录读写互斥。
static TRANSACTION_RECORD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn transaction_record_path(state_dir: &Path) -> PathBuf {
    state_dir.join(TRANSACTION_RECORD_FILE)
}

/// 读记录；文件不存在或内容损坏时当作空记录（损坏的记录下次写入时被覆盖）。
fn read_transaction_record(record: &Path) -> Vec<PathBuf> {
    fs::read(record)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<PathBuf>>(&bytes).ok())
        .unwrap_or_default()
}

/// 写记录；记录清空时删掉这一个记录文件，不在 state 目录里留空文件。
fn write_transaction_record(record: &Path, paths: &[PathBuf]) -> anyhow::Result<()> {
    if paths.is_empty() {
        return match fs::remove_file(record) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        };
    }
    crate::settings::atomic_write(record, &serde_json::to_vec_pretty(paths)?)
}

/// 更新开始、建事务目录**之前**把它记下来：中途崩溃留下的目录下次也能按记录清理；
/// 目录最终没建出来时，下次清理发现路径不存在就从记录里去掉。
fn record_transaction(record: &Path, transaction: &Path) -> anyhow::Result<()> {
    let _guard = TRANSACTION_RECORD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut paths = read_transaction_record(record);
    if !paths.iter().any(|path| path == transaction) {
        paths.push(transaction.to_path_buf());
    }
    write_transaction_record(record, &paths)
}

/// 清理记录里的事务目录（不列任何目录）。每个路径仍走 [`remove_transaction`] 的全部安全校验：
///
/// - 名字不是「前缀 + UUID」、没有父目录：记录不可信，直接从记录里去掉，不碰文件系统；
/// - 路径已经不存在（更新成功后 helper 已删掉、或目录没建出来）：从记录里去掉；
/// - `lock_root` 返回 `None`（该安装目录有更新正在进行）、最近还有改动、校验或删除失败：
///   保留在记录里，下次再试；
/// - 删除成功：从记录里去掉。
///
/// 返回实际处理过的路径与结果（给调用方写诊断日志）。
fn cleanup_recorded_transactions<G>(
    record: &Path,
    now: SystemTime,
    mut lock_root: impl FnMut(&Path) -> Option<G>,
) -> Vec<(PathBuf, Result<(), String>)> {
    let _guard = TRANSACTION_RECORD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let recorded = read_transaction_record(record);
    if recorded.is_empty() {
        return Vec::new();
    }
    let mut kept = Vec::new();
    let mut outcomes = Vec::new();
    for transaction in recorded {
        let root = transaction.parent().filter(|_| {
            transaction
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(is_transaction_dir_name)
        });
        let Some(root) = root else {
            outcomes.push((
                transaction,
                Err("记录里的路径不是更新事务目录，已从记录中移除".to_string()),
            ));
            continue;
        };
        if let Err(error) = fs::symlink_metadata(&transaction) {
            if error.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            outcomes.push((transaction.clone(), Err(format!("{error:#}"))));
            kept.push(transaction);
            continue;
        }
        let Some(_lock) = lock_root(root) else {
            outcomes.push((
                transaction.clone(),
                Err("该安装目录有更新正在进行，留待下次清理".to_string()),
            ));
            kept.push(transaction);
            continue;
        };
        let result = (|| -> anyhow::Result<()> {
            ensure_transaction_idle(&transaction, now)?;
            remove_transaction(root, &transaction)
        })();
        if result.is_err() {
            kept.push(transaction.clone());
        }
        outcomes.push((transaction, result.map_err(|error| format!("{error:#}"))));
    }
    if let Err(error) = write_transaction_record(record, &kept) {
        outcomes.push((
            record.to_path_buf(),
            Err(format!("更新事务记录写入失败：{error:#}")),
        ));
    }
    outcomes
}

/// 2.1.2 留下、没有记录过的旧残留，只在标准应用目录（`/Applications`、`~/Applications`）里
/// 按前缀扫描清理（纯函数）。其他位置（下载、桌面、文稿等）不扫描，避免反复触发隐私授权提示。
fn legacy_transaction_scan_allowed(root: &Path, home: Option<&Path>) -> bool {
    root == Path::new("/Applications") || home.is_some_and(|home| root == home.join("Applications"))
}

#[cfg(target_os = "macos")]
mod runtime {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::process::{Command, Stdio};
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    struct NativeOps;
    fn drain_bounded(
        mut reader: impl Read + Send + 'static,
    ) -> std::sync::mpsc::Receiver<std::io::Result<(Vec<u8>, bool)>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<(Vec<u8>, bool)> {
                let mut captured = Vec::new();
                let mut overflow = false;
                let mut buffer = [0u8; 8192];
                loop {
                    let count = reader.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    let retained = count.min((256 * 1024usize).saturating_sub(captured.len()));
                    captured.extend_from_slice(&buffer[..retained]);
                    overflow |= retained != count;
                }
                Ok((captured, overflow))
            })();
            let _ = sender.send(result);
        });
        receiver
    }
    fn output(program: &str, args: &[&std::ffi::OsStr]) -> anyhow::Result<std::process::Output> {
        let mut command = Command::new(program);
        command.args(args);
        output_command(command, Duration::from_secs(60))
    }
    fn output_command(
        mut command: Command,
        timeout: Duration,
    ) -> anyhow::Result<std::process::Output> {
        let program = command.get_program().to_string_lossy().to_string();
        let deadline = Instant::now() + timeout;
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = drain_bounded(child.stdout.take().context("更新命令 stdout 不可用")?);
        let stderr = drain_bounded(child.stderr.take().context("更新命令 stderr 不可用")?);
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                timed_out = true;
                break child.wait()?;
            }
            sleep(Duration::from_millis(50));
        };
        if timed_out {
            bail!("更新命令超时：{program}");
        }
        // 后代可能仍持有 pipe：共享同一个 deadline，不用无界 join 等待它们。
        let (stdout, stdout_overflow) = stdout
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("读取更新 stdout 超时或通道已关闭")??;
        let (stderr, stderr_overflow) = stderr
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("读取更新 stderr 超时或通道已关闭")??;
        if stdout_overflow || stderr_overflow {
            bail!("更新命令输出超过限制，未继续更新：{program}");
        }
        Ok(std::process::Output {
            status,
            stdout,
            stderr,
        })
    }
    fn checked(program: &str, args: &[&std::ffi::OsStr]) -> anyhow::Result<std::process::Output> {
        let result = output(program, args)?;
        if !result.status.success() {
            bail!(
                "更新命令失败 {program}：{}",
                String::from_utf8_lossy(&result.stderr).trim()
            );
        }
        Ok(result)
    }
    fn plist(bundle: &Path, key: &str) -> anyhow::Result<String> {
        let command = format!("Print :{key}");
        let path = bundle.join("Contents/Info.plist");
        let result = checked(
            "/usr/libexec/PlistBuddy",
            &["-c".as_ref(), command.as_ref(), path.as_os_str()],
        )?;
        Ok(String::from_utf8(result.stdout)?.trim().to_string())
    }
    fn optional_plist(bundle: &Path, key: &str) -> anyhow::Result<Option<String>> {
        let command = format!("Print :{key}");
        let path = bundle.join("Contents/Info.plist");
        let result = output(
            "/usr/libexec/PlistBuddy",
            &["-c".as_ref(), command.as_ref(), path.as_os_str()],
        )?;
        if result.status.success() {
            return Ok(Some(String::from_utf8(result.stdout)?.trim().to_string()));
        }
        if String::from_utf8_lossy(&result.stderr).contains("Does Not Exist")
            || String::from_utf8_lossy(&result.stdout).contains("Does Not Exist")
        {
            return Ok(None);
        }
        bail!(
            "读取 app 最低系统版本失败：{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    fn sysctl_flag(key: &str) -> anyhow::Result<bool> {
        let result = output("/usr/sbin/sysctl", &["-n".as_ref(), key.as_ref()])?;
        if result.status.success() {
            return Ok(String::from_utf8(result.stdout)?.trim() == "1");
        }
        if String::from_utf8_lossy(&result.stderr).contains("unknown oid") {
            return Ok(false);
        }
        bail!(
            "读取 Mac 架构信息失败：{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    impl BundleOps for NativeOps {
        fn inspect(&self, bundle: &Path) -> anyhow::Result<BundleInfo> {
            let signature = output(
                "/usr/bin/codesign",
                &["-d".as_ref(), "--verbose=4".as_ref(), bundle.as_os_str()],
            )?;
            let text = String::from_utf8_lossy(&signature.stderr);
            let team = text
                .lines()
                .find_map(|line| line.strip_prefix("TeamIdentifier="))
                .filter(|team| !team.is_empty() && *team != "not set")
                .map(ToString::to_string);
            Ok(BundleInfo {
                name: plist(bundle, "CFBundleName")?,
                unified: optional_plist(bundle, "CodexPlusUnifiedApp")?
                    .is_some_and(|value| value == "true"),
                id: plist(bundle, "CFBundleIdentifier")?,
                executable: plist(bundle, "CFBundleExecutable")?,
                version: plist(bundle, "CFBundleShortVersionString")?,
                team,
            })
        }
        fn verify_signature(&self, bundle: &Path) -> anyhow::Result<()> {
            let executable = bundle
                .join("Contents/MacOS")
                .join(plist(bundle, "CFBundleExecutable")?);
            if fs::metadata(&executable)?.mode() & 0o111 == 0 {
                bail!("更新 app 可执行文件没有执行权限：{}", bundle.display());
            }
            let mut magic = [0u8; 4];
            File::open(&executable)?.read_exact(&mut magic)?;
            if !matches!(
                u32::from_be_bytes(magic),
                0xfeedface
                    | 0xcefaedfe
                    | 0xfeedfacf
                    | 0xcffaedfe
                    | 0xcafebabe
                    | 0xbebafeca
                    | 0xcafebabf
                    | 0xbfbafeca
            ) {
                bail!("更新 app 的可执行文件不是 Mach-O：{}", bundle.display());
            }
            let archs = checked(
                "/usr/bin/lipo",
                &["-archs".as_ref(), executable.as_os_str()],
            )?;
            let archs = String::from_utf8(archs.stdout)?;
            let os = checked("/usr/bin/sw_vers", &["-productVersion".as_ref()])?;
            let os = String::from_utf8(os.stdout)?;
            let minimum_os = optional_plist(bundle, "LSMinimumSystemVersion")?;
            let host_arm64 = sysctl_flag("hw.optional.arm64")?;
            validate_target_environment(
                &archs.split_whitespace().collect::<Vec<_>>(),
                host_arm64,
                // 外部 sysctl.proc_translated 查询的是 sysctl 自身，不能用来判定
                // x86 helper 是否跑在 Rosetta；宿主 ARM + 当前进程 x86 即为证明。
                host_arm64 && cfg!(target_arch = "x86_64"),
                minimum_os.as_deref(),
                os.trim(),
            )?;
            checked(
                "/usr/bin/codesign",
                &[
                    "--verify".as_ref(),
                    "--deep".as_ref(),
                    "--strict".as_ref(),
                    bundle.as_os_str(),
                ],
            )
            .map(|_| ())
        }
        fn copy_bundle(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
            checked(
                "/usr/bin/ditto",
                &[
                    "--rsrc".as_ref(),
                    "--extattr".as_ref(),
                    source.as_os_str(),
                    target.as_os_str(),
                ],
            )
            .map(|_| ())
        }
    }

    fn manager_executable(root: &Path) -> PathBuf {
        let executable = root
            .join(BUNDLES[0].name)
            .join("Contents/MacOS")
            .join(BUNDLES[0].executable);
        fs::canonicalize(&executable).unwrap_or(executable)
    }
    fn write_state(plan: &UpdatePlan, status: &str, message: &str) -> anyhow::Result<()> {
        crate::settings::atomic_write(
            &plan.transaction.join("result.json"),
            &serde_json::to_vec(&serde_json::json!({"status":status,"message":message}))?,
        )?;
        let _ = crate::diagnostic_log::append_diagnostic_log(
            &format!("update.macos.{status}"),
            serde_json::json!({"version":plan.version,"installRoot":plan.install_root,"message":message,"transaction":plan.transaction}),
        );
        Ok(())
    }
    fn write_decision(plan: &UpdatePlan, decision: &str) -> anyhow::Result<()> {
        use std::io::Write;
        let temporary = plan
            .transaction
            .join(format!("decision-{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(decision.as_bytes())?;
        file.sync_all()?;
        // 同卷 hard_link 原子发布完整决定，且不会覆盖已存在的 approve/cancel。
        fs::hard_link(temporary, plan.transaction.join("decision"))?;
        Ok(())
    }
    fn read_decision(plan: &UpdatePlan) -> anyhow::Result<Option<String>> {
        let path = plan.transaction.join("decision");
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !meta.is_file()
            || meta.mode() & 0o077 != 0
            || meta.len() > 16
            || meta.uid() != fs::metadata(&plan.transaction)?.uid()
        {
            bail!("更新批准信息不是本事务的私有文件。");
        }
        Ok(Some(fs::read_to_string(path)?))
    }
    fn wait_approval(plan: &UpdatePlan) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if approval_allows_replacement(read_decision(plan)?.as_deref())? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                bail!("未收到 Codex++ 批准，旧 app 未替换。");
            }
            sleep(Duration::from_millis(100));
        }
    }
    fn process_path(pid: u32) -> anyhow::Result<Option<PathBuf>> {
        let pid = pid.to_string();
        let result = output(
            "/bin/ps",
            &[
                "-ww".as_ref(),
                "-p".as_ref(),
                pid.as_ref(),
                "-o".as_ref(),
                "comm=".as_ref(),
            ],
        )?;
        let text = String::from_utf8(result.stdout)?.trim().to_string();
        if text.is_empty() {
            return Ok(None);
        }
        let path = PathBuf::from(text);
        Ok(Some(fs::canonicalize(&path).unwrap_or(path)))
    }
    fn wait_manager(plan: &UpdatePlan) -> anyhow::Result<()> {
        let expected = manager_executable(&plan.install_root);
        let start = Instant::now();
        while let Some(path) = process_path(plan.manager_pid)? {
            if path != expected {
                bail!("Codex++ 进程身份已变化，未替换 app。");
            }
            if start.elapsed() > Duration::from_secs(60) {
                bail!("等待 Codex++ 正常退出超时，未替换 app。");
            }
            sleep(Duration::from_millis(100));
        }
        Ok(())
    }
    fn open_bundle(path: &Path) -> anyhow::Result<()> {
        checked("/usr/bin/open", &[path.as_os_str()]).map(|_| ())
    }
    fn validate_plan(path: &Path) -> anyhow::Result<UpdatePlan> {
        let meta = fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.mode() & 0o077 != 0 || meta.len() > 64 * 1024 {
            bail!("更新计划必须是私有、小型普通文件。");
        }
        let plan: UpdatePlan = serde_json::from_slice(&fs::read(path)?)?;
        let root = fs::canonicalize(&plan.install_root)?;
        let transaction = fs::canonicalize(&plan.transaction)?;
        let txn_meta = fs::metadata(&transaction)?;
        if plan.schema != 2
            || plan.manager_pid <= 1
            || plan.manager_pid == std::process::id()
            || root != plan.install_root
            || transaction != plan.transaction
            || transaction.parent() != Some(root.as_path())
            || txn_meta.mode() & 0o077 != 0
            || txn_meta.uid() != meta.uid()
            || !transaction
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".codex-plus-update-"))
            || fs::canonicalize(path)? != transaction.join("plan.json")
            || fs::canonicalize(std::env::current_exe()?)?
                != transaction.join("manager-update-helper")
        {
            bail!("更新计划或 helper 不属于当前安装目录的私有事务。");
        }
        if !plan.installer.is_file() || plan.installer.extension().is_none_or(|ext| ext != "dmg") {
            bail!("更新计划的 DMG 不存在。");
        }
        if process_path(plan.manager_pid)?.as_deref() != Some(manager_executable(&root).as_path()) {
            bail!("启动更新的 Codex++ 会话已失效或不属于本安装目录。");
        }
        for (index, spec) in BUNDLES.iter().copied().enumerate() {
            let info = validate_bundle(&NativeOps, &root.join(spec.name), spec)?;
            if info.version != plan.old_bundles[index].version
                || info.team != plan.old_bundles[index].team
            {
                bail!("旧 app 不符合更新计划。");
            }
        }
        Ok(plan)
    }

    fn prepare_detached_helper(executable: &Path, transaction: &Path) -> anyhow::Result<PathBuf> {
        let helper = transaction.join("manager-update-helper");
        fs::copy(executable, &helper)?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        // bundle 内的签名引用 Info.plist；裸副本须重新签名才能独立运行。
        checked(
            "/usr/bin/codesign",
            &[
                "--force".as_ref(),
                "--sign".as_ref(),
                "-".as_ref(),
                helper.as_os_str(),
            ],
        )
        .context("临时更新 helper 重新签名失败")?;
        checked(
            "/usr/bin/codesign",
            &["--verify".as_ref(), "--strict".as_ref(), helper.as_os_str()],
        )
        .context("临时更新 helper 签名校验失败")?;
        Ok(helper)
    }

    /// 拿安装目录的更新锁。更新 helper 运行期间一直持有这把锁；拿不到（或锁文件是符号链接）
    /// 返回 `None`，说明有更新在进行，这次不碰该目录的任何事务目录。
    /// 锁文件不存在时不创建，免得在安装目录里多留东西。
    fn lock_install_root(root: &Path) -> Option<Option<File>> {
        let lock_path = root.join(".codex-plus-update.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return None;
        }
        match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(file) => {
                if fs2::FileExt::try_lock_exclusive(&file).is_err() {
                    return None;
                }
                Some(Some(file))
            }
            Err(_) => Some(None),
        }
    }

    /// 持锁期间删掉安装目录里的更新锁文件（#2774：更新完成后 `.codex-plus-update.lock` 一直留在
    /// 「应用程序」里）。只删和手里这把锁是同一个文件的普通文件；同路径已被换成别的文件或链接时不动。
    /// 删除后锁仍随 `lock` 一起持有到函数返回，期间别的进程打开同路径会新建文件、拿到的是另一把锁，
    /// 所以只在不再需要互斥的收尾时调用。
    fn remove_held_install_lock(root: &Path, lock: &File) {
        use std::os::unix::fs::MetadataExt;
        let lock_path = root.join(".codex-plus-update.lock");
        let (Ok(on_disk), Ok(held)) = (fs::symlink_metadata(&lock_path), lock.metadata()) else {
            return;
        };
        if !on_disk.file_type().is_file() || on_disk.dev() != held.dev() || on_disk.ino() != held.ino() {
            return;
        }
        if let Err(error) = fs::remove_file(&lock_path) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "update.macos.lock_cleanup_skipped",
                serde_json::json!({"installRoot":root,"message":format!("{error:#}")}),
            );
        }
    }

    fn log_cleanup_outcomes(outcomes: Vec<(PathBuf, Result<(), String>)>) {
        for (path, result) in outcomes {
            let (event, message) = match result {
                Ok(()) => ("update.macos.transaction_cleaned", None),
                Err(message) => ("update.macos.transaction_cleanup_skipped", Some(message)),
            };
            let _ = crate::diagnostic_log::append_diagnostic_log(
                event,
                serde_json::json!({"transaction":path,"message":message}),
            );
        }
    }

    fn transaction_record() -> PathBuf {
        transaction_record_path(&crate::paths::default_app_state_dir())
    }

    /// 同一进程里的清理（启动后台线程、补跑、更新开始前）串行执行，避免同时拿安装目录的更新锁。
    static CLEANUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    /// 本进程已开始自动更新：后台补跑不再去碰更新锁，免得 helper 拿锁时撞上。
    static UPDATE_STARTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// 清理以前的更新留下的事务目录；只写诊断日志，从不报错。
    /// 返回是否还有留待下次的事务目录（更新锁被占、最近改动过、校验未通过等）。
    ///
    /// 先按 state 目录里的记录清理（不列目录）；`root` 是 `/Applications` 或 `~/Applications`
    /// 时再按前缀扫描一次，收掉 2.1.2 留下、没有记录的旧残留。
    fn cleanup_stale(root: &Path) -> bool {
        let _guard = CLEANUP_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cleanup_stale_locked(root)
    }

    fn cleanup_stale_locked(root: &Path) -> bool {
        let now = std::time::SystemTime::now();
        let record = transaction_record();
        log_cleanup_outcomes(cleanup_recorded_transactions(
            &record,
            now,
            lock_install_root,
        ));
        let mut pending = !read_transaction_record(&record).is_empty();
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .map(|home| fs::canonicalize(&home).unwrap_or(home));
        if !legacy_transaction_scan_allowed(root, home.as_deref()) {
            return pending;
        }
        let Some(lock) = lock_install_root(root) else {
            return true;
        };
        match cleanup_stale_transactions(root, now) {
            Ok(outcomes) => {
                pending |= outcomes.iter().any(|(_, result)| result.is_err());
                log_cleanup_outcomes(outcomes);
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "update.macos.transaction_cleanup_failed",
                    serde_json::json!({"installRoot":root,"message":format!("{error:#}")}),
                );
            }
        }
        // 拿到了锁说明没有更新在进行：以前的 helper 留下的锁文件一并收掉（#2774）。
        if let Some(lock) = &lock {
            remove_held_install_lock(root, lock);
        }
        pending
    }

    /// 启动时调用（后台线程）：只处理当前运行的 Codex++.app 所在目录；不是标准安装布局
    /// （如开发构建）就跳过。
    ///
    /// 旧版 helper 完成的更新（如 2.1.2 → 2.1.3）不删事务目录，新版启动时 helper 可能还没退出、
    /// 目录也刚改动过，第一次清理会跳过；所以有留待下次的就按 [`STARTUP_CLEANUP_RETRY_DELAYS`]
    /// 补跑，最多两次（#2774）。本进程开始自动更新后不再补跑。
    pub(super) fn cleanup_stale_for_current_app() {
        let Ok(executable) = std::env::current_exe().and_then(fs::canonicalize) else {
            return;
        };
        let Ok(root) = installed_root_from_executable(&executable) else {
            return;
        };
        run_cleanup_with_retries(&STARTUP_CLEANUP_RETRY_DELAYS, sleep, || {
            let _guard = CLEANUP_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if UPDATE_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
                return false;
            }
            cleanup_stale_locked(&root)
        });
    }

    pub(super) fn launch(installer: &Path, version: &str) -> anyhow::Result<()> {
        let executable = fs::canonicalize(std::env::current_exe()?)?;
        let root = installed_root_from_executable(&executable)?;
        let old_bundles = [validate_bundle(
            &NativeOps,
            &root.join(BUNDLES[0].name),
            BUNDLES[0],
        )?];
        if !super::super::is_newer_version(version, &old_bundles[0].version)? {
            bail!("更新版本未高于当前 Codex++ 版本。");
        }
        // 先清掉以前失败 / 中断留下的事务目录，再建本次的。先置标记：等后台清理这一轮
        // 结束（放开更新锁）后，它不会再补跑，helper 拿锁不会撞上。
        UPDATE_STARTED.store(true, std::sync::atomic::Ordering::SeqCst);
        cleanup_stale(&root);
        let transaction = root.join(transaction_dir_name(uuid::Uuid::new_v4()));
        // 先记下再建目录，之后启动 / 下次更新只按记录清理，不必列出 app 所在目录。
        // 记录失败不挡更新：成功后 helper 仍会删掉事务目录，只是中断时的残留要手动清。
        if let Err(error) = record_transaction(&transaction_record(), &transaction) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "update.macos.transaction_record_failed",
                serde_json::json!({"transaction":transaction,"message":format!("{error:#}")}),
            );
        }
        fs::create_dir(&transaction)
            .context("安装目录不可写；未请求提升权限，请在可写位置手动安装完整 app")?;
        fs::set_permissions(&transaction, fs::Permissions::from_mode(0o700))?;
        let plan = UpdatePlan {
            schema: 2,
            install_root: root.to_path_buf(),
            transaction,
            installer: fs::canonicalize(installer)?,
            version: version.to_string(),
            manager_pid: std::process::id(),
            old_bundles,
        };
        let helper = prepare_detached_helper(&executable, &plan.transaction)?;
        let path = plan.transaction.join("plan.json");
        use std::io::Write;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&serde_json::to_vec(&plan)?)?;
        file.sync_all()?;
        let stderr_path = plan.transaction.join("helper-stderr.log");
        let stderr = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&stderr_path)?;
        let mut child = Command::new(helper)
            .arg("--apply-codex-plus-update")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()?;
        let start = Instant::now();
        loop {
            if start.elapsed() > Duration::from_secs(300) {
                let _ = write_decision(&plan, "cancel");
                bail!("等待安装包准备完成超时，旧版未替换。");
            }
            if let Ok(bytes) = fs::read(plan.transaction.join("result.json")) {
                let state: serde_json::Value = serde_json::from_slice(&bytes)?;
                if state["status"] == "ready" {
                    write_decision(&plan, "approve")?;
                    return Ok(());
                }
                if state["status"] == "failed" {
                    bail!(
                        "更新 helper 未就绪：{}",
                        state["message"].as_str().unwrap_or("未知错误")
                    );
                }
            }
            if let Some(status) = child.try_wait()? {
                let mut bytes = Vec::new();
                File::open(&stderr_path)?
                    .take(8192)
                    .read_to_end(&mut bytes)?;
                let detail: String = String::from_utf8_lossy(&bytes).chars().take(1024).collect();
                bail!(
                    "更新 helper 提前退出（{status}），旧版未替换。{}",
                    detail.trim()
                );
            }
            sleep(Duration::from_millis(100));
        }
    }
    pub(super) fn run(path: &Path) -> anyhow::Result<()> {
        let plan = validate_plan(path)?;
        let lock_path = plan.install_root.join(".codex-plus-update.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            bail!("更新锁不允许符号链接。");
        }
        let lock: File = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(lock_path)?;
        fs2::FileExt::try_lock_exclusive(&lock).context("本安装目录已有另一个更新在进行")?;
        let mut approved = false;
        let outcome = (|| -> anyhow::Result<()> {
            let mount = plan.transaction.join("mount");
            fs::create_dir(&mount)?;
            let attached = checked(
                "/usr/bin/hdiutil",
                &[
                    "attach".as_ref(),
                    "-readonly".as_ref(),
                    "-nobrowse".as_ref(),
                    "-noautoopen".as_ref(),
                    "-mountpoint".as_ref(),
                    mount.as_os_str(),
                    plan.installer.as_os_str(),
                ],
            );
            let prepared = attached.and_then(|_| prepare_bundles(&NativeOps, &plan, &mount));
            // attach 部分成功后也可能报错，所有分支都尝试卸载本事务自己的挂载点。
            let detached = checked("/usr/bin/hdiutil", &["detach".as_ref(), mount.as_os_str()]);
            prepared?;
            detached?;
            write_state(&plan, "ready", "Codex++.app 已校验并完整复制，等待程序退出")?;
            wait_approval(&plan)?;
            approved = true;
            wait_manager(&plan)?;
            replace_bundles(&NativeOps, &plan)?;
            Ok(())
        })();
        let result = finish_update(&plan, outcome, approved);
        // 更新已结束（成功或失败）：持锁删掉锁文件，不在「应用程序」里留下 .codex-plus-update.lock（#2774）。
        remove_held_install_lock(&plan.install_root, &lock);
        result
    }

    /// 更新结果收尾：写状态、成功时重开新版并删事务目录，失败时视情况重开旧版。
    fn finish_update(
        plan: &UpdatePlan,
        outcome: anyhow::Result<()>,
        approved: bool,
    ) -> anyhow::Result<()> {
        match outcome {
            Ok(()) => {
                write_state(plan, "installed", "Codex++.app 已安装")?;
                let restarted = open_bundle(&plan.install_root.join(BUNDLES[0].name));
                if let Err(error) = &restarted {
                    let _ = write_state(
                        &plan,
                        "installed_restart_failed",
                        &format!("安装已完成但重新启动失败：{error}"),
                    );
                }
                // 新版已就位，事务目录（暂存副本、旧 app 备份、helper 副本）不再需要（#2774）。
                // 删掉正在运行的 helper 文件不影响本进程；删除失败只记诊断日志。
                // 之后不能再 write_state，否则 atomic_write 会把事务目录重新建出来。
                if let Err(error) = remove_transaction(&plan.install_root, &plan.transaction) {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "update.macos.transaction_cleanup_skipped",
                        serde_json::json!({"transaction":plan.transaction,"message":format!("{error:#}")}),
                    );
                }
                restarted
            }
            Err(error) => {
                let _ = write_state(&plan, "failed", &error.to_string());
                // 只在原 Codex++ 已退出时重开；验证或签名拒绝不改变旧 app。
                if approved && wait_manager(&plan).is_ok() {
                    let _ = open_bundle(&plan.install_root.join(BUNDLES[0].name));
                }
                Err(error)
            }
        }
    }

    /// 更新锁文件的收尾（#2774）：拿到锁才删、只删手里这把锁对应的普通文件。
    #[cfg(test)]
    pub(super) fn install_lock_cleanup_smoke(root: &Path) -> anyhow::Result<()> {
        let lock_path = root.join(".codex-plus-update.lock");
        // 没有更新在进行：拿到锁后删掉以前留下的锁文件。
        fs::write(&lock_path, "")?;
        let held = lock_install_root(root).context("锁不该被占用")?.context("锁文件应存在")?;
        remove_held_install_lock(root, &held);
        drop(held);
        anyhow::ensure!(!lock_path.exists(), "拿到锁后应删掉锁文件");
        // 锁文件不存在时也不创建。
        anyhow::ensure!(matches!(lock_install_root(root), Some(None)), "不存在时不创建");
        anyhow::ensure!(!lock_path.exists());

        // 另一个更新正持有锁：拿不到锁，不删。
        fs::write(&lock_path, "")?;
        let other = OpenOptions::new().read(true).write(true).open(&lock_path)?;
        fs2::FileExt::try_lock_exclusive(&other)?;
        anyhow::ensure!(lock_install_root(root).is_none(), "被占用时拿不到锁");
        anyhow::ensure!(lock_path.exists(), "被占用时不能删");
        drop(other);

        // 持锁期间同路径被换成别的文件：不删新文件。
        let held = lock_install_root(root).context("锁不该被占用")?.context("锁文件应存在")?;
        fs::rename(&lock_path, root.join("moved.lock"))?;
        fs::write(&lock_path, "new")?;
        remove_held_install_lock(root, &held);
        anyhow::ensure!(fs::read_to_string(&lock_path)? == "new", "换过的文件不能删");
        fs::remove_file(&lock_path)?;
        fs::remove_file(root.join("moved.lock"))?;

        // 锁文件是符号链接：不当锁用，也不删。
        let target = root.join("target.lock");
        fs::write(&target, "")?;
        std::os::unix::fs::symlink(&target, &lock_path)?;
        anyhow::ensure!(lock_install_root(root).is_none(), "符号链接不当锁用");
        anyhow::ensure!(fs::symlink_metadata(&lock_path)?.file_type().is_symlink(), "符号链接不能删");
        anyhow::ensure!(target.exists(), "链接目标不能删");
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn detached_helper_smoke(root: &Path) -> anyhow::Result<()> {
        let bundle = root.join(BUNDLES[0].name);
        signed_test_bundle(&bundle, BUNDLES[0], "1.0.0")?;
        let executable = bundle.join("Contents/MacOS").join(BUNDLES[0].executable);
        let original = fs::read(&executable)?;
        let raw = root.join("unsigned-detached-copy");
        fs::copy(&executable, &raw)?;
        anyhow::ensure!(
            !output(
                "/usr/bin/codesign",
                &["--verify".as_ref(), "--strict".as_ref(), raw.as_os_str()]
            )?
            .status
            .success(),
            "fixture must reproduce a signature that depends on the app Info.plist"
        );
        let transaction = root.join("private-helper");
        fs::create_dir(&transaction)?;
        let helper = prepare_detached_helper(&executable, &transaction)?;
        checked(helper.to_str().context("helper path is not UTF-8")?, &[])?;
        anyhow::ensure!(
            fs::read(&executable)? == original,
            "helper preparation changed the source app"
        );
        NativeOps.verify_signature(&bundle)?;
        Ok(())
    }

    #[cfg(test)]
    fn signed_test_bundle(path: &Path, spec: BundleSpec, version: &str) -> anyhow::Result<()> {
        let binary = path.join("Contents/MacOS").join(spec.executable);
        fs::create_dir_all(binary.parent().unwrap())?;
        fs::create_dir_all(path.join("Contents/Resources/nested"))?;
        fs::copy("/usr/bin/true", &binary)?;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700))?;
        fs::write(
            path.join("Contents/Resources/test.dat"),
            format!("private fixture resource {version}"),
        )?;
        fs::write(path.join("Contents/Resources/nested/version.dat"), version)?;
        fs::write(
            path.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>CodexPlusUnifiedApp</key><true/><key>CFBundleName</key><string>Codex++</string><key>CFBundleIdentifier</key><string>{}</string><key>CFBundleExecutable</key><string>{}</string><key>CFBundleShortVersionString</key><string>{version}</string><key>CFBundleVersion</key><string>{version}</string><key>CFBundlePackageType</key><string>APPL</string><key>LSMinimumSystemVersion</key><string>10.0.0</string></dict></plist>",
                spec.id, spec.executable,
            ),
        )?;
        checked(
            "/usr/bin/codesign",
            &[
                "--force".as_ref(),
                "--deep".as_ref(),
                "--sign".as_ref(),
                "-".as_ref(),
                path.as_os_str(),
            ],
        )?;
        NativeOps.verify_signature(path)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn native_smoke(root: &Path) -> anyhow::Result<()> {
        // 仅运行系统工具并读写调用方的 tempfile，不调用 ps/kill/open/hdiutil。
        checked("/usr/bin/true", &[])?;
        let large_output = root.join("large-output-fixture");
        fs::write(&large_output, vec![b'x'; 160 * 1024])?;
        let captured = checked("/bin/cat", &[large_output.as_os_str()])?;
        if captured.stdout.len() != 160 * 1024 {
            bail!("有界 pipe 读取丢失输出");
        }
        for spec in BUNDLES {
            let source = root.join(format!("source-{}", spec.name));
            let binary = source.join("Contents/MacOS").join(spec.executable);
            signed_test_bundle(&source, spec, "2.0.0")?;
            let original = validate_bundle(&NativeOps, &source, spec)?;
            NativeOps.verify_signature(&source)?;
            let copied = root.join(format!("copy-{}", spec.name));
            NativeOps.copy_bundle(&source, &copied)?;
            NativeOps.verify_signature(&copied)?;
            let copied_info = validate_bundle(&NativeOps, &copied, spec)?;
            if original.id != copied_info.id
                || original.version != copied_info.version
                || original.team != copied_info.team
                || fs::read(binary)?
                    != fs::read(copied.join("Contents/MacOS").join(spec.executable))?
                || fs::read(source.join("Contents/Resources/test.dat"))?
                    != fs::read(copied.join("Contents/Resources/test.dat"))?
            {
                bail!("完整 bundle 复制校验不一致");
            }
            let copied_binary = copied.join("Contents/MacOS").join(spec.executable);
            let permissions = fs::metadata(&copied_binary)?.permissions();
            fs::set_permissions(&copied_binary, fs::Permissions::from_mode(0o600))?;
            if !NativeOps
                .verify_signature(&copied)
                .unwrap_err()
                .to_string()
                .contains("没有执行权限")
            {
                bail!("未拒绝缺少执行权限的更新包");
            }
            fs::set_permissions(copied_binary, permissions)?;
            let copied_plist = copied.join("Contents/Info.plist");
            checked(
                "/usr/libexec/PlistBuddy",
                &[
                    "-c".as_ref(),
                    "Set :LSMinimumSystemVersion 99.0.0".as_ref(),
                    copied_plist.as_os_str(),
                ],
            )?;
            let error = NativeOps.verify_signature(&copied).unwrap_err();
            if !error.to_string().contains("至少需要 macOS 99.0.0") {
                return Err(error);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn native_dmg_transaction(
        root: &Path,
        fail_second_rename: bool,
    ) -> anyhow::Result<()> {
        // 真实系统工具与真实文件系统，仅本测试 tempfile；不走启动/退出应用路径。
        let install_root = root.join("临时 安装目录 Applications with spaces");
        let payload = root.join("真实 DMG 内容 with spaces");
        fs::create_dir(&install_root)?;
        fs::create_dir(&payload)?;
        for spec in BUNDLES {
            signed_test_bundle(&install_root.join(spec.name), spec, "1.0.0")?;
            signed_test_bundle(&payload.join(spec.name), spec, "2.0.0")?;
        }
        let old_bundles = [NativeOps.inspect(&install_root.join(BUNDLES[0].name))?];
        let transaction = install_root.join(".codex-plus-update-native-test");
        fs::create_dir(&transaction)?;
        fs::set_permissions(&transaction, fs::Permissions::from_mode(0o700))?;
        let image = root.join("真实 更新安装包.dmg");
        checked(
            "/usr/bin/hdiutil",
            &[
                "create".as_ref(),
                "-srcfolder".as_ref(),
                payload.as_os_str(),
                "-volname".as_ref(),
                "CPP private native fixture".as_ref(),
                "-fs".as_ref(),
                "HFS+".as_ref(),
                "-format".as_ref(),
                "UDZO".as_ref(),
                image.as_os_str(),
            ],
        )
        .context("创建真实测试 DMG 失败")?;
        let plan = UpdatePlan {
            schema: 2,
            install_root,
            transaction,
            installer: image,
            version: "2.0.0".into(),
            manager_pid: std::process::id(),
            old_bundles,
        };
        let mount = plan.transaction.join("private readonly mount");
        fs::create_dir(&mount)?;
        let attached = checked(
            "/usr/bin/hdiutil",
            &[
                "attach".as_ref(),
                "-readonly".as_ref(),
                "-nobrowse".as_ref(),
                "-noautoopen".as_ref(),
                "-mountpoint".as_ref(),
                mount.as_os_str(),
                plan.installer.as_os_str(),
            ],
        )
        .context("只读挂载真实测试 DMG 失败");
        let prepared = attached.and_then(|_| {
            if fs::write(mount.join("must-not-write"), b"readonly probe").is_ok() {
                bail!("测试 DMG 没有以只读方式挂载");
            }
            prepare_bundles(&NativeOps, &plan, &mount)
                .context("从真实 DMG 校验/复制 Codex++.app 失败")
        });
        let detached = checked("/usr/bin/hdiutil", &["detach".as_ref(), mount.as_os_str()])
            .context("卸载私有测试 DMG 失败");
        if let Err(error) = prepared {
            if let Err(detach_error) = detached {
                bail!("{error:#}；且 {detach_error:#}");
            }
            return Err(error);
        }
        detached?;

        struct SecondRenameFault {
            source: PathBuf,
            target: PathBuf,
        }
        impl BundleOps for SecondRenameFault {
            fn inspect(&self, bundle: &Path) -> anyhow::Result<BundleInfo> {
                NativeOps.inspect(bundle)
            }
            fn verify_signature(&self, bundle: &Path) -> anyhow::Result<()> {
                NativeOps.verify_signature(bundle)
            }
            fn copy_bundle(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
                NativeOps.copy_bundle(source, target)
            }
            fn rename(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
                if source == self.source && target == self.target {
                    bail!("injected second native replacement rename failure");
                }
                NativeOps.rename(source, target)
            }
        }
        if fail_second_rename {
            let ops = SecondRenameFault {
                source: plan.transaction.join("staged").join(BUNDLES[0].name),
                target: plan.install_root.join(BUNDLES[0].name),
            };
            let error = replace_bundles(&ops, &plan).unwrap_err();
            if !error.to_string().contains("旧 Codex++ app 已恢复") {
                return Err(error);
            }
        } else {
            replace_bundles(&NativeOps, &plan)?;
        }
        let expected = if fail_second_rename { "1.0.0" } else { "2.0.0" };
        for spec in BUNDLES {
            let target = plan.install_root.join(spec.name);
            let info = validate_bundle(&NativeOps, &target, spec)?;
            NativeOps.verify_signature(&target)?;
            if info.version != expected
                || fs::read_to_string(target.join("Contents/Resources/nested/version.dat"))?
                    != expected
            {
                bail!("真实 DMG 事务后的版本/嵌套资源不正确");
            }
            if !fail_second_rename {
                let backup = plan.transaction.join("backups").join(spec.name);
                NativeOps.verify_signature(&backup)?;
                if NativeOps.inspect(&backup)?.version != "1.0.0"
                    || fs::read_to_string(backup.join("Contents/Resources/nested/version.dat"))?
                        != "1.0.0"
                {
                    bail!("真实 DMG 事务没有保留完整旧 app 备份");
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn pipe_deadline_smoke(root: &Path) -> anyhow::Result<()> {
        let done = root.join("controlled-pipe-child.done");
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "update::macos::tests::macos_update_controlled_pipe_fixture",
                "--nocapture",
            ])
            .env("CODEX_PLUS_UPDATE_PIPE_FIXTURE", "parent")
            .env("CODEX_PLUS_UPDATE_PIPE_DONE", &done);
        let start = Instant::now();
        let error = output_command(command, Duration::from_millis(250)).unwrap_err();
        if start.elapsed() > Duration::from_millis(700) || !error.to_string().contains("超时") {
            bail!("继承 pipe 的受控子进程导致无界等待：{error}");
        }
        // 等自己的短寿命 fixture 结束，不终止任何其它进程。
        while !done.exists() && start.elapsed() < Duration::from_secs(3) {
            sleep(Duration::from_millis(10));
        }
        if !done.exists() {
            bail!("受控 pipe fixture 没有按时结束");
        }
        sleep(Duration::from_millis(100));
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub fn launch_update(installer: &Path, version: &str) -> anyhow::Result<()> {
    runtime::launch(installer, version)
}
#[cfg(target_os = "macos")]
pub fn run_update_helper(path: &Path) -> anyhow::Result<()> {
    runtime::run(path)
}
/// 启动时清理当前 Codex++.app 同级目录里以前的更新残留（`.codex-plus-update-<uuid>`）。
/// 失败只写诊断日志，不影响启动。
#[cfg(target_os = "macos")]
pub fn cleanup_stale_update_transactions() {
    runtime::cleanup_stale_for_current_app()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FixtureOps {
        reject_signature: bool,
        fail_rename: Option<usize>,
        renames: Cell<usize>,
    }
    impl Default for FixtureOps {
        fn default() -> Self {
            Self {
                reject_signature: false,
                fail_rename: None,
                renames: Cell::new(0),
            }
        }
    }
    fn copy_tree(source: &Path, target: &Path) -> anyhow::Result<()> {
        fs::create_dir(target)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let destination = target.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &destination)?;
            } else {
                fs::copy(entry.path(), destination)?;
            }
        }
        Ok(())
    }
    impl BundleOps for FixtureOps {
        fn inspect(&self, bundle: &Path) -> anyhow::Result<BundleInfo> {
            Ok(serde_json::from_slice(&fs::read(
                bundle.join("Contents/Info.plist"),
            )?)?)
        }
        fn verify_signature(&self, _: &Path) -> anyhow::Result<()> {
            if self.reject_signature {
                bail!("fixture signature rejected");
            }
            Ok(())
        }
        fn copy_bundle(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
            copy_tree(source, target)
        }
        fn rename(&self, source: &Path, target: &Path) -> anyhow::Result<()> {
            let count = self.renames.get() + 1;
            self.renames.set(count);
            if self.fail_rename == Some(count) {
                bail!("fixture rename failure");
            }
            fs::rename(source, target)?;
            Ok(())
        }
    }
    fn bundle(root: &Path, spec: BundleSpec, version: &str) -> BundleInfo {
        let path = root.join(spec.name);
        fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        fs::create_dir_all(path.join("Contents/Resources/nested")).unwrap();
        fs::create_dir_all(path.join("Contents/_CodeSignature")).unwrap();
        let info = BundleInfo {
            name: spec.name.strip_suffix(".app").unwrap_or(spec.name).into(),
            unified: spec.id == crate::install::APP_BUNDLE_ID,
            id: spec.id.into(),
            executable: spec.executable.into(),
            version: version.into(),
            team: Some("FIXTURE_TEAM".into()),
        };
        fs::write(
            path.join("Contents/Info.plist"),
            serde_json::to_vec(&info).unwrap(),
        )
        .unwrap();
        fs::write(
            path.join("Contents/MacOS").join(spec.executable),
            format!("binary-{version}"),
        )
        .unwrap();
        fs::write(
            path.join("Contents/Resources/nested/resource.dat"),
            format!("resource-{version}"),
        )
        .unwrap();
        fs::write(
            path.join("Contents/_CodeSignature/CodeResources"),
            format!("seal-{version}"),
        )
        .unwrap();
        info
    }
    /// 和 launch 一样建 0700 私有目录。
    fn create_private_dir(path: &Path) {
        fs::create_dir(path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }
    fn fixture(include_app: bool) -> (tempfile::TempDir, UpdatePlan, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("自定义 Applications with spaces");
        let payload = temp.path().join("只读 DMG fixture");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&payload).unwrap();
        let old_bundles = [bundle(&root, BUNDLES[0], "1.0.0")];
        if include_app {
            bundle(&payload, BUNDLES[0], "2.0.0");
        }
        let transaction = root.join(transaction_dir_name(uuid::Uuid::new_v4()));
        create_private_dir(&transaction);
        let plan = UpdatePlan {
            schema: 2,
            install_root: root,
            transaction,
            installer: temp.path().join("fixture.dmg"),
            version: "2.0.0".into(),
            manager_pid: 0,
            old_bundles,
        };
        (temp, plan, payload)
    }
    fn assert_old_apps(ops: &FixtureOps, plan: &UpdatePlan) {
        for spec in BUNDLES {
            assert_eq!(
                ops.inspect(&plan.install_root.join(spec.name))
                    .unwrap()
                    .version,
                "1.0.0"
            );
            assert_eq!(
                fs::read_to_string(
                    plan.install_root
                        .join(spec.name)
                        .join("Contents/Resources/nested/resource.dat")
                )
                .unwrap(),
                "resource-1.0.0"
            );
        }
    }

    #[test]
    fn macos_update_copies_single_full_bundle_and_preserves_backup_in_unicode_root() {
        let (_temp, plan, payload) = fixture(true);
        let ops = FixtureOps::default();
        prepare_bundles(&ops, &plan, &payload).unwrap();
        replace_bundles(&ops, &plan).unwrap();
        for spec in BUNDLES {
            let target = plan.install_root.join(spec.name);
            assert_eq!(ops.inspect(&target).unwrap().version, "2.0.0");
            assert_eq!(
                fs::read_to_string(target.join("Contents/Resources/nested/resource.dat")).unwrap(),
                "resource-2.0.0"
            );
            assert_eq!(
                fs::read_to_string(target.join("Contents/_CodeSignature/CodeResources")).unwrap(),
                "seal-2.0.0"
            );
            assert_eq!(
                ops.inspect(&plan.transaction.join("backups").join(spec.name))
                    .unwrap()
                    .version,
                "1.0.0"
            );
        }
    }

    #[test]
    fn macos_update_rolls_back_app_when_replacement_rename_fails() {
        let (_temp, plan, payload) = fixture(true);
        let ops = FixtureOps {
            fail_rename: Some(2),
            ..FixtureOps::default()
        };
        prepare_bundles(&ops, &plan, &payload).unwrap();
        assert!(
            replace_bundles(&ops, &plan)
                .unwrap_err()
                .to_string()
                .contains("旧 Codex++ app 已恢复")
        );
        assert_old_apps(&ops, &plan);
        assert!(
            !plan
                .transaction
                .join("rejected")
                .join(BUNDLES[0].name)
                .exists()
        );
    }

    #[test]
    fn macos_update_rejects_missing_single_app_before_any_replacement() {
        let (_temp, plan, payload) = fixture(false);
        let ops = FixtureOps::default();
        assert!(prepare_bundles(&ops, &plan, &payload).is_err());
        assert_eq!(ops.renames.get(), 0);
        assert_old_apps(&ops, &plan);
    }

    #[test]
    fn macos_update_accepts_unified_installation_and_requests_manual_legacy_migration() {
        // 按宿主平台构造路径，Windows 也会执行这组布局策略测试。
        let root = PathBuf::from("/Applications");
        let executable_path = |bundle: &str, executable: &str| {
            root.join(bundle).join("Contents/MacOS").join(executable)
        };
        assert_eq!(
            installed_root_from_executable(&executable_path("Codex++.app", "CodexPlusPlus"))
                .unwrap(),
            root
        );
        assert_eq!(
            installed_root_from_executable(&executable_path("codex++.app", "CodexPlusPlus"))
                .unwrap(),
            root
        );
        for executable in [executable_path(
            "Codex++ 管理工具.app",
            "CodexPlusPlusManager",
        )] {
            let error = installed_root_from_executable(&executable).unwrap_err();
            assert!(error.to_string().contains("手动迁移"));
            assert!(
                error
                    .to_string()
                    .contains("旧版自动更新器不支持单应用安装包")
            );
        }
        let renamed = installed_root_from_executable(&executable_path(
            "Codex++ 2.app",
            "CodexPlusPlus",
        ))
        .unwrap_err();
        assert!(renamed.to_string().contains("标准应用目录"));
        assert!(!renamed.to_string().contains("旧双应用"));
    }

    #[test]
    fn macos_update_rejects_legacy_silent_identity_with_shared_bundle_id() {
        let (_temp, plan, payload) = fixture(true);
        let ops = FixtureOps::default();
        let app = payload.join(BUNDLES[0].name);
        let mut info = ops.inspect(&app).unwrap();
        info.unified = false;
        fs::write(
            app.join("Contents/Info.plist"),
            serde_json::to_vec(&info).unwrap(),
        )
        .unwrap();
        assert!(prepare_bundles(&ops, &plan, &payload).is_err());
        assert_eq!(ops.renames.get(), 0);
        assert_old_apps(&ops, &plan);
    }

    #[test]
    fn macos_update_leaves_legacy_manager_bundle_in_place() {
        let (_temp, plan, payload) = fixture(true);
        let legacy = BundleSpec {
            name: "Codex++ 管理工具.app",
            id: "com.bigpizzav3.codexplusplus.manager",
            executable: "CodexPlusPlusManager",
        };
        bundle(&plan.install_root, legacy, "1.0.0");
        let ops = FixtureOps::default();
        prepare_bundles(&ops, &plan, &payload).unwrap();
        replace_bundles(&ops, &plan).unwrap();
        assert_eq!(ops.renames.get(), 2);
        assert_eq!(
            ops.inspect(&plan.install_root.join(legacy.name))
                .unwrap()
                .version,
            "1.0.0"
        );
        assert_eq!(
            ops.inspect(&plan.install_root.join(BUNDLES[0].name))
                .unwrap()
                .version,
            "2.0.0"
        );
    }

    #[test]
    fn macos_update_rejects_invalid_identity_and_team() {
        for invalid_team in [false, true] {
            let (_temp, plan, payload) = fixture(true);
            let ops = FixtureOps::default();
            let path = payload.join(BUNDLES[0].name).join("Contents/Info.plist");
            let mut info = ops.inspect(&payload.join(BUNDLES[0].name)).unwrap();
            if invalid_team {
                info.team = Some("UNRELATED_TEAM".into());
            } else {
                info.id = "com.unrelated.application".into();
            }
            fs::write(path, serde_json::to_vec(&info).unwrap()).unwrap();
            assert!(prepare_bundles(&ops, &plan, &payload).is_err());
            assert_old_apps(&ops, &plan);
        }
    }

    #[test]
    fn macos_update_rejects_signature_failure_without_modifying_old_apps() {
        let (_temp, plan, payload) = fixture(true);
        let ops = FixtureOps {
            reject_signature: true,
            ..FixtureOps::default()
        };
        assert!(
            prepare_bundles(&ops, &plan, &payload)
                .unwrap_err()
                .to_string()
                .contains("signature rejected")
        );
        assert_old_apps(&ops, &plan);
        assert_eq!(ops.renames.get(), 0);
    }

    #[test]
    fn macos_update_architecture_and_minimum_os_policy_matrix() {
        for (archs, arm, translated, expected) in [
            (vec!["arm64"], false, false, false),
            (vec!["x86_64"], false, false, true),
            (vec!["arm64"], true, true, true),
            (vec!["x86_64"], true, false, false),
            (vec!["x86_64"], true, true, true),
            (vec!["x86_64", "arm64"], true, false, true),
            (vec!["x86_64", "arm64"], false, false, true),
            (vec!["i386"], false, false, false),
        ] {
            assert_eq!(
                validate_target_environment(&archs, arm, translated, None, "15.0.0").is_ok(),
                expected
            );
        }
        assert!(
            validate_target_environment(&["arm64"], true, false, Some("16.0"), "15.0.1").is_err()
        );
        assert!(
            validate_target_environment(&["arm64"], true, false, Some("15.0"), "15.0.1").is_ok()
        );
    }

    #[test]
    fn macos_update_requires_explicit_parent_approval_after_preparation() {
        assert!(!approval_allows_replacement(None).unwrap());
        assert!(approval_allows_replacement(Some("cancel")).is_err());
        assert!(approval_allows_replacement(Some("malformed")).is_err());
        assert!(approval_allows_replacement(Some("approve")).unwrap());
    }

    /// 补齐真实 helper 跑完后事务目录里的其余条目。
    fn populate_helper_files(transaction: &Path) {
        for name in [
            "plan.json",
            "manager-update-helper",
            "helper-stderr.log",
            "result.json",
            "decision",
        ] {
            fs::write(transaction.join(name), name).unwrap();
        }
        fs::write(
            transaction.join(format!("decision-{}", uuid::Uuid::new_v4())),
            "approve",
        )
        .unwrap();
        // hdiutil detach 后留下的空挂载点。
        fs::create_dir(transaction.join("mount")).unwrap();
    }
    fn completed_fixture() -> (tempfile::TempDir, UpdatePlan, PathBuf) {
        let (temp, plan, payload) = fixture(true);
        let ops = FixtureOps::default();
        prepare_bundles(&ops, &plan, &payload).unwrap();
        replace_bundles(&ops, &plan).unwrap();
        populate_helper_files(&plan.transaction);
        (temp, plan, payload)
    }
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3600)
    }

    #[test]
    fn macos_update_transaction_name_requires_prefix_and_canonical_uuid() {
        let id = uuid::Uuid::new_v4();
        assert!(is_transaction_dir_name(&transaction_dir_name(id)));
        assert!(is_transaction_dir_name(
            ".codex-plus-update-9f06f429-f57b-4b76-a1f8-0eddc12459fd"
        ));
        for name in [
            ".codex-plus-update.lock".to_string(),
            ".codex-plus-update-fixture".to_string(),
            ".codex-plus-update-".to_string(),
            format!("codex-plus-update-{id}"),
            format!(".codex-plus-update-{}", id.to_string().to_uppercase()),
            format!(".codex-plus-update-{}", id.simple()),
            format!(".codex-plus-update-{id}x"),
            format!(".codex-plus-update-{id}/.."),
        ] {
            assert!(!is_transaction_dir_name(&name), "{name}");
        }
    }

    #[test]
    fn macos_update_removes_transaction_after_successful_install() {
        let (_temp, plan, payload) = completed_fixture();
        remove_transaction(&plan.install_root, &plan.transaction).unwrap();
        assert!(!plan.transaction.exists());
        // 新版 app 与安装包内容都不受影响。
        let ops = FixtureOps::default();
        assert_eq!(
            ops.inspect(&plan.install_root.join(BUNDLES[0].name))
                .unwrap()
                .version,
            "2.0.0"
        );
        assert!(
            payload
                .join(BUNDLES[0].name)
                .join("Contents/Info.plist")
                .is_file()
        );
    }

    #[test]
    fn macos_update_stale_cleanup_only_removes_own_transactions_in_install_root() {
        let (temp, plan, _payload) = completed_fixture();
        let root = plan.install_root.clone();
        // 安装目录之外的私有数据：任何诱饵都不能让清理碰到它。
        let outside = temp.path().join("外部用户数据");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("plan.json"), "user data").unwrap();
        fs::create_dir(outside.join(BUNDLES[0].name)).unwrap();
        fs::write(outside.join(BUNDLES[0].name).join("keep"), "keep").unwrap();
        let decoy = |name: String| {
            let path = root.join(name);
            create_private_dir(&path);
            fs::write(path.join("plan.json"), "decoy").unwrap();
            path
        };
        let uuid_name = || transaction_dir_name(uuid::Uuid::new_v4());
        let mut kept = vec![
            decoy(".codex-plus-update-fixture".into()),
            decoy(format!(
                ".codex-plus-update-{}",
                uuid::Uuid::new_v4().to_string().to_uppercase()
            )),
            decoy(format!("codex-plus-update-{}", uuid::Uuid::new_v4())),
        ];
        let foreign = decoy(uuid_name());
        fs::write(foreign.join("用户笔记.txt"), "not ours").unwrap();
        kept.push(foreign);
        let mounted = decoy(uuid_name());
        fs::create_dir(mounted.join("mount")).unwrap();
        fs::write(mounted.join("mount").join("卷内容"), "mounted dmg").unwrap();
        kept.push(mounted);
        let not_dir = root.join(uuid_name());
        fs::write(&not_dir, "file").unwrap();
        kept.push(not_dir);
        let lock = root.join(".codex-plus-update.lock");
        fs::write(&lock, "").unwrap();
        kept.push(lock);
        // 另一个目录里的合法事务不属于这个安装目录。
        let elsewhere = temp.path().join(uuid_name());
        create_private_dir(&elsewhere);
        fs::write(elsewhere.join("plan.json"), "other root").unwrap();
        kept.push(elsewhere);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let link = root.join(uuid_name());
            symlink(&outside, &link).unwrap();
            kept.push(link);
            let linked_backups = decoy(uuid_name());
            symlink(&outside, linked_backups.join("backups")).unwrap();
            kept.push(linked_backups);
            let shared = decoy(uuid_name());
            fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).unwrap();
            kept.push(shared);
        }

        let outcomes = cleanup_stale_transactions(&root, later()).unwrap();

        assert!(!plan.transaction.exists());
        let removed: Vec<_> = outcomes
            .iter()
            .filter(|(_, result)| result.is_ok())
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(removed, vec![plan.transaction.clone()]);
        for path in &kept {
            assert!(fs::symlink_metadata(path).is_ok(), "{}", path.display());
        }
        assert_eq!(
            fs::read_to_string(outside.join("plan.json")).unwrap(),
            "user data"
        );
        assert!(outside.join(BUNDLES[0].name).join("keep").is_file());
        assert_eq!(
            FixtureOps::default()
                .inspect(&root.join(BUNDLES[0].name))
                .unwrap()
                .version,
            "2.0.0"
        );
    }

    #[test]
    fn macos_update_stale_cleanup_keeps_recent_and_only_backup_transactions() {
        let (_temp, plan, _payload) = completed_fixture();
        let root = plan.install_root.clone();
        // 刚改动过的事务可能还在用，不清。
        let outcomes = cleanup_stale_transactions(&root, SystemTime::now()).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].1.as_ref().unwrap_err().contains("最近"));
        assert!(plan.transaction.exists());
        // 回滚没完成、目标位置没有 app 时，backups 是唯一一份旧 app，不清。
        fs::rename(
            root.join(BUNDLES[0].name),
            plan.transaction.join("rejected").join(BUNDLES[0].name),
        )
        .unwrap();
        let outcomes = cleanup_stale_transactions(&root, later()).unwrap();
        assert!(outcomes[0].1.as_ref().unwrap_err().contains("备份需保留"));
        assert!(
            plan.transaction
                .join("backups")
                .join(BUNDLES[0].name)
                .join("Contents/Info.plist")
                .is_file()
        );
    }

    #[test]
    fn macos_update_recorded_cleanup_only_touches_recorded_transactions() {
        let (temp, plan, _payload) = completed_fixture();
        let root = plan.install_root.clone();
        let record = transaction_record_path(&temp.path().join("state"));
        // 2.1.2 留下、没有记录过的残留：按记录清理时不列目录，也就不会碰到它。
        let (_other_temp, unrecorded, _) = completed_fixture();
        let unrecorded_in_root = root.join(transaction_dir_name(uuid::Uuid::new_v4()));
        fs::rename(&unrecorded.transaction, &unrecorded_in_root).unwrap();
        // 目录最终没建出来的记录、被篡改成非事务路径的记录。
        let never_created = root.join(transaction_dir_name(uuid::Uuid::new_v4()));
        let outside = temp.path().join("外部用户数据");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        for path in [&plan.transaction, &never_created, &outside] {
            record_transaction(&record, path).unwrap();
        }
        // 重复记录同一路径只留一条。
        record_transaction(&record, &plan.transaction).unwrap();
        assert_eq!(read_transaction_record(&record).len(), 3);

        let mut locked_roots = Vec::new();
        let outcomes = cleanup_recorded_transactions(&record, later(), |root: &Path| {
            locked_roots.push(root.to_path_buf());
            Some(())
        });

        assert!(!plan.transaction.exists());
        assert!(unrecorded_in_root.exists());
        assert!(outside.join("keep").is_file());
        assert_eq!(locked_roots, vec![root.clone()]);
        let removed: Vec<_> = outcomes
            .iter()
            .filter(|(_, result)| result.is_ok())
            .map(|(path, _)| path.clone())
            .collect();
        assert_eq!(removed, vec![plan.transaction.clone()]);
        // 全部处理完：记录文件删掉，不在 state 目录留空文件。
        assert!(!record.exists());
        assert!(cleanup_recorded_transactions(&record, later(), |_| Some(())).is_empty());
    }

    #[test]
    fn macos_update_recorded_cleanup_keeps_busy_recent_and_unsafe_entries() {
        let (temp, plan, _payload) = completed_fixture();
        let record = transaction_record_path(&temp.path().join("state"));
        record_transaction(&record, &plan.transaction).unwrap();
        // 安装目录有更新正在进行：不碰，留在记录里。
        let outcomes = cleanup_recorded_transactions(&record, later(), |_| None::<()>);
        assert!(outcomes[0].1.as_ref().unwrap_err().contains("正在进行"));
        assert!(plan.transaction.exists());
        // 最近还有改动：不碰，留在记录里。
        let outcomes = cleanup_recorded_transactions(&record, SystemTime::now(), |_| Some(()));
        assert!(outcomes[0].1.as_ref().unwrap_err().contains("最近"));
        assert!(plan.transaction.exists());
        // 混进了非更新内容：安全校验拒绝，留在记录里。
        fs::write(plan.transaction.join("用户笔记.txt"), "not ours").unwrap();
        let outcomes = cleanup_recorded_transactions(&record, later(), |_| Some(()));
        assert!(
            outcomes[0]
                .1
                .as_ref()
                .unwrap_err()
                .contains("非更新暂存内容")
        );
        assert!(plan.transaction.join("用户笔记.txt").is_file());
        assert_eq!(
            read_transaction_record(&record),
            vec![plan.transaction.clone()]
        );
        // 校验通过后照常删除并清空记录。
        fs::remove_file(plan.transaction.join("用户笔记.txt")).unwrap();
        let outcomes = cleanup_recorded_transactions(&record, later(), |_| Some(()));
        assert!(outcomes[0].1.is_ok());
        assert!(!plan.transaction.exists());
        assert!(!record.exists());
    }

    /// 把事务的 `result.json` 写成 helper 的某个状态（与 `write_state` 的格式一致）。
    fn write_result_state(transaction: &Path, status: &str) {
        fs::write(
            transaction.join("result.json"),
            serde_json::to_vec(&serde_json::json!({"status":status,"message":"fixture"})).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn macos_update_legacy_helper_leftover_is_cleaned() {
        // 2.1.2 的 helper 完成更新后留下的完整结构：plan / helper 副本 / stderr / 终态
        // result.json / decision(-uuid) / 空挂载点 / 空 staged / backups 里的旧 app / 空 rejected。
        let (_temp, plan, _payload) = completed_fixture();
        let root = plan.install_root.clone();
        write_result_state(&plan.transaction, "installed");
        assert!(plan.transaction.join("staged").is_dir());
        assert!(plan.transaction.join("rejected").is_dir());
        assert!(
            plan.transaction
                .join("backups")
                .join(BUNDLES[0].name)
                .is_dir()
        );
        // 新版刚启动：目录几秒前才改动过，helper 写了终态且已放开更新锁，马上就能清。
        let outcomes = cleanup_stale_transactions(&root, SystemTime::now()).unwrap();
        assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
        assert!(!plan.transaction.exists());
        assert!(
            root.join(BUNDLES[0].name)
                .join("Contents/Info.plist")
                .is_file()
        );

        // 用户在访达里点开过的旧残留（多了 .DS_Store），没有终态但早已闲置：照样清。
        let (_temp, plan, _payload) = completed_fixture();
        write_result_state(&plan.transaction, "ready");
        fs::write(plan.transaction.join(".DS_Store"), "finder").unwrap();
        let outcomes = cleanup_stale_transactions(&plan.install_root, later()).unwrap();
        assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
        assert!(!plan.transaction.exists());
    }

    #[test]
    fn macos_update_finished_transaction_cleaned_once_lock_is_free() {
        let (temp, plan, _payload) = completed_fixture();
        let record = transaction_record_path(&temp.path().join("state"));
        write_result_state(&plan.transaction, "installed_restart_failed");
        record_transaction(&record, &plan.transaction).unwrap();
        // helper 还没退出、仍持有更新锁：即使已写终态也不碰。
        let outcomes = cleanup_recorded_transactions(&record, SystemTime::now(), |_| None::<()>);
        assert!(outcomes[0].1.as_ref().unwrap_err().contains("正在进行"));
        assert!(plan.transaction.exists());
        assert_eq!(
            read_transaction_record(&record),
            vec![plan.transaction.clone()]
        );
        // helper 退出、锁放开后，不必等闲置期。
        let outcomes = cleanup_recorded_transactions(&record, SystemTime::now(), |_| Some(()));
        assert!(outcomes[0].1.is_ok(), "{outcomes:?}");
        assert!(!plan.transaction.exists());
        assert!(!record.exists());
    }

    #[test]
    fn macos_update_finished_state_does_not_bypass_safety_checks() {
        let now = SystemTime::now();
        let keep = |prepare: &dyn Fn(&UpdatePlan), expected: &str| {
            let (_temp, plan, _payload) = completed_fixture();
            prepare(&plan);
            let outcomes = cleanup_stale_transactions(&plan.install_root, now).unwrap();
            let message = outcomes[0].1.as_ref().unwrap_err();
            assert!(message.contains(expected), "{expected}: {message}");
            assert!(plan.transaction.is_dir());
        };
        // 没有终态（helper 还在准备 / 刚建好还没拿锁）：仍按闲置期跳过。
        keep(
            &|plan| write_result_state(&plan.transaction, "ready"),
            "最近",
        );
        keep(&|_| {}, "最近");
        // 终态文件是指向别处的符号链接：不认，回落到闲置期判断。
        #[cfg(unix)]
        keep(
            &|plan| {
                let fake = plan.install_root.parent().unwrap().join("伪造终态.json");
                fs::write(&fake, r#"{"status":"installed"}"#).unwrap();
                fs::remove_file(plan.transaction.join("result.json")).unwrap();
                std::os::unix::fs::symlink(&fake, plan.transaction.join("result.json")).unwrap();
            },
            "最近",
        );
        // 已写终态，但结构校验不通过：照样保留。
        keep(
            &|plan| {
                write_result_state(&plan.transaction, "installed");
                fs::write(plan.transaction.join("用户笔记.txt"), "not ours").unwrap();
            },
            "非更新暂存内容",
        );
        keep(
            &|plan| {
                write_result_state(&plan.transaction, "failed");
                fs::write(plan.transaction.join("mount").join("卷内容"), "dmg").unwrap();
            },
            "挂载点非空",
        );
        keep(
            &|plan| {
                write_result_state(&plan.transaction, "failed");
                fs::rename(
                    plan.install_root.join(BUNDLES[0].name),
                    plan.transaction.join("rejected").join(BUNDLES[0].name),
                )
                .unwrap();
            },
            "备份需保留",
        );
        keep(
            &|plan| {
                write_result_state(&plan.transaction, "installed");
                fs::create_dir(plan.transaction.join(".DS_Store")).unwrap();
            },
            "类型异常",
        );
        #[cfg(unix)]
        keep(
            &|plan| {
                use std::os::unix::fs::PermissionsExt;
                write_result_state(&plan.transaction, "installed");
                fs::set_permissions(&plan.transaction, fs::Permissions::from_mode(0o755)).unwrap();
            },
            "私有目录",
        );
    }

    #[test]
    fn macos_update_startup_cleanup_retries_only_while_pending() {
        let run = |results: &[bool]| {
            let mut slept = Vec::new();
            let mut calls = 0;
            run_cleanup_with_retries(
                &STARTUP_CLEANUP_RETRY_DELAYS,
                |delay| slept.push(delay),
                || {
                    calls += 1;
                    results[calls - 1]
                },
            );
            (calls, slept)
        };
        assert_eq!(run(&[false]), (1, vec![]));
        assert_eq!(
            run(&[true, false]),
            (2, vec![STARTUP_CLEANUP_RETRY_DELAYS[0]])
        );
        // 一直有留待下次的：补跑次数有上限，不常驻。
        assert_eq!(
            run(&[true, true, true]),
            (3, STARTUP_CLEANUP_RETRY_DELAYS.to_vec())
        );
        // 最后一次补跑落在闲置期之后，没写终态的残留也能在本次运行中收掉。
        assert!(STARTUP_CLEANUP_RETRY_DELAYS.iter().sum::<Duration>() > STALE_TRANSACTION_MIN_AGE);
    }

    #[test]
    fn macos_update_corrupt_record_is_treated_as_empty() {
        let temp = tempfile::tempdir().unwrap();
        let record = transaction_record_path(temp.path());
        fs::write(&record, "not json").unwrap();
        assert!(read_transaction_record(&record).is_empty());
        let transaction = temp.path().join(transaction_dir_name(uuid::Uuid::new_v4()));
        record_transaction(&record, &transaction).unwrap();
        assert_eq!(read_transaction_record(&record), vec![transaction]);
    }

    #[test]
    fn macos_update_legacy_scan_only_in_standard_applications_dirs() {
        let home = Path::new("/Users/me");
        assert!(legacy_transaction_scan_allowed(
            Path::new("/Applications"),
            Some(home)
        ));
        assert!(legacy_transaction_scan_allowed(
            Path::new("/Applications"),
            None
        ));
        assert!(legacy_transaction_scan_allowed(
            Path::new("/Users/me/Applications"),
            Some(home)
        ));
        for root in [
            "/Users/me/Downloads",
            "/Users/me/Desktop",
            "/Users/me/Documents",
            "/Users/me/Applications/Tools",
            "/Applications/Utilities",
            "/Users/other/Applications",
            "/",
        ] {
            assert!(
                !legacy_transaction_scan_allowed(Path::new(root), Some(home)),
                "{root}"
            );
        }
        assert!(!legacy_transaction_scan_allowed(
            Path::new("/Users/me/Applications"),
            None
        ));
    }

    #[test]
    fn macos_update_transaction_removal_rejects_paths_outside_install_root() {
        let (temp, plan, _payload) = completed_fixture();
        // 父目录必须恰好是安装目录：子目录里、其它目录里、相对路径都拒绝。
        let nested_root = plan.install_root.join("nested");
        fs::create_dir(&nested_root).unwrap();
        let nested = nested_root.join(transaction_dir_name(uuid::Uuid::new_v4()));
        create_private_dir(&nested);
        for (root, transaction) in [
            (plan.install_root.clone(), nested.clone()),
            (temp.path().to_path_buf(), plan.transaction.clone()),
            (
                plan.install_root.clone(),
                plan.install_root
                    .join("nested/..")
                    .join(plan.transaction.file_name().unwrap()),
            ),
        ] {
            assert!(remove_transaction(&root, &transaction).is_err());
        }
        assert!(nested.exists());
        assert!(plan.transaction.join("backups").exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_controlled_pipe_fixture() {
        match std::env::var("CODEX_PLUS_UPDATE_PIPE_FIXTURE").as_deref() {
            Ok("parent") => {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "update::macos::tests::macos_update_controlled_pipe_fixture",
                        "--nocapture",
                    ])
                    .env("CODEX_PLUS_UPDATE_PIPE_FIXTURE", "hold")
                    .stdout(std::process::Stdio::inherit())
                    .stderr(std::process::Stdio::inherit())
                    .spawn()
                    .unwrap();
                std::process::exit(0);
            }
            Ok("hold") => {
                std::thread::sleep(std::time::Duration::from_millis(900));
                std::fs::write(
                    std::env::var_os("CODEX_PLUS_UPDATE_PIPE_DONE").unwrap(),
                    "done",
                )
                .unwrap();
            }
            _ => {}
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_pipe_deadline_is_bounded_when_own_fixture_inherits_pipe() {
        let temp = tempfile::tempdir().unwrap();
        runtime::pipe_deadline_smoke(temp.path()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn install_lock_file_is_removed_only_while_held() {
        let temp = tempfile::tempdir().unwrap();
        runtime::install_lock_cleanup_smoke(temp.path()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_detached_signed_helper_executes_without_modifying_source_app() {
        let temp = tempfile::tempdir().unwrap();
        runtime::detached_helper_smoke(temp.path()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_native_signed_bundle_copy_smoke_uses_only_temp_fixture() {
        let temp = tempfile::tempdir().unwrap();
        runtime::native_smoke(temp.path()).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_real_native_dmg_installs_single_bundle_in_temporary_root() {
        let temp = tempfile::tempdir().unwrap();
        runtime::native_dmg_transaction(temp.path(), false).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_update_real_native_dmg_rolls_back_single_bundle_on_replacement_failure() {
        let temp = tempfile::tempdir().unwrap();
        runtime::native_dmg_transaction(temp.path(), true).unwrap();
    }
}
