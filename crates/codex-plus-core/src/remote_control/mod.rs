//! 远程控制：从浏览器（之后是手机）操控 Codex 桌面里的会话。
//!
//! 链路（调研与 P0 实测见 docs/reports/2026-11-15-remote-control-research.md）：
//! - Codex 桌面主进程的 IPC 路由提供 follower 协议：跟随会话、发消息、插话、中断、审批、回答提问；
//! - 侧车 app-server 只读会话列表与历史、新建会话（建完放掉写锁）；
//! - 没在桌面打开的会话经 CDP 让桌面打开（`navigate-to-route`），桌面成为 owner 后再跟随。
//!
//! 本模块负责开关、端口、令牌的持久化与服务的启停。服务跑在独立线程的 tokio 运行时里，
//! 由管理器进程托管：管理器开着、开关打开时在线。
//!
//! 两个入口共用一个服务：
//! - 本机回环 `127.0.0.1:port`（HTTP），认主令牌与设备令牌；
//! - 局域网 / Tailscale `0.0.0.0:lan_port`（自签 HTTPS，默认关），只认配对过的设备令牌。

pub mod devices;
pub mod follower;
pub mod headless;
pub mod ipc;
pub mod jiajia;
pub mod mirror;
pub mod patch;
pub mod server;
pub mod service;
pub mod sidecar;
pub mod tls;
pub mod uploads;
pub mod versions;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_PORT: u16 = 57360;
pub const DEFAULT_LAN_PORT: u16 = 57361;
const STATE_FILE: &str = "remote-control.json";
const THREAD_NAME: &str = "codex-plus-remote-control";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteControlState {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub token: String,
    /// 允许局域网 / Tailscale 上配对过的设备访问。
    #[serde(default)]
    pub lan_enabled: bool,
    #[serde(default = "default_lan_port")]
    pub lan_port: u16,
    #[serde(default)]
    pub devices: Vec<devices::Device>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

fn default_lan_port() -> u16 {
    DEFAULT_LAN_PORT
}

impl Default for RemoteControlState {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: String::new(),
            lan_enabled: false,
            lan_port: DEFAULT_LAN_PORT,
            devices: Vec::new(),
        }
    }
}

fn state_path_override() -> &'static Mutex<Option<PathBuf>> {
    static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);
    &OVERRIDE
}

/// 测试用：把状态文件（及同目录的证书）挪到临时目录；同时清掉内存里的设备表。
pub fn set_state_path_for_tests(path: Option<PathBuf>) {
    *state_path_override().lock().unwrap_or_else(|e| e.into_inner()) = path;
    devices::reload();
}

pub fn state_path() -> PathBuf {
    if let Some(path) = state_path_override().lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return path;
    }
    crate::paths::default_app_state_dir().join(STATE_FILE)
}

pub fn load_state_from(path: &std::path::Path) -> RemoteControlState {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// 桌面镜像要 Codex 在后台、被别的窗口挡住时照常出画面。Chromium 默认会：
/// 给被完全遮挡的窗口停帧 / 降帧、给后台渲染进程降优先级、节流后台计时器。焦点模拟（见 `mirror`）只骗过页面，
/// 管不到窗口遮挡这一层，所以从 Codex++ 启动 Codex 时就带上这三个开关。
pub const MIRROR_LAUNCH_ARGS: &[&str] = &[
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
];

/// 已有的不重复加（用户可能自己在「Codex 额外启动参数」里写过）。
pub fn append_mirror_launch_args(args: &mut Vec<String>) {
    for flag in MIRROR_LAUNCH_ARGS {
        if !args.iter().any(|arg| arg == flag) {
            args.push((*flag).to_string());
        }
    }
}

/// 远程控制开着才加：这些开关让 Codex 在后台也照常耗电，没在用远程控制就不必。
pub fn with_mirror_launch_args(mut args: Vec<String>) -> Vec<String> {
    if load_state_from(&state_path()).enabled {
        append_mirror_launch_args(&mut args);
    }
    args
}

pub fn save_state_to(path: &std::path::Path, state: &RemoteControlState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(path, &serde_json::to_vec_pretty(state)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 读改写状态文件（串行化，设备表与开关可能同时在改）。
pub fn update_state<R>(change: impl FnOnce(&mut RemoteControlState) -> R) -> anyhow::Result<(RemoteControlState, R)> {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = state_path();
    let mut state = load_state_from(&path);
    let result = change(&mut state);
    save_state_to(&path, &state)?;
    Ok((state, result))
}

/// 这台电脑给人看的名字（手机顶栏显示「xxx 的 MacBook Pro」那种）：macOS 取「电脑名称」，
/// Windows 取 COMPUTERNAME，其它取主机名。只算一次。
pub fn host_display_name() -> String {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("scutil").args(["--get", "ComputerName"]).output() {
                let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() && !name.is_empty() {
                    return name;
                }
            }
        }
        std::env::var("COMPUTERNAME")
            .ok()
            .filter(|name| !name.trim().is_empty())
            .or_else(|| tls::local_hostname().map(|name| name.trim_end_matches(".local").to_string()))
            .unwrap_or_else(|| "这台电脑".to_string())
    })
    .clone()
}

/// 64 位十六进制随机令牌。
pub fn generate_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub fn page_url(state: &RemoteControlState) -> String {
    format!("http://127.0.0.1:{}/#token={}", state.port, state.token)
}

/// 一次启动用的监听参数。
#[derive(Clone)]
pub struct ListenOptions {
    pub port: u16,
    pub token: String,
    pub lan: Option<LanOptions>,
}

#[derive(Clone)]
pub struct LanOptions {
    pub port: u16,
    pub tls: Arc<rustls::ServerConfig>,
    pub fingerprint: String,
}

impl ListenOptions {
    fn same_as(&self, other: &ListenOptions) -> bool {
        self.port == other.port
            && self.token == other.token
            && match (&self.lan, &other.lan) {
                (None, None) => true,
                (Some(a), Some(b)) => a.port == b.port && a.fingerprint == b.fingerprint,
                _ => false,
            }
    }
}

struct Running {
    options: ListenOptions,
    /// 局域网入口实际是否在听（端口被占时只有本机入口）。
    lan_listening: bool,
    stop: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

fn running_slot() -> &'static Mutex<Option<Running>> {
    static SLOT: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 同一进程里的其它模块（微信连接里的加加）直接用远程控制服务，不绕 HTTP。
///
/// 服务对象只活在监听线程自己的 tokio 运行时里：这里登记它的弱引用和那个运行时的句柄，
/// 调用方把要做的事 `spawn` 到这个运行时上跑（加加内部会 `tokio::spawn` 后台任务，
/// 必须落在服务自己的运行时里，否则调用方的运行时一停这些任务就跟着没了）。
/// 服务启动时登记、停止时清掉；只存弱引用，不会拖住服务的释放。
#[derive(Clone)]
struct HostedService {
    service: std::sync::Weak<service::RemoteService>,
    runtime: tokio::runtime::Handle,
}

fn hosted_slot() -> &'static Mutex<Option<HostedService>> {
    static SLOT: OnceLock<Mutex<Option<HostedService>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 在当前运行时（监听线程里）登记服务。
fn register_hosted(service: &Arc<service::RemoteService>) {
    *hosted_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(HostedService {
        service: Arc::downgrade(service),
        runtime: tokio::runtime::Handle::current(),
    });
}

/// 只清掉自己登记的那一份：重启时新服务可能已经登记上了。
fn unregister_hosted(service: &Arc<service::RemoteService>) {
    let mut slot = hosted_slot().lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|hosted| std::ptr::eq(hosted.service.as_ptr(), Arc::as_ptr(service))) {
        *slot = None;
    }
}

/// 正在运行的远程控制服务和它所在的运行时；没开（或正在停）时返回 None。
pub fn hosted_service() -> Option<(Arc<service::RemoteService>, tokio::runtime::Handle)> {
    let slot = hosted_slot().lock().unwrap_or_else(|e| e.into_inner());
    let hosted = slot.as_ref()?;
    Some((hosted.service.upgrade()?, hosted.runtime.clone()))
}

fn last_error_slot() -> &'static Mutex<Option<String>> {
    static SLOT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn lan_error_slot() -> &'static Mutex<Option<String>> {
    static SLOT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn set_last_error(error: Option<String>) {
    *last_error_slot().lock().unwrap_or_else(|e| e.into_inner()) = error;
}

fn set_lan_error(error: Option<String>) {
    *lan_error_slot().lock().unwrap_or_else(|e| e.into_inner()) = error;
}

/// 只开本机入口（测试与 example 用）。
pub fn start(port: u16, token: &str) -> anyhow::Result<()> {
    start_with(ListenOptions { port, token: token.to_string(), lan: None })
}

/// 按给定参数启动服务；参数没变时什么也不做。
pub fn start_with(options: ListenOptions) -> anyhow::Result<()> {
    let mut slot = running_slot().lock().unwrap_or_else(|e| e.into_inner());
    // 参数没变、且局域网入口（若要）确实在听时不用重启；上次端口被占就再试一次。
    if slot
        .as_ref()
        .is_some_and(|running| running.options.same_as(&options) && (options.lan.is_none() || running.lan_listening))
    {
        return Ok(());
    }
    if let Some(running) = slot.take() {
        stop_running(running);
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<bool>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let thread_options = options.clone();
    let thread = std::thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
            let options = thread_options;
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
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
                let port = options.port;
                let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("监听 127.0.0.1:{port} 失败：{error}")));
                        return;
                    }
                };
                let lan_listener = match &options.lan {
                    Some(lan) => match tokio::net::TcpListener::bind(("0.0.0.0", lan.port)).await {
                        Ok(listener) => {
                            set_lan_error(None);
                            Some((listener, tokio_rustls::TlsAcceptor::from(lan.tls.clone()), lan.port))
                        }
                        Err(error) => {
                            set_lan_error(Some(format!("监听局域网端口 {} 失败：{error}", lan.port)));
                            None
                        }
                    },
                    None => None,
                };
                let service = service::RemoteService::new(service::ServiceConfig::detect());
                // 加加的工具经本机入口转回来：先告诉它端口，再起后台循环（没配置过就什么也不做）。
                let jiajia = service.jiajia();
                jiajia.set_port(port);
                jiajia.start();
                register_hosted(&service);
                let local = Arc::new(server::ServerContext {
                    service: service.clone(),
                    token: options.token.clone(),
                    port,
                    scope: server::Scope::Loopback,
                });
                let _ = ready_tx.send(Ok(lan_listener.is_some()));
                let lan_task = async {
                    match lan_listener {
                        Some((listener, acceptor, lan_port)) => {
                            let context = Arc::new(server::ServerContext {
                                service: service.clone(),
                                token: String::new(),
                                port: lan_port,
                                scope: server::Scope::Lan,
                            });
                            server::serve_tls(listener, acceptor, context).await
                        }
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = server::serve(listener, local) => {}
                    _ = lan_task => {}
                    _ = stop_rx => {}
                }
                unregister_hosted(&service);
                service.shutdown().await;
            });
        })
        .context("无法创建远程控制线程")?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(lan_listening)) => {
            let lan_port = options.lan.as_ref().filter(|_| lan_listening).map(|lan| lan.port);
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "remote_control.started",
                json!({ "port": options.port, "lanPort": lan_port }),
            );
            *slot = Some(Running { options, lan_listening, stop: stop_tx, thread });
            set_last_error(None);
            Ok(())
        }
        Ok(Err(error)) => {
            let _ = thread.join();
            set_last_error(Some(error.to_string()));
            Err(error)
        }
        Err(_) => {
            set_last_error(Some("远程控制服务启动超时".into()));
            anyhow::bail!("远程控制服务启动超时")
        }
    }
}

fn stop_running(running: Running) {
    let _ = running.stop.send(());
    let _ = running.thread.join();
    let _ = crate::diagnostic_log::append_diagnostic_log("remote_control.stopped", json!({ "port": running.options.port }));
}

pub fn stop() {
    let running = running_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(running) = running {
        stop_running(running);
    }
}

pub fn running_port() -> Option<u16> {
    running_slot().lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|running| running.options.port)
}

/// 局域网入口正在监听的端口。
pub fn running_lan_port() -> Option<u16> {
    running_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|running| running.lan_listening)
        .and_then(|running| running.options.lan.as_ref().map(|lan| lan.port))
}

/// 按状态文件组装监听参数；局域网打开时准备好证书。
fn options_from_state(state: &RemoteControlState) -> anyhow::Result<ListenOptions> {
    let lan = if state.lan_enabled {
        let certificate = tls::ensure_certificate_at(&tls::tls_path(), &tls::local_addresses())?;
        Some(LanOptions { port: state.lan_port, tls: tls::server_config(&certificate)?, fingerprint: certificate.fingerprint() })
    } else {
        None
    };
    Ok(ListenOptions { port: state.port, token: state.token.clone(), lan })
}

/// 按状态文件启停服务。
fn apply_state(state: &RemoteControlState) -> anyhow::Result<()> {
    if !state.enabled {
        stop();
        return Ok(());
    }
    set_lan_error(None);
    // 证书出问题只影响局域网入口，本机入口照常起。
    let options = options_from_state(state).unwrap_or_else(|error| {
        set_lan_error(Some(format!("准备局域网证书失败：{error:#}")));
        ListenOptions { port: state.port, token: state.token.clone(), lan: None }
    });
    start_with(options)
}

/// 管理器启动时调用：开关打开就把服务拉起来。
pub fn start_from_saved_state() {
    let state = match update_state(|state| {
        if state.enabled && state.token.is_empty() {
            state.token = generate_token();
        }
    }) {
        Ok((state, ())) => state,
        Err(_) => load_state_from(&state_path()),
    };
    if !state.enabled {
        return;
    }
    if let Err(error) = apply_state(&state) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "remote_control.start_failed",
            json!({ "port": state.port, "message": error.to_string() }),
        );
    }
}

/// 打开 / 关闭远程控制。打开时没有令牌就生成一个。
pub fn set_enabled(enabled: bool) -> anyhow::Result<Value> {
    let (state, ()) = update_state(|state| {
        state.enabled = enabled;
        if enabled && state.token.is_empty() {
            state.token = generate_token();
        }
    })?;
    apply_state(&state)?;
    Ok(status())
}

/// 打开 / 关闭局域网访问（只影响配对设备，本机入口不变）。
pub fn set_lan_enabled(enabled: bool) -> anyhow::Result<Value> {
    let (state, ()) = update_state(|state| state.lan_enabled = enabled)?;
    if !enabled {
        devices::cancel_pairing();
    }
    apply_state(&state)?;
    Ok(status())
}

/// 换一个新令牌：之前打开的页面立刻失效（配对设备不受影响）。
pub fn reset_token() -> anyhow::Result<Value> {
    let (state, ()) = update_state(|state| state.token = generate_token())?;
    apply_state(&state)?;
    Ok(status())
}

/// 局域网可用地址与对应的网址。
fn lan_addresses(port: u16) -> Vec<Value> {
    tls::local_addresses()
        .into_iter()
        .map(|ip| {
            json!({
                "address": ip.to_string(),
                "kind": if tls::is_tailscale(&ip) { "tailscale" } else { "lan" },
                "url": format!("https://{ip}:{port}/"),
            })
        })
        .collect()
}

/// 开始配对：生成配对码，返回带二维码的状态。
pub fn start_pairing(role: &str) -> anyhow::Result<Value> {
    let role = devices::Role::parse(role)?;
    if running_lan_port().is_none() {
        anyhow::bail!("先打开局域网访问");
    }
    devices::start_pairing(role);
    Ok(status())
}

pub fn cancel_pairing() -> Value {
    devices::cancel_pairing();
    status()
}

pub fn set_device_role(id: &str, role: &str) -> anyhow::Result<Value> {
    devices::set_role(id, devices::Role::parse(role)?)?;
    Ok(status())
}

pub fn revoke_device(id: &str) -> anyhow::Result<Value> {
    devices::revoke(id)?;
    let _ = crate::diagnostic_log::append_diagnostic_log("remote_control.device_revoked", json!({ "id": id }));
    Ok(status())
}

/// 管理器与 Codex 页面共用的动作分发：`status`、`enable`、`disable`、`resetToken`、`enableLan`、`disableLan`、
/// `startPairing {role}`、`cancelPairing`、`setDeviceRole {id, role}`、`revokeDevice {id}`。
pub fn dispatch(action: &str, args: &Value) -> anyhow::Result<Value> {
    let arg = |key: &str| -> anyhow::Result<&str> {
        args.get(key).and_then(Value::as_str).with_context(|| format!("缺少参数 {key}"))
    };
    match action {
        "status" => Ok(status()),
        "enable" => set_enabled(true),
        "disable" => set_enabled(false),
        "resetToken" => reset_token(),
        "enableLan" => set_lan_enabled(true),
        "disableLan" => set_lan_enabled(false),
        "startPairing" => start_pairing(arg("role")?),
        "cancelPairing" => Ok(cancel_pairing()),
        "setDeviceRole" => set_device_role(arg("id")?, arg("role")?),
        "revokeDevice" => revoke_device(arg("id")?),
        other => anyhow::bail!("未知操作：{other}"),
    }
}

/// 服务由管理器进程托管：管理器启动时标记。Codex 页面经 bridge 调用时，若 bridge 跑在别的进程
/// （没有管理器），就不能在那里起服务——否则和管理器抢端口、开关状态两边对不上。
static HOSTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn mark_hosted_by_this_process() {
    HOSTED.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Codex 页面（`46-remote-control-page.js`）的 bridge 路由 `/remote-control`：`{ action, ...args }`。
/// 不在拓展路由白名单里：能开关服务、发配对码、移除设备。
pub fn bridge_route(payload: &Value) -> anyhow::Result<Value> {
    if !HOSTED.load(std::sync::atomic::Ordering::SeqCst) {
        anyhow::bail!("Codex++ 管理器未运行，暂时不能在 Codex 里操作远程控制；请先打开 Codex++。");
    }
    let action = payload.get("action").and_then(Value::as_str).unwrap_or("status");
    let mut value = dispatch(action, payload)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("status".into(), json!("ok"));
    }
    Ok(value)
}

fn render_qr_svg(content: &str) -> Option<String> {
    let code = qrcode::QrCode::new(content.as_bytes()).ok()?;
    Some(
        code.render::<qrcode::render::svg::Color>()
            .min_dimensions(220, 220)
            .dark_color(qrcode::render::svg::Color("#111827"))
            .light_color(qrcode::render::svg::Color("#ffffff"))
            .build(),
    )
}

pub fn status() -> Value {
    let state = load_state_from(&state_path());
    let running = running_port();
    let lan_port = running_lan_port();
    let last_error = last_error_slot().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let lan_error = lan_error_slot().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let addresses = lan_port.map(lan_addresses).unwrap_or_default();
    let fingerprint = lan_port.and_then(|_| tls::load_from(&tls::tls_path())).map(|cert| cert.fingerprint());
    let pairing = lan_port.and(devices::current_pairing()).map(|pairing| {
        let links: Vec<Value> = addresses
            .iter()
            .filter_map(|address| {
                let url = format!("{}#pair={}", address["url"].as_str()?, pairing.code);
                Some(json!({ "address": address["address"], "kind": address["kind"], "url": url, "qrSvg": render_qr_svg(&url) }))
            })
            .collect();
        json!({ "code": pairing.code, "role": pairing.role.as_str(), "expiresAt": pairing.expires_at, "links": links })
    });
    json!({
        "enabled": state.enabled,
        "running": running.is_some(),
        "port": state.port,
        "url": (state.enabled && running.is_some() && !state.token.is_empty()).then(|| page_url(&state)),
        "error": last_error,
        "lan": {
            "enabled": state.lan_enabled,
            "running": lan_port.is_some(),
            "port": state.lan_port,
            "addresses": addresses,
            "fingerprint": fingerprint,
            "error": if state.lan_enabled && state.enabled { lan_error } else { None },
        },
        "pairing": pairing,
        "devices": devices::list().iter().map(devices::Device::public).collect::<Vec<_>>(),
    })
}
