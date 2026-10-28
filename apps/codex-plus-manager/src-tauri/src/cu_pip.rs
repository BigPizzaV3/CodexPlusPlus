//! Computer Use 悬浮预览：CU 操作后台应用时，在屏幕右上角实时显示目标窗口。
//!
//! 对标 Codex 的画中画（Remote Hosted PiP）。官方那套只认 OpenAI 自家的宿主，
//! 这里用 `window_capture` 自己取流，画面是同一个窗口的实时内容。
//!
//! # 什么时候显示
//!
//! CU 服务（`--codex-plus-cu-mcp`）每次在结果里看到被操作的应用就写一条 `cu_activity`。
//! 本模块轮询它：
//!
//! - 目标应用**不在前台**时显示（同 Codex：只给后台被操作的应用开画中画）；
//! - 回合结束（Claude Code 的 Stop 钩子写 spool）、会话结束、空闲 20 秒、
//!   或用户切到了那个应用时收起；
//! - 用户点了关闭：这个会话里不再弹，直到下一个会话。
//!
//! 窗口显示时**不激活** Codex++，用户正在用的窗口不会失焦。
//!
//! # 浏览器
//!
//! 同 Codex：浏览器标签页的画面来自 `cua_repl` 结果里附带的截图（CU 服务写进
//! `cu/pip-frame.jpg`），每操作一次更新一次。agent 的标签页在后台，Chrome 在不在前台
//! 都显示；标签页都关了、回合结束或空闲 20 秒时收起。

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use codex_plus_core::cu_activity;
use codex_plus_core::window_capture::{self, CaptureMode, PreviewHandle};

const LABEL: &str = "cu-pip";
pub const FRAME_EVENT: &str = "cu-pip-frame";
pub const TARGET_EVENT: &str = "cu-pip-target";
const POLL: Duration = Duration::from_millis(400);
/// 这么久没有新动作就收起。
const IDLE: Duration = Duration::from_secs(20);
const WIDTH: f64 = 360.0;
const HEIGHT: f64 = 236.0;
const MARGIN: f64 = 16.0;

#[derive(Default)]
struct PipState {
    /// 当前目标：写入方会话、被操作的 pid、最后一次动作的时间。
    session: u32,
    pid: i32,
    last_act: Option<Instant>,
    /// 正在取流的窗口。
    window_id: Option<u32>,
    preview: Option<PreviewHandle>,
    visible: bool,
    /// 用户关掉预览的会话：这个会话里不再弹。
    dismissed_session: Option<u32>,
    /// 预览标题栏显示的应用名与窗口标题。
    target: serde_json::Value,
    /// 当前目标是浏览器标签页（画面取自截图文件，而不是窗口取流）。
    browser: bool,
    /// 已推给预览的截图文件修改时间。
    frame_seen: Option<std::time::SystemTime>,
    /// 浏览器档：正在操作的标签页编号（「切到该标签页」用）。
    tab: String,
}

static STATE: Mutex<Option<PipState>> = Mutex::new(None);

fn with_state<T>(run: impl FnOnce(&mut PipState) -> T) -> T {
    let mut guard = STATE.lock().unwrap_or_else(|error| error.into_inner());
    run(guard.get_or_insert_with(PipState::default))
}

fn enabled() -> bool {
    codex_plus_core::settings::SettingsStore::default()
        .load()
        .map(|settings| settings.computer_use_pip)
        .unwrap_or(true)
}

fn log(event: &str, detail: serde_json::Value) {
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(event, detail);
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

/// 启动轮询线程。只在 macOS 上有意义（取流与前台判断都依赖 AppKit / ScreenCaptureKit）。
pub fn start(app: AppHandle) {
    if !cfg!(target_os = "macos") {
        return;
    }
    std::thread::spawn(move || {
        // 启动前就有的记录不算：管理器重启时不该把很久以前的操作弹出来。
        let mut last_at = now_millis();
        let spool = codex_plus_core::claude_hooks::spool_path();
        let mut spool_seen = codex_plus_core::claude_hooks::spool_len(&spool);
        loop {
            std::thread::sleep(POLL);
            tick(&app, &mut last_at, &spool, &mut spool_seen);
        }
    });
}

fn tick(app: &AppHandle, last_at: &mut u64, spool: &std::path::Path, spool_seen: &mut u64) {
    // 回合结束：收起（同 Codex，一轮结束画中画就收）。
    let spool_now = codex_plus_core::claude_hooks::spool_len(spool);
    if spool_now != *spool_seen {
        let ended = spool_now > *spool_seen;
        *spool_seen = spool_now;
        if ended {
            with_state(|state| state.last_act = None);
        }
    }

    if let Some(activity) = cu_activity::read()
        && activity.at > *last_at
    {
        *last_at = activity.at;
        match activity.event.as_str() {
            "act" if activity.kind == "browser" => with_state(|state| {
                log(
                    "cu_pip.browser",
                    json!({ "title": activity.title, "session": activity.session }),
                );
                if state.dismissed_session == Some(activity.session) || !enabled() {
                    return;
                }
                state.browser = true;
                state.window_id = None;
                state.session = activity.session;
                // 「切到该应用」用：Chrome 的进程。
                state.pid = resolve_pid(&activity.app_name).unwrap_or(0);
                state.tab = activity.tab.clone();
                state.target = json!({ "appName": activity.app_name, "title": activity.title, "kind": "browser" });
                state.last_act = Some(Instant::now());
            }),
            "act" => {
                // JS 档只有应用名：按名字找进程。
                let pid = if activity.pid > 0 {
                    activity.pid
                } else {
                    resolve_pid(&activity.app_name).unwrap_or(0)
                };
                if pid > 0 {
                    with_state(|state| {
                        log(
                            "cu_pip.activity",
                            json!({ "pid": pid, "app": activity.app_name, "session": activity.session }),
                        );
                        if state.dismissed_session == Some(activity.session) || !enabled() {
                            return;
                        }
                        if state.pid != pid || state.browser {
                            state.window_id = None;
                        }
                        state.browser = false;
                        state.session = activity.session;
                        state.pid = pid;
                        state.last_act = Some(Instant::now());
                    });
                }
            }
            "end" => with_state(|state| {
                if state.session == activity.session {
                    state.last_act = None;
                }
            }),
            _ => {}
        }
    }

    let (pid, active, browser) = with_state(|state| {
        let active = state.last_act.is_some_and(|at| at.elapsed() < IDLE);
        if !active {
            state.last_act = None;
        }
        (state.pid, active, state.browser)
    });
    if browser {
        if active {
            show_browser(app);
        } else {
            hide(app);
        }
        return;
    }
    let want = active && pid > 0 && cu_activity::frontmost_pid() != Some(pid);
    if want {
        show(app, pid);
    } else {
        hide(app);
    }
}

/// 浏览器标签页：截图文件有更新就推给预览。
fn show_browser(app: &AppHandle) {
    let Some(window) = ensure_window(app) else {
        return;
    };
    stop_preview();
    let path = cu_activity::frame_path();
    let modified = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok();
    let (fresh, first, target) = with_state(|state| {
        let fresh = modified.is_some() && modified != state.frame_seen;
        let first = state.frame_seen.is_none();
        if fresh {
            state.frame_seen = modified;
        }
        (fresh, first, state.target.clone())
    });
    if fresh {
        let Ok(jpeg) = std::fs::read(&path) else {
            return;
        };
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
        // 预览收到目标事件会清掉旧画面，所以先发目标、再发画面。
        let _ = window.emit_to(LABEL, TARGET_EVENT, target);
        let _ = window.emit_to(
            LABEL,
            FRAME_EVENT,
            json!({ "data": format!("data:image/jpeg;base64,{encoded}") }),
        );
        if first {
            log("cu_pip.show_browser", json!({ "bytes": jpeg.len() }));
        }
    }
    let already = with_state(|state| state.visible);
    if !already && modified.is_some() {
        order_front(app, &window);
        with_state(|state| state.visible = true);
    }
}

/// 按应用名找进程：屏幕上有窗口、名字一致的那个应用。
fn resolve_pid(app_name: &str) -> Option<i32> {
    if app_name.is_empty() {
        return None;
    }
    window_capture::list_windows()
        .ok()?
        .into_iter()
        .filter(|window| window.app_name == app_name && window.on_screen)
        .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        .map(|window| window.pid)
}

/// 目标应用的主窗口：屏幕上、面积最大的那个。
fn pick_window(pid: i32) -> Option<(u32, String, String)> {
    let windows = window_capture::list_windows().ok()?;
    windows
        .into_iter()
        .filter(|window| {
            window.pid == pid && window.on_screen && window.width >= 120.0 && window.height >= 80.0
        })
        .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        .map(|window| (window.window_id, window.app_name, window.title))
}

fn show(app: &AppHandle, pid: i32) {
    let need_capture = with_state(|state| state.window_id.is_none() || state.preview.is_none());
    if need_capture {
        let Some((window_id, app_name, title)) = pick_window(pid) else {
            // 没有屏幕录制权限或窗口不在屏幕上：没有可显示的画面就不弹。
            log(
                "cu_pip.no_window",
                json!({ "pid": pid, "permitted": window_capture::has_permission() }),
            );
            hide(app);
            return;
        };
        let Some(window) = ensure_window(app) else {
            return;
        };
        stop_preview();
        let frame_app = app.clone();
        let handle = window_capture::start_preview(
            window_id,
            0.5,
            12.0,
            // 截图模式：目标窗口不会出现系统的「正在共享」标志。
            CaptureMode::Screenshot,
            move |jpeg| {
                use base64::Engine as _;
                let encoded = base64::engine::general_purpose::STANDARD.encode(&jpeg);
                let _ = frame_app.emit_to(
                    LABEL,
                    FRAME_EVENT,
                    json!({ "data": format!("data:image/jpeg;base64,{encoded}") }),
                );
            },
        );
        let handle = match handle {
            Ok(handle) => handle,
            Err(error) => {
                log(
                    "cu_pip.capture_failed",
                    json!({ "pid": pid, "windowId": window_id, "error": error.to_string() }),
                );
                hide(app);
                return;
            }
        };
        log(
            "cu_pip.show",
            json!({ "pid": pid, "windowId": window_id, "app": app_name }),
        );
        let target = json!({ "pid": pid, "appName": app_name, "title": title });
        with_state(|state| {
            state.window_id = Some(window_id);
            state.preview = Some(handle);
            state.target = target.clone();
        });
        let _ = window.emit_to(LABEL, TARGET_EVENT, target);
    }
    let already = with_state(|state| state.visible);
    if !already && let Some(window) = app.get_webview_window(LABEL) {
        order_front(app, &window);
        with_state(|state| state.visible = true);
    }
}

fn hide(app: &AppHandle) {
    let visible = with_state(|state| std::mem::replace(&mut state.visible, false));
    stop_preview();
    with_state(|state| {
        state.window_id = None;
        // 下次显示时重新推一次画面。
        state.frame_seen = None;
    });
    if visible && let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

fn stop_preview() {
    if let Some(handle) = with_state(|state| state.preview.take()) {
        handle.stop();
    }
}

/// 预览窗口懒创建：第一次需要时才建，之后只显示 / 隐藏。
fn ensure_window(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(LABEL) {
        return Some(window);
    }
    let (x, y) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| {
            let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
            // 右上角，避开菜单栏。
            (size.width - WIDTH - MARGIN, 38.0)
        })
        .unwrap_or((MARGIN, 38.0));
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("/index.html#cu-pip".into()))
        .title("Codex++ Computer Use")
        .inner_size(WIDTH, HEIGHT)
        .min_inner_size(220.0, 150.0)
        .position(x, y)
        .decorations(false)
        .resizable(true)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .shadow(true)
        .focused(false)
        // 小窗所属的 Codex++ 通常不在前台：不开这个，第一下点击只用来激活窗口，
        // 网页收不到按下事件，标题栏拖不动、按钮第一下没反应。
        .accept_first_mouse(true)
        .visible(false)
        .build()
        .inspect_err(|error| {
            log(
                "cu_pip.window_failed",
                json!({ "error": error.to_string() }),
            )
        })
        .ok()
}

/// 浮到最上层并显示，不激活 Codex++。AppKit 调用必须在主线程。
fn order_front(app: &AppHandle, window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        let Ok(pointer) = window.ns_window() else {
            return;
        };
        let address = pointer as usize;
        let _ = app.run_on_main_thread(move || {
            // SAFETY: 指针来自 Tauri 的 `ns_window()`，窗口在本进程内一直存在（只隐藏不销毁）。
            unsafe {
                cu_activity::show_floating_without_activation(address as *mut std::ffi::c_void)
            };
        });
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, window.show());
    }
}

/// 预览窗口加载后取当前目标（窗口刚建好时可能错过了 TARGET_EVENT）。
#[tauri::command]
pub fn cu_pip_target() -> serde_json::Value {
    with_state(|state| state.target.clone())
}

/// 预览上的关闭：这个会话里不再弹。
#[tauri::command]
pub fn cu_pip_dismiss(app: AppHandle) {
    with_state(|state| {
        state.dismissed_session = Some(state.session);
        state.last_act = None;
    });
    hide(&app);
}

/// 预览上的「切到该应用」：把目标应用切到前台，预览随之收起。
///
/// 浏览器档同 Codex 的点预览跳标签。Codex 走扩展宿主的签名通道发 `focusTab`，第三方连不上；
/// Codex 开放给模型的 `management` 也不许切换标签（`tabs.update` 的 `active`、`tabs.highlight`
/// 实测都被拒）。这里用 Chrome 的 AppleScript：它的标签 `id` 和扩展里的标签编号是同一个
/// session id。第一次会弹系统的「自动化」授权；没授权或没找到时退回只激活 Chrome。
#[tauri::command]
pub fn cu_pip_activate() -> bool {
    let (pid, browser, tab) = with_state(|state| (state.pid, state.browser, state.tab.clone()));
    if browser && let Some(script) = focus_tab_script(&tab) {
        // 授权弹窗会一直等用户回应，不能卡住界面线程。
        std::thread::spawn(move || {
            let focused = std::process::Command::new("/usr/bin/osascript")
                .args(["-e", &script])
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).trim() == "true")
                .unwrap_or(false);
            log(
                "cu_pip.focus_tab",
                json!({ "tab": tab, "focused": focused }),
            );
            if !focused && pid > 0 {
                cu_activity::activate_pid(pid);
            }
        });
        return true;
    }
    pid > 0 && cu_activity::activate_pid(pid)
}

/// 按标签编号切到那个标签并把 Chrome 提到前台的 AppleScript。编号必须是数字，
/// 不把任意文本拼进脚本。
fn focus_tab_script(tab: &str) -> Option<String> {
    let id: u64 = tab.parse().ok()?;
    Some(format!(
        r#"tell application "Google Chrome"
  repeat with w in windows
    set i to 0
    repeat with t in tabs of w
      set i to i + 1
      if (id of t) is {id} then
        set active tab index of w to i
        set index of w to 1
        activate
        return true
      end if
    end repeat
  end repeat
end tell
return false"#
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_script_only_for_numeric_tabs() {
        let script = focus_tab_script("305258454").unwrap();
        assert!(script.contains("if (id of t) is 305258454 then"));
        assert!(focus_tab_script("1 then do shell script").is_none());
        assert!(focus_tab_script("").is_none());
    }
}
