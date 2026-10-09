#[cfg(windows)]
use std::ffi::{OsStr, OsString};
#[cfg(windows)]
use std::iter::once;
#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt, OsStringExt};
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::sync::OnceLock;

#[cfg(windows)]
use anyhow::Context;
#[cfg(windows)]
use windows::Win32::Foundation::{
    BOOL, CloseHandle, FILETIME, HANDLE, HWND, LPARAM, MAX_PATH, WPARAM,
};
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
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
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
use windows::core::{Interface, PCWSTR, PROPVARIANT, PWSTR};

#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

/// 当前进程是否以提权（管理员 / 高完整性令牌）运行。
///
/// 启动器用它守卫 MSIX 打包应用激活：Windows 不允许提权进程激活打包应用，
/// 该场景必须明确失败，不能静默回退到按路径启动（issue #2351）。
#[cfg(windows)]
pub fn current_process_is_elevated() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

#[cfg(not(windows))]
pub fn current_process_is_elevated() -> bool {
    false
}

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
pub fn process_birth_id(process_id: u32) -> Option<u64> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()? };
    if handle.is_invalid() {
        return None;
    }
    let _guard = HandleGuard(handle);
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

/// 持有实际调试端口所属进程的句柄；PID 被复用也不会换成另一个实例。
#[cfg(windows)]
pub(crate) struct TrackedWindowsProcess {
    pub process_id: u32,
    pub birth_id: u64,
    handle: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
impl TrackedWindowsProcess {
    pub fn capture(
        process_id: u32,
        expected_executable: &std::path::Path,
    ) -> anyhow::Result<Option<Self>> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                process_id,
            )
        }
        .with_context(|| format!("failed to open Windows process id {process_id}"))?;
        let handle = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        let native_handle = HANDLE(handle.as_raw_handle());
        let mut image = vec![0_u16; 32768];
        let mut len = image.len() as u32;
        unsafe {
            QueryFullProcessImageNameW(
                native_handle,
                Default::default(),
                PWSTR(image.as_mut_ptr()),
                &mut len,
            )
        }
        .context("failed to query tracked Windows process image")?;
        let actual = PathBuf::from(OsString::from_wide(&image[..len as usize]));
        let expected = std::fs::canonicalize(expected_executable)
            .unwrap_or_else(|_| expected_executable.to_path_buf());
        let normalize = |path: &std::path::Path| {
            path.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_ascii_lowercase()
        };
        if normalize(&actual) != normalize(&expected) {
            return Ok(None);
        }
        let mut birth = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        unsafe { GetProcessTimes(native_handle, &mut birth, &mut exit, &mut kernel, &mut user) }
            .context("failed to query tracked Windows process creation time")?;
        Ok(Some(Self {
            process_id,
            birth_id: ((birth.dwHighDateTime as u64) << 32) | birth.dwLowDateTime as u64,
            handle,
        }))
    }

    pub fn is_alive(&self) -> anyhow::Result<bool> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        let result = unsafe { WaitForSingleObject(HANDLE(self.handle.as_raw_handle()), 0) };
        if result == WAIT_OBJECT_0 {
            Ok(false)
        } else if result == WAIT_TIMEOUT {
            Ok(true)
        } else {
            Err(std::io::Error::last_os_error())
                .context("failed to probe tracked Windows process")
        }
    }
}

// Windows SDK 的稳定 IP Helper ABI。只查询 listener，不启动 netstat/PowerShell，
// 也不为这一项查询扩大 windows crate 的 feature 面。
#[cfg(windows)]
#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetExtendedTcpTable(
        table: *mut std::ffi::c_void,
        size: *mut u32,
        order: i32,
        family: u32,
        class: i32,
        reserved: u32,
    ) -> u32;
}

#[cfg(windows)]
fn tcp_listener_table(family: u32) -> anyhow::Result<Vec<u8>> {
    const OWNER_PID_LISTENER: i32 = 3;
    const INSUFFICIENT_BUFFER: u32 = 122;
    let mut size = 0_u32;
    // Vec<u32> 保持 SDK table 的 DWORD 对齐；大小变化时有限重试。
    for _ in 0..4 {
        if size > 16 * 1024 * 1024 {
            anyhow::bail!("Windows TCP listener table exceeds size limit");
        }
        let mut words = vec![0_u32; (size as usize).div_ceil(4)];
        let pointer = if words.is_empty() {
            std::ptr::null_mut()
        } else {
            words.as_mut_ptr().cast()
        };
        let result =
            unsafe { GetExtendedTcpTable(pointer, &mut size, 0, family, OWNER_PID_LISTENER, 0) };
        match result {
            0 => {
                let bytes = words
                    .into_iter()
                    .flat_map(u32::to_ne_bytes)
                    .collect::<Vec<_>>();
                anyhow::ensure!(
                    size as usize <= bytes.len(),
                    "invalid Windows TCP listener table size"
                );
                return Ok(bytes[..size as usize].to_vec());
            }
            INSUFFICIENT_BUFFER => continue,
            code => {
                return Err(std::io::Error::from_raw_os_error(code as i32))
                    .context("failed to query Windows TCP listener owners");
            }
        }
    }
    anyhow::bail!("Windows TCP listener table kept changing during query")
}

#[cfg(windows)]
pub(crate) fn loopback_listener_process_ids(port: u16) -> anyhow::Result<Vec<u32>> {
    let mut ids = Vec::new();
    let mut errors = Vec::new();
    for (family, ipv6) in [(2, false), (23, true)] {
        match tcp_listener_table(family)
            .and_then(|bytes| parse_listener_process_ids(&bytes, ipv6, port))
        {
            Ok(found) => ids.extend(found),
            Err(error) => errors.push(format!("{error:#}")),
        }
    }
    ids.sort_unstable();
    ids.dedup();
    anyhow::ensure!(
        !ids.is_empty() || errors.is_empty(),
        "{}",
        errors.join("; ")
    );
    Ok(ids)
}

#[cfg(any(windows, test))]
fn unique_listener_process_id(ids: &[u32]) -> anyhow::Result<Option<u32>> {
    let first = ids.first().copied();
    anyhow::ensure!(ids.iter().all(|pid| Some(*pid) == first), "ambiguous Windows debug listener owners across address families");
    Ok(first)
}

#[cfg(windows)]
pub(crate) fn capture_debug_listener(
    port: u16,
    executable: &std::path::Path,
) -> anyhow::Result<Option<TrackedWindowsProcess>> {
    // 两个地址族若归属不同进程，不能按 PID 排序任意挑一个。
    let Some(pid) = unique_listener_process_id(&loopback_listener_process_ids(port)?)? else {
        return Ok(None);
    };
    let Some(process) = TrackedWindowsProcess::capture(pid, executable)? else {
        return Ok(None);
    };
    // 取得句柄后再次核对唯一 owner，拒绝陈旧 TCP table 和 PID 复用。
    if unique_listener_process_id(&loopback_listener_process_ids(port)?)? != Some(pid) {
        return Ok(None);
    }
    Ok(Some(process))
}

/// 两种 SDK row 的 DWORD 布局均为四字节对齐；逐字段读取并检查长度。
#[cfg(any(windows, test))]
fn parse_listener_process_ids(bytes: &[u8], ipv6: bool, port: u16) -> anyhow::Result<Vec<u32>> {
    let word = |bytes: &[u8]| u32::from_ne_bytes(bytes[..4].try_into().unwrap());
    anyhow::ensure!(bytes.len() >= 4, "truncated Windows TCP listener table");
    let row_size = if ipv6 { 56_usize } else { 24_usize };
    let count = word(bytes) as usize;
    anyhow::ensure!(
        count <= (bytes.len() - 4) / row_size,
        "truncated Windows TCP listener rows"
    );
    let mut ids = Vec::new();
    for row in bytes[4..].chunks_exact(row_size).take(count) {
        let (address, local_port, state, pid) = if ipv6 {
            (
                &row[..16],
                word(&row[20..]),
                word(&row[48..]),
                word(&row[52..]),
            )
        } else {
            (&row[4..8], word(&row[8..]), word(row), word(&row[20..]))
        };
        let loopback = if ipv6 {
            address == std::net::Ipv6Addr::LOCALHOST.octets()
        } else {
            address == [127, 0, 0, 1]
        };
        let wildcard = address.iter().all(|byte| *byte == 0);
        if state == 2
            && u16::from_be(local_port as u16) == port
            && (loopback || wildcard)
            && pid != 0
        {
            ids.push(pid);
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod listener_owner_tests {
    use super::*;

    fn table(ipv6: bool, rows: &[(u32, u16, bool, u32)]) -> Vec<u8> {
        let mut bytes = (rows.len() as u32).to_ne_bytes().to_vec();
        for &(pid, port, local, state) in rows {
            let mut row = vec![0_u8; if ipv6 { 56 } else { 24 }];
            let (port_at, state_at, pid_at) = if ipv6 {
                row[15] = if local { 1 } else { 2 };
                (20, 48, 52)
            } else {
                row[4..8].copy_from_slice(if local {
                    &[127, 0, 0, 1]
                } else {
                    &[192, 0, 2, 1]
                });
                (8, 0, 20)
            };
            row[port_at..port_at + 4].copy_from_slice(&(port.to_be() as u32).to_ne_bytes());
            row[state_at..state_at + 4].copy_from_slice(&state.to_ne_bytes());
            row[pid_at..pid_at + 4].copy_from_slice(&pid.to_ne_bytes());
            bytes.extend(row);
        }
        bytes
    }

    #[test]
    fn listener_owners_match_only_requested_loopback_port_for_both_address_families() {
        for ipv6 in [false, true] {
            let bytes = table(
                ipv6,
                &[
                    (10, 9229, true, 2),
                    (20, 9230, true, 2),
                    (30, 9229, false, 2),
                    (40, 9229, true, 5),
                ],
            );
            assert_eq!(
                parse_listener_process_ids(&bytes, ipv6, 9229).unwrap(),
                vec![10]
            );
        }
    }

    #[test]
    fn wildcard_listeners_can_own_the_loopback_endpoint() {
        for ipv6 in [false, true] {
            let mut bytes = table(ipv6, &[(10, 9229, true, 2)]);
            let address = if ipv6 { 4..20 } else { 8..12 };
            bytes[address].fill(0);
            assert_eq!(
                parse_listener_process_ids(&bytes, ipv6, 9229).unwrap(),
                vec![10]
            );
        }
    }

    #[test]
    fn listener_owner_table_rejects_truncated_or_inconsistent_rows() {
        assert!(parse_listener_process_ids(&[], false, 9229).is_err());
        for ipv6 in [false, true] {
            let mut bytes = table(ipv6, &[(10, 9229, true, 2)]);
            bytes.pop();
            assert!(parse_listener_process_ids(&bytes, ipv6, 9229).is_err());
        }
    }

    #[test]
    fn listener_owner_selection_rejects_different_ipv4_and_ipv6_instances() {
        assert_eq!(unique_listener_process_id(&[]).unwrap(), None);
        assert_eq!(unique_listener_process_id(&[10, 10]).unwrap(), Some(10));
        assert!(unique_listener_process_id(&[10, 20]).is_err());
        assert!(unique_listener_process_id(&[20, 10]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn native_loopback_listener_owner_is_captured_by_handle_and_checked_against_image() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let ids = loopback_listener_process_ids(listener.local_addr().unwrap().port()).unwrap();
        let pid = std::process::id();
        assert!(ids.contains(&pid));
        let executable = std::env::current_exe().unwrap();
        let tracked = TrackedWindowsProcess::capture(pid, &executable)
            .unwrap()
            .unwrap();
        assert!(tracked.is_alive().unwrap());
        assert_eq!(Some(tracked.birth_id), process_birth_id(pid));
        assert!(
            TrackedWindowsProcess::capture(pid, &executable.with_file_name("another.exe"))
                .unwrap()
                .is_none()
        );
    }
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
}
