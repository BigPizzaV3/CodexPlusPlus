use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
#[cfg(windows)]
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalDictationConfig {
    pub enabled: bool,
    pub hotkey: String,
}

impl Default for GlobalDictationConfig {
    fn default() -> Self {
        Self { enabled: false, hotkey: "Ctrl+Shift+Space".to_string() }
    }
}

fn config_path() -> PathBuf {
    crate::paths::default_app_state_dir().join("global-dictation.json")
}

pub fn load_config() -> GlobalDictationConfig {
    let Ok(text) = fs::read_to_string(config_path()) else { return GlobalDictationConfig::default(); };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_config(config: &GlobalDictationConfig) -> anyhow::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
    crate::settings::atomic_write(&path, serde_json::to_string_pretty(config)?.as_bytes())
}

#[derive(Debug)]
pub struct GlobalHotkeyHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for GlobalHotkeyHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

#[cfg(any(windows, test))]
fn parse_windows_hotkey(value: &str) -> anyhow::Result<(u32, u32)> {
    let mut modifiers = 0_u32;
    let mut key = None;
    for part in value.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= 0x0002,
            "shift" => modifiers |= 0x0004,
            "alt" => modifiers |= 0x0001,
            "win" | "meta" | "command" => modifiers |= 0x0008,
            "space" => key = Some(0x20),
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => key = Some(value.as_bytes()[0].to_ascii_uppercase() as u32),
            value if value.starts_with('f') && value[1..].parse::<u32>().is_ok() => {
                let number = value[1..].parse::<u32>().unwrap();
                anyhow::ensure!((1..=12).contains(&number), "F 键范围只支持 F1-F12");
                key = Some(0x70 + number - 1);
            }
            _ => anyhow::bail!("不支持的全局热键按键：{part}"),
        }
    }
    anyhow::ensure!(modifiers != 0 && key.is_some(), "全局热键需要修饰键和按键，例如 Ctrl+Shift+Space");
    Ok((modifiers, key.unwrap()))
}

#[cfg(windows)]
fn parse_hotkey(value: &str) -> anyhow::Result<(u32, u32)> {
    parse_windows_hotkey(value)
}

#[cfg(any(windows, test))]
fn windows_registration_hotkey(value: &str) -> anyhow::Result<(u32, u32)> {
    let (modifiers, key) = parse_windows_hotkey(value)?;
    // MOD_NOREPEAT：按住热键时仅发送一次 WM_HOTKEY，松开后再次按下才切换听写。
    Ok((modifiers | 0x4000, key))
}

#[cfg(target_os = "macos")]
fn parse_hotkey(value: &str) -> anyhow::Result<(u32, u32)> {
    let mut modifiers = 0_u32;
    let mut key = None;
    for part in value.split('+').map(str::trim).filter(|part| !part.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= 1 << 12,
            "shift" => modifiers |= 1 << 9,
            "alt" | "option" => modifiers |= 1 << 11,
            "cmd" | "command" | "meta" | "win" => modifiers |= 1 << 8,
            "space" => key = Some(49),
            value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphabetic() => {
                key = Some(match value.as_bytes()[0].to_ascii_lowercase() {
                    b'a' => 0, b's' => 1, b'd' => 2, b'f' => 3, b'h' => 4,
                    b'g' => 5, b'z' => 6, b'x' => 7, b'c' => 8, b'v' => 9,
                    b'b' => 11, b'q' => 12, b'w' => 13, b'e' => 14, b'r' => 15,
                    b'y' => 16, b't' => 17, b'1' => 18, b'2' => 19, b'3' => 20,
                    b'4' => 21, b'6' => 22, b'5' => 23, b'=' => 24, b'9' => 25,
                    b'7' => 26, b'-' => 27, b'8' => 28, b'0' => 29,
                    b']' => 30, b'o' => 31, b'u' => 32, b'[' => 33, b'i' => 34,
                    b'p' => 35, b'l' => 37, b'j' => 38, b'k' => 40, b'n' => 45,
                    b'm' => 46, _ => anyhow::bail!("不支持的 macOS 热键按键：{part}"),
                });
            }
            _ => anyhow::bail!("不支持的 macOS 热键按键：{part}"),
        }
    }
    anyhow::ensure!(modifiers != 0 && key.is_some(), "全局热键需要修饰键和按键，例如 Ctrl+Shift+Space");
    Ok((modifiers, key.unwrap()))
}

#[cfg(windows)]
pub fn start<F>(hotkey: &str, callback: F) -> anyhow::Result<GlobalHotkeyHandle>
where F: Fn() + Send + 'static {
    use std::ffi::c_void;
    #[repr(C)] struct Msg { hwnd: *mut c_void, message: u32, w_param: usize, l_param: isize, time: u32, point: [i32; 2], private: u32 }
    unsafe extern "system" {
        fn RegisterHotKey(hwnd: *mut c_void, id: i32, modifiers: u32, key: u32) -> i32;
        fn UnregisterHotKey(hwnd: *mut c_void, id: i32) -> i32;
        fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
    }
    let (modifiers, key) = windows_registration_hotkey(hotkey)?;
    // 应用程序热键 ID 只能在 0x0000..=0xBFFF；原来的 0xC0D3_0001 超出 i32 范围，Windows 上编译不过。
    const HOTKEY_ID: i32 = 0x0C0D;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    // RegisterHotKey(NULL, ..) 把 WM_HOTKEY 投递到「注册线程」的消息队列，
    // UnregisterHotKey 也必须由注册线程调用：注册、收消息、注销都放在同一个线程里。
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<bool>();
    let thread = std::thread::Builder::new().name("codex-plus-global-dictation".into()).spawn(move || {
        let registered = unsafe { RegisterHotKey(std::ptr::null_mut(), HOTKEY_ID, modifiers, key) } != 0;
        let _ = ready_tx.send(registered);
        if !registered {
            return;
        }
        while !thread_stop.load(Ordering::SeqCst) {
            let mut msg = Msg { hwnd: std::ptr::null_mut(), message: 0, w_param: 0, l_param: 0, time: 0, point: [0, 0], private: 0 };
            while unsafe { PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, 1) } != 0 {
                if msg.message == 0x0312 && msg.w_param == HOTKEY_ID as usize { callback(); }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        unsafe { UnregisterHotKey(std::ptr::null_mut(), HOTKEY_ID); }
    })?;
    if !ready_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
        stop.store(true, Ordering::SeqCst);
        let _ = thread.join();
        anyhow::bail!("全局热键注册失败，可能已被其他程序占用：{hotkey}");
    }
    Ok(GlobalHotkeyHandle { stop, thread: Some(thread) })
}

#[cfg(not(windows))]
pub fn start<F>(_hotkey: &str, _callback: F) -> anyhow::Result<GlobalHotkeyHandle>
where F: Fn() + Send + 'static {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;

        type EventRef = *mut c_void;
        type EventTargetRef = *mut c_void;
        type EventHandlerRef = *mut c_void;
        type EventHotKeyRef = *mut c_void;
        type Status = i32;

        #[repr(C)]
        #[derive(Clone, Copy)]
        struct EventTypeSpec { event_class: u32, event_kind: u32 }

        type EventHandlerCallRef = *mut c_void;
        type EventHandlerProc = unsafe extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> Status;

        #[link(name = "Carbon", kind = "framework")]
        unsafe extern "C" {
            fn GetApplicationEventTarget() -> EventTargetRef;
            fn RegisterEventHotKey(key_code: u32, modifiers: u32, id: u32, target: EventTargetRef, options: u32, out_ref: *mut EventHotKeyRef) -> Status;
            fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> Status;
            fn InstallEventHandler(target: EventTargetRef, handler: Option<EventHandlerProc>, count: u32, types: *const EventTypeSpec, user_data: *mut c_void, out_ref: *mut EventHandlerRef) -> Status;
            fn RemoveEventHandler(handler: EventHandlerRef) -> Status;
            fn ReceiveNextEvent(count: u32, types: *const EventTypeSpec, timeout: f64, pull: u8, out_event: *mut EventRef) -> Status;
            fn SendEventToEventTarget(event: EventRef, target: EventTargetRef) -> Status;
            fn ReleaseEvent(event: EventRef);
        }

        let (modifiers, key_code) = parse_hotkey(_hotkey)?;
        let target = unsafe { GetApplicationEventTarget() };
        let event_type = EventTypeSpec { event_class: u32::from_be_bytes(*b"keyb"), event_kind: 5 };
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let callback: Box<dyn Fn() + Send> = Box::new(_callback);
        let callback_ptr = Box::into_raw(Box::new(callback));
        unsafe extern "C" fn carbon_handler(_call: EventHandlerCallRef, _event: EventRef, user_data: *mut c_void) -> Status {
            // SAFETY: user_data is the boxed callback allocated for this handler and is
            // freed only after the event loop and handler are removed.
            unsafe { (&*user_data.cast::<Box<dyn Fn() + Send>>())(); }
            0
        }
        let handler_proc: EventHandlerProc = carbon_handler;
        let mut handler: EventHandlerRef = std::ptr::null_mut();
        let install_status = unsafe { InstallEventHandler(target, Some(handler_proc), 1, &event_type, callback_ptr.cast(), &mut handler) };
        if install_status != 0 {
            unsafe { drop(Box::from_raw(callback_ptr)); }
            anyhow::bail!("macOS 全局热键事件处理器初始化失败：{install_status}");
        }
        let mut hot_key: EventHotKeyRef = std::ptr::null_mut();
        let register_status = unsafe { RegisterEventHotKey(key_code, modifiers, 0xC0D30001, target, 0, &mut hot_key) };
        if register_status != 0 {
            unsafe { RemoveEventHandler(handler); drop(Box::from_raw(callback_ptr)); }
            anyhow::bail!("macOS 全局热键注册失败，可能已被其他程序占用：{_hotkey}");
        }
        let target_addr = target as usize;
        let hot_key_addr = hot_key as usize;
        let handler_addr = handler as usize;
        let callback_addr = callback_ptr as usize;
        let thread = std::thread::Builder::new().name("codex-plus-global-dictation".into()).spawn(move || {
            let target = target_addr as EventTargetRef;
            let hot_key = hot_key_addr as EventHotKeyRef;
            let handler = handler_addr as EventHandlerRef;
            let callback_ptr = callback_addr as *mut Box<dyn Fn() + Send>;
            while !thread_stop.load(Ordering::SeqCst) {
                let mut event: EventRef = std::ptr::null_mut();
                let status = unsafe { ReceiveNextEvent(1, &event_type, 0.1, 1, &mut event) };
                if status == 0 && !event.is_null() {
                    unsafe { SendEventToEventTarget(event, target); ReleaseEvent(event); }
                }
            }
            unsafe {
                UnregisterEventHotKey(hot_key);
                RemoveEventHandler(handler);
                drop(Box::from_raw(callback_ptr));
            }
        })?;
        return Ok(GlobalHotkeyHandle { stop, thread: Some(thread) });
    }
    #[cfg(not(target_os = "macos"))]
    anyhow::bail!("全局听写热键首版仅支持 Windows 和 macOS");
}

static HANDLE: OnceLock<Mutex<Option<GlobalHotkeyHandle>>> = OnceLock::new();

pub fn replace_handle(handle: Option<GlobalHotkeyHandle>) {
    let slot = HANDLE.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap() = handle;
}

pub fn validate_hotkey(value: &str) -> anyhow::Result<()> {
    #[cfg(any(windows, target_os = "macos"))] { let _ = parse_hotkey(value)?; }
    #[cfg(not(any(windows, target_os = "macos")))] { anyhow::ensure!(!value.trim().is_empty(), "热键不能为空"); }
    Ok(())
}

pub fn hotkeys_match(left: &str, right: &str) -> bool {
    #[cfg(any(windows, target_os = "macos"))]
    {
        parse_hotkey(left).ok().is_some_and(|left| parse_hotkey(right).ok() == Some(left))
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        left.trim().eq_ignore_ascii_case(right.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_hotkey_registration_disables_key_repeat_and_keeps_key_combination() {
        for hotkey in ["Ctrl+Shift+Space", "Alt+F4", "Win+A"] {
            let (modifiers, key) = parse_windows_hotkey(hotkey).unwrap();
            let (registered_modifiers, registered_key) = windows_registration_hotkey(hotkey).unwrap();
            assert_eq!(registered_modifiers & 0x4000, 0x4000);
            assert_eq!(registered_modifiers & !0x4000, modifiers);
            assert_eq!(registered_key, key);
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn hotkey_identity_compares_parsed_combinations() {
        assert!(hotkeys_match("Ctrl+Shift+Space", "shift + CONTROL + space"));
        assert!(!hotkeys_match("Ctrl+Shift+Space", "Ctrl+Alt+Space"));
        assert!(!hotkeys_match("invalid", "invalid"));
    }
}
