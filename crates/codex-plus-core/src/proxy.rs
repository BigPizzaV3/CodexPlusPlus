use std::collections::HashMap;

pub fn has_proxy_environment(env: &HashMap<String, String>) -> bool {
    [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ]
    .into_iter()
    .any(|name| env.get(name).is_some_and(|value| !value.is_empty()))
}

pub fn detect_system_proxy() -> Option<String> {
    platform_system_proxy()
}

/// 自动代理必须针对实际目标 URL 求值，不能把 PAC 地址当成 HTTP 代理地址。
pub fn detect_system_proxy_for_url(url: &str) -> Option<String> {
    if let Some(proxy) = detect_system_proxy() {
        return Some(proxy);
    }
    #[cfg(windows)]
    { windows_pac_proxy(url) }
    #[cfg(not(windows))]
    { let _ = url; None }
}

#[cfg(windows)]
fn windows_pac_proxy(url: &str) -> Option<String> {
    use windows::Win32::Foundation::{GlobalFree, HGLOBAL};
    use windows::Win32::Networking::WinHttp::*;
    use windows::core::{PCWSTR, w};
    unsafe {
        let mut config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG::default();
        WinHttpGetIEProxyConfigForCurrentUser(&mut config).ok()?;
        let pac = wide_ptr_to_string(config.lpszAutoConfigUrl.0).unwrap_or_default();
        let auto_detect = config.fAutoDetect.as_bool();
        for pointer in [config.lpszAutoConfigUrl, config.lpszProxy, config.lpszProxyBypass] {
            if !pointer.is_null() { let _ = GlobalFree(HGLOBAL(pointer.0.cast())); }
        }
        if pac.is_empty() && !auto_detect { return None; }
        let session = WinHttpOpen(w!("CodexPlusPlus"), WINHTTP_ACCESS_TYPE_NO_PROXY,
            PCWSTR::null(), PCWSTR::null(), 0);
        if session.is_null() { return None; }
        let _ = WinHttpSetTimeouts(session, 5000, 5000, 5000, 5000);
        let pac_wide: Vec<u16> = pac.encode_utf16().chain(Some(0)).collect();
        let url_wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let mut options = WINHTTP_AUTOPROXY_OPTIONS {
            dwFlags: if pac.is_empty() { WINHTTP_AUTOPROXY_AUTO_DETECT } else { WINHTTP_AUTOPROXY_CONFIG_URL },
            dwAutoDetectFlags: if pac.is_empty() { WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A } else { 0 },
            lpszAutoConfigUrl: if pac.is_empty() { PCWSTR::null() } else { PCWSTR(pac_wide.as_ptr()) },
            ..Default::default()
        };
        let mut info = WINHTTP_PROXY_INFO::default();
        let result = WinHttpGetProxyForUrl(session, PCWSTR(url_wide.as_ptr()), &mut options, &mut info);
        let proxy = if result.is_ok() && info.dwAccessType != WINHTTP_ACCESS_TYPE_NO_PROXY {
            wide_ptr_to_string(info.lpszProxy.0).and_then(|value| parse_windows_proxy_server(&value))
        } else { None };
        for pointer in [info.lpszProxy, info.lpszProxyBypass] {
            if !pointer.is_null() { let _ = GlobalFree(HGLOBAL(pointer.0.cast())); }
        }
        let _ = WinHttpCloseHandle(session);
        proxy
    }
}

/// 把 Win32 返回的以 0 结尾的 UTF-16 指针转成字符串，NULL 返回 None。
///
/// WinHTTP 在未配置 PAC / 代理时把这些字段留成 NULL。`PWSTR::to_string()` 不判空，
/// 会让 ucrtbase 的 wcslen 解引用空指针，整个进程直接闪退（#2642 / #2661）。
///
/// # Safety
///
/// 非 NULL 时 `ptr` 必须指向以 0 结尾、可读的 UTF-16 字符串。
#[cfg(any(test, windows))]
unsafe fn wide_ptr_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

fn normalize_proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains("://") {
        return Some(value.to_string());
    }
    Some(format!("http://{value}"))
}

fn parse_windows_proxy_server(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if value.contains('=') {
        for wanted_scheme in ["https", "http"] {
            for entry in value.split(';').map(str::trim) {
                let Some((scheme, proxy)) = entry.split_once('=') else {
                    continue;
                };
                if scheme.eq_ignore_ascii_case(wanted_scheme) {
                    return normalize_proxy_url(proxy);
                }
            }
        }
        return None;
    }

    normalize_proxy_url(value)
}

#[cfg(any(test, target_os = "macos"))]
fn parse_macos_scutil_proxy(output: &str) -> Option<String> {
    let mut values = HashMap::new();
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        values.insert(key.trim(), value.trim());
    }

    for (enable_key, host_key, port_key) in [
        ("HTTPEnable", "HTTPProxy", "HTTPPort"),
        ("HTTPSEnable", "HTTPSProxy", "HTTPSPort"),
    ] {
        if values.get(enable_key) != Some(&"1") {
            continue;
        }
        let host = values.get(host_key).copied().unwrap_or_default();
        let port = values.get(port_key).copied().unwrap_or_default();
        if !host.is_empty() && !port.is_empty() {
            return normalize_proxy_url(&format!("{host}:{port}"));
        }
    }

    None
}

#[cfg(windows)]
fn platform_system_proxy() -> Option<String> {
    windows_system_proxy()
}

#[cfg(target_os = "macos")]
fn platform_system_proxy() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_scutil_proxy(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn platform_system_proxy() -> Option<String> {
    None
}

#[cfg(windows)]
fn windows_system_proxy() -> Option<String> {
    use std::ffi::{OsStr, OsString};
    use std::iter::once;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::PCWSTR;

    const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(once(0)).collect()
    }

    fn read_dword(subkey: &str, name: &str) -> Option<u32> {
        let subkey = wide_null(subkey);
        let name = wide_null(name);
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                RRF_RT_REG_DWORD,
                None,
                Some((&mut value as *mut u32).cast()),
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        Some(value)
    }

    fn read_string(subkey: &str, name: &str) -> Option<String> {
        let subkey = wide_null(subkey);
        let name = wide_null(name);
        let flags = REG_ROUTINE_FLAGS(RRF_RT_REG_SZ.0 | RRF_RT_REG_EXPAND_SZ.0);
        let mut size = 0u32;
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                flags,
                None,
                None,
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        if size == 0 {
            return None;
        }

        let mut value = vec![0u16; (size as usize).div_ceil(2)];
        unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                PCWSTR(name.as_ptr()),
                flags,
                None,
                Some(value.as_mut_ptr().cast()),
                Some(&mut size),
            )
        }
        .ok()
        .ok()?;
        let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        Some(
            OsString::from_wide(&value[..len])
                .to_string_lossy()
                .to_string(),
        )
    }

    if read_dword(INTERNET_SETTINGS, "ProxyEnable")? == 0 {
        return None;
    }

    parse_windows_proxy_server(&read_string(INTERNET_SETTINGS, "ProxyServer")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_ptr_to_string_treats_null_as_missing() {
        // 未配置 PAC 时 WinHTTP 返回 NULL，必须得到 None 而不是解引用空指针。
        assert_eq!(unsafe { wide_ptr_to_string(std::ptr::null()) }, None);
    }

    #[test]
    fn wide_ptr_to_string_reads_until_nul() {
        let wide: Vec<u16> = "http://pac.example.test/proxy.pac\0ignored"
            .encode_utf16()
            .collect();
        assert_eq!(
            unsafe { wide_ptr_to_string(wide.as_ptr()) },
            Some("http://pac.example.test/proxy.pac".to_string())
        );
        let empty = [0u16];
        assert_eq!(
            unsafe { wide_ptr_to_string(empty.as_ptr()) },
            Some(String::new())
        );
    }

    #[test]
    fn windows_proxy_server_prefers_https_scheme_entry() {
        assert_eq!(
            parse_windows_proxy_server(
                "http=proxy.example.test:8080;https=secure-proxy.example.test:8443"
            ),
            Some("http://secure-proxy.example.test:8443".to_string())
        );
    }

    #[test]
    fn windows_proxy_server_prefixes_plain_host() {
        assert_eq!(
            parse_windows_proxy_server("proxy.example.test:8080"),
            Some("http://proxy.example.test:8080".to_string())
        );
    }

    #[test]
    fn macos_scutil_proxy_parses_enabled_http_proxy() {
        let output = r#"
<dictionary> {
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : proxy.example.test
  HTTPSEnable : 0
}
"#;

        assert_eq!(
            parse_macos_scutil_proxy(output),
            Some("http://proxy.example.test:8080".to_string())
        );
    }

    #[test]
    fn macos_scutil_proxy_ignores_disabled_proxy() {
        let output = r#"
<dictionary> {
  HTTPEnable : 0
  HTTPPort : 8080
  HTTPProxy : proxy.example.test
}
"#;

        assert_eq!(parse_macos_scutil_proxy(output), None);
    }
}
