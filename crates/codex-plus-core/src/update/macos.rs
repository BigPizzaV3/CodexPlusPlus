//! macOS 更新在同卷私有目录中准备完整 app，再用 rename 事务替换；不修改包内文件。
use std::fs;
use std::path::{Path, PathBuf};

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
    if !expected
        .to_string_lossy()
        .eq_ignore_ascii_case(&executable.to_string_lossy())
    {
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
        let transaction = root.join(format!(".codex-plus-update-{}", uuid::Uuid::new_v4()));
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
        let helper = plan.transaction.join("manager-update-helper");
        fs::copy(&executable, &helper)?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        // 从已签名 app 直接复制裸 Mach-O 会保留依赖原 Info.plist 的签名，
        // 脱离 bundle 后 macOS 会立即拒绝执行。临时 helper 只负责本机更新，
        // 用 ad-hoc 签名重新封装它；正式 app 的 Developer ID 签名不受影响。
        checked(
            "/usr/bin/codesign",
            &[
                "--force".as_ref(),
                "--sign".as_ref(),
                "-".as_ref(),
                helper.as_os_str(),
            ],
        )?;
        let path = plan.transaction.join("plan.json");
        use std::io::Write;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(&serde_json::to_vec(&plan)?)?;
        file.sync_all()?;
        let mut child = Command::new(helper)
            .arg("--apply-codex-plus-update")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
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
            if child.try_wait()?.is_some() {
                bail!("更新 helper 提前退出，旧版未替换。");
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
        let result = (|| -> anyhow::Result<()> {
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
        match result {
            Ok(()) => {
                write_state(
                    &plan,
                    "installed",
                    "Codex++.app 已安装；旧 app 保留在事务 backups 目录",
                )?;
                let restarted = (|| -> anyhow::Result<()> {
                    open_bundle(&plan.install_root.join(BUNDLES[0].name))?;
                    Ok(())
                })();
                if let Err(error) = restarted {
                    let _ = write_state(
                        &plan,
                        "installed_restart_failed",
                        &format!("安装已完成但重新启动失败，旧 app 备份已保留：{error}"),
                    );
                    return Err(error);
                }
                Ok(())
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
        let transaction = root.join(".codex-plus-update-fixture");
        fs::create_dir(&transaction).unwrap();
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
        assert_eq!(
            installed_root_from_executable(Path::new(
                "/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus"
            ))
            .unwrap(),
            PathBuf::from("/Applications")
        );
        assert_eq!(
            installed_root_from_executable(Path::new(
                "/Applications/codex++.app/Contents/MacOS/CodexPlusPlus"
            ))
            .unwrap(),
            PathBuf::from("/Applications")
        );
        for executable in ["/Applications/Codex++ 管理工具.app/Contents/MacOS/CodexPlusPlusManager"]
        {
            let error = installed_root_from_executable(Path::new(executable)).unwrap_err();
            assert!(error.to_string().contains("手动迁移"));
            assert!(
                error
                    .to_string()
                    .contains("旧版自动更新器不支持单应用安装包")
            );
        }
        let renamed = installed_root_from_executable(Path::new(
            "/Applications/Codex++ 2.app/Contents/MacOS/CodexPlusPlus",
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
