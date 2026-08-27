#[cfg(windows)]
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::iter::once;
#[cfg(windows)]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use windows::Win32::Foundation::{
    BOOL, CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_PARAMETER, FILETIME, HANDLE, HWND,
    LPARAM, MAX_PATH, NO_ERROR, WPARAM,
};
#[cfg(windows)]
use windows::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_OWNER_PID,
    MIB_TCPTABLE_OWNER_PID, TCP_TABLE_OWNER_PID_LISTENER,
};
#[cfg(windows)]
use windows::Win32::Networking::WinSock::{AF_INET, AF_INET6};
#[cfg(windows)]
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile,
};
#[cfg(windows)]
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ,
    RegCloseKey, RegCreateKeyW, RegDeleteKeyW, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW,
    RegSetValueExW,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
#[cfg(windows)]
use windows::Win32::UI::Shell::{
    ExtractIconExW, FOLDERID_Desktop, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    ShellExecuteW, ShellLink,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWMINNOACTIVE;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SW_RESTORE, SW_SHOW, SetForegroundWindow,
    ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    HICON, ICON_BIG, ICON_SMALL, SendMessageW, WM_SETICON,
};
#[cfg(windows)]
use windows::core::{HRESULT, Interface, PCWSTR, PROPVARIANT, PWSTR};

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsProcessInfo {
    pub process_id: u32,
    pub parent_process_id: u32,
    pub exe_file: String,
    pub executable_path: Option<PathBuf>,
}

#[cfg(windows)]
pub struct ComApartment;

#[cfg(windows)]
impl ComApartment {
    pub fn init() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutSpec {
    pub path: PathBuf,
    pub target: PathBuf,
    pub arguments: String,
    pub working_directory: Option<PathBuf>,
    pub description: String,
    pub icon: Option<PathBuf>,
    pub show_minimized: bool,
}

#[cfg(windows)]
pub fn create_shortcut(spec: &ShortcutSpec) -> anyhow::Result<()> {
    if let Some(parent) = spec.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _com = ComApartment::init().context("初始化 COM 失败")?;
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .context("创建 ShellLink COM 对象失败")?;
        shell_link
            .SetPath(PCWSTR(wide_null(spec.target.as_os_str()).as_ptr()))
            .context("设置快捷方式目标失败")?;
        shell_link
            .SetArguments(PCWSTR(wide_null(spec.arguments.as_str()).as_ptr()))
            .context("设置快捷方式参数失败")?;
        if let Some(working_directory) = &spec.working_directory {
            shell_link
                .SetWorkingDirectory(PCWSTR(wide_null(working_directory.as_os_str()).as_ptr()))
                .context("设置快捷方式工作目录失败")?;
        }
        shell_link
            .SetDescription(PCWSTR(wide_null(spec.description.as_str()).as_ptr()))
            .context("设置快捷方式描述失败")?;
        if let Some(icon) = &spec.icon {
            shell_link
                .SetIconLocation(PCWSTR(wide_null(icon.as_os_str()).as_ptr()), 0)
                .context("设置快捷方式图标失败")?;
        }
        if spec.show_minimized {
            shell_link
                .SetShowCmd(SW_SHOWMINNOACTIVE)
                .context("设置快捷方式窗口模式失败")?;
        }
        let persist_file: IPersistFile = shell_link.cast().context("获取 IPersistFile 失败")?;
        persist_file
            .Save(PCWSTR(wide_null(spec.path.as_os_str()).as_ptr()), true)
            .context("保存快捷方式失败")?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn desktop_dir() -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None).ok()?;
        let value = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.as_ptr().cast()));
        value
    }
}

#[cfg(windows)]
pub fn open_url(url: &str) -> anyhow::Result<()> {
    let operation = wide_null("open");
    let file = wide_null(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWMINNOACTIVE,
        )
    };
    let code = result.0 as isize;
    if code <= 32 {
        anyhow::bail!("ShellExecuteW returned {code}");
    }
    Ok(())
}

#[cfg(windows)]
pub fn set_current_user_string_value(subkey: &str, name: &str, value: &str) -> anyhow::Result<()> {
    with_created_current_user_key(subkey, |key| {
        let value = wide_null(value);
        let bytes = slice_as_u8(&value);
        unsafe {
            RegSetValueExW(
                key,
                PCWSTR(wide_null(name).as_ptr()),
                0,
                REG_SZ,
                Some(bytes),
            )
        }
        .ok()
        .with_context(|| format!("写入注册表值 {subkey}\\{name} 失败"))
    })
}

#[cfg(windows)]
pub fn delete_current_user_value(subkey: &str, name: &str) -> anyhow::Result<()> {
    let subkey = wide_null(subkey);
    let name = wide_null(name);
    let mut key = HKEY::default();
    if unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_SET_VALUE,
            &mut key,
        )
    }
    .is_err()
    {
        return Ok(());
    }
    let _guard = RegistryKeyGuard(key);
    unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) }
        .ok()
        .or_else(|_| Ok(()))
}

#[cfg(windows)]
pub fn read_current_user_string_values(
    subkey: &str,
) -> anyhow::Result<Vec<(String, Option<String>)>> {
    read_registry_string_values(HKEY_CURRENT_USER, subkey)
}

#[cfg(windows)]
pub fn read_local_machine_string_values(
    subkey: &str,
) -> anyhow::Result<Vec<(String, Option<String>)>> {
    read_registry_string_values(HKEY_LOCAL_MACHINE, subkey)
}

#[cfg(windows)]
fn read_registry_string_values(
    root: HKEY,
    subkey: &str,
) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let subkey = wide_null(subkey);
    let mut key = HKEY::default();
    if unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), 0, KEY_READ, &mut key) }.is_err() {
        return Ok(Vec::new());
    }
    let _guard = RegistryKeyGuard(key);
    let mut values = Vec::new();
    for index in 0.. {
        let mut name = vec![0u16; 256];
        let mut name_len = name.len() as u32;
        let mut value_type = 0u32;
        let mut data = vec![0u8; 8192];
        let mut data_len = data.len() as u32;
        let result = unsafe {
            RegEnumValueW(
                key,
                index,
                PWSTR(name.as_mut_ptr()),
                &mut name_len,
                None,
                Some(&mut value_type),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if result.is_err() {
            break;
        }
        let name = OsString::from_wide(&name[..name_len as usize])
            .to_string_lossy()
            .to_string();
        let value = if value_type == REG_SZ.0 || value_type == REG_EXPAND_SZ.0 {
            let units = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr().cast::<u16>(),
                    (data_len as usize).div_ceil(2),
                )
            };
            let len = units.iter().position(|ch| *ch == 0).unwrap_or(units.len());
            Some(
                OsString::from_wide(&units[..len])
                    .to_string_lossy()
                    .to_string(),
            )
        } else {
            None
        };
        values.push((name, value));
    }
    Ok(values)
}

#[cfg(windows)]
pub fn delete_current_user_key(subkey: &str) -> anyhow::Result<()> {
    let subkey = wide_null(subkey);
    unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(subkey.as_ptr())) }
        .ok()
        .or_else(|_| Ok(()))
}

#[cfg(windows)]
pub fn enumerate_processes() -> Vec<WindowsProcessInfo> {
    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };
    if snapshot.is_invalid() {
        return Vec::new();
    }
    let _guard = HandleGuard(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut processes = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_err() {
        return Vec::new();
    }
    loop {
        let process_id = entry.th32ProcessID;
        processes.push(WindowsProcessInfo {
            process_id,
            parent_process_id: entry.th32ParentProcessID,
            exe_file: nul_terminated_wide_to_string(&entry.szExeFile),
            executable_path: query_process_image_path(process_id),
        });
        if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
            break;
        }
    }
    processes
}

#[cfg(windows)]
pub fn tcp_listener_process_ids(address: SocketAddr) -> std::io::Result<Vec<u32>> {
    let port = address.port();
    let mut process_ids = match address.ip() {
        IpAddr::V4(address) => tcp4_listener_process_ids(address, port),
        IpAddr::V6(address) => tcp6_listener_process_ids(address, port),
    }?;
    process_ids.sort_unstable();
    process_ids.dedup();
    Ok(process_ids)
}

#[cfg(windows)]
pub fn loopback_tcp_listener_process_ids(port: u16) -> std::io::Result<Vec<u32>> {
    let mut process_ids = tcp4_listener_process_ids(Ipv4Addr::LOCALHOST, port)?;
    process_ids.extend(tcp6_listener_process_ids(Ipv6Addr::LOCALHOST, port)?);
    process_ids.sort_unstable();
    process_ids.dedup();
    Ok(process_ids)
}

#[cfg(windows)]
fn tcp4_listener_process_ids(address: Ipv4Addr, port: u16) -> std::io::Result<Vec<u32>> {
    let buffer = tcp_listener_table_buffer(AF_INET.0 as u32)?;
    if buffer.is_empty() {
        return Ok(Vec::new());
    }
    let table = buffer.as_ptr().cast::<MIB_TCPTABLE_OWNER_PID>();
    let count = unsafe { (*table).dwNumEntries as usize };
    validate_tcp_table_size(
        buffer.len(),
        count,
        std::mem::size_of::<MIB_TCPROW_OWNER_PID>(),
    )?;
    let rows = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).table).cast::<MIB_TCPROW_OWNER_PID>(),
            count,
        )
    };
    Ok(rows
        .iter()
        .filter(|row| {
            network_port(row.dwLocalPort) == port && ipv4_listener_matches(row.dwLocalAddr, address)
        })
        .map(|row| row.dwOwningPid)
        .collect())
}

#[cfg(windows)]
fn tcp6_listener_process_ids(address: Ipv6Addr, port: u16) -> std::io::Result<Vec<u32>> {
    let buffer = tcp_listener_table_buffer(AF_INET6.0 as u32)?;
    if buffer.is_empty() {
        return Ok(Vec::new());
    }
    let table = buffer.as_ptr().cast::<MIB_TCP6TABLE_OWNER_PID>();
    let count = unsafe { (*table).dwNumEntries as usize };
    validate_tcp_table_size(
        buffer.len(),
        count,
        std::mem::size_of::<MIB_TCP6ROW_OWNER_PID>(),
    )?;
    let rows = unsafe {
        std::slice::from_raw_parts(
            std::ptr::addr_of!((*table).table).cast::<MIB_TCP6ROW_OWNER_PID>(),
            count,
        )
    };
    Ok(rows
        .iter()
        .filter(|row| {
            network_port(row.dwLocalPort) == port && ipv6_listener_matches(row.ucLocalAddr, address)
        })
        .map(|row| row.dwOwningPid)
        .collect())
}

#[cfg(windows)]
fn tcp_listener_table_buffer(address_family: u32) -> std::io::Result<Vec<u32>> {
    let mut byte_len = 0u32;
    let status = unsafe {
        GetExtendedTcpTable(
            None,
            &mut byte_len,
            false,
            address_family,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        )
    };
    if status == NO_ERROR.0 && byte_len == 0 {
        return Ok(Vec::new());
    }
    if status != ERROR_INSUFFICIENT_BUFFER.0 && status != NO_ERROR.0 {
        return Err(std::io::Error::from_raw_os_error(status as i32));
    }

    for _ in 0..3 {
        let word_len = (byte_len as usize).div_ceil(std::mem::size_of::<u32>());
        let mut buffer = vec![0u32; word_len];
        let status = unsafe {
            GetExtendedTcpTable(
                Some(buffer.as_mut_ptr().cast()),
                &mut byte_len,
                false,
                address_family,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if status == NO_ERROR.0 {
            return Ok(buffer);
        }
        if status != ERROR_INSUFFICIENT_BUFFER.0 {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::Other,
        "TCP listener table kept changing while being read",
    ))
}

#[cfg(windows)]
fn validate_tcp_table_size(
    buffer_words: usize,
    entry_count: usize,
    entry_size: usize,
) -> std::io::Result<()> {
    let required = std::mem::size_of::<u32>()
        .checked_add(entry_count.checked_mul(entry_size).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "TCP table size overflow")
        })?)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "TCP table size overflow")
        })?;
    let available = buffer_words
        .checked_mul(std::mem::size_of::<u32>())
        .unwrap_or(usize::MAX);
    if required > available {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "TCP listener table is truncated",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn network_port(value: u32) -> u16 {
    u16::from_be(value as u16)
}

#[cfg(windows)]
fn ipv4_listener_matches(value: u32, requested: Ipv4Addr) -> bool {
    let local = Ipv4Addr::from(u32::from_be(value));
    local.is_unspecified() || local == requested
}

#[cfg(windows)]
fn ipv6_listener_matches(value: [u8; 16], requested: Ipv6Addr) -> bool {
    let local = Ipv6Addr::from(value);
    local.is_unspecified() || local == requested
}

#[cfg(windows)]
pub fn terminate_process(process_id: u32) -> bool {
    let Ok(handle) = (unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        )
    }) else {
        return false;
    };
    if handle.is_invalid() {
        return false;
    }
    let _guard = HandleGuard(handle);
    unsafe { TerminateProcess(handle, 0) }.is_ok()
}

#[cfg(windows)]
pub fn terminate_process_if_identity_matches(
    process_id: u32,
    expected_birth_id: u64,
    expected_path: &Path,
) -> std::io::Result<bool> {
    let handle = match unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            process_id,
        )
    } {
        Ok(handle) => handle,
        Err(error) if error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
            return Ok(false);
        }
        Err(error) => return Err(std::io::Error::other(error.to_string())),
    };
    if handle.is_invalid() {
        return Ok(false);
    }
    let _guard = HandleGuard(handle);
    let Some(birth_id) = process_birth_id_from_handle(handle) else {
        return Ok(false);
    };
    let Some(path) = query_process_image_path_from_handle(handle) else {
        return Ok(false);
    };
    if birth_id != expected_birth_id || !paths_equal_case_insensitive(&path, expected_path) {
        return Ok(false);
    }
    unsafe { TerminateProcess(handle, 0) }
        .map(|_| true)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(windows)]
pub fn process_birth_id(process_id: u32) -> Option<u64> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()? };
    if handle.is_invalid() {
        return None;
    }
    let _guard = HandleGuard(handle);
    process_birth_id_from_handle(handle)
}

#[cfg(windows)]
fn process_birth_id_from_handle(handle: HANDLE) -> Option<u64> {
    let mut creation_time = FILETIME::default();
    let mut exit_time = FILETIME::default();
    let mut kernel_time = FILETIME::default();
    let mut user_time = FILETIME::default();
    unsafe {
        GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        )
        .ok()?;
    }
    Some(((creation_time.dwHighDateTime as u64) << 32) | creation_time.dwLowDateTime as u64)
}

#[cfg(windows)]
pub fn process_started_at_secs_from_birth_id(birth_id: u64) -> Option<u64> {
    const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
    const TICKS_PER_SECOND: u64 = 10_000_000;

    birth_id
        .checked_sub(WINDOWS_TO_UNIX_EPOCH_100NS)
        .map(|unix_ticks| unix_ticks / TICKS_PER_SECOND)
}

#[cfg(windows)]
pub fn activate_process_window(process_id: u32) -> bool {
    let Some(hwnd) = process_window(process_id, false) else {
        return false;
    };
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        SetForegroundWindow(hwnd).as_bool()
    }
}

#[cfg(windows)]
pub fn apply_codexplusplus_icon_to_process_window(
    process_id: u32,
    icon_resource_path: PathBuf,
) -> bool {
    let Some(hwnd) = visible_window_for_process(process_id) else {
        return false;
    };
    let mut applied = false;
    if apply_window_icons(hwnd, &icon_resource_path) {
        applied = true;
    }
    if apply_taskbar_properties(hwnd, &icon_resource_path).is_ok() {
        applied = true;
    }
    applied
}

#[cfg(windows)]
fn query_process_image_path(process_id: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()? };
    if handle.is_invalid() {
        return None;
    }
    let _guard = HandleGuard(handle);
    query_process_image_path_from_handle(handle)
}

#[cfg(windows)]
fn query_process_image_path_from_handle(handle: HANDLE) -> Option<PathBuf> {
    let mut buffer = vec![0u16; MAX_PATH as usize * 4];
    let mut len = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle,
            Default::default(),
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
        .ok()?;
    }
    Some(PathBuf::from(OsString::from_wide(&buffer[..len as usize])))
}

#[cfg(windows)]
fn paths_equal_case_insensitive(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}

#[cfg(windows)]
fn visible_window_for_process(process_id: u32) -> Option<HWND> {
    process_window(process_id, true)
}

#[cfg(windows)]
fn process_window(process_id: u32, visible_only: bool) -> Option<HWND> {
    let mut state = ActivateWindowState {
        process_id,
        hwnd: HWND::default(),
        visible_only,
        score: ProcessWindowScore::None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(find_process_window_proc),
            LPARAM((&mut state as *mut ActivateWindowState) as isize),
        );
    }
    if state.hwnd.is_invalid() {
        None
    } else {
        Some(state.hwnd)
    }
}

#[cfg(windows)]
struct ActivateWindowState {
    process_id: u32,
    hwnd: HWND,
    visible_only: bool,
    score: ProcessWindowScore,
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProcessWindowScore {
    None,
    Fallback,
    Titled,
    AppWindow,
    TauriWindow,
}

#[cfg(windows)]
unsafe extern "system" fn find_process_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let state = unsafe { &mut *(lparam.0 as *mut ActivateWindowState) };
    if state.visible_only && !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return BOOL(1);
    }
    let mut window_process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut window_process_id));
    }
    if window_process_id == state.process_id {
        let title_length = unsafe { GetWindowTextLengthW(hwnd) };
        let extended_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        let mut class_name = [0u16; 256];
        let class_name_length = unsafe { GetClassNameW(hwnd, &mut class_name) }.max(0) as usize;
        let class_name = String::from_utf16_lossy(&class_name[..class_name_length]);
        let score = process_window_score(title_length > 0, extended_style, &class_name);
        if score > state.score {
            state.hwnd = hwnd;
            state.score = score;
        }
        if score == ProcessWindowScore::TauriWindow {
            return BOOL(0);
        }
    }
    BOOL(1)
}

#[cfg(windows)]
fn process_window_score(
    has_title: bool,
    extended_style: u32,
    class_name: &str,
) -> ProcessWindowScore {
    let is_app_window = extended_style & WS_EX_APPWINDOW.0 != 0;
    let is_tool_window = extended_style & WS_EX_TOOLWINDOW.0 != 0;
    if is_tool_window || is_auxiliary_window_class(class_name) {
        ProcessWindowScore::Fallback
    } else if class_name.eq_ignore_ascii_case("Tauri Window") {
        ProcessWindowScore::TauriWindow
    } else if is_app_window && !is_tool_window {
        ProcessWindowScore::AppWindow
    } else if has_title {
        ProcessWindowScore::Titled
    } else {
        ProcessWindowScore::Fallback
    }
}

#[cfg(windows)]
fn is_auxiliary_window_class(class_name: &str) -> bool {
    matches!(
        class_name.to_ascii_lowercase().as_str(),
        "ime" | "msctfime ui" | "tray_icon_app" | "tao thread event target"
    )
}

#[cfg(windows)]
fn apply_window_icons(hwnd: HWND, icon_resource_path: &PathBuf) -> bool {
    let Some((large_icon, small_icon)) = load_cached_icons(icon_resource_path) else {
        return false;
    };
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_BIG as usize),
            LPARAM(large_icon.0 as isize),
        );
        SendMessageW(
            hwnd,
            WM_SETICON,
            WPARAM(ICON_SMALL as usize),
            LPARAM(small_icon.0 as isize),
        );
    }
    true
}

#[cfg(windows)]
fn load_cached_icons(icon_resource_path: &PathBuf) -> Option<(HICON, HICON)> {
    static ICONS: OnceLock<(usize, usize)> = OnceLock::new();
    let icons = ICONS.get_or_init(|| {
        let path = wide_null(icon_resource_path.as_os_str());
        let mut large_icon = HICON::default();
        let mut small_icon = HICON::default();
        let loaded = unsafe {
            ExtractIconExW(
                PCWSTR(path.as_ptr()),
                0,
                Some(&mut large_icon),
                Some(&mut small_icon),
                1,
            )
        };
        if loaded == 0 {
            (0, 0)
        } else {
            (large_icon.0 as usize, small_icon.0 as usize)
        }
    });
    if icons.0 == 0 || icons.1 == 0 {
        None
    } else {
        Some((
            HICON(icons.0 as *mut core::ffi::c_void),
            HICON(icons.1 as *mut core::ffi::c_void),
        ))
    }
}

#[cfg(windows)]
fn apply_taskbar_properties(hwnd: HWND, icon_resource_path: &PathBuf) -> anyhow::Result<()> {
    use windows::Win32::Storage::EnhancedStorage::{
        PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
        PKEY_AppUserModel_RelaunchDisplayNameResource, PKEY_AppUserModel_RelaunchIconResource,
    };

    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(hwnd)? };
    let icon_resource = format!("{},0", icon_resource_path.to_string_lossy());
    let relaunch_command = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "codex-plus-plus.exe".to_string());
    set_property_string(
        &store,
        &PKEY_AppUserModel_ID,
        "com.bigpizzav3.codexplusplus.codex",
    )?;
    set_property_string(
        &store,
        &PKEY_AppUserModel_RelaunchIconResource,
        &icon_resource,
    )?;
    set_property_string(
        &store,
        &PKEY_AppUserModel_RelaunchDisplayNameResource,
        "Codex++",
    )?;
    set_property_string(
        &store,
        &PKEY_AppUserModel_RelaunchCommand,
        &relaunch_command,
    )?;
    unsafe {
        store.Commit()?;
    }
    Ok(())
}

#[cfg(windows)]
fn set_property_string(
    store: &IPropertyStore,
    key: &windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY,
    value: &str,
) -> anyhow::Result<()> {
    let variant = PROPVARIANT::from(value);
    unsafe {
        store.SetValue(key, &variant)?;
    }
    Ok(())
}

#[cfg(windows)]
fn with_created_current_user_key<T>(
    subkey: &str,
    f: impl FnOnce(HKEY) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyW(
            HKEY_CURRENT_USER,
            PCWSTR(wide_null(subkey).as_ptr()),
            &mut key,
        )
    }
    .ok()
    .with_context(|| format!("打开注册表键 HKCU\\{subkey} 失败"))?;
    let _guard = RegistryKeyGuard(key);
    f(key)
}

#[cfg(windows)]
fn slice_as_u8(value: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), std::mem::size_of_val(value)) }
}

#[cfg(windows)]
fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(once(0)).collect()
}

#[cfg(windows)]
fn nul_terminated_wide_to_string(value: &[u16]) -> String {
    let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    OsString::from_wide(&value[..len])
        .to_string_lossy()
        .to_string()
}

#[cfg(windows)]
struct HandleGuard(HANDLE);

#[cfg(windows)]
impl Drop for HandleGuard {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(windows)]
struct RegistryKeyGuard(HKEY);

#[cfg(windows)]
impl Drop for RegistryKeyGuard {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn application_window_outranks_titled_ime_and_tool_windows() {
        let ime_score = process_window_score(true, 0, "IME");
        let tool_score = process_window_score(false, WS_EX_TOOLWINDOW.0, "Tao Thread Event Target");
        let app_score = process_window_score(true, WS_EX_APPWINDOW.0, "Chrome_WidgetWin_1");
        let tauri_score = process_window_score(true, 0, "Tauri Window");
        let auxiliary_app_score = process_window_score(true, WS_EX_APPWINDOW.0, "tray_icon_app");

        assert!(tauri_score > app_score);
        assert!(app_score > ime_score);
        assert_eq!(ime_score, tool_score);
        assert_eq!(auxiliary_app_score, ProcessWindowScore::Fallback);
    }

    #[test]
    fn network_port_decodes_the_ip_helper_byte_order() {
        assert_eq!(network_port(u16::to_be(9229) as u32), 9229);
    }

    #[test]
    fn tcp_listener_process_ids_finds_the_current_ipv4_listener() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();

        let process_ids = tcp_listener_process_ids(address).unwrap();

        assert!(process_ids.contains(&std::process::id()));
    }

    #[test]
    fn listener_address_matching_rejects_other_local_addresses() {
        assert!(ipv4_listener_matches(
            u32::from(Ipv4Addr::LOCALHOST).to_be(),
            Ipv4Addr::LOCALHOST
        ));
        assert!(!ipv4_listener_matches(
            u32::from(Ipv4Addr::new(127, 0, 0, 2)).to_be(),
            Ipv4Addr::LOCALHOST
        ));
        assert!(ipv6_listener_matches(
            Ipv6Addr::LOCALHOST.octets(),
            Ipv6Addr::LOCALHOST
        ));
    }

    #[test]
    fn identity_checked_termination_treats_a_missing_process_as_gone() {
        let result =
            terminate_process_if_identity_matches(u32::MAX, 1, Path::new(r"C:\missing\Codex.exe"))
                .unwrap();

        assert!(!result);
    }
}
