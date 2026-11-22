//! 「Claude Desktop 自动接管」：用户自己打开官方 Claude 时，退出它再经完整功能模式重开。
//!
//! 和 Codex 的自动接管（`auto_takeover`）共用一套判据：宽限期、每小时重启额度、失败退避、
//! 「本次不接管」、实例代际键都直接复用 `auto_takeover::decide`。这里只补 Claude 自己的部分：
//!
//! 1. **只在完整功能模式接管**：启动模式是 3p（`claudeDesktopLocal.enabled == false`）或补丁副本
//!    （`claude_localization::enhance_delivery == "copy"`）时一律不动。
//! 2. **认实例靠命令行**：经本地服务启动的 Claude 带 `--proxy-server=http://127.0.0.1:<port>`，
//!    ClaudeMom 的第三方模式带 `--claude-mom-gateway=`；都没有的就是用户自己开的官方 Claude。
//!    补丁副本（`claude-localization/Claude Enhanced…`）永远不碰，它在跑时也不接管——
//!    `perform("launch")` 见到任何 Claude 进程都会拒绝启动，退了官方也起不来。
//! 3. **先体检再退出**：证书没信任、找不到 Node、没有可用上游时 `perform("launch")` 必然失败，
//!    这时退出用户的 Claude 只会让他什么都没有，所以先查，查不过就报降级、不动进程。
//! 4. **「疑似有人在用」**：Claude 没有像 Codex 会话 jsonl 那样只属于 Desktop 的本地信号
//!    （`~/.claude/projects` 命令行的 Claude Code 也在写），这里改用「实例年龄」：
//!    第一次被看到时就已开了超过 [`FRESH_INSTANCE_SECS`] 的实例（比如打开开关时 Claude 早就开着），
//!    很可能正在用，不自动退出，等用户点「立即接管」。按第一次看到时算由管理器记住。
//!
//! 纯逻辑（解析、分类、判据）无 IO，便于单测；探测与退出在文件后半部分，触发源在管理器进程。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::auto_takeover::{AppProbe, InstanceKey, SessionSnapshot, TakeoverDecision, TakeoverInputs};

/// 经本机本地服务启动的 Claude 独有的参数前缀（完整功能模式）。
pub const PROXY_FLAG_PREFIX: &str = "--proxy-server=http://127.0.0.1:";
/// ClaudeMom 第三方模式启动 Claude 时带的标记。
pub const GATEWAY_FLAG_PREFIX: &str = "--claude-mom-gateway=";
/// 实例开了这么久才被看到，就当作「可能正在用」，不自动退出。
pub const FRESH_INSTANCE_SECS: u64 = 120;

/// 支持的平台：完整功能模式本身只支持 macOS 和 Windows。
pub const PLATFORM_SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// Claude 的启动模式（与 Desktop 增强页「启动模式」下拉一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Full,
    ThreeP,
    Copy,
}

impl LaunchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ThreeP => "3p",
            Self::Copy => "copy",
        }
    }

    /// 和前端 `claudeLaunch` 同一条规则：生效方式是副本就是副本，否则看完整功能模式开没开。
    pub fn from_settings(settings: &crate::settings::BackendSettings) -> Self {
        if crate::claude_localization::enhance_delivery(settings) == crate::claude_localization::DELIVERY_COPY {
            Self::Copy
        } else if settings.claude_desktop_local.enabled {
            Self::Full
        } else {
            Self::ThreeP
        }
    }
}

/// 一个 Claude 主进程是谁开的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceKind {
    /// 用户自己开的官方 Claude：要接管的那种。
    Official,
    /// 经本地服务启动的完整功能模式实例：已接管。
    FullMode,
    /// ClaudeMom 第三方模式启动的：不是我们这边的，不动。
    GatewayMode,
    /// 补丁副本：独立的应用，永远不碰。
    EnhancedCopy,
    /// 带 `--user-data-dir` / 调试端口之类启动的隔离实例：不是裸启动，不动。
    Foreign,
}

/// `{pid} {命令行}` 清单里的一行。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeProcess {
    pub pid: u32,
    pub command_line: String,
}

/// 解析 `claude_local::desktop_process_listing` 的输出（macOS `pgrep -fl` / Windows 同形状）。
pub fn parse_listing(listing: &str) -> Vec<ClaudeProcess> {
    listing
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (pid, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            Some(ClaudeProcess {
                pid: pid.trim().parse().ok()?,
                command_line: rest.trim().to_string(),
            })
        })
        .collect()
}

/// 命令行按空白切分成参数，引号里的空白不切（Windows 的 `"C:\Program Files\…\Claude.exe"`）。
fn arguments(command_line: &str) -> Vec<String> {
    let mut quoted = false;
    let mut current = String::new();
    let mut items = Vec::new();
    for character in command_line.chars() {
        if character == '"' {
            quoted = !quoted;
        } else if !quoted && character.is_whitespace() {
            if !current.is_empty() {
                items.push(std::mem::take(&mut current));
            }
        } else {
            current.push(character);
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

/// Electron 子进程（renderer / gpu / utility / crashpad）：命令行里有 `--type=`。
/// macOS 的 `Claude Helper` 也会被 `pgrep` 列出来，同样靠它排除。
pub fn is_child_process(command_line: &str) -> bool {
    arguments(command_line)
        .iter()
        .any(|argument| argument == "--type" || argument.starts_with("--type="))
}

fn normalized(text: &str) -> String {
    text.replace('\\', "/").to_lowercase()
}

/// 进程是不是补丁副本：路径在 `claude-localization` 状态目录下，或是副本 / 旧版副本的固定名字。
fn is_enhanced_copy(command_line: &str, copy_root: &Path) -> bool {
    let line = normalized(command_line);
    let root = normalized(&copy_root.to_string_lossy());
    let root = root.trim_end_matches('/');
    if !root.is_empty() && line.contains(&format!("{root}/")) {
        return true;
    }
    [
        "/claude enhanced.app/",
        "/claude enhanced/claude.exe",
        "/claude cn.app/",
        "/claude cn/claude.exe",
    ]
    .iter()
    .any(|needle| line.contains(needle))
}

/// 给一个**主进程**的命令行分类。顺序即优先级：副本最先（它也可能带别的参数）。
pub fn classify(command_line: &str, copy_root: &Path) -> InstanceKind {
    if is_enhanced_copy(command_line, copy_root) {
        return InstanceKind::EnhancedCopy;
    }
    let arguments = arguments(command_line);
    if arguments.iter().any(|argument| argument.starts_with(PROXY_FLAG_PREFIX)) {
        return InstanceKind::FullMode;
    }
    if arguments.iter().any(|argument| argument.starts_with(GATEWAY_FLAG_PREFIX)) {
        return InstanceKind::GatewayMode;
    }
    if arguments.iter().any(|argument| {
        argument.starts_with("--user-data-dir")
            || argument.starts_with("--remote-debugging-port")
            || argument.starts_with("--inspect")
    }) {
        return InstanceKind::Foreign;
    }
    InstanceKind::Official
}

/// 一次进程扫描的分类结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeScan {
    /// 清单读到了（读失败 ≠ 没在运行）。
    pub ok: bool,
    /// 有进程的命令行读不到：分不清主进程和子进程，保守按「未知」处理。
    pub unreadable: bool,
    pub mains: Vec<(ClaudeProcess, InstanceKind)>,
    /// 非副本的子进程（退出后要等它们也走完，Claude 退出时还在回写配置）。
    pub children: Vec<ClaudeProcess>,
    pub copy_running: bool,
}

impl ClaudeScan {
    pub fn from_listing(listing: Option<&str>, copy_root: &Path) -> Self {
        let Some(listing) = listing else {
            return Self::default();
        };
        let mut scan = Self {
            ok: true,
            ..Self::default()
        };
        for process in parse_listing(listing) {
            if process.command_line.is_empty() {
                scan.unreadable = true;
                continue;
            }
            let copy = is_enhanced_copy(&process.command_line, copy_root);
            if copy {
                scan.copy_running = true;
            }
            if is_child_process(&process.command_line) {
                if !copy {
                    scan.children.push(process);
                }
                continue;
            }
            let kind = classify(&process.command_line, copy_root);
            scan.mains.push((process, kind));
        }
        scan
    }

    fn pids(&self, kind: InstanceKind) -> Vec<u32> {
        self.mains
            .iter()
            .filter(|(_, current)| *current == kind)
            .map(|(process, _)| process.pid)
            .collect()
    }

    /// 除副本之外还有没有 Claude 进程（含子进程）：`perform("launch")` 要求一个都没有。
    pub fn any_non_copy_running(&self) -> bool {
        !self.children.is_empty()
            || self
                .mains
                .iter()
                .any(|(_, kind)| *kind != InstanceKind::EnhancedCopy)
    }

    pub fn command_line(&self, pid: u32) -> Option<&str> {
        self.mains
            .iter()
            .find(|(process, _)| process.pid == pid)
            .map(|(process, _)| process.command_line.as_str())
    }
}

/// 扫描结果折算成共用判据要的「进程 / 会话」部分。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeProbe {
    pub app: AppProbe,
    /// 完整功能模式的实例在跑（已接管）。
    pub attached: bool,
    /// 有别的 Claude 主进程（第二个官方实例、隔离实例、ClaudeMom 第三方模式）：退出会误伤，跳过。
    pub other_instance: bool,
    pub copy_running: bool,
    /// 要接管的那个官方实例已经开了多少秒。
    pub instance_age_secs: Option<u64>,
}

/// `identity(pid)` 给出实例代际键和已运行秒数（IO 由调用方提供，便于单测）。
pub fn probe_from_scan(
    scan: &ClaudeScan,
    identity: impl Fn(u32) -> Option<(InstanceKey, Option<u64>)>,
) -> ClaudeProbe {
    let mut probe = ClaudeProbe {
        copy_running: scan.copy_running,
        ..ClaudeProbe::default()
    };
    if !scan.ok || scan.unreadable {
        probe.app = AppProbe::Unknown;
        return probe;
    }
    let official = scan.pids(InstanceKind::Official);
    let full = scan.pids(InstanceKind::FullMode);
    probe.other_instance = official.len() > 1
        || (!official.is_empty() && !full.is_empty())
        || !scan.pids(InstanceKind::GatewayMode).is_empty()
        || !scan.pids(InstanceKind::Foreign).is_empty();
    let target = match (official.first(), full.first()) {
        (Some(pid), _) => Some(*pid),
        (None, Some(pid)) => {
            probe.attached = true;
            Some(*pid)
        }
        (None, None) => None,
    };
    probe.app = match target {
        None => AppProbe::NotRunning,
        Some(pid) => match identity(pid) {
            Some((key, age)) => {
                if !probe.attached {
                    probe.instance_age_secs = age;
                }
                AppProbe::Running(key)
            }
            None => AppProbe::Unknown,
        },
    };
    probe
}

/// 已接管时给共用判据的会话快照：完整功能模式的实例在跑就算「running」。
pub fn session_for(probe: &ClaudeProbe) -> SessionSnapshot {
    if probe.attached {
        SessionSnapshot {
            owned: true,
            ready: true,
            status: Some("running".to_string()),
            debug_port: None,
            started_at_ms: None,
        }
    } else {
        SessionSnapshot::default()
    }
}

/// 「疑似有人在用」：官方实例开了很久才被看到。
pub fn instance_seems_in_use(age_secs: Option<u64>) -> bool {
    age_secs.is_some_and(|age| age > FRESH_INSTANCE_SECS)
}

/// Claude 判据的输入：共用判据的输入加上 Claude 自己的两条。
#[derive(Debug, Clone)]
pub struct ClaudeTakeoverInputs {
    pub mode: LaunchMode,
    pub copy_running: bool,
    pub base: TakeoverInputs,
}

/// Claude 的判据：先判启动模式和补丁副本，其余（在途、未知、跳过、宽限、额度、退避、在用）交给共用判据。
pub fn decide(inputs: &ClaudeTakeoverInputs) -> TakeoverDecision {
    if !inputs.base.enabled {
        return TakeoverDecision::Skip("disabled");
    }
    if !inputs.base.platform_supported {
        return TakeoverDecision::Skip("platform_unsupported");
    }
    match inputs.mode {
        LaunchMode::ThreeP => return TakeoverDecision::Skip("mode_3p"),
        LaunchMode::Copy => return TakeoverDecision::Skip("mode_copy"),
        LaunchMode::Full => {}
    }
    // 副本在跑：它是另一个应用，退官方不会动它，但 `perform("launch")` 见到它就拒绝启动。
    if inputs.copy_running {
        return TakeoverDecision::Skip("enhanced_copy_running");
    }
    crate::auto_takeover::decide(&inputs.base)
}

/// 完整功能模式起不来的原因（稳定英文码，前端翻译）。顺序按用户该先处理哪一个。
pub fn launch_blocker(
    supported: bool,
    node_available: bool,
    ca_trusted: bool,
    three_p_mode: bool,
    upstream_problem: Option<&str>,
) -> Option<&'static str> {
    if !supported {
        return Some("claude_not_installed");
    }
    if !node_available {
        return Some("node_missing");
    }
    // ClaudeMom 的第三方模式不经代理，证书用不上（与 perform("launch") 的判断一致）。
    if !three_p_mode && !ca_trusted {
        return Some("ca_untrusted");
    }
    if upstream_problem.is_some() {
        return Some("upstream_not_ready");
    }
    None
}

// ---------------------------------------------------------------- 探测与退出（IO）

/// 补丁副本所在目录（与 `claude_localization` 的 runtime_dir 同一处）。
pub fn copy_root() -> PathBuf {
    crate::paths::default_app_state_dir().join("claude-localization")
}

/// 读一次进程清单并分类。
pub fn scan_now() -> ClaudeScan {
    if !PLATFORM_SUPPORTED {
        return ClaudeScan::default();
    }
    let listing = claude_local::desktop_process_listing().ok();
    ClaudeScan::from_listing(listing.as_deref(), &copy_root())
}

/// 实例代际键（pid + 启动时刻）与已运行秒数。
pub fn instance_identity(pid: u32) -> Option<(InstanceKey, Option<u64>)> {
    match crate::watcher::inspect_process_instance(pid) {
        crate::watcher::ProcessInstanceState::Running {
            started_at_secs,
            birth_id,
        } => {
            let incarnation = birth_id
                .or_else(|| started_at_secs.map(|secs| secs.to_string()))
                .unwrap_or_default();
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .ok();
            let age = started_at_secs.zip(now).map(|(started, now)| now.saturating_sub(started));
            Some((InstanceKey::new(pid, incarnation), age))
        }
        _ => None,
    }
}

/// 扫描 + 折算，一步到位。返回扫描结果给调用方复用（退出时要用命令行定位应用）。
pub fn probe_now() -> (ClaudeScan, ClaudeProbe) {
    let scan = scan_now();
    let probe = probe_from_scan(&scan, instance_identity);
    (scan, probe)
}

/// 退出前的体检：完整功能模式此刻能不能起来。`Err((原因码, 说明))`。
pub fn check_launch_ready() -> Result<(), (&'static str, String)> {
    let status = crate::claude_local_backend::status().map_err(|error| ("status_failed", format!("{error:#}")))?;
    let upstream = crate::claude_local_backend::upstream_problem();
    match launch_blocker(
        status.supported,
        status.node_available,
        status.ca_trusted,
        status.config.three_p_mode,
        upstream.as_deref(),
    ) {
        None => Ok(()),
        Some(code) => {
            let message = match code {
                "claude_not_installed" => "未检测到官方 Claude Desktop。".to_string(),
                "node_missing" => "未找到支持 fetch 和 WebSocket 的 Node.js（需要 Node 22+）。".to_string(),
                "ca_untrusted" => "本地证书还没有信任，请先在 Desktop 增强页信任证书。".to_string(),
                _ => upstream.unwrap_or_default(),
            };
            Err((code, message))
        }
    }
}

/// 接管失败时把官方 Claude 原样开回去（不带任何参数），别让用户什么都没有。
pub fn reopen_official() -> anyhow::Result<()> {
    let status = crate::claude_local_backend::status()?;
    anyhow::ensure!(status.supported, "未检测到官方 Claude Desktop");
    claude_local::launch_claude(Path::new(&status.source_app), |_| {})
}

/// 退出的力度：先礼后兵。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStage {
    /// macOS：AppleEvent quit（走完整退出流程）；Windows：WM_CLOSE。
    Graceful,
    /// macOS：SIGTERM（Electron 照样走退出流程）；Windows 同 Kill。
    Terminate,
    /// macOS：SIGKILL；Windows：`taskkill /F /T`。
    Kill,
}

/// macOS 上从主进程命令行取出 `.app` 路径（`…/Claude.app/Contents/MacOS/Claude …`）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_bundle_path(command_line: &str) -> Option<&str> {
    let index = command_line.find("/Contents/MacOS/Claude")?;
    let bundle = &command_line[..index];
    bundle.ends_with(".app").then_some(bundle)
}

/// 请某个官方 Claude 主进程退出。只发请求、不等待；返回请求是否发出去了。
#[allow(unused_variables)]
pub fn request_exit(pid: u32, command_line: &str, stage: ExitStage) -> bool {
    #[cfg(target_os = "macos")]
    {
        return match stage {
            ExitStage::Graceful => match macos_bundle_path(command_line) {
                // 按 .app 路径发 quit，不按名字：名字可能解析到补丁副本或别的同名应用。
                // `ignoring application responses`：Claude 弹「确定退出？」时不让 osascript 卡两分钟。
                Some(bundle) => std::process::Command::new("osascript")
                    .args([
                        "-e",
                        "on run argv",
                        "-e",
                        "ignoring application responses",
                        "-e",
                        "tell application (POSIX file (item 1 of argv) as text) to quit",
                        "-e",
                        "end ignoring",
                        "-e",
                        "end run",
                        bundle,
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|status| status.success()),
                None => false,
            },
            ExitStage::Terminate | ExitStage::Kill => std::process::Command::new("/bin/kill")
                .args([
                    if stage == ExitStage::Kill { "-KILL" } else { "-TERM" },
                    &pid.to_string(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success()),
        };
    }
    #[cfg(windows)]
    {
        // Windows 上关掉窗口 Claude 可能只是缩到托盘；优雅请求无效时由调用方升级成强制结束。
        return crate::windows_integration::close_process_tree(pid, stage != ExitStage::Graceful);
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

/// 主进程退了以后还剩下的子进程（Windows 的 crashpad 等）一并强制结束。
/// 调用前已确认没有别的 Claude 主进程（只剩被退出实例的遗留），补丁副本的进程不在 `children` 里。
pub fn reap_children(scan: &ClaudeScan) -> usize {
    let mut reaped = 0;
    for child in &scan.children {
        if request_exit(child.pid, &child.command_line, ExitStage::Kill) {
            reaped += 1;
        }
    }
    reaped
}

/// 等到除副本之外一个 Claude 进程都不剩（含子进程）。
pub fn wait_all_exited(budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    loop {
        let scan = scan_now();
        if scan.ok && !scan.unreadable && !scan.any_non_copy_running() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_takeover::{ActivePolicy, TAKEOVER_GRACE_MS};

    fn root() -> PathBuf {
        PathBuf::from("/Users/me/Library/Application Support/Codex++/claude-localization")
    }

    const OFFICIAL_MAC: &str = "/Applications/Claude.app/Contents/MacOS/Claude";
    const HELPER_MAC: &str = "/Applications/Claude.app/Contents/Frameworks/Claude Helper (Renderer).app/Contents/MacOS/Claude Helper (Renderer) --type=renderer --lang=zh-CN";
    const FULL_MAC: &str = "/Applications/Claude.app/Contents/MacOS/Claude --proxy-server=http://127.0.0.1:53514 --lang=zh-CN";
    const COPY_MAC: &str = "/Users/me/Library/Application Support/Codex++/claude-localization/Claude Enhanced.app/Contents/MacOS/Claude --inspect=0";

    #[test]
    fn listing_parses_pid_and_command_line() {
        let listing = format!("123 {OFFICIAL_MAC}\n  456 {HELPER_MAC}\nbad line\n789\n");
        let parsed = parse_listing(&listing);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], ClaudeProcess { pid: 123, command_line: OFFICIAL_MAC.to_string() });
        assert_eq!(parsed[1].pid, 456);
        assert_eq!(parsed[2], ClaudeProcess { pid: 789, command_line: String::new() });
    }

    #[test]
    fn children_are_recognized_by_type_flag_outside_quotes() {
        assert!(is_child_process(HELPER_MAC));
        assert!(is_child_process(r#""C:\Users\me\AppData\Local\AnthropicClaude\app-1.0\claude.exe" --type=gpu-process"#));
        assert!(!is_child_process(OFFICIAL_MAC));
        assert!(!is_child_process(r#""C:\Program Files\x --type=renderer\Claude.exe""#));
    }

    #[test]
    fn classification_covers_every_launch_path() {
        let root = root();
        assert_eq!(classify(OFFICIAL_MAC, &root), InstanceKind::Official);
        assert_eq!(classify(FULL_MAC, &root), InstanceKind::FullMode);
        assert_eq!(classify(COPY_MAC, &root), InstanceKind::EnhancedCopy);
        assert_eq!(
            classify("/Applications/Claude.app/Contents/MacOS/Claude --claude-mom-gateway=http://127.0.0.1:5351", &root),
            InstanceKind::GatewayMode
        );
        assert_eq!(
            classify("/Applications/Claude.app/Contents/MacOS/Claude --user-data-dir=/tmp/x", &root),
            InstanceKind::Foreign
        );
        // Windows：副本在状态目录下（大小写、分隔符不同也认），以及固定的副本目录名。
        let windows_root = PathBuf::from(r"C:\Users\me\AppData\Roaming\Codex++\claude-localization");
        assert_eq!(
            classify(r#""c:\users\me\appdata\roaming\codex++\claude-localization\Claude Enhanced\Claude.exe""#, &windows_root),
            InstanceKind::EnhancedCopy
        );
        assert_eq!(
            classify(r#""D:\elsewhere\Claude Enhanced\Claude.exe""#, &windows_root),
            InstanceKind::EnhancedCopy
        );
        assert_eq!(
            classify(r#""C:\Users\me\AppData\Local\AnthropicClaude\app-1.0\claude.exe""#, &windows_root),
            InstanceKind::Official
        );
        assert_eq!(
            classify(r#""C:\Program Files\WindowsApps\Claude_1.0_x64__abc\app\Claude.exe" --proxy-server=http://127.0.0.1:5351"#, &windows_root),
            InstanceKind::FullMode
        );
        // 相似但不是我们的参数：不能误认成已接管。
        assert_eq!(
            classify("/Applications/Claude.app/Contents/MacOS/Claude --proxy-server=http://10.0.0.1:8080", &root),
            InstanceKind::Official
        );
    }

    fn identity(pid: u32) -> Option<(InstanceKey, Option<u64>)> {
        Some((InstanceKey::new(pid, format!("start-{pid}")), Some(5)))
    }

    fn scan(lines: &[(u32, &str)]) -> ClaudeScan {
        let listing = lines
            .iter()
            .map(|(pid, line)| format!("{pid} {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        ClaudeScan::from_listing(Some(&listing), &root())
    }

    #[test]
    fn official_launch_is_a_takeover_candidate() {
        let scan = scan(&[(10, OFFICIAL_MAC), (11, HELPER_MAC)]);
        let probe = probe_from_scan(&scan, identity);
        assert_eq!(probe.app, AppProbe::Running(InstanceKey::new(10, "start-10")));
        assert!(!probe.attached && !probe.other_instance && !probe.copy_running);
        assert_eq!(probe.instance_age_secs, Some(5));
        assert!(scan.any_non_copy_running());
    }

    #[test]
    fn full_mode_instance_counts_as_attached() {
        let probe = probe_from_scan(&scan(&[(20, FULL_MAC), (21, HELPER_MAC)]), identity);
        assert!(probe.attached);
        assert!(!probe.other_instance);
        assert!(session_for(&probe).is_running());
    }

    #[test]
    fn patched_copy_is_never_a_candidate_but_blocks_takeover() {
        let copy_helper = format!("{} --type=renderer", COPY_MAC.replace("/MacOS/Claude --inspect=0", "/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper"));
        let only_copy = scan(&[(30, COPY_MAC), (31, &copy_helper)]);
        let probe = probe_from_scan(&only_copy, identity);
        assert_eq!(probe.app, AppProbe::NotRunning);
        assert!(probe.copy_running);
        assert!(!only_copy.any_non_copy_running(), "副本的进程不算要等的 Claude");

        let both = scan(&[(30, COPY_MAC), (40, OFFICIAL_MAC)]);
        let probe = probe_from_scan(&both, identity);
        assert_eq!(probe.app, AppProbe::Running(InstanceKey::new(40, "start-40")));
        assert!(probe.copy_running);
    }

    #[test]
    fn second_or_isolated_instances_count_as_other() {
        let two = probe_from_scan(&scan(&[(1, OFFICIAL_MAC), (2, OFFICIAL_MAC)]), identity);
        assert!(two.other_instance);
        let isolated = probe_from_scan(
            &scan(&[(1, OFFICIAL_MAC), (2, "/Applications/Claude.app/Contents/MacOS/Claude --user-data-dir=/tmp/x")]),
            identity,
        );
        assert!(isolated.other_instance);
        let mixed = probe_from_scan(&scan(&[(1, OFFICIAL_MAC), (2, FULL_MAC)]), identity);
        assert!(mixed.other_instance);
    }

    #[test]
    fn unreadable_or_failed_listing_is_unknown() {
        assert_eq!(probe_from_scan(&ClaudeScan::from_listing(None, &root()), identity).app, AppProbe::Unknown);
        let probe = probe_from_scan(&scan(&[(1, OFFICIAL_MAC), (2, "")]), identity);
        assert_eq!(probe.app, AppProbe::Unknown);
        // 代际键拿不到（进程刚退）也不能当成可接管。
        let probe = probe_from_scan(&scan(&[(1, OFFICIAL_MAC)]), |_| None);
        assert_eq!(probe.app, AppProbe::Unknown);
        assert_eq!(probe_from_scan(&scan(&[]), identity).app, AppProbe::NotRunning);
    }

    fn base(probe: &ClaudeProbe) -> TakeoverInputs {
        TakeoverInputs {
            enabled: true,
            platform_supported: true,
            session: session_for(probe),
            app: probe.app.clone(),
            cdp_ready: probe.attached,
            actual_port: 0,
            multi_instance_running: probe.other_instance,
            takeover_in_progress: false,
            ms_since_our_launch: None,
            ms_since_last_attempt: None,
            ms_since_wake: None,
            consecutive_failures: 0,
            user_active: instance_seems_in_use(probe.instance_age_secs),
            active_policy: ActivePolicy::Defer,
            paused_instance: None,
            restart_history_ms: Vec::new(),
            now_ms: 1_000_000,
        }
    }

    fn inputs(lines: &[(u32, &str)], mode: LaunchMode) -> ClaudeTakeoverInputs {
        let probe = probe_from_scan(&scan(lines), identity);
        ClaudeTakeoverInputs {
            mode,
            copy_running: probe.copy_running,
            base: base(&probe),
        }
    }

    #[test]
    fn decision_takes_over_only_official_in_full_mode() {
        assert!(decide(&inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full)).is_takeover());
        assert_eq!(decide(&inputs(&[(1, OFFICIAL_MAC)], LaunchMode::ThreeP)), TakeoverDecision::Skip("mode_3p"));
        assert_eq!(decide(&inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Copy)), TakeoverDecision::Skip("mode_copy"));
        assert_eq!(decide(&inputs(&[(1, FULL_MAC)], LaunchMode::Full)), TakeoverDecision::Wait("already_attached"));
        assert_eq!(decide(&inputs(&[], LaunchMode::Full)), TakeoverDecision::Wait("app_not_running"));
        assert_eq!(
            decide(&inputs(&[(1, OFFICIAL_MAC), (2, COPY_MAC)], LaunchMode::Full)),
            TakeoverDecision::Skip("enhanced_copy_running")
        );
        assert_eq!(
            decide(&inputs(&[(1, OFFICIAL_MAC), (2, OFFICIAL_MAC)], LaunchMode::Full)),
            TakeoverDecision::Skip("multi_instance")
        );
        let mut off = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::ThreeP);
        off.base.enabled = false;
        assert_eq!(decide(&off), TakeoverDecision::Skip("disabled"));
    }

    #[test]
    fn shared_guards_still_apply() {
        // 宽限期：从我们启动完成算起。
        let mut grace = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full);
        grace.base.ms_since_our_launch = Some(TAKEOVER_GRACE_MS - 1);
        assert_eq!(decide(&grace), TakeoverDecision::Wait("grace"));
        // 「本次不接管」只管那一个实例。
        let mut paused = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full);
        paused.base.paused_instance = Some(InstanceKey::new(1, "start-1"));
        assert_eq!(decide(&paused), TakeoverDecision::Wait("paused_for_instance"));
        // 开了很久的实例：可能正在用，不自动退出。
        let mut old = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full);
        old.base.user_active = instance_seems_in_use(Some(FRESH_INSTANCE_SECS + 1));
        assert_eq!(decide(&old), TakeoverDecision::Wait("user_active"));
        assert!(!instance_seems_in_use(Some(FRESH_INSTANCE_SECS)));
        assert!(!instance_seems_in_use(None));
        // 退避与在途。
        let mut backoff = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full);
        backoff.base.consecutive_failures = 1;
        backoff.base.ms_since_last_attempt = Some(1_000);
        assert_eq!(decide(&backoff), TakeoverDecision::Wait("backoff"));
        let mut busy = inputs(&[(1, OFFICIAL_MAC)], LaunchMode::Full);
        busy.base.takeover_in_progress = true;
        assert_eq!(decide(&busy), TakeoverDecision::Wait("launch_in_progress"));
    }

    #[test]
    fn launch_blockers_are_reported_in_order() {
        assert_eq!(launch_blocker(false, false, false, false, Some("x")), Some("claude_not_installed"));
        assert_eq!(launch_blocker(true, false, false, false, None), Some("node_missing"));
        assert_eq!(launch_blocker(true, true, false, false, None), Some("ca_untrusted"));
        assert_eq!(launch_blocker(true, true, false, true, None), None, "第三方模式不需要证书");
        assert_eq!(launch_blocker(true, true, true, false, Some("没有模型")), Some("upstream_not_ready"));
        assert_eq!(launch_blocker(true, true, true, false, None), None);
    }

    #[test]
    fn macos_bundle_path_comes_from_the_main_command_line() {
        assert_eq!(macos_bundle_path(FULL_MAC), Some("/Applications/Claude.app"));
        assert_eq!(
            macos_bundle_path("/Users/me/Applications/Claude.app/Contents/MacOS/Claude"),
            Some("/Users/me/Applications/Claude.app")
        );
        assert_eq!(macos_bundle_path("/usr/local/bin/claude"), None);
    }
}
