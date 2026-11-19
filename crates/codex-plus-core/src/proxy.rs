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

/// 回环地址：Codex++ 的网关、协议代理、helper 都只监听在本机。
const LOOPBACK_NO_PROXY_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// Codex++ 启动 Codex 时给它的 `NO_PROXY`（#3131）。
///
/// Codex 的 app-server 用 reqwest 的系统代理：先读 `HTTP(S)_PROXY` / `NO_PROXY` 环境变量，
/// Windows 上再读 WinINET 注册表（`ProxyEnable` / `ProxyServer`，`NO_PROXY` 为空时才用
/// `ProxyOverride`）——`NO_PROXY` 对注册表代理同样生效（hyper-util `matcher.rs` 的
/// `Builder::from_system`，`intercept` 先查 `no`）。MSIX 包视图里残留一个失效的
/// `ProxyServer` 时，发往本地网关 `127.0.0.1:57321` 的请求也被送进代理，客户端一直
/// Reconnecting。这里让回环地址始终直连。
///
/// 不能把用户原有的排除项冲掉：
/// - 继承的 `NO_PROXY` / `no_proxy` 原样保留，回环地址补在后面；
/// - 原来没设 `NO_PROXY` 时 reqwest 会用注册表的 `ProxyOverride`，而设了之后就不再读它，
///   所以把（本进程能看到的）`ProxyOverride` 按 reqwest 的规则折算进来，绕过列表不变。
pub fn codex_launch_no_proxy() -> String {
    let inherited = ["NO_PROXY", "no_proxy"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .collect::<Vec<_>>()
        .join(",");
    codex_launch_no_proxy_from(&inherited, windows_proxy_override().as_deref())
}

/// 要给启动的 Codex 进程设的环境变量（`command.envs(...)`），值是 [`codex_launch_no_proxy`]。
/// Windows 环境变量名不分大小写，两个名字是同一个变量；其它平台两个都设，免得只认小写的
/// 工具拿到旧值。
pub fn codex_launch_no_proxy_env() -> Vec<(&'static str, String)> {
    let value = codex_launch_no_proxy();
    if cfg!(windows) {
        vec![("NO_PROXY", value)]
    } else {
        vec![("NO_PROXY", value.clone()), ("no_proxy", value)]
    }
}

/// [`codex_launch_no_proxy`] 的纯函数部分：`inherited` 是继承的 `NO_PROXY`（逗号分隔，
/// 可为空），`proxy_override` 是启用中的 WinINET `ProxyOverride` 原值。
pub fn codex_launch_no_proxy_from(inherited: &str, proxy_override: Option<&str>) -> String {
    let base = if inherited.split(',').any(|entry| !entry.trim().is_empty()) {
        inherited.to_string()
    } else {
        proxy_override
            .map(normalize_windows_proxy_override)
            .unwrap_or_default()
    };
    let mut entries: Vec<String> = Vec::new();
    for entry in base
        .split(',')
        .map(str::trim)
        .chain(LOOPBACK_NO_PROXY_HOSTS)
    {
        if !entry.is_empty() && !entries.iter().any(|seen| seen.eq_ignore_ascii_case(entry)) {
            entries.push(entry.to_string());
        }
    }
    entries.join(",")
}

/// 把 WinINET 的 `ProxyOverride`（分号分隔、`10.*` 这类通配）折算成 `NO_PROXY` 写法，
/// 规则与 hyper-util 读注册表时一致（`win::normalize_proxy_override`）：
/// `a.b.*` 这类 IPv4 尾部通配变成 CIDR，其余原样，最后去掉所有 `*.` 前缀。
pub fn normalize_windows_proxy_override(value: &str) -> String {
    fn ipv4_wildcard_to_cidr(value: &str) -> Option<String> {
        let parts = value.split('.').collect::<Vec<_>>();
        let wildcard = parts.iter().position(|part| *part == "*")?;
        if wildcard == 0 || wildcard > 3 || parts[wildcard..].iter().any(|part| *part != "*") {
            return None;
        }
        let mut octets = [0u8; 4];
        for (index, part) in parts[..wildcard].iter().enumerate() {
            octets[index] = part.parse().ok()?;
        }
        Some(format!(
            "{}.{}.{}.{}/{}",
            octets[0],
            octets[1],
            octets[2],
            octets[3],
            wildcard * 8
        ))
    }
    value
        .split(';')
        .map(|entry| {
            let entry = entry.trim();
            ipv4_wildcard_to_cidr(entry).unwrap_or_else(|| entry.to_string())
        })
        .collect::<Vec<_>>()
        .join(",")
        .replace("*.", "")
}

/// 启用中的 WinINET 系统代理的 `ProxyOverride`；未启用代理或读不到时为 None。
#[cfg(windows)]
fn windows_proxy_override() -> Option<String> {
    if wininet::read_dword(wininet::INTERNET_SETTINGS, "ProxyEnable")? == 0 {
        return None;
    }
    wininet::read_string(wininet::INTERNET_SETTINGS, "ProxyOverride")
}

#[cfg(not(windows))]
fn windows_proxy_override() -> Option<String> {
    None
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

/// macOS 系统代理的查询结果短时复用多久。
#[cfg(target_os = "macos")]
const MACOS_SYSTEM_PROXY_TTL: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(target_os = "macos")]
fn platform_system_proxy() -> Option<String> {
    // scutil 是外部进程：实测每次约 35ms，而且阻塞调用线程。网关和协议代理每个上游请求
    // 都要查一次（经本地代理再到网关时要查两次），几秒内复用上一次的结果（#2761）。
    // 代价是系统代理改了之后最多晚几秒生效。
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Option<String>)>> =
        std::sync::Mutex::new(None);
    if let Ok(cache) = CACHE.lock()
        && let Some((at, proxy)) = cache.as_ref()
        && at.elapsed() < MACOS_SYSTEM_PROXY_TTL
    {
        return proxy.clone();
    }
    let proxy = macos_scutil_system_proxy();
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((std::time::Instant::now(), proxy.clone()));
    }
    proxy
}

#[cfg(target_os = "macos")]
fn macos_scutil_system_proxy() -> Option<String> {
    let output = std::process::Command::new("scutil")
        .arg("--proxy")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_macos_scutil_proxy(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "linux")]
fn platform_system_proxy() -> Option<String> {
    for var in [
        "https_proxy",
        "HTTPS_PROXY",
        "all_proxy",
        "ALL_PROXY",
        "http_proxy",
        "HTTP_PROXY",
    ] {
        if let Ok(value) = std::env::var(var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                if let Some(normalized) = normalize_proxy_url(trimmed) {
                    return Some(normalized);
                }
            }
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn platform_system_proxy() -> Option<String> {
    None
}

#[cfg(windows)]
fn windows_system_proxy() -> Option<String> {
    use self::wininet::{INTERNET_SETTINGS, read_dword, read_string};

    if read_dword(INTERNET_SETTINGS, "ProxyEnable")? == 0 {
        return None;
    }

    parse_windows_proxy_server(&read_string(INTERNET_SETTINGS, "ProxyServer")?)
}

/// HKCU 下 WinINET 代理设置的读取（系统代理检测与 Codex 启动的 NO_PROXY 共用）。
#[cfg(windows)]
mod wininet {
    use std::ffi::{OsStr, OsString};
    use std::iter::once;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, REG_ROUTINE_FLAGS, RRF_RT_REG_DWORD, RRF_RT_REG_EXPAND_SZ,
        RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::PCWSTR;

    pub(super) const INTERNET_SETTINGS: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    fn wide_null(value: impl AsRef<OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(once(0)).collect()
    }

    pub(super) fn read_dword(subkey: &str, name: &str) -> Option<u32> {
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

    pub(super) fn read_string(subkey: &str, name: &str) -> Option<String> {
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
    fn codex_launch_no_proxy_appends_loopback_and_keeps_user_entries() {
        // #3131：回环地址补在用户原有排除项后面，按出现顺序去重。
        assert_eq!(
            codex_launch_no_proxy_from(" 10.0.0.0/8, *.internal ,localhost,10.0.0.0/8", None),
            "10.0.0.0/8,*.internal,localhost,127.0.0.1,::1"
        );
        assert_eq!(codex_launch_no_proxy_from("", None), "localhost,127.0.0.1,::1");
        assert_eq!(codex_launch_no_proxy_from(" , ", None), "localhost,127.0.0.1,::1");
        // 已经设了 NO_PROXY 时 reqwest 不再读 ProxyOverride，这里也不掺进来。
        assert_eq!(
            codex_launch_no_proxy_from("corp.example", Some("10.*")),
            "corp.example,localhost,127.0.0.1,::1"
        );
    }

    #[test]
    fn codex_launch_no_proxy_keeps_wininet_bypass_list_when_no_env_value() {
        // 原来靠 ProxyOverride 绕过的地址，设了 NO_PROXY 之后也要照旧绕过。
        assert_eq!(
            codex_launch_no_proxy_from("", Some("localhost;127.*;10.*;192.168.*;*.corp.example;<local>")),
            "localhost,127.0.0.0/8,10.0.0.0/8,192.168.0.0/16,corp.example,<local>,127.0.0.1,::1"
        );
    }

    #[test]
    fn windows_proxy_override_normalization_matches_reqwest() {
        // 与 hyper-util `win::normalize_proxy_override` 的测试用例一致。
        let normalized = normalize_windows_proxy_override("127.*;10.226.*;192.168.40.*;www.example.com;*.example.org");
        assert_eq!(
            normalized,
            "127.0.0.0/8,10.226.0.0/16,192.168.40.0/24,www.example.com,example.org"
        );
        // 不是 IPv4 尾部通配的条目不转 CIDR。
        assert_eq!(normalize_windows_proxy_override(" <local> ; 300.*"), "<local>,300.*");
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
