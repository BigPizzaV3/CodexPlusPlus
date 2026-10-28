//! Windows 上「以 MSIX 包上下文启动一个新的 Codex 桌面实例」的启动机制。
//!
//! # 为什么必须留在包上下文里
//!
//! 官方 Windows 版 Codex / ChatGPT 桌面是 MSIX 打包应用，可执行文件位于
//! `…\WindowsApps\OpenAI.Codex_<版本>_x64__<publisher>\app\ChatGPT.exe`。这类进程的
//! 可用性依赖「包身份（package identity）」：包目录 ACL、虚拟化文件/注册表视图，以及
//! 运行时 `GetCurrentPackageFullName` 的返回，都要求进程属于某个已注册包。绕过 AUMID
//! 激活、直接 `CreateProcess` 包内 exe（launcher 现有按路径启动分支）时，新进程没有包
//! 身份：`GetCurrentPackageFullName` 返回 `APPMODEL_ERROR_NO_PACKAGE`(15700)，包目录读取
//! 被 ACL 拒绝（Access denied），实例根本起不来。
//!
//! 多开又不能直接用「AUMID 激活」：`ActivateApplication` 只会把已经存在（或共享用户数据）
//! 的主实例拉到前台，起不出彼此隔离的第二份。所以这里走「包上下文里先起 cmd.exe，再由它
//! 拉起 exe」：`Invoke-CommandInDesktopPackage -PreventBreakaway` 让被创建进程**及其所有
//! 子进程**都带上该 AppId 的包身份，`cmd.exe` 里 `start` 出来的 Codex 因而仍有包身份。
//!
//! # 依赖的是非契约接口
//!
//! `Invoke-CommandInDesktopPackage` 属于 Appx 模块（<https://learn.microsoft.com/powershell/module/appx/invoke-commandindesktoppackage>），
//! 微软文档把它定义为**调试工具**，并列了三条限制：
//! - 新进程**不会**被放进 AppContainer；
//! - 它的令牌「similar to, but not identical to, a real AppId process」；
//! - 「No guarantees are made about the behavior of the created process」「You shouldn't rely
//!   on any specific side-effects of using this command, as they're undefined and subject to
//!   change」。
//!
//! 也就是说它不是受支持的启动契约，Windows 更新后参数或行为都可能变；这条路径只能在实机上
//! 验证，不能当成「确定可用」。因此本模块的定位是：**只负责包上下文这一条路径**，并把
//! 「失败怎么办」交给调用方，形成明确的 fallback 策略：
//! - [`activate_in_package_context`] 失败（命令报错 / 超时 / 非 Windows）→ 调用方退回
//!   [`fallback_direct_launch`]，保留现有按路径启动行为，至少不比现状更差；
//! - 激活成功但 [`wait_for_instance_ready`] 超时 → 同样允许退回按路径启动，但调用方必须先
//!   确认没有别的实例已经占住该调试端口，否则会双开。
//!
//! # 平台约定
//!
//! 包上下文是 MSIX 专有机制，因此 [`activate_in_package_context`] 在非 Windows 平台**返回
//! Err**（而不是等价 no-op）。no-op 会让调用方误判「包上下文激活成功」，把失败推迟到就绪
//! 探测超时之后才暴露。其余函数（wrapper 拼装、就绪探测、按路径兜底启动）都是平台无关的。
//!
//! # 必须实机确认的点（本机 macOS 无法验证）
//!
//! 1. `Invoke-CommandInDesktopPackage` 在目标 Windows 版本上是否存在、是否需要显式导入
//!    Appx 模块、是否要求非提权上下文（文档提到提权时行为不符合预期）；
//! 2. `-Args '/d /c "<wrapper>"'` 里的引号能否被 cmd.exe 逐字收到（尤其路径含空格时）；
//! 3. cmdlet 是否返回带 `Id` 的对象——决定 [`activate_in_package_context`] 返回的 PID 是否
//!    可用；返回 [`UNKNOWN_PROCESS_ID`] 说明没报回；
//! 4. `start /b` 出来的子进程在 cmd.exe 退出后是否继续存活（控制台生命周期）；
//! 5. 含非 ASCII 的路径（中文用户名）在 cmd 按 OEM 代码页解析 .cmd 时是否仍然正确。
//!
//! 以上任何一条都不要当成已验证；`cargo test` 只覆盖纯函数与 CDP 探测这两块。

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};

use crate::paths::STATE_DIR_ENV;

/// 包上下文 wrapper 的文件名（写在实例状态目录里）。
pub const WRAPPER_FILE_NAME: &str = "packaged-launch.cmd";

/// `Invoke-CommandInDesktopPackage` 没有报回进程 PID 时的返回值。
///
/// 该 cmdlet 的返回值不是文档化契约（见模块头），调用方不能把它当成 Codex 应用进程：
/// 真正的应用进程要用「谁在监听调试端口」去发现（launcher 已有 `capture_debug_listener`）。
pub const UNKNOWN_PROCESS_ID: u32 = 0;

/// 激活命令的整体超时。powershell 正常是秒级返回；卡死说明环境异常，交给 fallback。
#[cfg(windows)]
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);

/// CDP 就绪轮询间隔。
const READY_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// 包上下文启动的结果。
///
/// 本模块不持有全局状态，字段由调用方组装/读取。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackagedContextLaunch {
    /// 实际使用的 AUMID（`<PackageFamilyName>!<AppId>`）。
    pub app_user_model_id: String,
    /// 写在实例目录里的 wrapper 脚本路径。
    pub wrapper_path: PathBuf,
    /// 激活时 cmdlet 报回的 PID；报不回来时是 `Some`/`None` 由调用方决定如何填，
    /// 见 [`PackagedContextLaunch::known_process_id`] 与 [`UNKNOWN_PROCESS_ID`]。
    pub process_id: Option<u32>,
}

impl PackagedContextLaunch {
    /// 只有拿到真实 PID 时才返回 `Some`：用于过滤 cmdlet 未报回时的哨兵值。
    pub fn known_process_id(&self) -> Option<u32> {
        self.process_id.filter(|id| *id != UNKNOWN_PROCESS_ID)
    }
}

/// 拆出 AUMID 里的 PackageFamilyName 与 Application Id。
///
/// AUMID 的形状是 `<PackageFamilyName>!<AppId>`（见 `app_paths::packaged_app_user_model_id`）。
/// 这里只做形状校验，不查系统里的注册状态——「包是否已注册」由调用方在解析 AUMID 时确认
/// （`app_paths::packaged_app_user_model_id` 会查 `GetPackagesByPackageFamily`，拿未注册包的
/// AUMID 去激活会报 0x80073CF1，见 issue #2140）。
pub fn split_app_user_model_id(app_user_model_id: &str) -> anyhow::Result<(String, String)> {
    let trimmed = app_user_model_id.trim();
    let Some((package_family_name, app_id)) = trimmed.split_once('!') else {
        bail!("AUMID 缺少 `!` 分隔符（期望 `<PackageFamilyName>!<AppId>`）：{trimmed}");
    };
    if package_family_name.is_empty() || app_id.is_empty() || app_id.contains('!') {
        bail!("AUMID 形状不合法（期望 `<PackageFamilyName>!<AppId>`）：{trimmed}");
    }
    Ok((package_family_name.to_string(), app_id.to_string()))
}

/// 挑选 wrapper 要用的回环调试端口。
///
/// 顺序（与 `ports::select_packaged_codex_debug_port` 的判据一致：能绑定，或该端口上已经是
/// Codex 自己的 CDP 端点）：
/// 1. `preferred` 非 0 且可用 → 用它。调用方的选择是权威的，脚本端口必须与调用方注入的端口
///    一致，否则注入会打到别的端口上；
/// 2. 否则按 `known_ports` 顺序取第一个可用端口；
/// 3. 再否则让系统分配一个空闲回环端口。
///
/// 返回 `None` 只代表连系统分配都失败（正常情况下不会发生）。
pub fn select_debug_port(preferred: u16, known_ports: &[u16]) -> Option<u16> {
    select_debug_port_with(
        preferred,
        known_ports,
        crate::ports::can_bind_loopback_port,
        crate::cdp::endpoint_available,
        crate::ports::find_available_loopback_port,
    )
}

/// [`select_debug_port`] 的核心。探测函数以闭包注入，便于在 macOS 上确定性地单测
/// （与 `ports::select_packaged_codex_debug_port_with` 同一风格），也避免测试真的占端口。
fn select_debug_port_with(
    preferred: u16,
    known_ports: &[u16],
    can_bind: impl Fn(u16) -> bool,
    is_existing_cdp: impl Fn(u16) -> bool,
    find_available: impl Fn() -> u16,
) -> Option<u16> {
    if preferred != 0 && (can_bind(preferred) || is_existing_cdp(preferred)) {
        return Some(preferred);
    }
    for port in known_ports.iter().copied() {
        if port == 0 || port == preferred {
            continue;
        }
        if can_bind(port) {
            return Some(port);
        }
    }
    let fallback = find_available();
    (fallback != 0).then_some(fallback)
}

/// 生成包上下文 wrapper 脚本内容（纯函数，可在 macOS 上单测）。
///
/// 脚本由 `cmd.exe /d /c "<wrapper>"` 在包上下文里执行，负责：
/// 1. 设置实例环境变量 `CODEX_HOME` / `CODEX_SQLITE_HOME` / `CODEX_PLUS_STATE_DIR` /
///    `CODEX_PLUS_MULTI_INSTANCE=1`（与多开 worker 的实例身份一致，见 `paths::STATE_DIR_ENV`）；
/// 2. `start "" /b "<exe>"` 拉起包内 exe，并带上调试端口与实例用户数据目录。
///
/// 只有路径与端口会进脚本，不放任何凭据；脚本一律 CRLF——`.cmd` 用 LF 时 cmd.exe 的行为
/// 不可靠（尤其 `start` 一行）。
///
/// `known_ports` 的语义见 [`select_debug_port`]。注意调用方若需要知道实际端口，应先用
/// [`select_debug_port`] 算好再传进来（本函数内部会重新校验一次，极端竞争下可能改选，
/// 那时脚本内的端口才是权威）。
pub fn build_wrapper_script(
    state_dir: &Path,
    executable: &Path,
    codex_home: &Path,
    sqlite_home: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    known_ports: &[u16],
) -> anyhow::Result<String> {
    let port = select_debug_port(debug_port, known_ports).ok_or_else(|| {
        anyhow::anyhow!("找不到可用的回环调试端口（请求 {debug_port}，候选 {known_ports:?}）")
    })?;

    build_wrapper_script_for_port(
        state_dir,
        executable,
        codex_home,
        sqlite_home,
        user_data_dir,
        port,
    )
}

fn build_wrapper_script_for_port(
    state_dir: &Path,
    executable: &Path,
    codex_home: &Path,
    sqlite_home: &Path,
    user_data_dir: &Path,
    port: u16,
) -> anyhow::Result<String> {
    let state_dir_literal = batch_path_literal(state_dir)?;
    let executable_literal = batch_path_literal(executable)?;
    let codex_home_literal = batch_path_literal(codex_home)?;
    let sqlite_home_literal = batch_path_literal(sqlite_home)?;
    let user_data_dir_literal = batch_path_literal(user_data_dir)?;
    let executable_dir = executable
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(batch_path_literal)
        .transpose()?;

    let mut script = String::new();
    script.push_str("@echo off\r\n");
    // 只 setlocal，不开 delayedexpansion：路径里的 `!` 在 delayedexpansion 下会被吃掉。
    script.push_str("setlocal\r\n");
    script.push_str(&format!("set \"CODEX_HOME={codex_home_literal}\"\r\n"));
    script.push_str(&format!(
        "set \"CODEX_SQLITE_HOME={sqlite_home_literal}\"\r\n"
    ));
    script.push_str(&format!("set \"{STATE_DIR_ENV}={state_dir_literal}\"\r\n"));
    script.push_str("set \"CODEX_PLUS_MULTI_INSTANCE=1\"\r\n");
    if let Some(dir) = executable_dir {
        // 尽力把工作目录设到包内 exe 所在目录：Electron 会用相对 cwd 找资源。
        // 失败（ACL 拒绝）时 cmd 只打印错误并继续下一行，不影响启动。
        script.push_str(&format!("cd /d \"{dir}\"\r\n"));
    }
    // `start` 的第一个参数是窗口标题，必须是空标题，否则含空格的 exe 路径会被当成标题。
    // `/b` 不新建控制台窗口；`start` 立即返回，cmd 退出后应用继续跑。
    script.push_str(&format!(
        "start \"\" /b \"{executable_literal}\" --remote-debugging-port={port} \
         --remote-allow-origins=http://127.0.0.1:{port} --user-data-dir=\"{user_data_dir_literal}\"\r\n"
    ));
    script.push_str("endlocal\r\n");
    Ok(script)
}

/// 把 wrapper 内容写进 `<state_dir>/packaged-launch.cmd`，返回脚本路径。
pub fn write_wrapper_script(state_dir: &Path, script: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("创建实例状态目录失败：{}", state_dir.display()))?;
    let path = state_dir.join(WRAPPER_FILE_NAME);
    std::fs::write(&path, script.as_bytes())
        .with_context(|| format!("写入包上下文 wrapper 失败：{}", path.display()))?;
    Ok(path)
}

/// [`build_wrapper_script`] + [`write_wrapper_script`] 的组合，调用方最常用的一步。
pub fn prepare_wrapper_script(
    state_dir: &Path,
    executable: &Path,
    codex_home: &Path,
    sqlite_home: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    known_ports: &[u16],
) -> anyhow::Result<PathBuf> {
    Ok(prepare_wrapper_script_with_port(
        state_dir,
        executable,
        codex_home,
        sqlite_home,
        user_data_dir,
        debug_port,
        known_ports,
    )?
    .path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedWrapper {
    pub path: PathBuf,
    pub debug_port: u16,
}

/// 端口只选择一次，并与写入的脚本一起返回；调用方必须传播这个最终端口。
pub fn prepare_wrapper_script_with_port(
    state_dir: &Path,
    executable: &Path,
    codex_home: &Path,
    sqlite_home: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    known_ports: &[u16],
) -> anyhow::Result<PreparedWrapper> {
    // 新实例不能复用一个已经监听的 CDP；那可能属于另一实例。
    let debug_port = select_debug_port_with(
        debug_port,
        known_ports,
        crate::ports::can_bind_loopback_port,
        |_| false,
        || crate::ports::find_available_loopback_port(),
    )
    .context("找不到可用的回环调试端口")?;
    let script = build_wrapper_script_for_port(
        state_dir,
        executable,
        codex_home,
        sqlite_home,
        user_data_dir,
        debug_port,
    )?;
    Ok(PreparedWrapper {
        path: write_wrapper_script(state_dir, &script)?,
        debug_port,
    })
}

/// 主实例包上下文 wrapper 的文件名（与多开的 [`WRAPPER_FILE_NAME`] 分开，互不覆盖）。
pub const MAIN_WRAPPER_FILE_NAME: &str = "packaged-launch-main.cmd";

/// 生成主实例的包上下文 wrapper 脚本（纯函数，可在 macOS 上单测）。
///
/// 用于 AUMID 激活失败后的兜底（issue #2700：Codex 26.1021 起，带调试参数的 AUMID
/// 激活会让进程立即退出）。与多开 wrapper 的区别：
/// - 不设置 `CODEX_HOME` / `CODEX_PLUS_STATE_DIR` / `CODEX_PLUS_MULTI_INSTANCE`，主实例沿用
///   默认数据目录与实例身份；
/// - 参数原样取自 AUMID 激活要用的那份（调试端口 + 用户附加参数），保证两条路径启动出的
///   实例一致。
pub fn build_main_instance_wrapper_script(
    executable: &Path,
    args: &[String],
) -> anyhow::Result<String> {
    let executable_literal = batch_path_literal(executable)?;
    let executable_dir = executable
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(batch_path_literal)
        .transpose()?;
    let arguments = args
        .iter()
        .map(|arg| batch_argument_literal(arg))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let mut script = String::new();
    script.push_str("@echo off\r\n");
    script.push_str("setlocal\r\n");
    if let Some(dir) = executable_dir {
        // 与多开 wrapper 一致：尽力切到 exe 所在目录，失败时 cmd 继续执行下一行。
        script.push_str(&format!("cd /d \"{dir}\"\r\n"));
    }
    let mut start_line = format!("start \"\" /b \"{executable_literal}\"");
    for argument in &arguments {
        start_line.push(' ');
        start_line.push_str(argument);
    }
    script.push_str(&start_line);
    script.push_str("\r\n");
    script.push_str("endlocal\r\n");
    Ok(script)
}

/// [`build_main_instance_wrapper_script`] + 写入 `<state_dir>/packaged-launch-main.cmd`。
pub fn prepare_main_instance_wrapper(
    state_dir: &Path,
    executable: &Path,
    args: &[String],
) -> anyhow::Result<PathBuf> {
    let script = build_main_instance_wrapper_script(executable, args)?;
    std::fs::create_dir_all(state_dir)
        .with_context(|| format!("创建状态目录失败：{}", state_dir.display()))?;
    let path = state_dir.join(MAIN_WRAPPER_FILE_NAME);
    std::fs::write(&path, script.as_bytes())
        .with_context(|| format!("写入主实例包上下文 wrapper 失败：{}", path.display()))?;
    Ok(path)
}

/// 单个命令行参数写进 `.cmd` 的字面量。
///
/// `%` 写成 `%%`；含空白或 cmd 元字符（`& | < > ^ ( )`）时整体加双引号，避免被 cmd 当成
/// 管道/重定向。参数里的双引号与换行无法安全转义，直接拒绝。
fn batch_argument_literal(arg: &str) -> anyhow::Result<String> {
    if arg.contains('"') || arg.contains('\r') || arg.contains('\n') || arg.contains('\0') {
        bail!("启动参数包含无法写进启动脚本的字符（引号或换行）：{arg}");
    }
    let escaped = arg.replace('%', "%%");
    let needs_quotes = escaped.is_empty()
        || escaped
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '&' | '|' | '<' | '>' | '^' | '(' | ')'));
    Ok(if needs_quotes {
        format!("\"{escaped}\"")
    } else {
        escaped
    })
}

/// AUMID 激活后的验证窗口：窗口内既没开放调试端口、激活进程也一直活着，就不再判定，
/// 交回原有注入流程（保持旧行为）。
pub const AUMID_ACTIVATION_VERIFY_WINDOW: Duration = Duration::from_secs(15);

/// 激活进程退出后，再给调试端口这么久的出现时间才判定激活失败：
/// 防止「中转进程退出、真正进程稍后才监听」被误判。
pub const AUMID_ACTIVATION_EXIT_GRACE: Duration = Duration::from_secs(2);

/// AUMID 激活后单次观察的结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AumidActivationVerdict {
    /// 调试端口已就绪，激活成功。
    Ready,
    /// 继续观察。
    Pending,
    /// 激活进程已退出且调试端口始终没出现：改用包上下文 wrapper 启动（issue #2700）。
    FallbackToPackageContext,
    /// 窗口耗尽但进程仍在（或无法判断）：不回退，交回原有注入流程，避免双开。
    Inconclusive,
}

/// AUMID 激活结果的判定器（纯逻辑，时间由调用方注入，可在任意平台单测）。
///
/// `process_alive` 为 `None` 表示这一轮无法判断激活进程是否存活；无法判断时绝不回退，
/// 只按窗口耗尽处理。
#[derive(Debug, Clone)]
pub struct AumidActivationWatch {
    started_at: Instant,
    exited_since: Option<Instant>,
}

impl AumidActivationWatch {
    pub fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            exited_since: None,
        }
    }

    pub fn observe(
        &mut self,
        now: Instant,
        cdp_ready: bool,
        process_alive: Option<bool>,
    ) -> AumidActivationVerdict {
        if cdp_ready {
            return AumidActivationVerdict::Ready;
        }
        match process_alive {
            Some(false) => {
                let since = *self.exited_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= AUMID_ACTIVATION_EXIT_GRACE {
                    return AumidActivationVerdict::FallbackToPackageContext;
                }
                // 已确认退出时不受总窗口限制，等退出宽限走完再给结论。
                return AumidActivationVerdict::Pending;
            }
            Some(true) => self.exited_since = None,
            None => {}
        }
        if now.saturating_duration_since(self.started_at) >= AUMID_ACTIVATION_VERIFY_WINDOW {
            AumidActivationVerdict::Inconclusive
        } else {
            AumidActivationVerdict::Pending
        }
    }
}

/// 拼出在包上下文里执行 wrapper 的 PowerShell 脚本（纯函数，可在 macOS 上单测）。
///
/// 形状是
/// `Invoke-CommandInDesktopPackage -PackageFamilyName '<PFN>' -AppId '<AppId>' -Command 'cmd.exe'
/// -Args '/d /c "<wrapper>"' -PreventBreakaway`；与社区配方相比只多了两处：
/// 前置 `$ErrorActionPreference = 'Stop'`（失败要变成非零退出码，否则会被当成激活成功）、
/// 以及仅在 cmdlet 真报回 `Id` 时输出 `PID=` 标记。所有动态值都走 PowerShell 单引号字面量，
/// 内部的 `'` 按 `''` 转义。
///
/// 只做形状校验与转义，不查注册表、不碰系统状态。
pub fn activation_script(app_user_model_id: &str, wrapper_path: &Path) -> anyhow::Result<String> {
    let (package_family_name, app_id) = split_app_user_model_id(app_user_model_id)?;
    activation_script_for(&package_family_name, &app_id, wrapper_path)
}

fn activation_script_for(
    package_family_name: &str,
    app_id: &str,
    wrapper_path: &Path,
) -> anyhow::Result<String> {
    let package_family_name = powershell_literal(package_family_name)?;
    let app_id = powershell_literal(app_id)?;
    let wrapper = powershell_literal(&path_text(wrapper_path)?)?;

    let mut script = String::new();
    // cmdlet 报错时让 powershell.exe 以非零码退出；否则「激活失败」会被当成成功，调用方
    // 就不会走 fallback，用户侧表现是点了没反应。
    script.push_str("$ErrorActionPreference = 'Stop'; ");
    // 返回值不是文档化契约（见模块头）：只有真报回带 Id 的对象时才输出 PID 标记，
    // 解析侧只认这个前缀，避免把无关数字当成 PID。
    script.push_str("$created = Invoke-CommandInDesktopPackage ");
    script.push_str(&format!(
        "-PackageFamilyName '{package_family_name}' -AppId '{app_id}' "
    ));
    script.push_str("-Command 'cmd.exe' ");
    script.push_str(&format!("-Args '/d /c \"{wrapper}\"' "));
    script.push_str("-PreventBreakaway; ");
    script.push_str("if ($created -and $created.Id) { \"PID=$($created.Id)\" }");
    Ok(script)
}

/// `powershell.exe` 的参数（`-Command` 之后整段脚本作为一个参数传入，避免二次引用解析）。
#[cfg(any(windows, test))]
fn activation_powershell_arguments(script: &str) -> Vec<String> {
    vec![
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-Command".to_string(),
        script.to_string(),
    ]
}

/// 在 MSIX 包上下文里执行 wrapper，返回激活时拿到的进程 PID。
///
/// Windows 实现调用
/// `powershell.exe -NoProfile -NonInteractive -Command "Invoke-CommandInDesktopPackage …"`；
/// 非 Windows 平台返回 Err（原因见模块头的「平台约定」）。
///
/// 返回值：cmdlet 报回了 `Id` 时是它的 PID，否则是 [`UNKNOWN_PROCESS_ID`]。**不要**把它当成
/// Codex 应用进程：cmdlet 创建的是包上下文里的 `cmd.exe`，它启动应用后很快退出，应用进程要
/// 按调试端口归属去发现。也不要因为它不可用就判激活失败——真正的判据是
/// [`wait_for_instance_ready`]。
pub async fn activate_in_package_context(
    app_user_model_id: &str,
    wrapper_path: &Path,
) -> anyhow::Result<u32> {
    activate_in_package_context_impl(app_user_model_id, wrapper_path).await
}

#[cfg(windows)]
async fn activate_in_package_context_impl(
    app_user_model_id: &str,
    wrapper_path: &Path,
) -> anyhow::Result<u32> {
    let (package_family_name, app_id) = split_app_user_model_id(app_user_model_id)?;
    let wrapper_path = wrapper_path.to_path_buf();
    if !wrapper_path.is_file() {
        bail!(
            "包上下文 wrapper 不存在，无法激活：{}",
            wrapper_path.display()
        );
    }

    let activation = tokio::task::spawn_blocking(move || {
        activate_in_package_context_blocking(&package_family_name, &app_id, &wrapper_path)
    });
    tokio::time::timeout(ACTIVATION_TIMEOUT, activation)
        .await
        .with_context(|| {
            format!(
                "包上下文激活超时（{} 秒），已放弃本次激活",
                ACTIVATION_TIMEOUT.as_secs()
            )
        })?
        .context("包上下文激活任务异常退出")?
}

#[cfg(not(windows))]
async fn activate_in_package_context_impl(
    _app_user_model_id: &str,
    _wrapper_path: &Path,
) -> anyhow::Result<u32> {
    bail!("包上下文激活仅在 Windows 上可用（MSIX 包身份是 Windows 专有机制）")
}

#[cfg(windows)]
fn activate_in_package_context_blocking(
    package_family_name: &str,
    app_id: &str,
    wrapper_path: &Path,
) -> anyhow::Result<u32> {
    use std::os::windows::process::CommandExt;

    let script = activation_script_for(package_family_name, app_id, wrapper_path)?;
    let output = Command::new("powershell.exe")
        .args(activation_powershell_arguments(&script))
        .creation_flags(crate::windows_integration::CREATE_NO_WINDOW)
        .output()
        .context("启动 powershell.exe 执行包上下文激活失败")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = first_non_empty_line(&stderr).unwrap_or_else(|| "（无错误输出）".to_string());
        bail!(
            "Invoke-CommandInDesktopPackage 激活失败（退出码 {}）：{detail}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
    }

    Ok(
        parse_reported_process_id(&String::from_utf8_lossy(&output.stdout))
            .unwrap_or(UNKNOWN_PROCESS_ID),
    )
}

/// 该实例是否已就绪：CDP 端口能连上并返回 Codex 页面 == 启动成功。
///
/// 判据用 `cdp::endpoint_available`（要求 `/json` 里确实有 Codex 的页面目标），不是「端口
/// 被占用」——端口被无关进程占着不算就绪。`timeout` 为 0 表示只探一次。
///
/// 注意：每次探测本身最长会阻塞约 600ms（连接 + 读取各 300ms），所以总时长可能略微超过
/// `timeout`；这是同步阻塞函数，在 async 上下文里应放到 `spawn_blocking` 里调。
pub fn wait_for_instance_ready(debug_port: u16, timeout: Duration) -> bool {
    wait_for_instance_ready_with(
        debug_port,
        timeout,
        crate::cdp::endpoint_available,
        READY_POLL_INTERVAL,
    )
}

/// [`wait_for_instance_ready`] 的核心。探测与轮询间隔以参数注入，便于确定性地单测。
fn wait_for_instance_ready_with(
    debug_port: u16,
    timeout: Duration,
    is_ready: impl Fn(u16) -> bool,
    poll_interval: Duration,
) -> bool {
    if debug_port == 0 {
        return false;
    }
    let now = Instant::now();
    let Some(deadline) = now.checked_add(timeout) else {
        return is_ready(debug_port);
    };
    loop {
        if is_ready(debug_port) {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(poll_interval.min(deadline.saturating_duration_since(now)));
    }
}

/// 兜底：包上下文激活失败或就绪超时后，退回按路径直接启动（保留 launcher 现有行为）。
///
/// 返回子进程 PID；`Err` 表示连兜底都起不来，调用方必须如实报错，不能静默成功。
///
/// 环境变量与 wrapper 保持一致（`CODEX_PLUS_STATE_DIR` + `CODEX_PLUS_MULTI_INSTANCE=1`）：
/// 两条路径如果实例身份不一致，兜底会落到主实例的状态目录上——正是 `paths::STATE_DIR_ENV`
/// 文档里记录过的坑。注意按路径启动的进程没有 MSIX 包身份，在打包版上它很可能立刻退出，
/// 所以这只是「至少不比现状更差」的兜底，不是等价替代。
pub fn fallback_direct_launch(
    executable: &Path,
    args: &[String],
    state_dir: &Path,
) -> anyhow::Result<u32> {
    let child = direct_launch_command(executable, args, state_dir)
        .spawn()
        .with_context(|| format!("按路径启动 Codex 失败：{}", executable.display()))?;
    // 句柄直接丢弃：子进程继续跑（std 的 Child drop 不会杀进程），调用方按 PID 跟踪。
    Ok(child.id())
}

fn direct_launch_command(executable: &Path, args: &[String], state_dir: &Path) -> Command {
    let mut command = Command::new(executable);
    command
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .env(STATE_DIR_ENV, state_dir)
        .env("CODEX_PLUS_MULTI_INSTANCE", "1");
    if let Some(dir) = executable
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
    {
        command.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
    }
    command
}

/// 路径写进 `.cmd` 的字面量：`%` 必须写成 `%%`，否则 cmd 会当变量展开。
///
/// 双引号/换行是 Windows 路径的非法字符，出现即拒绝，避免脚本被注入（也避免赋值的引号
/// 被提前闭合）。`!` 不需要转义：脚本只 `setlocal`，没开 delayedexpansion。
fn batch_path_literal(path: &Path) -> anyhow::Result<String> {
    Ok(path_text(path)?.replace('%', "%%"))
}

/// 路径 → 字符串，并挡掉无法安全拼进脚本的字符。
fn path_text(path: &Path) -> anyhow::Result<String> {
    let text = path.to_string_lossy();
    if text.is_empty() {
        bail!("路径不能为空");
    }
    if text.contains('"') || text.contains('\r') || text.contains('\n') {
        bail!("路径包含无法写进启动脚本的字符（引号或换行）：{text}");
    }
    Ok(text.into_owned())
}

/// PowerShell 单引号字符串字面量：内部的 `'` 用 `''` 转义。
fn powershell_literal(value: &str) -> anyhow::Result<String> {
    if value.is_empty() {
        bail!("PowerShell 参数不能为空");
    }
    if value.contains('\r') || value.contains('\n') || value.contains('\0') {
        bail!("PowerShell 参数包含不可见控制字符，拒绝拼进命令：{value}");
    }
    Ok(value.replace('\'', "''"))
}

/// 只认 `PID=<数字>` 这一种输出，避免把 cmdlet 的其它数字输出误当成进程号。
#[cfg(any(windows, test))]
fn parse_reported_process_id(stdout: &str) -> Option<u32> {
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix("PID=")
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|id| *id != UNKNOWN_PROCESS_ID)
    })
}

#[cfg(any(windows, test))]
fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::{Read, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::thread;

    #[test]
    fn prepared_wrapper_receipt_matches_reselected_script_port() {
        let temp = tempfile::tempdir().unwrap();
        let busy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let preferred = busy.local_addr().unwrap().port();
        let wrapper = prepare_wrapper_script_with_port(
            temp.path(),
            Path::new("/app/ChatGPT.exe"),
            &temp.path().join("home"),
            &temp.path().join("sqlite"),
            &temp.path().join("user data"),
            preferred,
            &[],
        )
        .unwrap();
        assert_ne!(wrapper.debug_port, preferred);
        let script = std::fs::read_to_string(wrapper.path).unwrap();
        assert!(script.contains(&format!("--remote-debugging-port={}", wrapper.debug_port)));
        assert!(script.contains(&format!(
            "--remote-allow-origins=http://127.0.0.1:{}",
            wrapper.debug_port
        )));
        assert!(!script.contains(&format!("--remote-debugging-port={preferred}")));
    }

    const AUMID: &str = "OpenAI.Codex_2p2nqsd0c76g0!App";
    /// Windows 形式的路径只在字面量断言里用：macOS 上 `Path::parent()` 按 `/` 切分，
    /// 对 `C:\a\b.exe` 取不到目录，所以涉及 `cd /d` 的用例一律用 `/` 分隔的路径。
    const WINDOWS_EXE: &str =
        r"C:\Program Files\WindowsApps\OpenAI.Codex_1.2.3_x64__2p2nqsd0c76g0\app\ChatGPT.exe";

    /// 写死端口号会让测试在「该端口恰好被并行跑的其它测试/进程占着」时假失败，
    /// 所以每次现取一个空闲回环端口。
    fn free_port() -> u16 {
        let port = crate::ports::find_available_loopback_port();
        assert!(port > 0, "测试环境应能分配空闲回环端口");
        port
    }

    #[test]
    fn wrapper_script_carries_instance_identity_ports_and_user_data_dir() {
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("instance 1").join(".codex-session-delete");
        let executable = Path::new("/Applications/Codex App/Contents/app/ChatGPT.exe");
        let codex_home = Path::new("/Users/a b/.codex");
        let sqlite_home = Path::new("/Users/a b/.codex-sqlite");
        let user_data_dir = Path::new("/Users/a b/Codex++/instance 1/user-data");
        let port = free_port();

        let script = build_wrapper_script(
            &state_dir,
            executable,
            codex_home,
            sqlite_home,
            user_data_dir,
            port,
            &[],
        )
        .unwrap();

        assert!(script.starts_with("@echo off\r\n"));
        assert!(script.ends_with("endlocal\r\n"));
        assert!(script.contains("setlocal\r\n"));
        assert!(script.contains("set \"CODEX_HOME=/Users/a b/.codex\"\r\n"));
        assert!(script.contains("set \"CODEX_SQLITE_HOME=/Users/a b/.codex-sqlite\"\r\n"));
        assert!(script.contains(&format!(
            "set \"CODEX_PLUS_STATE_DIR={}\"\r\n",
            state_dir.display()
        )));
        assert!(script.contains("set \"CODEX_PLUS_MULTI_INSTANCE=1\"\r\n"));
        assert!(script.contains(&format!("--remote-debugging-port={port}")));
        assert!(script.contains(&format!("--remote-allow-origins=http://127.0.0.1:{port}")));
        assert!(script.contains("--user-data-dir=\"/Users/a b/Codex++/instance 1/user-data\""));
        // start 的第一个参数必须是空标题，否则含空格的 exe 路径会被当成窗口标题。
        assert!(script.contains(&format!(
            "start \"\" /b \"/Applications/Codex App/Contents/app/ChatGPT.exe\" \
             --remote-debugging-port={port}"
        )));
        assert!(script.contains("cd /d \"/Applications/Codex App/Contents/app\"\r\n"));
        // .cmd 一律 CRLF：LF 的批处理在 cmd.exe 上行为不可靠。
        // （不能用 `str::lines()`：它会顺手把 `\r` 吃掉。）
        assert!(
            script
                .split('\n')
                .all(|line| line.is_empty() || line.ends_with('\r'))
        );
    }

    #[test]
    fn wrapper_script_quotes_windows_style_paths_with_spaces() {
        let script = build_wrapper_script(
            Path::new(r"C:\Users\a b\.codex-session-delete"),
            Path::new(WINDOWS_EXE),
            Path::new(r"C:\Users\a b\.codex"),
            Path::new(r"C:\Users\a b\.codex-sqlite"),
            Path::new(r"C:\Users\a b\AppData\Local\Codex++\instance 1\user data"),
            free_port(),
            &[],
        )
        .unwrap();

        assert!(script.contains(&format!("start \"\" /b \"{WINDOWS_EXE}\"")));
        assert!(script.contains(
            "--user-data-dir=\"C:\\Users\\a b\\AppData\\Local\\Codex++\\instance 1\\user data\""
        ));
        assert!(script.contains("set \"CODEX_HOME=C:\\Users\\a b\\.codex\""));
    }

    #[test]
    fn wrapper_script_escapes_percent_signs_for_batch() {
        let script = build_wrapper_script(
            Path::new(r"C:\state\%TEMP%\1"),
            Path::new(r"C:\app\ChatGPT.exe"),
            Path::new(r"C:\codex 100%"),
            Path::new(r"C:\sqlite"),
            Path::new(r"C:\user data"),
            9401,
            &[],
        )
        .unwrap();

        // `%` 不转义会被 cmd 当成变量展开，实例路径就被改写了。
        assert!(script.contains("set \"CODEX_HOME=C:\\codex 100%%\""));
        assert!(script.contains("set \"CODEX_PLUS_STATE_DIR=C:\\state\\%%TEMP%%\\1\""));
    }

    #[test]
    fn wrapper_script_rejects_paths_that_would_break_out_of_quotes() {
        let error = build_wrapper_script(
            Path::new("C:\\state\" & calc & \""),
            Path::new(r"C:\app\ChatGPT.exe"),
            Path::new(r"C:\codex"),
            Path::new(r"C:\sqlite"),
            Path::new(r"C:\user data"),
            9401,
            &[],
        )
        .unwrap_err();

        assert!(error.to_string().contains("无法写进启动脚本"));
    }

    #[test]
    fn wrapper_script_omits_working_directory_when_executable_has_no_parent() {
        let script = build_wrapper_script(
            Path::new("state"),
            Path::new("ChatGPT.exe"),
            Path::new("codex"),
            Path::new("sqlite"),
            Path::new("user-data"),
            9401,
            &[],
        )
        .unwrap();

        assert!(!script.contains("cd /d"));
    }

    #[test]
    fn wrapper_script_uses_available_port_from_the_known_pool() {
        // 请求的端口被真实占用（且不是 Codex CDP 端点）时，退回候选池里的可用端口。
        let busy = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let busy_port = busy.local_addr().unwrap().port();
        let free_port = crate::ports::find_available_loopback_port();
        assert_ne!(busy_port, free_port);

        let script = build_wrapper_script(
            Path::new("/state"),
            Path::new("/app/ChatGPT.exe"),
            Path::new("/codex"),
            Path::new("/sqlite"),
            Path::new("/user-data"),
            busy_port,
            &[free_port],
        )
        .unwrap();

        assert!(script.contains(&format!("--remote-debugging-port={free_port}")));
        assert!(script.contains(&format!(
            "--remote-allow-origins=http://127.0.0.1:{free_port}"
        )));
    }

    #[test]
    fn debug_port_prefers_the_requested_port_when_usable() {
        // 能绑定 → 直接用。
        assert_eq!(
            select_debug_port_with(9229, &[9301], |port| port == 9229, |_| false, || 9400),
            Some(9229)
        );
        // 已被 Codex 自己的 CDP 端点占用 → 也沿用（否则注入会打到另一个端口）。
        assert_eq!(
            select_debug_port_with(9229, &[9301], |_| false, |port| port == 9229, || 9400),
            Some(9229)
        );
    }

    #[test]
    fn debug_port_falls_back_to_known_pool_then_to_system_choice() {
        assert_eq!(
            select_debug_port_with(
                9229,
                &[0, 9229, 9301, 9302],
                |port| port == 9302,
                |_| false,
                || 9400
            ),
            Some(9302)
        );
        assert_eq!(
            select_debug_port_with(0, &[], |_| false, |_| false, || 9400),
            Some(9400)
        );
        // 连系统分配都拿不到端口时如实返回 None，让调用方报错而不是拿 0 去启动。
        assert_eq!(
            select_debug_port_with(0, &[0], |_| false, |_| false, || 0),
            None
        );
    }

    #[test]
    fn write_wrapper_script_uses_a_fixed_name_inside_the_state_dir() {
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("instance").join("state");

        let path = write_wrapper_script(&state_dir, "@echo off\r\nendlocal\r\n").unwrap();

        assert_eq!(path, state_dir.join(WRAPPER_FILE_NAME));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "@echo off\r\nendlocal\r\n"
        );
    }

    #[test]
    fn prepare_wrapper_script_builds_and_writes_in_one_step() {
        let temp = tempfile::tempdir().unwrap();
        let state_dir = temp.path().join("instance state");
        let port = free_port();

        let path = prepare_wrapper_script(
            &state_dir,
            Path::new("/app dir/ChatGPT.exe"),
            Path::new("/codex"),
            Path::new("/sqlite"),
            Path::new("/user data"),
            port,
            &[],
        )
        .unwrap();

        assert_eq!(path, state_dir.join(WRAPPER_FILE_NAME));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(&format!("--remote-debugging-port={port}")));
        assert!(content.contains("--user-data-dir=\"/user data\""));
    }

    #[test]
    fn activation_script_targets_the_package_family_name_and_app_id() {
        let script =
            activation_script(AUMID, Path::new(r"C:\state dir\packaged-launch.cmd")).unwrap();

        assert!(script.contains("-PackageFamilyName 'OpenAI.Codex_2p2nqsd0c76g0'"));
        assert!(script.contains("-AppId 'App'"));
        assert!(script.contains("-Command 'cmd.exe'"));
        assert!(script.contains(r#"-Args '/d /c "C:\state dir\packaged-launch.cmd"'"#));
        assert!(script.contains("-PreventBreakaway"));
        assert!(script.contains("$ErrorActionPreference = 'Stop'"));
        // 脚本里只允许出现包身份、路径与 PID 标记：不能带凭据。
        let lowered = script.to_ascii_lowercase();
        assert!(!lowered.contains("auth.json"));
        assert!(!lowered.contains("token"));
        assert!(!lowered.contains("api_key"));
    }

    #[test]
    fn activation_script_escapes_single_quotes_in_the_wrapper_path() {
        let script = activation_script(AUMID, Path::new(r"C:\Users\O'Brien\w.cmd")).unwrap();

        assert!(script.contains(r#"-Args '/d /c "C:\Users\O''Brien\w.cmd"'"#));
    }

    #[test]
    fn activation_script_rejects_malformed_aumids() {
        let wrapper = Path::new("w.cmd");
        for aumid in [
            "",
            "OpenAI.Codex_2p2nqsd0c76g0",
            "!App",
            "OpenAI.Codex_2p2nqsd0c76g0!",
            "OpenAI.Codex_2p2nqsd0c76g0!App!Extra",
        ] {
            assert!(
                activation_script(aumid, wrapper).is_err(),
                "AUMID 应当被拒绝：{aumid}"
            );
        }
    }

    #[test]
    fn activation_powershell_arguments_match_the_non_interactive_form() {
        assert_eq!(
            activation_powershell_arguments("Get-Date"),
            vec!["-NoProfile", "-NonInteractive", "-Command", "Get-Date"]
        );
    }

    #[test]
    fn reported_process_id_requires_the_explicit_pid_marker() {
        assert_eq!(parse_reported_process_id("PID=4321\r\n"), Some(4321));
        assert_eq!(
            parse_reported_process_id("noise\r\n  PID=77  \r\n"),
            Some(77)
        );
        // 没有标记的裸数字、非数字、哨兵 0 都不算 PID。
        assert_eq!(parse_reported_process_id("4321"), None);
        assert_eq!(parse_reported_process_id("PID=abc"), None);
        assert_eq!(parse_reported_process_id("PID=0"), None);
        assert_eq!(parse_reported_process_id(""), None);
    }

    #[test]
    fn first_non_empty_line_skips_blank_lines() {
        assert_eq!(
            first_non_empty_line("\r\n  \r\n激活失败\r\n"),
            Some("激活失败".to_string())
        );
        assert_eq!(first_non_empty_line("   \r\n"), None);
    }

    #[test]
    fn known_process_id_filters_the_unknown_sentinel() {
        let launch = PackagedContextLaunch {
            app_user_model_id: AUMID.to_string(),
            wrapper_path: PathBuf::from("w.cmd"),
            process_id: Some(UNKNOWN_PROCESS_ID),
        };
        assert_eq!(launch.known_process_id(), None);

        let launch = PackagedContextLaunch {
            process_id: Some(4242),
            ..launch
        };
        assert_eq!(launch.known_process_id(), Some(4242));
    }

    #[test]
    fn instance_ready_polls_until_the_endpoint_answers() {
        let probes = Cell::new(0);

        let ready = wait_for_instance_ready_with(
            9229,
            Duration::from_millis(500),
            |_| {
                probes.set(probes.get() + 1);
                probes.get() >= 3
            },
            Duration::from_millis(1),
        );

        assert!(ready);
        assert_eq!(probes.get(), 3);
    }

    #[test]
    fn instance_ready_keeps_probing_until_the_deadline() {
        let probes = Cell::new(0);

        let ready = wait_for_instance_ready_with(
            9229,
            Duration::from_millis(30),
            |_| {
                probes.set(probes.get() + 1);
                false
            },
            Duration::from_millis(1),
        );

        assert!(!ready);
        assert!(probes.get() > 1);
    }

    #[test]
    fn instance_ready_probes_exactly_once_when_timeout_is_zero() {
        for answer in [false, true] {
            let probes = Cell::new(0);
            let ready = wait_for_instance_ready_with(
                9229,
                Duration::ZERO,
                |_| {
                    probes.set(probes.get() + 1);
                    answer
                },
                Duration::from_millis(1),
            );
            assert_eq!(ready, answer);
            assert_eq!(probes.get(), 1);
        }
    }

    #[test]
    fn instance_ready_rejects_a_zero_port() {
        assert!(!wait_for_instance_ready(0, Duration::from_millis(10)));
    }

    fn serve_cdp_once(build_body: impl FnOnce(u16) -> String) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let body = build_body(port);
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 1024];
                let _ = stream.read(&mut request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (port, handle)
    }

    #[test]
    fn instance_ready_accepts_a_real_codex_cdp_endpoint() {
        let (port, server) = serve_cdp_once(|port| {
            format!(
                r#"[{{"id":"codex","type":"page","title":"Codex","url":"app://-/index.html","webSocketDebuggerUrl":"ws://127.0.0.1:{port}/devtools/page/1"}}]"#
            )
        });

        assert!(wait_for_instance_ready(port, Duration::from_secs(2)));
        server.join().unwrap();
    }

    #[test]
    fn instance_ready_rejects_an_ordinary_http_endpoint() {
        // 端口上确实有 HTTP 服务、但不是 Codex 的 CDP：不能判成就绪。
        let (port, server) = serve_cdp_once(|_| r#"{"status":"ok"}"#.to_string());

        assert!(!wait_for_instance_ready(port, Duration::from_millis(300)));
        server.join().unwrap();
    }

    #[test]
    fn direct_launch_command_carries_the_instance_identity() {
        let state_dir = Path::new("/state/instance 1");
        let args = vec![
            "--remote-debugging-port=9401".to_string(),
            "--user-data-dir=/user data".to_string(),
        ];

        let command = direct_launch_command(
            Path::new("/Applications/Codex App/ChatGPT.exe"),
            &args,
            state_dir,
        );

        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new("/Applications/Codex App/ChatGPT.exe")
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            args.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>()
        );
        assert_eq!(
            command.get_current_dir(),
            Some(Path::new("/Applications/Codex App"))
        );
        let envs = command.get_envs().collect::<Vec<_>>();
        assert!(envs.contains(&(
            std::ffi::OsStr::new("CODEX_PLUS_STATE_DIR"),
            Some(std::ffi::OsStr::new("/state/instance 1"))
        )));
        assert!(envs.contains(&(
            std::ffi::OsStr::new("CODEX_PLUS_MULTI_INSTANCE"),
            Some(std::ffi::OsStr::new("1"))
        )));
    }

    #[cfg(unix)]
    #[test]
    fn fallback_direct_launch_spawns_the_executable_and_reports_its_pid() {
        let executable = Path::new("/usr/bin/true");
        if !executable.exists() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();

        let pid = fallback_direct_launch(executable, &[], temp.path()).unwrap();

        assert!(pid > 0);
    }

    #[test]
    fn main_instance_wrapper_starts_exe_with_debug_args_and_no_instance_env() {
        let script = build_main_instance_wrapper_script(
            Path::new(r"C:\Program Files\WindowsApps\OpenAI.Codex_1_x64__p\app\ChatGPT.exe"),
            &[
                "--remote-debugging-port=9229".to_string(),
                "--remote-allow-origins=http://127.0.0.1:9229".to_string(),
                r"--user-data-dir=C:\Users\a b\100%\x&y".to_string(),
            ],
        )
        .unwrap();

        assert!(script.contains("\r\n"));
        assert!(!script.contains("CODEX_PLUS_MULTI_INSTANCE"));
        assert!(!script.contains("CODEX_HOME"));
        assert!(!script.contains(STATE_DIR_ENV));
        assert!(script.contains(
            "start \"\" /b \"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1_x64__p\\app\\ChatGPT.exe\" \
             --remote-debugging-port=9229 --remote-allow-origins=http://127.0.0.1:9229 \
             \"--user-data-dir=C:\\Users\\a b\\100%%\\x&y\"\r\n"
        ));
    }

    #[test]
    fn main_instance_wrapper_rejects_quotes_in_arguments() {
        let error = build_main_instance_wrapper_script(
            Path::new(r"C:\app\ChatGPT.exe"),
            &["--flag=\"x\" & calc".to_string()],
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("引号"));
    }

    #[test]
    fn prepare_main_instance_wrapper_writes_separate_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = prepare_main_instance_wrapper(
            temp.path(),
            Path::new(r"C:\app\ChatGPT.exe"),
            &["--remote-debugging-port=9229".to_string()],
        )
        .unwrap();
        assert_eq!(path, temp.path().join(MAIN_WRAPPER_FILE_NAME));
        assert_ne!(MAIN_WRAPPER_FILE_NAME, WRAPPER_FILE_NAME);
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("--remote-debugging-port=9229")
        );
    }

    #[test]
    fn aumid_watch_reports_ready_as_soon_as_cdp_is_available() {
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        assert_eq!(
            watch.observe(start, true, Some(false)),
            AumidActivationVerdict::Ready
        );
    }

    #[test]
    fn aumid_watch_falls_back_after_activation_process_exits_without_cdp() {
        // issue #2700：激活进程秒退、9229 不监听 → 退出宽限后改走包上下文。
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        let exited = start + Duration::from_millis(500);
        assert_eq!(
            watch.observe(exited, false, Some(false)),
            AumidActivationVerdict::Pending
        );
        assert_eq!(
            watch.observe(exited + Duration::from_secs(1), false, Some(false)),
            AumidActivationVerdict::Pending
        );
        assert_eq!(
            watch.observe(exited + AUMID_ACTIVATION_EXIT_GRACE, false, Some(false)),
            AumidActivationVerdict::FallbackToPackageContext
        );
    }

    #[test]
    fn aumid_watch_does_not_fall_back_when_cdp_appears_during_exit_grace() {
        // 中转进程退出、真正进程稍后监听：不能误判为激活失败。
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        assert_eq!(
            watch.observe(start, false, Some(false)),
            AumidActivationVerdict::Pending
        );
        assert_eq!(
            watch.observe(start + Duration::from_secs(1), true, Some(false)),
            AumidActivationVerdict::Ready
        );
    }

    #[test]
    fn aumid_watch_keeps_old_behavior_when_process_stays_alive() {
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        assert_eq!(
            watch.observe(start + Duration::from_secs(5), false, Some(true)),
            AumidActivationVerdict::Pending
        );
        assert_eq!(
            watch.observe(start + AUMID_ACTIVATION_VERIFY_WINDOW, false, Some(true)),
            AumidActivationVerdict::Inconclusive
        );
    }

    #[test]
    fn aumid_watch_never_falls_back_when_liveness_is_unknown() {
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        assert_eq!(
            watch.observe(start + Duration::from_secs(1), false, None),
            AumidActivationVerdict::Pending
        );
        assert_eq!(
            watch.observe(start + AUMID_ACTIVATION_VERIFY_WINDOW, false, None),
            AumidActivationVerdict::Inconclusive
        );
    }

    #[test]
    fn aumid_watch_resets_exit_grace_when_process_is_seen_alive_again() {
        let start = Instant::now();
        let mut watch = AumidActivationWatch::new(start);
        watch.observe(start, false, Some(false));
        watch.observe(start + Duration::from_secs(1), false, Some(true));
        assert_eq!(
            watch.observe(start + Duration::from_secs(3), false, Some(false)),
            AumidActivationVerdict::Pending
        );
    }
}
