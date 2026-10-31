//! Codex 选了「在 WSL 中运行」时，让 WSL 里的 Codex 连得上 Windows 上的网关（#2773）。
//!
//! 网关挂在 helper 上，只监听 Windows 本机 `127.0.0.1`。Codex 在 WSL 里运行时，真正发请求的
//! codex 进程在 WSL2 虚拟机里；默认 NAT 网络下 WSL 的 `127.0.0.1` 是虚拟机自己，连不到 Windows。
//! WSL 里的默认网关就是 Windows 上 `vEthernet (WSL...)` 虚拟网卡的 IPv4，这里在**这一个地址**上
//! 额外监听同一端口，并把写给 Codex 的网关地址换成它。
//!
//! 安全边界：
//! - 不监听 `0.0.0.0` / 所有网卡，只绑 WSL 虚拟网卡这一个地址；拿不到就保持现状（只有回环）；
//! - 这个监听上只放行网关路径 `/gw/*`，网关对每个请求都校验签发给该 Agent 的 token；
//!   helper 上其它不鉴权的接口（状态、设置、听写、本地协议代理 `/v1/*` 等）一律 403
//!   （见 [`wsl_listener_allows`]，由 `launcher` 的连接处理函数执行）；
//! - 只在 Windows 上、且 Codex 的 `config.toml` 里 `[desktop] runCodexInWindowsSubsystemForLinux = true`
//!   时生效；`.wslconfig` 开了 mirrored 网络时 WSL 与 Windows 共用回环，也不需要额外监听。
//!
//! 「在 WSL 中运行」的识别直接读 Codex 桌面版自己写的设置（与模型目录指针 #2591 用的是同一个键），
//! 不另设开关：手动开关容易和 Codex 里的实际选择对不上，而这个键就是 Codex 启动 WSL 的依据。

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context;

/// 写给 Codex 的网关地址里，不在 WSL 模式时用的主机。
pub const LOOPBACK_HOST: &str = "127.0.0.1";

/// 识别结果：网关地址该用回环还是 WSL 虚拟网卡。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WslGatewayHost {
    /// 保持现状：只用 `127.0.0.1`。
    Loopback(LoopbackReason),
    /// 额外监听并写给 Codex 的 WSL 虚拟网卡地址。
    Wsl(Ipv4Addr),
}

impl WslGatewayHost {
    /// 写给 Codex 的 WSL 网卡地址；保持回环时为 `None`。
    pub fn address(self) -> Option<Ipv4Addr> {
        match self {
            Self::Wsl(addr) => Some(addr),
            Self::Loopback(_) => None,
        }
    }
}

/// 为什么保持回环（诊断日志用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopbackReason {
    NotWindows,
    CodexNotInWsl,
    /// `.wslconfig` 用的是 mirrored 网络，WSL 里的 127.0.0.1 就是 Windows。
    MirroredNetworking,
    /// 找不到 WSL 虚拟网卡（WSL 还没启动、或查询失败）：暂时的，已写进 Codex 的地址保持现状。
    AdapterUnavailable,
    /// 拿到了网卡地址，但在它上面监听失败：WSL 监听不在，写给 Codex 的地址要改回回环。
    BindFailed,
}

impl LoopbackReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotWindows => "not_windows",
            Self::CodexNotInWsl => "codex_not_in_wsl",
            Self::MirroredNetworking => "mirrored_networking",
            Self::AdapterUnavailable => "adapter_unavailable",
            Self::BindFailed => "bind_failed",
        }
    }
}

/// 选监听 / 写给 Codex 的地址（纯函数）。
pub fn select_wsl_gateway_host(
    windows: bool,
    codex_in_wsl: bool,
    mirrored: bool,
    adapter: Option<Ipv4Addr>,
) -> WslGatewayHost {
    if !windows {
        return WslGatewayHost::Loopback(LoopbackReason::NotWindows);
    }
    if !codex_in_wsl {
        return WslGatewayHost::Loopback(LoopbackReason::CodexNotInWsl);
    }
    if mirrored {
        return WslGatewayHost::Loopback(LoopbackReason::MirroredNetworking);
    }
    match adapter.filter(|addr| usable_adapter_address(*addr)) {
        Some(addr) => WslGatewayHost::Wsl(addr),
        None => WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable),
    }
}

/// WSL 虚拟网卡地址只认私有网段：回环、0.0.0.0、169.254（网卡没起来时的自动地址）、
/// 公网地址都不要，宁可保持现状也不在不该监听的地址上开口子。
fn usable_adapter_address(addr: Ipv4Addr) -> bool {
    addr.is_private() && !addr.is_loopback() && !addr.is_unspecified() && !addr.is_link_local()
}

/// 写给 Codex 的网关主机（纯函数）。
pub fn codex_gateway_host(wsl_host: Option<Ipv4Addr>) -> String {
    wsl_host.map_or_else(|| LOOPBACK_HOST.to_string(), |addr| addr.to_string())
}

/// Codex 接入网关时用不用 ChatGPT 官方会话身份（纯函数）。
///
/// 官方身份下请求先发到本地协议代理（`openai_base_url`，`/v1/*`）再转给网关，那一跳没有网关 token；
/// WSL 监听不放行它（见 [`wsl_listener_allows`]）。所以 WSL 模式下一律用带网关 token 的自定义供应商。
pub fn codex_gateway_uses_official_identity(can_sign_in: bool, wsl_host: Option<Ipv4Addr>) -> bool {
    can_sign_in && wsl_host.is_none()
}

/// WSL 监听上收到的请求放不放行（纯函数）：只放网关路径，网关自己再校验 token。
pub fn wsl_listener_allows(path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    // 防路径穿越式的写法（`/gw/../settings`）：helper 不做规范化，这里直接拒掉。
    crate::gateway::is_gateway_path(path) && !path.split('/').any(|part| part == "..")
}

/// 已写进 Codex 的网关地址还对不对（纯函数）：要不要重写一次托管供应商。
///
/// - 拿到了 WSL 地址且监听就绪：主机不同就重写；
/// - Codex 在 WSL 里但暂时拿不到网卡（WSL 还没启动）：保持现状，等下一轮；
/// - 其余保持回环的情况（不在 WSL 里、`.wslconfig` 改成了 mirrored、WSL 网卡上绑定失败）：
///   WSL 监听已经不在了，之前写过非回环地址就改回回环，不能让 Codex 一直连失效的 172.x 地址。
pub fn codex_gateway_host_stale(written_host: Option<&str>, decision: WslGatewayHost) -> bool {
    match decision {
        WslGatewayHost::Wsl(addr) => written_host != Some(addr.to_string().as_str()),
        WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable) => false,
        WslGatewayHost::Loopback(_) => written_host.is_some_and(|host| !is_loopback_host(host)),
    }
}

/// `http://172.20.0.1:57321/gw/codex/v1` → `172.20.0.1`。
pub fn url_host(url: &str) -> Option<&str> {
    let rest = url
        .trim()
        .split_once("://")
        .map_or(url.trim(), |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    if authority.starts_with('[') {
        return authority.split_once(']').map(|(host, _)| &host[1..]);
    }
    let host = authority
        .split_once(':')
        .map_or(authority, |(host, _)| host);
    (!host.is_empty()).then_some(host)
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|addr| addr.is_loopback())
}

/// Codex 桌面版 `config.toml` 里的「在 WSL 中运行」（纯函数）。
pub fn codex_runs_in_wsl_config(config: &str) -> bool {
    config
        .parse::<toml_edit::DocumentMut>()
        .ok()
        .and_then(|doc| {
            doc.get("desktop")?
                .get("runCodexInWindowsSubsystemForLinux")?
                .as_bool()
        })
        .unwrap_or(false)
}

/// 当前机器上 Codex 是否在 WSL 里运行：只在 Windows 上读设置，其它平台恒为 false。
pub fn codex_runs_in_wsl(codex_home: &Path) -> bool {
    cfg!(windows)
        && std::fs::read_to_string(codex_home.join("config.toml"))
            .is_ok_and(|text| codex_runs_in_wsl_config(&text))
}

/// `.wslconfig` 是否开了 mirrored 网络（纯函数）：`[wsl2]` 下 `networkingMode=mirrored`。
pub fn wslconfig_uses_mirrored_networking(text: &str) -> bool {
    let mut in_wsl2 = false;
    let mut mirrored = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_wsl2 = section.trim().eq_ignore_ascii_case("wsl2");
            continue;
        }
        if !in_wsl2 {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("networkingMode") {
            continue;
        }
        // 行尾注释与引号去掉；同一个键写了多次以最后一次为准。
        let value = value.split(['#', ';']).next().unwrap_or_default().trim();
        let value = value.trim_matches(|c| c == '"' || c == '\'');
        mirrored = value.eq_ignore_ascii_case("mirrored");
    }
    mirrored
}

/// 从网卡查询输出里挑出 WSL 虚拟网卡的 IPv4（纯函数）：每行一个地址，取第一个可用的。
pub fn parse_wsl_adapter_ipv4(output: &str) -> Option<Ipv4Addr> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<Ipv4Addr>().ok())
        .find(|addr| usable_adapter_address(*addr))
}

/// 查 Windows 上 `vEthernet (WSL...)` 网卡的地址。
///
/// 先确认上次拿到的地址还在本机（绑一个临时 UDP 端口，几乎没有开销）；不在了才调
/// PowerShell 重新查，并且最多一分钟查一次，避免 WSL 没启动时反复起进程。
fn wsl_adapter_ipv4() -> Option<Ipv4Addr> {
    const REQUERY_INTERVAL: Duration = Duration::from_secs(60);
    struct Cache {
        addr: Option<Ipv4Addr>,
        queried_at: Option<Instant>,
    }
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new(Cache {
            addr: None,
            queried_at: None,
        })
    });
    let mut cache = cache.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(addr) = cache.addr
        && std::net::UdpSocket::bind((addr, 0)).is_ok()
    {
        return Some(addr);
    }
    if cache
        .queried_at
        .is_some_and(|at| at.elapsed() < REQUERY_INTERVAL)
    {
        return None;
    }
    cache.queried_at = Some(Instant::now());
    cache.addr = query_wsl_adapter_output().and_then(|output| parse_wsl_adapter_ipv4(&output));
    cache.addr
}

#[cfg(windows)]
fn query_wsl_adapter_output() -> Option<String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    // 只输出地址本身，不依赖系统语言（ipconfig / netsh 的输出是本地化的）。
    const SCRIPT: &str = "Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | \
        Where-Object { $_.InterfaceAlias -like 'vEthernet (WSL*' } | \
        ForEach-Object { $_.IPAddress }";
    const TIMEOUT: Duration = Duration::from_secs(8);
    let powershell = std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .map(|root| {
            std::path::PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .unwrap_or_else(|| std::path::PathBuf::from("powershell.exe"));
    let mut child = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(crate::windows_create_no_window())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut output = String::new();
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    Some(output)
}

#[cfg(not(windows))]
fn query_wsl_adapter_output() -> Option<String> {
    None
}

fn wslconfig_mirrored() -> bool {
    std::env::var_os("USERPROFILE")
        .map(|home| Path::new(&home).join(".wslconfig"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|text| wslconfig_uses_mirrored_networking(&text))
}

/// 识别当前该用的网关主机；在 WSL 模式下确保 WSL 监听已经起来。
///
/// 返回 [`WslGatewayHost::Wsl`] 时该地址上的监听已就绪，可以写进 Codex 的配置；其余情况
/// （不是 Windows、Codex 不在 WSL 里、mirrored 网络、拿不到网卡、绑定失败）返回
/// [`WslGatewayHost::Loopback`] 并带上原因：新写配置时用回环地址，核对已写地址时
/// 按 [`codex_gateway_host_stale`] 决定改回回环还是保持现状。
/// 不在 WSL 模式时顺手关掉之前开的 WSL 监听。
pub fn codex_gateway_wsl_host(codex_home: &Path, port: u16) -> WslGatewayHost {
    let in_wsl = codex_runs_in_wsl(codex_home);
    let mirrored = in_wsl && wslconfig_mirrored();
    let adapter = if in_wsl && !mirrored {
        wsl_adapter_ipv4()
    } else {
        None
    };
    let decision = select_wsl_gateway_host(cfg!(windows), in_wsl, mirrored, adapter);
    let (decision, outcome) = match decision {
        WslGatewayHost::Wsl(addr) => match ensure_wsl_listener(addr, port) {
            Ok(()) => (decision, Ok(addr)),
            // 绑定失败时 ensure_wsl_listener 已经撤掉了旧监听：按回环处理，不保持现状。
            Err(error) => (
                WslGatewayHost::Loopback(LoopbackReason::BindFailed),
                Err(format!(
                    "{}: {error:#}",
                    LoopbackReason::BindFailed.as_str()
                )),
            ),
        },
        WslGatewayHost::Loopback(reason) => {
            if reason != LoopbackReason::AdapterUnavailable {
                stop_wsl_listener();
            }
            (decision, Err(reason.as_str().to_string()))
        }
    };
    log_outcome_change(port, &outcome);
    decision
}

/// 识别结果只在变化时记一次诊断日志：守护线程每半分钟查一次，不能刷屏。
fn log_outcome_change(port: u16, outcome: &Result<Ipv4Addr, String>) {
    static LAST: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    if !cfg!(windows) {
        return;
    }
    let key = match outcome {
        Ok(addr) => format!("wsl:{addr}:{port}"),
        Err(reason) => format!("loopback:{reason}"),
    };
    let mut last = LAST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if last.as_deref() == Some(key.as_str()) {
        return;
    }
    *last = Some(key);
    let detail = match outcome {
        Ok(addr) => serde_json::json!({ "host": addr.to_string(), "port": port }),
        Err(reason) => serde_json::json!({ "host": LOOPBACK_HOST, "port": port, "reason": reason }),
    };
    let _ = crate::diagnostic_log::append_diagnostic_log("gateway.wsl_host", detail);
}

/// WSL 监听绑定成功后写进诊断日志的排查提示。
pub const WSL_LISTENER_FIREWALL_HINT: &str = "已在 WSL 虚拟网卡上监听；如 WSL 内的 Codex 仍连不上网关，\
     检查 Windows 防火墙（含 Hyper-V 防火墙）是否放行 Codex++ 的入站连接";

struct WslListener {
    addr: Ipv4Addr,
    port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn listener_slot() -> &'static Mutex<Option<WslListener>> {
    static SLOT: OnceLock<Mutex<Option<WslListener>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// WSL 监听当前绑定的地址（诊断与测试用）。
pub fn wsl_listener_address() -> Option<(Ipv4Addr, u16)> {
    listener_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .map(|listener| (listener.addr, listener.port))
}

/// 确保 helper 在 `addr:port` 上有一个只放行网关路径的监听。幂等；地址或端口变了就换掉旧的。
pub fn ensure_wsl_listener(addr: Ipv4Addr, port: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        usable_adapter_address(addr),
        "拒绝在非 WSL 虚拟网卡地址 {addr} 上监听"
    );
    let mut slot = listener_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if slot
        .as_ref()
        .is_some_and(|listener| listener.addr == addr && listener.port == port)
    {
        return Ok(());
    }
    if let Some(previous) = slot.take() {
        shutdown(previous);
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("codex-plus-wsl-gateway".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(error.into()));
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::bind((addr, port)).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow::Error::from(error)
                            .context(format!("在 WSL 网卡 {addr}:{port} 上监听失败"))));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut stop_rx = stop_rx;
                loop {
                    tokio::select! {
                        _ = &mut stop_rx => break,
                        accepted = listener.accept() => {
                            if let Ok((stream, remote)) = accepted {
                                tokio::spawn(async move {
                                    let _ = crate::launcher::handle_helper_connection_on(
                                        stream,
                                        Some(remote),
                                        crate::launcher::HelperListener::Wsl,
                                    )
                                    .await;
                                });
                            }
                        }
                    }
                }
            });
        })
        .context("启动 WSL 网关监听线程失败")?;
    let ready = ready_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Err(anyhow::anyhow!("等待 WSL 网关监听 {addr}:{port} 超时")));
    let mut listener = WslListener {
        addr,
        port,
        stop: Some(stop_tx),
        thread: Some(thread),
    };
    match ready {
        Ok(()) => {
            // 这里只确认了 Windows 这边能绑定，WSL 能不能连进来还要看 Windows 防火墙；
            // 不做连通性探测，只在诊断日志里给出排查方向。
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.wsl_listener.listening",
                serde_json::json!({
                    "address": format!("{addr}:{port}"),
                    "hint": WSL_LISTENER_FIREWALL_HINT,
                }),
            );
            *slot = Some(listener);
            Ok(())
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.wsl_listener.bind_failed",
                serde_json::json!({ "address": format!("{addr}:{port}"), "message": format!("{error:#}") }),
            );
            if let Some(stop) = listener.stop.take() {
                let _ = stop.send(());
            }
            if let Some(thread) = listener.thread.take() {
                let _ = thread.join();
            }
            Err(error)
        }
    }
}

/// 关掉 WSL 监听（没有就什么都不做）。
pub fn stop_wsl_listener() {
    let previous = listener_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some(previous) = previous {
        shutdown(previous);
    }
}

fn shutdown(mut listener: WslListener) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "gateway.wsl_listener.stopping",
        serde_json::json!({ "address": format!("{}:{}", listener.addr, listener.port) }),
    );
    if let Some(stop) = listener.stop.take() {
        let _ = stop.send(());
    }
    if let Some(thread) = listener.thread.take() {
        let _ = thread.join();
    }
}

/// 管理器运行期间（仅 Windows）每半分钟核对一次：WSL 虚拟网卡在 Windows 开机后要等 WSL 启动才有，
/// 地址每次重启也可能变；Codex++ 往往比 WSL 先起来，启动时拿不到就靠这里补上。
/// 写进 Codex 的地址不对时重写托管供应商（只动开着网关、且没被别的工具接管的 Codex）。
pub fn start_codex_gateway_wsl_watch() {
    static STARTED: OnceLock<()> = OnceLock::new();
    if !cfg!(windows) || STARTED.set(()).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("codex-plus-wsl-gateway-watch".to_string())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                watch_tick();
            }
        });
}

fn watch_tick() {
    let codex_enabled = crate::gateway::store::GatewayStore::default()
        .load()
        .is_ok_and(|gateway| {
            gateway
                .active_binding(&crate::tools::ToolId::Codex)
                .is_some()
        });
    let Some(port) = crate::resident_helper::resident_helper_port() else {
        stop_wsl_listener();
        return;
    };
    if !codex_enabled {
        stop_wsl_listener();
        return;
    }
    let ctx = crate::gateway::apply::ApplyContext::default_paths(port);
    match crate::gateway::apply::sync_codex_wsl_endpoint(&ctx) {
        Ok(true) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.wsl_endpoint_rewritten",
                serde_json::json!({ "port": port }),
            );
        }
        Ok(false) => {}
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.wsl_endpoint_sync_failed",
                serde_json::json!({ "message": format!("{error:#}") }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WSL_IP: Ipv4Addr = Ipv4Addr::new(172, 29, 16, 1);

    #[test]
    fn listens_on_wsl_adapter_only_on_windows_in_wsl_mode() {
        assert_eq!(
            select_wsl_gateway_host(true, true, false, Some(WSL_IP)),
            WslGatewayHost::Wsl(WSL_IP)
        );
        assert_eq!(
            select_wsl_gateway_host(false, true, false, Some(WSL_IP)),
            WslGatewayHost::Loopback(LoopbackReason::NotWindows)
        );
        assert_eq!(
            select_wsl_gateway_host(true, false, false, Some(WSL_IP)),
            WslGatewayHost::Loopback(LoopbackReason::CodexNotInWsl)
        );
        assert_eq!(
            select_wsl_gateway_host(true, true, true, Some(WSL_IP)),
            WslGatewayHost::Loopback(LoopbackReason::MirroredNetworking)
        );
        assert_eq!(
            select_wsl_gateway_host(true, true, false, None),
            WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable)
        );
    }

    #[test]
    fn never_listens_on_wildcard_loopback_or_public_addresses() {
        for addr in [
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::LOCALHOST,
            Ipv4Addr::new(169, 254, 3, 4),
            Ipv4Addr::new(8, 8, 8, 8),
        ] {
            assert_eq!(
                select_wsl_gateway_host(true, true, false, Some(addr)),
                WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable),
                "{addr}"
            );
            assert!(ensure_wsl_listener(addr, 1).is_err(), "{addr}");
        }
    }

    #[test]
    fn codex_gateway_host_and_identity_follow_wsl_host() {
        assert_eq!(codex_gateway_host(None), "127.0.0.1");
        assert_eq!(codex_gateway_host(Some(WSL_IP)), "172.29.16.1");
        assert!(codex_gateway_uses_official_identity(true, None));
        assert!(!codex_gateway_uses_official_identity(true, Some(WSL_IP)));
        assert!(!codex_gateway_uses_official_identity(false, None));
    }

    #[test]
    fn wsl_listener_only_allows_gateway_paths() {
        for path in [
            "/gw/codex/v1/responses",
            "/gw/codex/v1/models",
            "/gw/claude/v1/messages?beta=true",
        ] {
            assert!(wsl_listener_allows(path), "{path}");
        }
        for path in [
            "/backend/status",
            "/proxy-status",
            "/settings/set",
            "/dictation/transcribe",
            "/v1/responses",
            "/v1/models",
            "/gwx/codex",
            "/gw/../settings/set",
            "/",
            "",
        ] {
            assert!(!wsl_listener_allows(path), "{path}");
        }
    }

    #[test]
    fn stale_host_detection_keeps_status_quo_only_while_adapter_is_missing() {
        let wsl = WslGatewayHost::Wsl(WSL_IP);
        // 拿到 WSL 地址：写的是回环或旧地址就重写，一致就不动。
        assert!(codex_gateway_host_stale(Some("127.0.0.1"), wsl));
        assert!(codex_gateway_host_stale(Some("172.18.0.1"), wsl));
        assert!(!codex_gateway_host_stale(Some("172.29.16.1"), wsl));
        // WSL 还没启动（暂时拿不到网卡）：不动，等下一轮。
        let pending = WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable);
        assert!(!codex_gateway_host_stale(Some("172.18.0.1"), pending));
        assert!(!codex_gateway_host_stale(Some("127.0.0.1"), pending));
        // 关掉了「在 WSL 中运行」、`.wslconfig` 改成 mirrored、WSL 网卡上绑定失败、不是 Windows：
        // WSL 监听已经不在，写过的 172.x 地址改回回环；本来就是回环不动。
        for reason in [
            LoopbackReason::CodexNotInWsl,
            LoopbackReason::MirroredNetworking,
            LoopbackReason::BindFailed,
            LoopbackReason::NotWindows,
        ] {
            let loopback = WslGatewayHost::Loopback(reason);
            assert!(
                codex_gateway_host_stale(Some("172.18.0.1"), loopback),
                "{reason:?}"
            );
            assert!(
                !codex_gateway_host_stale(Some("127.0.0.1"), loopback),
                "{reason:?}"
            );
            assert!(
                !codex_gateway_host_stale(Some("localhost"), loopback),
                "{reason:?}"
            );
            assert!(!codex_gateway_host_stale(None, loopback), "{reason:?}");
        }
    }

    #[test]
    fn gateway_host_address_only_for_wsl_listener() {
        assert_eq!(WslGatewayHost::Wsl(WSL_IP).address(), Some(WSL_IP));
        assert_eq!(
            WslGatewayHost::Loopback(LoopbackReason::BindFailed).address(),
            None
        );
        assert_eq!(
            codex_gateway_host(WslGatewayHost::Wsl(WSL_IP).address()),
            "172.29.16.1"
        );
        assert_eq!(
            codex_gateway_host(
                WslGatewayHost::Loopback(LoopbackReason::AdapterUnavailable).address()
            ),
            LOOPBACK_HOST
        );
    }

    #[test]
    fn listener_firewall_hint_points_to_windows_firewall() {
        assert!(WSL_LISTENER_FIREWALL_HINT.contains("Windows 防火墙"));
        assert!(WSL_LISTENER_FIREWALL_HINT.contains("Codex++"));
        assert!(!WSL_LISTENER_FIREWALL_HINT.contains("  "));
    }

    #[test]
    fn url_host_extracts_host() {
        assert_eq!(
            url_host("http://127.0.0.1:57321/gw/codex/v1"),
            Some("127.0.0.1")
        );
        assert_eq!(
            url_host("http://172.29.16.1:57321/gw/codex/v1"),
            Some("172.29.16.1")
        );
        assert_eq!(url_host("http://[::1]:57321/v1"), Some("::1"));
        assert_eq!(
            url_host("https://api.example.com/v1"),
            Some("api.example.com")
        );
        assert_eq!(url_host(""), None);
    }

    #[test]
    fn reads_codex_desktop_wsl_setting() {
        assert!(codex_runs_in_wsl_config(
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = true\n"
        ));
        assert!(!codex_runs_in_wsl_config(
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = false\n"
        ));
        assert!(!codex_runs_in_wsl_config(
            "runCodexInWindowsSubsystemForLinux = true\n"
        ));
        assert!(!codex_runs_in_wsl_config("not toml ["));
        assert!(!codex_runs_in_wsl_config(""));
    }

    #[test]
    fn detects_mirrored_networking_in_wslconfig() {
        assert!(wslconfig_uses_mirrored_networking(
            "[wsl2]\nnetworkingMode=mirrored\n"
        ));
        assert!(wslconfig_uses_mirrored_networking(
            "# 注释\n[WSL2]\nmemory=8GB\n  NetworkingMode = \"Mirrored\" # 镜像\n"
        ));
        assert!(!wslconfig_uses_mirrored_networking(
            "[wsl2]\nnetworkingMode=nat\n"
        ));
        assert!(!wslconfig_uses_mirrored_networking(
            "[experimental]\nnetworkingMode=mirrored\n"
        ));
        assert!(!wslconfig_uses_mirrored_networking(
            "[wsl2]\n#networkingMode=mirrored\n"
        ));
        assert!(!wslconfig_uses_mirrored_networking(""));
    }

    #[test]
    fn parses_adapter_address_from_query_output() {
        assert_eq!(parse_wsl_adapter_ipv4("172.29.16.1\r\n"), Some(WSL_IP));
        assert_eq!(
            parse_wsl_adapter_ipv4("169.254.10.2\n\n  172.29.16.1  \n192.168.1.5\n"),
            Some(WSL_IP)
        );
        assert_eq!(parse_wsl_adapter_ipv4(""), None);
        assert_eq!(
            parse_wsl_adapter_ipv4("garbage\n0.0.0.0\n127.0.0.1\n"),
            None
        );
    }
}
