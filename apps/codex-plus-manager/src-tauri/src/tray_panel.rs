//! 托盘快捷面板：左键点托盘图标弹出一个小窗口，分页签看 Codex 状态、切换直连供应商、
//! 看用量与额度、给网关里的各 Agent 换模型。
//!
//! 由整体设置里的「点击托盘图标弹出快捷面板」控制（默认开）。开关仍存在 gateway.json 的
//! `trayPanel` 字段里（沿用旧位置，老用户的选择不丢）。关掉时托盘恢复原来的行为
//! （macOS 点击弹菜单；Windows 左键唤起主窗口，见 issue #2676）。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{
    AppHandle, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "tray-panel";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 540.0;
/// 面板与托盘图标之间的间距（逻辑像素）。
const GAP: f64 = 6.0;
/// macOS 上面板的圆角半径（逻辑像素），和菜单栏弹出面板的观感一致。
#[cfg(target_os = "macos")]
const CORNER_RADIUS: f64 = 12.0;

/// 开关的内存缓存。托盘事件（含鼠标移入移出）很频繁，不能每次都读 gateway.json；
/// 启动后第一次用到时读一次，之后只在设置里切换或 `refresh_tray_mode` 时更新。
static ENABLED: OnceLock<AtomicBool> = OnceLock::new();

fn read_enabled_from_disk() -> bool {
    codex_plus_core::gateway::store::GatewayStore::default()
        .load()
        .map(|settings| settings.tray_panel)
        .unwrap_or(true)
}

fn cache() -> &'static AtomicBool {
    ENABLED.get_or_init(|| AtomicBool::new(read_enabled_from_disk()))
}

/// 是否打开了快捷面板（读缓存）。
pub fn enabled() -> bool {
    cache().load(Ordering::Relaxed)
}

fn set_cached(value: bool) {
    cache().store(value, Ordering::Relaxed);
}

/// 按设置调整托盘：面板打开时左键不再弹菜单（菜单留给右键）。
pub fn apply_tray_mode<R: Runtime>(app: &AppHandle<R>, tray_id: &str) {
    if let Some(tray) = app.tray_by_id(tray_id) {
        if enabled() {
            let _ = tray.set_show_menu_on_left_click(false);
        } else {
            let _ = tray.set_show_menu_on_left_click(!cfg!(windows));
        }
    }
}

/// 面板关心的托盘事件：左键单击（抬起）切换面板，左键双击收起面板。其余（移动、移入移出、右键）不管。
#[derive(Debug, Clone, Copy, PartialEq)]
enum PanelEvent {
    Toggle(PhysicalPosition<f64>),
    DoubleClick,
}

fn panel_event(event: &TrayIconEvent) -> Option<PanelEvent> {
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            position,
            ..
        } => Some(PanelEvent::Toggle(*position)),
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => Some(PanelEvent::DoubleClick),
        _ => None,
    }
}

/// 托盘事件交给面板处理时返回 true（左键单击切换面板；双击时收起面板，由调用方唤起主窗口）。
/// 先按事件类型筛掉鼠标移动等无关事件，再看开关，开关本身也只读缓存。
pub fn handle_tray_event<R: Runtime>(app: &AppHandle<R>, event: &TrayIconEvent) -> bool {
    let Some(kind) = panel_event(event) else {
        return false;
    };
    if !enabled() {
        return false;
    }
    match kind {
        PanelEvent::Toggle(position) => {
            toggle(app, position);
            true
        }
        PanelEvent::DoubleClick => {
            hide(app);
            false
        }
    }
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// 面板因失去焦点收起的时刻。Windows 上按下托盘图标先让面板失焦收起，松开时的点击事件再来切换，
/// 会把刚收起的面板又打开；刚因失焦收起的这次点击就当作「收起」。
static HIDDEN_BY_BLUR_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
const BLUR_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

fn toggle<R: Runtime>(app: &AppHandle<R>, anchor: PhysicalPosition<f64>) {
    let window = match app.get_webview_window(LABEL) {
        Some(window) => window,
        None => match create(app) {
            Some(window) => window,
            None => return,
        },
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    let just_hidden = HIDDEN_BY_BLUR_AT
        .lock()
        .ok()
        .and_then(|mut at| at.take())
        .is_some_and(|at| at.elapsed() < BLUR_CLICK_WINDOW);
    if just_hidden {
        return;
    }
    place(&window, anchor);
    let _ = window.show();
    round_corners(app, &window);
    let _ = window.set_focus();
}

/// 无边框窗口默认是直角；显示后裁成圆角并按新轮廓重算阴影。AppKit 调用必须在主线程。
fn round_corners<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    {
        let Ok(pointer) = window.ns_window() else {
            return;
        };
        let address = pointer as usize;
        let _ = app.run_on_main_thread(move || {
            // SAFETY: 指针来自 Tauri 的 `ns_window()`，面板只隐藏不销毁，窗口一直有效。
            unsafe {
                codex_plus_core::macos_window::round_corners(
                    address as *mut std::ffi::c_void,
                    CORNER_RADIUS,
                )
            };
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, window);
}

fn create<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::WebviewWindow<R>> {
    let window =
        WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("/index.html#tray-panel".into()))
            .title("Codex++")
            .inner_size(WIDTH, HEIGHT)
            .decorations(false)
            .resizable(false)
            .always_on_top(true)
            .visible_on_all_workspaces(true)
            .skip_taskbar(true)
            .shadow(true)
            .accept_first_mouse(true)
            .visible(false)
            .build()
            .ok()?;
    // 点到别处就收起，和菜单栏面板的习惯一致。
    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            if handle.is_visible().unwrap_or(false)
                && let Ok(mut at) = HIDDEN_BY_BLUR_AT.lock()
            {
                *at = Some(std::time::Instant::now());
            }
            let _ = handle.hide();
        }
    });
    Some(window)
}

/// 放在托盘图标旁边：图标在屏幕上半部（macOS 菜单栏）时放在下方，否则（Windows 任务栏）放在上方；
/// 水平方向以图标为中心，并保证不超出屏幕。
fn place<R: Runtime>(window: &tauri::WebviewWindow<R>, anchor: PhysicalPosition<f64>) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = WIDTH * scale;
    let height = HEIGHT * scale;
    let gap = GAP * scale;
    let monitor = window
        .available_monitors()
        .ok()
        .and_then(|monitors| {
            monitors.into_iter().find(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                anchor.x >= f64::from(position.x)
                    && anchor.x < f64::from(position.x) + f64::from(size.width)
                    && anchor.y >= f64::from(position.y)
                    && anchor.y < f64::from(position.y) + f64::from(size.height)
            })
        })
        .or_else(|| window.primary_monitor().ok().flatten());
    let (left, top, right, bottom) = monitor
        .map(|monitor| {
            let position = monitor.position();
            let size = monitor.size();
            (
                f64::from(position.x),
                f64::from(position.y),
                f64::from(position.x) + f64::from(size.width),
                f64::from(position.y) + f64::from(size.height),
            )
        })
        .unwrap_or((0.0, 0.0, anchor.x + width, anchor.y + height));
    let x = (anchor.x - width / 2.0).clamp(left + gap, (right - width - gap).max(left));
    let below = anchor.y < (top + bottom) / 2.0;
    let y = if below {
        anchor.y + gap * 3.0
    } else {
        anchor.y - height - gap * 3.0
    };
    let y = y.clamp(top + gap, (bottom - height - gap).max(top));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

/// 面板上的按钮：打开主窗口、退出、收起。
///
/// 轻量模式下主窗口可能已销毁，「打开」要重建窗口：同步命令里建窗口在 Windows 上会死锁，
/// 所以这里是异步命令，并把显示（含重建）放到主线程执行（#2677）。
#[tauri::command]
pub async fn tray_panel_action<R: Runtime>(app: AppHandle<R>, action: String) {
    match action.as_str() {
        "open" => {
            hide(&app);
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || crate::show_main_window_from(&handle));
        }
        "quit" => {
            crate::request_app_exit(&app);
        }
        _ => hide(&app),
    }
}

/// 整体设置里读开关当前的值。
#[tauri::command]
pub fn tray_panel_enabled() -> bool {
    enabled()
}

/// 整体设置里切换开关：写回 gateway.json 的 `trayPanel`，更新缓存并立即生效。返回保存后的值。
#[tauri::command]
pub fn set_tray_panel_enabled<R: Runtime>(
    app: AppHandle<R>,
    enabled: bool,
) -> Result<bool, String> {
    codex_plus_core::gateway::store::GatewayStore::default()
        .update(|settings| {
            settings.tray_panel = enabled;
            Ok(())
        })
        .map_err(|error| format!("保存失败：{error:#}"))?;
    set_cached(enabled);
    apply_tray_mode(&app, crate::TRAY_ID);
    if !enabled {
        hide(&app);
    }
    crate::gateway_commands::notify_settings_changed();
    Ok(enabled)
}

/// 重新从 gateway.json 读开关（例如导入了设置），立即生效。
#[tauri::command]
pub fn refresh_tray_mode<R: Runtime>(app: AppHandle<R>) {
    set_cached(read_enabled_from_disk());
    // 关窗进入轻量模式的开关也存在 gateway.json，一并重读。
    crate::lightweight::refresh_from_disk();
    apply_tray_mode(&app, crate::TRAY_ID);
    if !enabled() {
        hide(&app);
    }
}

#[cfg(test)]
mod tests {
    use super::{PanelEvent, panel_event};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent, TrayIconId};
    use tauri::{PhysicalPosition, Rect};

    fn rect() -> Rect {
        Rect::default()
    }

    #[test]
    fn only_left_clicks_reach_the_panel() {
        let id = TrayIconId::new("t");
        let position = PhysicalPosition::new(10.0, 20.0);
        let click = |button, button_state| TrayIconEvent::Click {
            id: id.clone(),
            position,
            rect: rect(),
            button,
            button_state,
        };
        assert_eq!(
            panel_event(&click(MouseButton::Left, MouseButtonState::Up)),
            Some(PanelEvent::Toggle(position))
        );
        assert_eq!(
            panel_event(&click(MouseButton::Left, MouseButtonState::Down)),
            None
        );
        assert_eq!(
            panel_event(&click(MouseButton::Right, MouseButtonState::Up)),
            None
        );
        assert_eq!(
            panel_event(&TrayIconEvent::DoubleClick {
                id: id.clone(),
                position,
                rect: rect(),
                button: MouseButton::Left,
            }),
            Some(PanelEvent::DoubleClick)
        );
        // 鼠标移动、移入移出都不该触发面板逻辑（也就不会去读开关）。
        assert_eq!(
            panel_event(&TrayIconEvent::Move {
                id: id.clone(),
                position,
                rect: rect(),
            }),
            None
        );
        assert_eq!(
            panel_event(&TrayIconEvent::Enter {
                id: id.clone(),
                position,
                rect: rect(),
            }),
            None
        );
    }
}
