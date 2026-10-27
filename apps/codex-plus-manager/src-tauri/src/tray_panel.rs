//! 托盘快捷面板：左键点托盘图标弹出一个小窗口，看各 Agent 当前的模型、直接切换，看最近的请求。
//!
//! 由网关设置里的「点击托盘图标弹出快捷面板」控制（默认开）。关掉时托盘恢复原来的行为
//! （macOS 点击弹菜单；Windows 左键唤起主窗口，见 issue #2676）。

use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{
    AppHandle, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "tray-panel";
const WIDTH: f64 = 420.0;
const HEIGHT: f64 = 600.0;
/// 面板与托盘图标之间的间距（逻辑像素）。
const GAP: f64 = 6.0;

/// 网关设置里是否打开了快捷面板。
pub fn enabled() -> bool {
    codex_plus_core::gateway::store::GatewayStore::default()
        .load()
        .map(|settings| settings.tray_panel)
        .unwrap_or(true)
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

/// 托盘事件交给面板处理时返回 true（左键单击切换面板；双击时收起面板，由调用方唤起主窗口）。
pub fn handle_tray_event<R: Runtime>(app: &AppHandle<R>, event: &TrayIconEvent) -> bool {
    if !enabled() {
        return false;
    }
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            position,
            ..
        } => {
            toggle(app, *position);
            true
        }
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => {
            hide(app);
            false
        }
        _ => false,
    }
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

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
    place(&window, anchor);
    let _ = window.show();
    let _ = window.set_focus();
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
#[tauri::command]
pub fn tray_panel_action<R: Runtime>(app: AppHandle<R>, action: String) {
    match action.as_str() {
        "open" => {
            hide(&app);
            crate::show_main_window_from(&app);
        }
        "quit" => {
            crate::request_app_exit(&app);
        }
        _ => hide(&app),
    }
}

/// 网关设置里切换开关后调用：立即生效。
#[tauri::command]
pub fn refresh_tray_mode<R: Runtime>(app: AppHandle<R>) {
    apply_tray_mode(&app, crate::TRAY_ID);
    if !enabled() {
        hide(&app);
    }
}
