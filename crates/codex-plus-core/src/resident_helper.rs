//! 进程内常驻的本地协议代理 helper。
//!
//! 背景：协议转换 / 聚合 / 请求避震等能力依赖 `config.toml` 里写死的本地代理
//! 地址（默认 `http://127.0.0.1:57321/v1`）。helper 过去只在
//! `launch_and_inject` 里随 Codex 会话启动，Codex 一退出就关闭；于是「不从
//! Codex++ 启动 Codex」（终端 CLI、IDE 插件、直接双击官方 App）时，config 指向
//! 的端口没人监听，请求直接连不上。
//!
//! 本模块让管理器在运行期托管一个常驻 helper：
//! - Codex++ 启动 / 恢复 / 供应商切换后，只要 live `config.toml` 指向本地代理，
//!   就把 helper 拉起来（端口取 `protocol_proxy::protocol_proxy_port()`）；
//! - Codex 会话结束不关闭，只有 Codex++ 退出时才停止；
//! - 启动链路里的 `DefaultLaunchHooks::start_helper` 会识别常驻 helper 并复用，
//!   避免二次 bind 同一个端口失败。
//!
//! 因此只要 Codex++ 在跑，config.toml 里的本地代理地址就是可用的，与 Codex
//! 由谁启动无关。

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::Context;

use crate::launcher::{DefaultLaunchHooks, LaunchHooks};

/// 等待常驻 helper 线程确认「已绑定（或绑定失败）」的上限。
///
/// bind 本身是立即返回的，这里只防线程调度极端卡死，不作为端口等待逻辑。
const START_TIMEOUT: Duration = Duration::from_secs(10);

const RESIDENT_HELPER_THREAD: &str = "codex-plus-resident-helper";

/// 管理器重启/更新后旧进程可能还没放开监听：短暂重试等它让位，而不是直接
/// 放弃常驻（否则这次启动会退回「会话 helper」，Codex 退出后又没人监听了）。
const ADDRESS_IN_USE_RETRY_ATTEMPTS: u32 = 5;
const ADDRESS_IN_USE_RETRY_INTERVAL: Duration = Duration::from_millis(200);

struct ResidentHelper {
    port: u16,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn resident_slot() -> &'static Mutex<Option<ResidentHelper>> {
    static SLOT: OnceLock<Mutex<Option<ResidentHelper>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// 串行化 ensure：并发调用（多个管理器命令同时触发）时只让一个线程去 bind，
/// 否则后到的线程会拿到 AddrInUse 并误报失败。
fn ensure_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn lock_slot() -> std::sync::MutexGuard<'static, Option<ResidentHelper>> {
    resident_slot()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// 常驻 helper 当前监听的端口；没有常驻 helper 时返回 `None`。
pub fn resident_helper_port() -> Option<u16> {
    lock_slot().as_ref().map(|helper| helper.port)
}

/// 确保常驻 helper 监听在 `port` 上。
///
/// 幂等：已经监听同一端口时直接返回成功；已在其它端口时先停旧的再起新的。
/// 端口被「上一个 Codex++ 进程」短暂占用时会小窗口重试；被其它程序持续占用时
/// 返回错误，由调用方决定是否降级（不在这里静默换端口，因为该端口已经写进
/// `config.toml` 的 `base_url`）。
pub fn ensure_resident_helper(port: u16) -> anyhow::Result<()> {
    let _ensure_guard = ensure_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if resident_helper_port() == Some(port) {
        return Ok(());
    }
    if let Some(previous) = { lock_slot().take() } {
        stop_helper(previous, "replace");
    }
    let mut attempt = 0u32;
    loop {
        match start_helper(port) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < ADDRESS_IN_USE_RETRY_ATTEMPTS
                    && crate::launcher::error_is_address_in_use(&error) =>
            {
                attempt += 1;
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "resident_helper.bind_retry",
                    serde_json::json!({ "port": port, "attempt": attempt }),
                );
                std::thread::sleep(ADDRESS_IN_USE_RETRY_INTERVAL);
            }
            Err(error) => return Err(error),
        }
    }
}

/// 停止常驻 helper。Codex++ 退出时调用；Codex 会话结束不应调用。
pub fn stop_resident_helper() {
    let helper = { lock_slot().take() };
    if let Some(helper) = helper {
        stop_helper(helper, "shutdown");
    }
}

fn start_helper(port: u16) -> anyhow::Result<()> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::Builder::new()
        .name(RESIDENT_HELPER_THREAD.to_string())
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
                let hooks = std::sync::Arc::new(DefaultLaunchHooks::default());
                if let Err(error) = hooks.start_helper(port).await {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                // 只有收到停止信号才结束；Codex 会话生命周期与本线程无关。
                let _ = stop_rx.await;
                hooks.shutdown_helper(port).await;
            });
        })
        .context("启动常驻 helper 线程失败")?;

    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => {
            let mut slot = lock_slot();
            if slot.is_some() {
                // 正常不会发生（ensure 已串行化），防御性收尾避免泄漏线程。
                drop(slot);
                let _ = stop_tx.send(());
                let _ = thread.join();
                anyhow::bail!("常驻 helper 已被其他线程占用");
            }
            *slot = Some(ResidentHelper {
                port,
                stop: Some(stop_tx),
                thread: Some(thread),
            });
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "resident_helper.listening",
                serde_json::json!({ "port": port }),
            );
            Ok(())
        }
        Ok(Err(error)) => {
            let _ = thread.join();
            Err(error)
        }
        Err(_) => {
            let _ = stop_tx.send(());
            let _ = thread.join();
            anyhow::bail!("等待常驻 helper 绑定端口 {port} 超时")
        }
    }
}

fn stop_helper(mut helper: ResidentHelper, reason: &str) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "resident_helper.stopping",
        serde_json::json!({ "port": helper.port, "reason": reason }),
    );
    if let Some(stop) = helper.stop.take() {
        let _ = stop.send(());
    }
    if let Some(thread) = helper.thread.take() {
        let _ = thread.join();
    }
}
