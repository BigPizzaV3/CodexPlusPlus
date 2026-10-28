//! 轻量模式（#2677）：关闭管理器主窗口时销毁主窗口（连同 WebView 渲染进程）释放内存，
//! 后端进程、2.0 网关、托盘、托盘快捷面板、guard 端口等照常运行；
//! 托盘、Dock、二次启动等入口再打开时按原配置重建主窗口（见 `crate::show_main_window`）。
//!
//! 开关在整体设置里（默认开），值存在 gateway.json 的 `lightweightOnClose`，
//! 与「点击托盘图标弹出快捷面板」同处、同样由专用命令读写。关掉时回到只隐藏窗口的旧行为。
//!
//! 关窗时先照旧隐藏，稍等再销毁：设置页的自动保存有 500ms 防抖，销毁 WebView 会丢掉还没发出的保存。
//! 等待期间窗口被重新打开（或获得焦点）就取消这次销毁。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Listener, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

/// 主窗口前端注册好托盘请求监听后发出：快捷面板的请求可以发过去了。
pub const MAIN_WINDOW_READY_EVENT: &str = "codex-plus-main-window-ready";
/// 隐藏后等多久再销毁：覆盖设置自动保存的防抖和在途的保存请求。
const RELEASE_DELAY: Duration = Duration::from_secs(3);
/// 快捷面板等重建的主窗口前端加载好的上限。
const READY_TIMEOUT: Duration = Duration::from_secs(20);
const READY_POLL: Duration = Duration::from_millis(50);

/// 开关的内存缓存：关窗时不必读盘，启动后第一次用到时读一次，之后只在设置里切换时更新。
static ENABLED: OnceLock<AtomicBool> = OnceLock::new();
/// 主窗口前端是否已就绪（监听已注册）。新建窗口和窗口销毁时清掉，收到就绪事件时置位。
static MAIN_WINDOW_READY: AtomicBool = AtomicBool::new(false);
/// 销毁计划的代号：每次排期或取消都加一，到点时代号变了就说明这次计划已作废。
static RELEASE_GENERATION: AtomicU64 = AtomicU64::new(0);
/// 销毁前记下的窗口位置与大小，重建时恢复，免得每次都回到默认尺寸。
static LAST_GEOMETRY: Mutex<Option<Geometry>> = Mutex::new(None);

#[derive(Debug, Clone, Copy)]
struct Geometry {
    /// 非最大化时的外框位置与内容区大小；最大化时不更新，保留最大化前的值。
    bounds: Option<(PhysicalPosition<i32>, PhysicalSize<u32>)>,
    maximized: bool,
}

fn read_enabled_from_disk() -> bool {
    codex_plus_core::gateway::store::GatewayStore::default()
        .load()
        .map(|settings| settings.lightweight_on_close)
        .unwrap_or(false)
}

fn cache() -> &'static AtomicBool {
    ENABLED.get_or_init(|| AtomicBool::new(read_enabled_from_disk()))
}

/// 是否开启了轻量模式（读缓存）。
pub fn enabled() -> bool {
    cache().load(Ordering::Relaxed)
}

/// 监听主窗口前端的就绪事件。setup 里调用一次。
pub fn install_ready_listener<R: Runtime>(app: &AppHandle<R>) {
    app.listen_any(MAIN_WINDOW_READY_EVENT, |_| {
        MAIN_WINDOW_READY.store(true, Ordering::SeqCst);
    });
}

/// 新建主窗口或主窗口销毁后，前端需要重新加载。
pub fn mark_main_window_loading() {
    MAIN_WINDOW_READY.store(false, Ordering::SeqCst);
}

/// 取消尚未执行的销毁（窗口被重新打开、获得焦点或要处理快捷面板的请求）。
pub fn cancel_pending_release() {
    RELEASE_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// 主窗口隐藏后排期销毁。轻量模式关闭时什么都不做（保持只隐藏的旧行为）。
pub fn schedule_release<R: Runtime>(app: &AppHandle<R>) {
    if !enabled() {
        return;
    }
    let generation = RELEASE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(RELEASE_DELAY);
        let handle = app.clone();
        // 与重建、显示窗口一样在主线程上执行，互相之间不会交错。
        let _ = app.run_on_main_thread(move || release_if_hidden(&handle, generation));
    });
}

fn release_if_hidden<R: Runtime>(app: &AppHandle<R>, generation: u64) {
    if RELEASE_GENERATION.load(Ordering::SeqCst) != generation
        || !enabled()
        || crate::app_is_exiting()
    {
        return;
    }
    let Some(window) = app.get_webview_window(crate::MAIN_WINDOW_LABEL) else {
        return;
    };
    // 拿不到可见状态时按可见处理，宁可不销毁。
    if window.is_visible().unwrap_or(true) {
        return;
    }
    mark_main_window_loading();
    let result = window.destroy();
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "manager.lightweight_release",
        serde_json::json!({ "ok": result.is_ok() }),
    );
}

/// 关窗前记下位置和大小。最小化时的坐标不可信（Windows 上是 -32000），不记。
pub fn remember_geometry<R: Runtime>(window: &WebviewWindow<R>) {
    if !enabled() || window.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let Ok(mut slot) = LAST_GEOMETRY.lock() else {
        return;
    };
    let previous = slot.and_then(|geometry| geometry.bounds);
    let bounds = if maximized {
        previous
    } else {
        match (window.outer_position(), window.inner_size()) {
            (Ok(position), Ok(size)) => Some((position, size)),
            _ => previous,
        }
    };
    *slot = Some(Geometry { bounds, maximized });
}

/// 重建的窗口恢复上次的位置与大小（窗口仍隐藏）。位置不在任何屏幕上时（例如外接屏已拔掉）只恢复大小。
pub fn restore_geometry<R: Runtime>(window: &WebviewWindow<R>) {
    let Some((position, size)) = LAST_GEOMETRY
        .lock()
        .ok()
        .and_then(|slot| slot.and_then(|geometry| geometry.bounds))
    else {
        return;
    };
    let _ = window.set_size(size);
    let on_screen = window.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|monitor| {
            let origin = monitor.position();
            let extent = monitor.size();
            // 标题栏附近的一点落在屏幕内即可拖回来。
            let x = position.x + 40;
            let y = position.y + 10;
            x >= origin.x
                && y >= origin.y
                && x < origin.x + extent.width as i32
                && y < origin.y + extent.height as i32
        })
    });
    if on_screen {
        let _ = window.set_position(position);
    }
}

/// 上次关窗时是否最大化。只在重建后第一次显示时使用（隐藏状态下最大化在部分平台会直接显示窗口）。
pub fn was_maximized() -> bool {
    LAST_GEOMETRY
        .lock()
        .ok()
        .and_then(|slot| slot.map(|geometry| geometry.maximized))
        .unwrap_or(false)
}

fn wait_until_ready(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if MAIN_WINDOW_READY.load(Ordering::SeqCst) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(READY_POLL);
    }
}

/// 快捷面板发请求前调用：主窗口不在就按原配置重建（保持隐藏），等前端注册好监听再返回。
/// 返回主窗口是否就绪；超时也返回，面板照常发请求，由面板自己的超时兜底。
#[tauri::command]
pub async fn main_window_prepare<R: Runtime>(app: AppHandle<R>) -> bool {
    // 有请求要处理：先别销毁，处理完由主窗口重新排期。
    cancel_pending_release();
    let handle = app.clone();
    // 创建窗口放到主线程（同步命令里建窗口在 Windows 上会死锁），也和销毁串行。
    let _ = app.run_on_main_thread(move || {
        crate::ensure_main_window(&handle);
    });
    tauri::async_runtime::spawn_blocking(|| wait_until_ready(READY_TIMEOUT))
        .await
        .unwrap_or(false)
}

/// 主窗口在后台处理完事情（启动时最小化、快捷面板的请求）后调用：仍隐藏着就按轻量模式排期销毁。
#[tauri::command]
pub fn main_window_release_when_hidden<R: Runtime>(app: AppHandle<R>) {
    let hidden = app
        .get_webview_window(crate::MAIN_WINDOW_LABEL)
        .is_some_and(|window| !window.is_visible().unwrap_or(true));
    if hidden {
        schedule_release(&app);
    }
}

/// 整体设置里读开关当前的值。
#[tauri::command]
pub fn lightweight_mode_enabled() -> bool {
    enabled()
}

/// 整体设置里切换开关：写回 gateway.json 的 `lightweightOnClose`，更新缓存。返回保存后的值。
#[tauri::command]
pub fn set_lightweight_mode_enabled(enabled: bool) -> Result<bool, String> {
    codex_plus_core::gateway::store::GatewayStore::default()
        .update(|settings| {
            settings.lightweight_on_close = enabled;
            Ok(())
        })
        .map_err(|error| format!("保存失败：{error:#}"))?;
    cache().store(enabled, Ordering::Relaxed);
    if !enabled {
        cancel_pending_release();
    }
    crate::gateway_commands::notify_settings_changed();
    Ok(enabled)
}

/// 重新从 gateway.json 读开关（例如导入了设置）。
pub fn refresh_from_disk() {
    cache().store(read_enabled_from_disk(), Ordering::Relaxed);
}
