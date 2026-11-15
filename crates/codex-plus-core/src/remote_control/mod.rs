//! 远程控制：从浏览器（之后是手机）操控 Codex 桌面里的会话。
//!
//! 链路（调研与 P0 实测见 docs/reports/2026-11-15-remote-control-research.md）：
//! - Codex 桌面主进程的 IPC 路由提供 follower 协议：跟随会话、发消息、插话、中断、审批、回答提问；
//! - 侧车 app-server 只读会话列表与历史、新建会话（建完放掉写锁）；
//! - 没在桌面打开的会话经 CDP 让桌面打开（`navigate-to-route`），桌面成为 owner 后再跟随。
//!
//! 本模块负责开关、端口、令牌的持久化与服务的启停。服务跑在独立线程的 tokio 运行时里，
//! 由管理器进程托管：管理器开着、开关打开时在线。P1 只监听 127.0.0.1。

pub mod follower;
pub mod ipc;
pub mod patch;
pub mod server;
pub mod service;
pub mod sidecar;
pub mod versions;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DEFAULT_PORT: u16 = 57360;
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
}

fn default_port() -> u16 {
    DEFAULT_PORT
}

impl Default for RemoteControlState {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_PORT, token: String::new() }
    }
}

pub fn state_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(STATE_FILE)
}

pub fn load_state_from(path: &std::path::Path) -> RemoteControlState {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
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

/// 64 位十六进制随机令牌。
pub fn generate_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

pub fn page_url(state: &RemoteControlState) -> String {
    format!("http://127.0.0.1:{}/#token={}", state.port, state.token)
}

struct Running {
    port: u16,
    token: String,
    stop: tokio::sync::oneshot::Sender<()>,
    thread: std::thread::JoinHandle<()>,
}

fn running_slot() -> &'static Mutex<Option<Running>> {
    static SLOT: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn last_error_slot() -> &'static Mutex<Option<String>> {
    static SLOT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

fn set_last_error(error: Option<String>) {
    *last_error_slot().lock().unwrap_or_else(|e| e.into_inner()) = error;
}

/// 按给定端口与令牌启动服务；已在同一端口与令牌上运行时什么也不做。
pub fn start(port: u16, token: &str) -> anyhow::Result<()> {
    let mut slot = running_slot().lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|running| running.port == port && running.token == token) {
        return Ok(());
    }
    if let Some(running) = slot.take() {
        stop_running(running);
    }
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let token_for_thread = token.to_string();
    let thread = std::thread::Builder::new()
        .name(THREAD_NAME.into())
        .spawn(move || {
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
                let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(anyhow::anyhow!("监听 127.0.0.1:{port} 失败：{error}")));
                        return;
                    }
                };
                let service = service::RemoteService::new(service::ServiceConfig::detect());
                let context = Arc::new(server::ServerContext { service: service.clone(), token: token_for_thread, port });
                let _ = ready_tx.send(Ok(()));
                tokio::select! {
                    _ = server::serve(listener, context) => {}
                    _ = stop_rx => {}
                }
                service.shutdown().await;
            });
        })
        .context("无法创建远程控制线程")?;
    match ready_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => {
            *slot = Some(Running { port, token: token.to_string(), stop: stop_tx, thread });
            set_last_error(None);
            let _ = crate::diagnostic_log::append_diagnostic_log("remote_control.started", json!({ "port": port }));
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
    let _ = crate::diagnostic_log::append_diagnostic_log("remote_control.stopped", json!({ "port": running.port }));
}

pub fn stop() {
    let running = running_slot().lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(running) = running {
        stop_running(running);
    }
}

pub fn running_port() -> Option<u16> {
    running_slot().lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|running| running.port)
}

/// 管理器启动时调用：开关打开就把服务拉起来。
pub fn start_from_saved_state() {
    let mut state = load_state_from(&state_path());
    if !state.enabled {
        return;
    }
    if state.token.is_empty() {
        state.token = generate_token();
        let _ = save_state_to(&state_path(), &state);
    }
    if let Err(error) = start(state.port, &state.token) {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "remote_control.start_failed",
            json!({ "port": state.port, "message": error.to_string() }),
        );
    }
}

/// 打开 / 关闭远程控制。打开时没有令牌就生成一个。
pub fn set_enabled(enabled: bool) -> anyhow::Result<Value> {
    let path = state_path();
    let mut state = load_state_from(&path);
    state.enabled = enabled;
    if enabled && state.token.is_empty() {
        state.token = generate_token();
    }
    save_state_to(&path, &state)?;
    if enabled {
        start(state.port, &state.token)?;
    } else {
        stop();
    }
    Ok(status())
}

/// 换一个新令牌：之前打开的页面立刻失效。
pub fn reset_token() -> anyhow::Result<Value> {
    let path = state_path();
    let mut state = load_state_from(&path);
    state.token = generate_token();
    save_state_to(&path, &state)?;
    if state.enabled {
        start(state.port, &state.token)?;
    }
    Ok(status())
}

pub fn status() -> Value {
    let state = load_state_from(&state_path());
    let running = running_port();
    let last_error = last_error_slot().lock().unwrap_or_else(|e| e.into_inner()).clone();
    json!({
        "enabled": state.enabled,
        "running": running.is_some(),
        "port": state.port,
        "url": (state.enabled && running.is_some() && !state.token.is_empty()).then(|| page_url(&state)),
        "error": last_error,
    })
}
