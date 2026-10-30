use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[cfg(any(windows, test))]
mod windows_process_tree;
#[cfg(windows)]
use windows_process_tree::CodexInstanceProcessTree;

/// 端口只是连接地址；停止实例必须核验保存的进程身份，不能依赖 CDP 健康。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceIdentity {
    pub instance_id: String,
    pub generation: u64,
    pub debug_port: u16,
    pub helper_port: u16,
    pub user_data_dir: PathBuf,
    pub browser_id: String,
    pub process_id: u32,
    pub process_birth_id: u64,
    pub executable: PathBuf,
}

pub fn lock_instance_lifecycle(root: &Path) -> anyhow::Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("lifecycle.lock"))?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn valid_stop_identity(identity: &CodexInstanceIdentity) -> bool {
    !identity.instance_id.trim().is_empty()
        && identity.generation != 0
        && !identity.browser_id.is_empty()
        && identity.process_id != 0
        && identity.process_id != std::process::id()
        && identity.process_birth_id != 0
        && identity.debug_port != 0
        && identity.helper_port != 0
        && identity.user_data_dir.is_absolute()
        && identity.executable.is_absolute()
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn record_stop_verification_failure(
    identity: &CodexInstanceIdentity,
    result: &anyhow::Result<bool>,
) {
    let detail = match result {
        Ok(true) => return,
        Ok(false) => serde_json::json!({
            "reason": "identity_changed",
        }),
        Err(error) => serde_json::json!({
            "reason": "verification_error",
            "message": format!("{error:#}"),
        }),
    };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "multi_worker.stop_verification_failed",
        serde_json::json!({
            "instanceId": identity.instance_id,
            "generation": identity.generation,
            "processId": identity.process_id,
            "debugPort": identity.debug_port,
            "detail": detail,
        }),
    );
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn canonical_identity_user_data(path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!path.as_os_str().is_empty(), "多开实例必须指定精确数据目录");
    Ok(path.canonicalize()?)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn codex_identity_arguments_match(arguments: &[String], identity: &CodexInstanceIdentity) -> bool {
    codex_arguments_match(arguments, &identity.user_data_dir, identity.debug_port)
}

/// 同时检查两个 loopback 地址族，不能用页面列表不可读掩盖 Browser 冲突。
#[cfg(any(windows, test))]
async fn check_stop_browser_identity(identity: &CodexInstanceIdentity) -> anyhow::Result<()> {
    use anyhow::Context;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_millis(750))
        .build()?;
    for host in ["127.0.0.1", "[::1]"] {
        let response = client
            .get(format!(
                "http://{host}:{}/json/version",
                identity.debug_port
            ))
            .send()
            .await;
        let mut response = match response {
            Ok(response) => response
                .error_for_status()
                .context("CDP Browser 响应无效")?,
            // 仅传输失联允许退回保存身份；收到可读但无效的身份仍然拒绝。
            Err(error) if error.is_connect() || error.is_timeout() => continue,
            Err(error) => return Err(error).context("无法核验 CDP Browser"),
        };
        let mut bytes = Vec::new();
        let mut disconnected = false;
        loop {
            let chunk = match response.chunk().await {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(error) if error.is_timeout() || error.is_body() || error.is_connect() => {
                    // 已收到完整可读身份时仍执行否决，不能用后续断连掩盖冲突。
                    if let Ok(browser) =
                        serde_json::from_slice::<crate::cdp::CdpBrowserIdentity>(&bytes)
                    {
                        verify_stop_browser(identity, &browser)?;
                    }
                    disconnected = true;
                    break;
                }
                Err(error) => return Err(error).context("CDP Browser 响应读取失败"),
            };
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 64 * 1024,
                "CDP Browser 响应过大"
            );
            bytes.extend_from_slice(&chunk);
        }
        if disconnected {
            continue;
        }
        let browser: crate::cdp::CdpBrowserIdentity =
            serde_json::from_slice(&bytes).context("CDP Browser 身份格式无效")?;
        verify_stop_browser(identity, &browser)?;
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn verify_stop_browser(
    identity: &CodexInstanceIdentity,
    browser: &crate::cdp::CdpBrowserIdentity,
) -> anyhow::Result<()> {
    crate::cdp::validate_cdp_websocket_url(&browser.web_socket_debugger_url, identity.debug_port)?;
    anyhow::ensure!(
        browser.browser_id()? == identity.browser_id,
        "CDP Browser 身份冲突，拒绝停止或结束 worker"
    );
    Ok(())
}

pub(crate) fn codex_arguments_match(
    arguments: &[String],
    user_data_dir: &Path,
    debug_port: u16,
) -> bool {
    let mut user_paths = Vec::new();
    let mut ports = Vec::new();
    let mut tokens = arguments.iter().skip(1);
    while let Some(token) = tokens.next() {
        if let Some(value) = token.strip_prefix("--user-data-dir=") {
            user_paths.push(value.as_ref());
        } else if token == "--user-data-dir" {
            let Some(value) = tokens.next() else {
                return false;
            };
            user_paths.push(value.as_str());
        } else if let Some(value) = token.strip_prefix("--remote-debugging-port=") {
            ports.push(value);
        } else if token == "--remote-debugging-port" {
            let Some(value) = tokens.next() else {
                return false;
            };
            ports.push(value.as_str());
        }
    }
    if ports.len() != 1 || ports[0].parse::<u16>().ok() != Some(debug_port) || debug_port == 0 {
        return false;
    }
    if user_paths.len() != 1 || !user_data_dir.is_absolute() {
        return false;
    }
    let actual_path = Path::new(user_paths[0]);
    if !actual_path.is_absolute() {
        return false;
    }
    let Ok(expected) = user_data_dir.canonicalize() else {
        return false;
    };
    let Ok(actual) = actual_path.canonicalize() else {
        return false;
    };
    #[cfg(windows)]
    {
        expected
            .to_string_lossy()
            .eq_ignore_ascii_case(&actual.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        expected == actual
    }
}

async fn codex_browser_id(debug_port: u16) -> anyhow::Result<String> {
    let targets = crate::cdp::list_targets(debug_port).await?;
    crate::cdp::pick_injectable_codex_page_target(&targets)?;
    crate::cdp::browser_identity(debug_port).await?.browser_id()
}

#[cfg(target_os = "macos")]
fn macos_process_arguments(pid: u32) -> anyhow::Result<Vec<String>> {
    // KERN_PROCARGS2 返回 NUL 分隔的原始 argv，路径含空格时也不靠 ps 文本猜测。
    unsafe extern "C" {
        fn sysctl(
            name: *mut i32,
            namelen: u32,
            oldp: *mut std::ffi::c_void,
            oldlenp: *mut usize,
            newp: *mut std::ffi::c_void,
            newlen: usize,
        ) -> i32;
    }
    let mut argmax_mib = [1_i32, 8]; // CTL_KERN / KERN_ARGMAX。
    let mut argmax = 0_i32;
    let mut argmax_size = std::mem::size_of::<i32>();
    let result = unsafe {
        sysctl(
            argmax_mib.as_mut_ptr(),
            2,
            (&mut argmax as *mut i32).cast(),
            &mut argmax_size,
            std::ptr::null_mut(),
            0,
        )
    };
    anyhow::ensure!(
        result == 0 && (5..=2 * 1024 * 1024).contains(&argmax),
        "无法查询进程参数长度限制：{}",
        std::io::Error::last_os_error()
    );
    let mut mib = [1_i32, 49, i32::try_from(pid)?];
    let mut bytes = vec![0_u8; argmax as usize];
    let mut size = bytes.len();
    let result = unsafe {
        sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    anyhow::ensure!(
        result == 0 && size >= 5 && size <= bytes.len(),
        "无法读取多开进程 argv：{}",
        std::io::Error::last_os_error()
    );
    bytes.truncate(size);
    let count = i32::from_ne_bytes(bytes[..4].try_into()?);
    anyhow::ensure!((1..=32768).contains(&count), "多开进程 argv 数量无效");
    let mut offset = 4
        + bytes[4..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| anyhow::anyhow!("进程映像路径未终止"))?
        + 1;
    while bytes.get(offset) == Some(&0) {
        offset += 1;
    }
    let mut args = Vec::new();
    for _ in 0..count {
        let tail = bytes
            .get(offset..)
            .ok_or_else(|| anyhow::anyhow!("多开进程 argv 被截断"))?;
        let end = tail
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| anyhow::anyhow!("多开进程 argv 未终止"))?;
        args.push(std::str::from_utf8(&tail[..end])?.to_owned());
        offset += end + 1;
    }
    Ok(args)
}

#[cfg(target_os = "linux")]
fn linux_process_arguments(pid: u32) -> anyhow::Result<Vec<String>> {
    let content = std::fs::read(format!("/proc/{pid}/cmdline"))?;
    let args = content
        .split(|&byte| byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .collect::<Vec<_>>();
    anyhow::ensure!(!args.is_empty(), "进程参数为空");
    Ok(args)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn unix_process_arguments(pid: u32) -> anyhow::Result<Vec<String>> {
    #[cfg(target_os = "macos")]
    { macos_process_arguments(pid) }
    #[cfg(target_os = "linux")]
    { linux_process_arguments(pid) }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn unix_birth_token(pid: u32) -> Option<u64> {
    let crate::watcher::ProcessInstanceState::Running {
        birth_id: Some(birth),
        ..
    } = crate::watcher::inspect_process_instance(pid)
    else {
        return None;
    };
    Some(unix_birth_hash(&birth))
}

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn unix_birth_hash(birth: &str) -> u64 {
    birth.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub async fn capture_codex_instance_identity(
    instance_id: &str,
    generation: u64,
    debug_port: u16,
    helper_port: u16,
    user_data_dir: &Path,
    executable: &Path,
) -> anyhow::Result<CodexInstanceIdentity> {
    anyhow::ensure!(
        !instance_id.is_empty() && generation != 0,
        "多开实例身份不完整"
    );
    let browser_id = codex_browser_id(debug_port).await?;
    let candidates = crate::watcher::find_codex_processes_for_debug_port(debug_port);
    anyhow::ensure!(candidates.len() == 1, "无法唯一确认多开进程");
    let process_id = candidates[0];
    let process_birth_id =
        unix_birth_token(process_id).ok_or_else(|| anyhow::anyhow!("无法确认多开进程创建身份"))?;
    let identity = CodexInstanceIdentity {
        instance_id: instance_id.into(),
        generation,
        debug_port,
        helper_port,
        user_data_dir: canonical_identity_user_data(user_data_dir)?,
        browser_id,
        process_id,
        process_birth_id,
        executable: executable.canonicalize()?,
    };
    anyhow::ensure!(
        codex_instance_identity_matches(&identity).await?,
        "多开进程身份已改变"
    );
    Ok(identity)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub async fn codex_instance_identity_matches(
    identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    if !valid_stop_identity(identity)
        || identity.generation == 0
        || identity.instance_id.is_empty()
        || identity.browser_id.is_empty()
        || identity.debug_port == 0
        || identity.helper_port == 0
        || unix_birth_token(identity.process_id) != Some(identity.process_birth_id)
    {
        return Ok(false);
    }
    let args = unix_process_arguments(identity.process_id)?;
    if !codex_identity_arguments_match(&args, identity)
        || args
            .first()
            .and_then(|path| Path::new(path).canonicalize().ok())
            .as_ref()
            != Some(&identity.executable)
    {
        return Ok(false);
    }
    Ok(
        codex_browser_id(identity.debug_port).await? == identity.browser_id
            && unix_birth_token(identity.process_id) == Some(identity.process_birth_id),
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub async fn terminate_verified_codex_instance(
    identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    let result = async {
        if !codex_instance_identity_matches(identity).await? {
            return Ok(false);
        }
        let browser = crate::cdp::browser_identity(identity.debug_port).await?;
        if browser.browser_id()? != identity.browser_id {
            return Ok(false);
        }
        // 发给这一代 Browser 的专属 WebSocket，不按 PID/端口发送 kill。
        crate::bridge::send_cdp_command(
            &browser.web_socket_debugger_url,
            "Browser.close",
            serde_json::json!({}),
        )
        .await?;
        Ok(true)
    }
    .await;
    record_stop_verification_failure(identity, &result);
    result
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn capture_codex_instance_identity(
    _instance_id: &str,
    _generation: u64,
    _debug_port: u16,
    _helper_port: u16,
    _user_data_dir: &std::path::Path,
    _executable: &std::path::Path,
) -> anyhow::Result<CodexInstanceIdentity> {
    anyhow::bail!("当前系统无法安全确认多开进程身份")
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn codex_instance_identity_matches(
    _identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    Ok(false)
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub async fn terminate_verified_codex_instance(
    _identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(windows)]
pub async fn capture_codex_instance_identity(
    instance_id: &str,
    generation: u64,
    debug_port: u16,
    helper_port: u16,
    user_data_dir: &Path,
    executable: &Path,
) -> anyhow::Result<CodexInstanceIdentity> {
    use anyhow::Context;
    anyhow::ensure!(
        !instance_id.trim().is_empty() && generation != 0 && debug_port != 0 && helper_port != 0,
        "多开实例身份字段不完整"
    );
    let user_data_dir = canonical_identity_user_data(user_data_dir)?;
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let browser_id = codex_browser_id(debug_port).await?;
    let process = crate::windows_integration::capture_debug_listener(debug_port, &executable)?
        .context("无法确认多开调试端口所属进程")?;
    let arguments = crate::windows_integration::process_arguments(process.process_id)
        .context("无法读取多开进程数据目录")?;
    let identity = CodexInstanceIdentity {
        instance_id: instance_id.into(),
        generation,
        debug_port,
        helper_port,
        user_data_dir,
        browser_id,
        process_id: process.process_id,
        process_birth_id: process.birth_id,
        executable,
    };
    anyhow::ensure!(
        codex_identity_arguments_match(&arguments, &identity),
        "多开进程参数与实例身份不一致"
    );
    anyhow::ensure!(
        process.is_alive()? && codex_instance_identity_matches(&identity).await?,
        "多开进程在确认身份期间已改变"
    );
    Ok(identity)
}

#[cfg(windows)]
pub async fn codex_instance_identity_matches(
    identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    if !valid_stop_identity(identity)
        || identity.instance_id.is_empty()
        || identity.generation == 0
        || identity.browser_id.is_empty()
        || identity.process_id == 0
        || identity.process_birth_id == 0
        || identity.debug_port == 0
        || identity.helper_port == 0
    {
        return Ok(false);
    }
    let Some(process) = crate::windows_integration::capture_debug_listener(
        identity.debug_port,
        &identity.executable,
    )?
    else {
        return Ok(false);
    };
    if process.process_id != identity.process_id || process.birth_id != identity.process_birth_id {
        return Ok(false);
    }
    let Some(arguments) = crate::windows_integration::process_arguments(process.process_id) else {
        return Ok(false);
    };
    if !codex_identity_arguments_match(&arguments, identity) {
        return Ok(false);
    }
    Ok(
        codex_browser_id(identity.debug_port).await? == identity.browser_id
            && process.is_alive()?,
    )
}

#[cfg(windows)]
pub async fn terminate_verified_codex_instance(
    identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    let result = async {
        if !valid_stop_identity(identity) {
            return Ok(false);
        }
        // 捕获到退出确认全程持有句柄，CDP 只提供额外的否决证据。
        let Some(mut tree) = CodexInstanceProcessTree::capture(identity)? else {
            // 原代根已确认退出；不把正常退出竞争视为身份冲突或接管其他进程。
            return Ok(true);
        };
        check_stop_browser_identity(identity).await?;
        tree.stop_and_wait()?;
        Ok(true)
    }
    .await;
    record_stop_verification_failure(identity, &result);
    result
}

#[cfg(any(windows, test))]
pub(crate) fn worker_identity_matches(
    expected_birth: Option<u64>,
    actual_birth: Option<u64>,
    arguments: &[String],
    options_path: &Path,
) -> bool {
    // 旧清单没有创建身份，或者 PID 已复用时，宁可留下 worker 也不猜测归属。
    if expected_birth.is_none()
        || expected_birth != actual_birth
        || arguments.len() != 3
        || arguments[1] != "--codex-plus-multi-worker"
    {
        return false;
    }
    let Ok(expected) = options_path.canonicalize() else {
        return false;
    };
    let Ok(actual) = Path::new(&arguments[2]).canonicalize() else {
        return false;
    };
    expected == actual
}

#[cfg(windows)]
pub fn codex_process_generation_exited(
    pid: u32,
    expected_birth: Option<u64>,
) -> anyhow::Result<bool> {
    process_generation_exited_with(
        pid,
        expected_birth,
        crate::windows_integration::process_lifetime,
    )
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn process_generation_exited_with(
    pid: u32,
    expected_birth: Option<u64>,
    query: impl FnOnce(u32) -> anyhow::Result<Option<(u64, bool)>>,
) -> anyhow::Result<bool> {
    if pid == 0 || expected_birth == Some(0) {
        return Ok(false);
    }
    let Some((birth, exited)) = query(pid)? else {
        return Ok(true);
    };
    if exited {
        return Ok(true);
    }
    anyhow::ensure!(birth != 0, "无法确认进程创建身份");
    Ok(expected_birth.is_some_and(|expected| expected != birth))
}

#[cfg(target_os = "macos")]
pub fn codex_process_generation_exited(
    pid: u32,
    expected_birth: Option<u64>,
) -> anyhow::Result<bool> {
    process_generation_exited_with(pid, expected_birth, |pid| {
        let output = std::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat=", "-o", "lstart="])
            .env("LC_ALL", "C")
            .output()?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(None);
        }
        if !output.status.success() {
            return Err(std::io::Error::other("无法读取进程创建身份").into());
        }
        Ok(Some(parse_macos_process_lifetime(&output.stdout)?))
    })
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_process_lifetime(bytes: &[u8]) -> std::io::Result<(u64, bool)> {
    let invalid = || std::io::Error::other("无法读取进程状态或创建身份");
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?.trim();
    let (state, birth) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    if state.starts_with('Z') {
        return Ok((0, true));
    }
    let birth = birth.trim();
    if state.is_empty() || birth.split_whitespace().count() != 5 {
        return Err(invalid());
    }
    Ok((unix_birth_hash(birth), false))
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_process_lifetime(stat: &str) -> std::io::Result<(u64, bool)> {
    let invalid = || std::io::Error::other("无法读取 /proc 进程创建身份");
    let state = stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().next())
        .ok_or_else(invalid)?;
    let birth = crate::watcher::parse_linux_proc_stat_starttime(stat).ok_or_else(invalid)?;
    if birth.parse::<u64>().is_err() || state.len() != 1 {
        return Err(invalid());
    }
    Ok((unix_birth_hash(&birth), matches!(state, "Z" | "X")))
}

#[cfg(target_os = "linux")]
fn linux_process_lifetime(pid: u32) -> anyhow::Result<Option<(u64, bool)>> {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => Ok(Some(parse_linux_process_lifetime(&stat)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(target_os = "linux")]
pub fn codex_process_generation_exited(pid: u32, expected_birth: Option<u64>) -> anyhow::Result<bool> {
    process_generation_exited_with(pid, expected_birth, |pid| {
        let Some(snapshot) = linux_process_lifetime(pid)? else {
            return Ok(None);
        };
        if snapshot.1 {
            return Ok(Some(snapshot));
        }
        // 保留已发布的 Unix 出生标记；/proc starttime 复核包围 ps 读取，避免读取期间换代。
        let birth = unix_birth_token(pid);
        let current = linux_process_lifetime(pid)?;
        if current.is_none_or(|(_, exited)| exited) {
            return Ok(current);
        }
        if current != Some(snapshot) {
            return Err(std::io::Error::other("读取进程身份期间发生换代").into());
        }
        let birth = birth.ok_or_else(|| std::io::Error::other("无法确认进程创建身份"))?;
        Ok(Some((birth, false)))
    })
}

#[cfg(windows)]
pub fn codex_user_data_is_absent(
    executable: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    helper_port: u16,
) -> anyhow::Result<bool> {
    use crate::windows_integration::bounded_process_snapshot;
    anyhow::ensure!(
        debug_port != 0 && helper_port != 0 && debug_port != helper_port,
        "实例端口无效"
    );
    anyhow::ensure!(
        executable.is_absolute() && user_data_dir.is_absolute(),
        "多开路径必须是绝对路径"
    );
    let resolved_executable = scan_image_path_with(executable, |path| path.canonicalize())?;
    let user_data_dir = user_data_dir.canonicalize()?;
    let name = resolved_executable
        .as_deref()
        .unwrap_or(executable)
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("缺少桌面程序名"))?;
    for entry in bounded_process_snapshot(32768)? {
        if !entry.exe_file.eq_ignore_ascii_case(&name.to_string_lossy()) {
            continue;
        }
        if windows_process_uses_user_data(entry.process_id, executable, &user_data_dir)? {
            return Ok(false);
        }
    }
    loopback_ports_are_free_with(
        debug_port,
        helper_port,
        |host, port| std::net::TcpListener::bind((host, port)),
        |port| {
            let owners = crate::windows_integration::loopback_listener_process_ids(port)?;
            anyhow::ensure!(!owners.is_empty(), "占用端口的进程归属尚未确认");
            for pid in &owners {
                if windows_process_uses_user_data(*pid, executable, &user_data_dir)? {
                    return Ok(false);
                }
            }
            anyhow::ensure!(
                crate::windows_integration::loopback_listener_process_ids(port)? == owners,
                "端口归属在核验期间改变"
            );
            Ok(true)
        },
    )
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn process_read_or_exited<T>(
    read: anyhow::Result<T>,
    exited: impl FnOnce() -> anyhow::Result<bool>,
) -> anyhow::Result<Option<T>> {
    match read {
        Ok(value) => Ok(Some(value)),
        Err(_) if matches!(exited(), Ok(true)) => Ok(None),
        Err(error) => Err(std::io::Error::other(error).into()),
    }
}

#[cfg(any(windows, target_os = "macos", test))]
fn scan_image_path_with(
    path: &Path,
    resolve: impl FnOnce(&Path) -> std::io::Result<PathBuf>,
) -> std::io::Result<Option<PathBuf>> {
    use std::io::{Error, ErrorKind};
    if !path.is_absolute() {
        return Err(Error::from(ErrorKind::InvalidInput));
    }
    // 映像允许安装路径消失或权限受限；数据目录仍由调用方严格解析。
    match resolve(path) {
        Ok(resolved) => Ok(Some(resolved)),
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::PermissionDenied) => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn scan_images_match_with(
    actual: &Path,
    expected: &Path,
    mut resolve: impl FnMut(&Path) -> std::io::Result<PathBuf>,
) -> std::io::Result<bool> {
    let resolved_actual = scan_image_path_with(actual, &mut resolve)?;
    let resolved_expected = scan_image_path_with(expected, resolve)?;
    Ok(match (resolved_actual, resolved_expected) {
        (Some(actual), Some(expected)) => windows_process_tree::same_resolved_path(&actual, &expected),
        _ => windows_process_tree::same_resolved_path(actual, expected),
    })
}

#[cfg(windows)]
fn windows_process_uses_user_data(
    pid: u32,
    executable: &Path,
    user_data: &Path,
) -> anyhow::Result<bool> {
    use crate::windows_integration::{VerifiedStopProcess, process_lifetime};
    let captured = process_read_or_exited(VerifiedStopProcess::capture(pid, false), || {
        Ok(process_lifetime(pid)?.is_none_or(|(_, exited)| exited))
    })?;
    let Some(Some(process)) = captured else {
        return Ok(false);
    };
    if !process.is_alive()? {
        return Ok(false);
    }
    let Some(image_matches) = process_read_or_exited(
        process.image().and_then(|image| {
            Ok(scan_images_match_with(&image, executable, |path| path.canonicalize())?)
        }),
        || Ok(!process.is_alive()?),
    )?
    else {
        return Ok(false);
    };
    if !image_matches {
        return Ok(false);
    }
    let Some(arguments) = process_read_or_exited(process.arguments(), || Ok(!process.is_alive()?))?
    else {
        return Ok(false);
    };
    arguments_user_data_matches(&arguments, user_data)
}

#[cfg(target_os = "macos")]
pub fn codex_user_data_is_absent(
    executable: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    helper_port: u16,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        debug_port != 0 && helper_port != 0 && debug_port != helper_port,
        "实例端口无效"
    );
    anyhow::ensure!(
        executable.is_absolute() && user_data_dir.is_absolute(),
        "多开路径必须是绝对路径"
    );
    let resolved_executable = scan_image_path_with(executable, |path| path.canonicalize())?;
    let fallback_uid = resolved_executable.is_none().then(|| unsafe { libc::geteuid() });
    let executable = resolved_executable.unwrap_or_else(|| executable.to_path_buf());
    let user_data_dir = user_data_dir.canonicalize()?;
    let snapshot = macos_process_snapshot()?;
    if !macos_snapshot_user_data_is_absent_with(
        &snapshot,
        &executable,
        &user_data_dir,
        fallback_uid,
        macos_process_arguments,
        macos_process_snapshot,
    )? {
        return Ok(false);
    }
    loopback_ports_are_free_with(
        debug_port,
        helper_port,
        |host, port| std::net::TcpListener::bind((host, port)),
        |port| {
            let owners = macos_listener_process_ids(port)?;
            for pid in &owners {
                let process = snapshot
                    .get(pid)
                    .ok_or_else(|| std::io::Error::other("监听进程不在本次快照中"))?;
                if !macos_process_may_use_user_data(process, &executable, fallback_uid) {
                    continue;
                }
                let arguments = process_read_or_exited(macos_process_arguments(*pid), || {
                    Ok(!macos_process_snapshot()?.contains_key(pid))
                })?;
                if let Some(arguments) = arguments
                    && arguments_user_data_matches(&arguments, &user_data_dir)?
                {
                    return Ok(false);
                }
            }
            anyhow::ensure!(
                macos_listener_process_ids(port)? == owners,
                "端口归属在核验期间改变"
            );
            Ok(true)
        },
    )
}

#[cfg(target_os = "macos")]
fn macos_process_snapshot() -> anyhow::Result<std::collections::BTreeMap<u32, MacosSnapshotProcess>> {
    let output = std::process::Command::new("ps")
        .args(["-axww", "-o", "pid=,uid=,stat=,comm="])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("无法读取进程快照").into());
    }
    parse_macos_process_snapshot(&output.stdout)
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone)]
struct MacosSnapshotProcess {
    uid: u32,
    image: PathBuf,
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_process_snapshot(
    bytes: &[u8],
) -> anyhow::Result<std::collections::BTreeMap<u32, MacosSnapshotProcess>> {
    anyhow::ensure!(
        bytes.len() <= 8 * 1024 * 1024 && bytes.ends_with(b"\n"),
        "进程快照截断或过大"
    );
    let snapshot = std::str::from_utf8(bytes)?;
    anyhow::ensure!(
        !snapshot.trim().is_empty() && snapshot.lines().count() <= 32768,
        "进程快照无效"
    );
    let mut processes = std::collections::BTreeMap::new();
    for line in snapshot.lines() {
        let (pid, rest) = line
            .trim()
            .split_once(char::is_whitespace)
            .ok_or_else(|| anyhow::anyhow!("进程快照缺少映像"))?;
        let pid = pid.parse::<u32>()?;
        let (uid, rest) = rest.trim().split_once(char::is_whitespace)
            .ok_or_else(|| anyhow::anyhow!("进程快照缺少用户身份"))?;
        let uid = uid.parse::<u32>()?;
        let (state, image) = rest.trim().split_once(char::is_whitespace)
            .ok_or_else(|| anyhow::anyhow!("进程快照缺少状态或映像"))?;
        anyhow::ensure!(!image.trim().is_empty(), "进程快照缺少映像");
        if state.starts_with('Z') {
            continue;
        }
        anyhow::ensure!(
            processes.insert(pid, MacosSnapshotProcess { uid, image: PathBuf::from(image.trim()) }).is_none(),
            "进程快照含重复 PID"
        );
    }
    Ok(processes)
}

#[cfg(any(target_os = "macos", test))]
fn macos_image_is_candidate(image: &Path, executable: &Path) -> bool {
    image == executable
        || executable
            .parent()
            .and_then(Path::parent)
            .is_some_and(|contents| image.starts_with(contents.join("Frameworks")))
}

#[cfg(any(target_os = "macos", test))]
fn macos_process_may_use_user_data(
    process: &MacosSnapshotProcess,
    executable: &Path,
    fallback_uid: Option<u32>,
) -> bool {
    fallback_uid.is_some_and(|uid| process.uid == uid)
        || macos_image_is_candidate(&process.image, executable)
}

#[cfg(any(target_os = "macos", test))]
fn macos_snapshot_user_data_is_absent_with(
    snapshot: &std::collections::BTreeMap<u32, MacosSnapshotProcess>,
    executable: &Path,
    user_data: &Path,
    fallback_uid: Option<u32>,
    mut arguments: impl FnMut(u32) -> anyhow::Result<Vec<String>>,
    refresh: impl FnOnce() -> anyhow::Result<std::collections::BTreeMap<u32, MacosSnapshotProcess>>,
) -> anyhow::Result<bool> {
    let mut failures = Vec::new();
    for (pid, process) in snapshot {
        if !macos_process_may_use_user_data(process, executable, fallback_uid) {
            continue;
        }
        let args = match arguments(*pid) {
            Ok(args) => args,
            Err(error) => {
                failures.push((*pid, error));
                continue;
            }
        };
        match arguments_user_data_matches(&args, user_data) {
            Ok(true) => return Ok(false),
            Ok(false) => {}
            Err(error) if error.is::<std::io::Error>() => failures.push((*pid, error)),
            Err(error) => return Err(error),
        }
    }
    if !failures.is_empty() {
        let current = refresh()?;
        for (pid, error) in failures {
            if current.contains_key(&pid) {
                return Err(std::io::Error::other(error).into());
            }
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn macos_listener_process_ids(port: u16) -> anyhow::Result<Vec<u32>> {
    let output = std::process::Command::new("/usr/sbin/lsof")
        .args(["-nP", "-a", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other("无法查询监听进程归属").into());
    }
    parse_macos_listener_process_ids(&output.stdout)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_listener_process_ids(bytes: &[u8]) -> anyhow::Result<Vec<u32>> {
    anyhow::ensure!(
        bytes.len() <= 1024 * 1024 && bytes.ends_with(b"\n"),
        "监听进程快照截断或过大"
    );
    let text = std::str::from_utf8(bytes)?;
    let mut ids = Vec::new();
    for line in text.lines() {
        let pid = line.parse::<u32>()?;
        anyhow::ensure!(pid != 0 && ids.len() < 32768, "监听进程快照无效");
        ids.push(pid);
    }
    ids.sort_unstable();
    ids.dedup();
    anyhow::ensure!(!ids.is_empty(), "占用端口的进程归属尚未确认");
    Ok(ids)
}

#[cfg(target_os = "linux")]
pub fn codex_user_data_is_absent(
    executable: &Path,
    user_data_dir: &Path,
    debug_port: u16,
    helper_port: u16,
) -> anyhow::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    anyhow::ensure!(executable.is_absolute() && user_data_dir.is_absolute(), "多开路径必须是绝对路径");
    let user_data = user_data_dir.canonicalize()?;
    let uid = std::fs::metadata("/proc/self")?.uid();
    // 同用户的任意安装版本都不能继续使用该目录；僵尸已不持有目录或监听端口。
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else {
            continue;
        };
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.uid() != uid {
            continue;
        }
        let Some((birth, false)) = linux_process_lifetime(pid)? else {
            continue;
        };
        let arguments = process_read_or_exited(linux_process_arguments(pid), || {
            Ok(linux_process_lifetime(pid)?.is_none_or(|current| current.1 || current.0 != birth))
        })?;
        if let Some(arguments) = arguments
            && arguments_user_data_matches(&arguments, &user_data)?
        {
            return Ok(false);
        }
    }
    // 无法确认占用者时保留 unknown，不因端口可达而停止其他实例。
    loopback_ports_are_free_with(debug_port, helper_port,
        |host, port| std::net::TcpListener::bind((host, port)), |_| Ok(false))
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn loopback_ports_are_free_with<T>(
    debug_port: u16,
    helper_port: u16,
    mut bind: impl FnMut(&str, u16) -> std::io::Result<T>,
    mut owners_are_unrelated: impl FnMut(u16) -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        debug_port != 0 && helper_port != 0 && debug_port != helper_port,
        "实例端口无效"
    );
    let mut reservations = Vec::new();
    for port in [debug_port, helper_port] {
        let mut occupied = false;
        for host in ["127.0.0.1", "::1"] {
            match bind(host, port) {
                Ok(listener) => reservations.push(listener),
                Err(error)
                    if host == "::1"
                        && (error.kind() == std::io::ErrorKind::AddrNotAvailable
                            || matches!(error.raw_os_error(), Some(47 | 49 | 97 | 10047 | 10049))) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => occupied = true,
                Err(error) => return Err(error.into()),
            }
        }
        if occupied && !owners_are_unrelated(port)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(any(windows, target_os = "macos", target_os = "linux", test))]
fn arguments_user_data_matches(arguments: &[String], expected: &Path) -> anyhow::Result<bool> {
    let mut paths = Vec::new();
    let mut tokens = arguments.iter().skip(1);
    while let Some(token) = tokens.next() {
        if token == "--user-data-dir" {
            paths.push(
                tokens
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("数据目录参数缺少值"))?
                    .as_str(),
            );
        } else if let Some(value) = token.strip_prefix("--user-data-dir=") {
            paths.push(value);
        }
    }
    anyhow::ensure!(paths.len() <= 1, "数据目录参数重复，无法确认归属");
    let Some(path) = paths.first() else {
        return Ok(false);
    };
    anyhow::ensure!(!path.is_empty(), "数据目录参数为空");
    let path = Path::new(path);
    anyhow::ensure!(path.is_absolute(), "进程数据目录是相对路径，无法确认归属");
    let actual = path.canonicalize()?;
    #[cfg(windows)]
    return Ok(actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy()));
    #[cfg(not(windows))]
    Ok(actual == expected)
}

#[cfg(any(windows, test))]
pub(crate) fn worker_image_matches(image: &Path, arguments: &[String]) -> bool {
    let Some(argv_image) = arguments.first().map(Path::new) else {
        return false;
    };
    // 相对 argv[0] 不能借用新管理器的工作目录解释为旧 worker 映像。
    if !image.is_absolute() || !argv_image.is_absolute() {
        return false;
    }
    let (Ok(actual), Ok(argument)) = (image.canonicalize(), argv_image.canonicalize()) else {
        return false;
    };
    #[cfg(windows)]
    {
        actual
            .to_string_lossy()
            .eq_ignore_ascii_case(&argument.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        actual == argument
    }
}

/// Child 尚未回收，PID 不会被复用；Windows 直接读取持有的句柄。
pub fn worker_birth_id(child: &std::process::Child) -> Option<u64> {
    #[cfg(windows)]
    {
        crate::windows_integration::child_process_birth_id(child)
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        unix_birth_token(child.id())
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = child;
        None
    }
}

#[cfg(windows)]
pub fn terminate_verified_worker(
    pid: u32,
    options_path: &Path,
    expected_birth: Option<u64>,
) -> anyhow::Result<bool> {
    crate::windows_integration::terminate_verified_multi_worker(pid, options_path, expected_birth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn absence_image_fallback_only_accepts_missing_or_denied_absolute_images() {
        use std::io::{Error, ErrorKind};
        let image = Path::new(r"C:\Program Files\WindowsApps\Codex\ChatGPT.exe");
        let equivalent = Path::new(r"\\?\C:\PROGRAM FILES\WINDOWSAPPS\CODEX\CHATGPT.EXE");
        let neighbor = Path::new(r"C:\Program Files\OtherApps\Codex\ChatGPT.exe");
        for kind in [ErrorKind::NotFound, ErrorKind::PermissionDenied] {
            assert!(scan_image_path_with(image, |_| Err(Error::from(kind))).unwrap().is_none());
            assert!(scan_images_match_with(image, equivalent, |_| Err(Error::from(kind))).unwrap());
            assert!(!scan_images_match_with(image, neighbor, |_| Err(Error::from(kind))).unwrap());
            assert!(scan_images_match_with(Path::new("ChatGPT.exe"), image, |_| {
                Err(Error::from(kind))
            }).is_err());
        }
        for kind in [ErrorKind::Other, ErrorKind::InvalidData, ErrorKind::TimedOut] {
            assert!(scan_images_match_with(image, image, |_| Err(Error::from(kind))).is_err());
            assert!(scan_images_match_with(image, neighbor, |path| {
                Err(Error::from(if path == image { ErrorKind::PermissionDenied } else { kind }))
            }).is_err());
        }
    }

    #[cfg(windows)]
    #[test]
    fn absence_image_comparison_prefers_resolved_paths_and_preserves_original_fallback() {
        use std::io::{Error, ErrorKind};
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("ChatGPT.exe");
        let neighbor = temp.path().join("Neighbor.exe");
        let mut calls = 0;
        assert!(!scan_images_match_with(&image, &image, |_| {
            calls += 1;
            Ok(if calls == 1 { image.clone() } else { neighbor.clone() })
        }).unwrap());
        assert!(scan_images_match_with(&image, &image, |path| {
            if path == image && calls == 2 {
                calls += 1;
                Ok(PathBuf::from(format!(r"\\?\{}", path.display())))
            } else {
                Err(Error::from(ErrorKind::PermissionDenied))
            }
        }).unwrap());
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        std::fs::write(&image, "").unwrap();
        let alias = temp.path().join("nested/../ChatGPT.exe");
        assert!(scan_images_match_with(&image, &alias, |path| path.canonicalize()).unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn absence_image_missing_installation_does_not_block_native_neighbor_ports() {
        let temp = tempfile::tempdir().unwrap();
        let removed_image = temp.path().join("removed-installation/absence-image-fixture.exe");
        let user_data = temp.path().join("own-data");
        std::fs::create_dir(&user_data).unwrap();
        let debug = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let helper = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let debug_port = debug.local_addr().unwrap().port();
        let helper_port = helper.local_addr().unwrap().port();
        assert!(codex_user_data_is_absent(&removed_image, &user_data, debug_port, helper_port).unwrap());
        assert!(codex_user_data_is_absent(
            &removed_image, &temp.path().join("missing-user-data"), debug_port, helper_port
        ).is_err());
        assert!(debug.local_addr().is_ok() && helper.local_addr().is_ok());
    }

    #[test]
    fn unix_generation_queries_accept_zombies_and_retry_missing_birth() {
        let birth = "Fri Oct 30 19:18:27 2026";
        let live = parse_macos_process_lifetime(format!(" S+ {birth}\n").as_bytes()).unwrap();
        assert_eq!(live, (unix_birth_hash(birth), false));
        assert!(!process_generation_exited_with(42, Some(live.0), |_| Ok(Some(live))).unwrap());
        assert!(process_generation_exited_with(42, Some(live.0 + 1), |_| Ok(Some(live))).unwrap());
        for state in ["Z", "Z+", "Z  Fri Oct 30 19:18:27 2026"] {
            let lifetime = parse_macos_process_lifetime(state.as_bytes()).unwrap();
            assert!(process_generation_exited_with(42, Some(live.0), |_| Ok(Some(lifetime))).unwrap());
        }
        for missing in ["", "S", "S Fri Oct 30", "S\n"] {
            let error = process_generation_exited_with(42, Some(live.0), |_| {
                Ok(Some(parse_macos_process_lifetime(missing.as_bytes())?))
            }).unwrap_err();
            assert!(error.is::<std::io::Error>());
        }
    }

    #[test]
    fn linux_proc_snapshot_checks_starttime_and_handles_spaced_parentheses_and_zombies() {
        let stat = |state, birth| format!("42 (Codex (test) worker) {state} {}{birth} 0 0\n", "0 ".repeat(18));
        let live = parse_linux_process_lifetime(&stat("S", "12345")).unwrap();
        assert_eq!(live, (unix_birth_hash("12345"), false));
        for state in ["Z", "X"] {
            let lifetime = parse_linux_process_lifetime(&stat(state, "12345")).unwrap();
            assert!(process_generation_exited_with(42, Some(live.0), |_| Ok(Some(lifetime))).unwrap());
        }
        let reused = parse_linux_process_lifetime(&stat("R", "12346")).unwrap();
        assert!(process_generation_exited_with(42, Some(live.0), |_| Ok(Some(reused))).unwrap());
        assert!(!process_generation_exited_with(42, None, |_| Ok(Some(live))).unwrap());
        for invalid in ["42 (missing) S", "invalid", &stat("S", "not-a-time")] {
            assert!(parse_linux_process_lifetime(invalid).is_err());
        }
    }

    #[test]
    fn unix_scan_image_fallback_requires_absolute_path_and_known_resolution_errors() {
        use std::io::{Error, ErrorKind};
        let temp = tempfile::tempdir().unwrap();
        let removed = temp.path().join("removed-app/Contents/MacOS/Codex");
        assert!(scan_image_path_with(&removed, |path| path.canonicalize()).unwrap().is_none());
        assert!(scan_image_path_with(&removed, |_| Err(ErrorKind::PermissionDenied.into())).unwrap().is_none());
        assert!(scan_image_path_with(&removed, |_| Err(ErrorKind::InvalidData.into())).is_err());
        assert!(scan_image_path_with(Path::new("relative-app"), |_| Ok(removed.clone())).is_err());
        assert!(scan_image_path_with(&removed, |_| Err(Error::from(ErrorKind::Interrupted))).is_err());
    }

    #[test]
    fn macos_snapshot_preserves_spaced_images_and_rejects_incomplete_output() {
        let snapshot = parse_macos_process_snapshot(b" 42 501 S /Applications/Codex Dev.app/Contents/MacOS/Codex Dev\n 43 501 S+ /Applications/Codex Dev.app/Contents/Frameworks/Helper (Renderer)\n 44 501 Z /Applications/Codex Dev.app/Contents/MacOS/Codex Dev\n").unwrap();
        let executable = Path::new("/Applications/Codex Dev.app/Contents/MacOS/Codex Dev");
        assert_eq!(snapshot.get(&42).unwrap().image.as_path(), executable);
        assert_eq!(snapshot.get(&42).unwrap().uid, 501);
        assert!(!snapshot.contains_key(&44));
        assert!(macos_image_is_candidate(
            &snapshot.get(&43).unwrap().image,
            executable
        ));
        for invalid in [
            &b""[..],
            &b"\n"[..],
            &b"42"[..],
            &b"42 image"[..],
            &b"42 \n"[..],
            &b"42 501 S image\n42 501 S other\n"[..],
            &b"not-pid 501 S image\n"[..],
            &b"42 not-uid S image\n"[..],
        ] {
            assert!(parse_macos_process_snapshot(invalid).is_err());
        }
        assert!(parse_macos_process_snapshot(&vec![b'x'; 8 * 1024 * 1024 + 1]).is_err());
        assert!(parse_macos_process_snapshot("42 S image\n".repeat(32769).as_bytes()).is_err());
        assert_eq!(
            parse_macos_listener_process_ids(b"42\n43\n42\n").unwrap(),
            vec![42, 43]
        );
        for invalid in [
            &b""[..],
            &b"42"[..],
            &b"0\n"[..],
            &b"x42\n"[..],
            &b"42\nbad\n"[..],
        ] {
            assert!(parse_macos_listener_process_ids(invalid).is_err());
        }
    }

    #[test]
    fn macos_snapshot_refreshes_once_for_many_failed_reads_and_never_assumes_exit() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().join("own data");
        let neighbor = temp.path().join("neighbor data");
        std::fs::create_dir(&own).unwrap();
        std::fs::create_dir(&neighbor).unwrap();
        let own = own.canonicalize().unwrap();
        let executable = Path::new("/Applications/Codex Dev.app/Contents/MacOS/Codex Dev");
        let snapshot = parse_macos_process_snapshot(
            (1..=100)
                .map(|pid| {
                    format!(
                        "{pid} 501 S /Applications/Codex Dev.app/Contents/Frameworks/Helper (Renderer)\n"
                    )
                })
                .collect::<String>()
                .as_bytes(),
        )
        .unwrap();
        let reads = Cell::new(0);
        assert!(
            macos_snapshot_user_data_is_absent_with(
                &snapshot,
                executable,
                &own,
                None,
                |_| {
                    reads.set(reads.get() + 1);
                    Ok(vec![
                        "Helper".into(),
                        format!("--user-data-dir={}", neighbor.display()),
                    ])
                },
                || panic!("successful reads do not spawn another ps")
            )
            .unwrap()
        );
        assert_eq!(reads.get(), 100);
        assert!(
            !macos_snapshot_user_data_is_absent_with(
                &snapshot,
                executable,
                &own,
                None,
                |_| {
                    Ok(vec![
                        "Helper".into(),
                        format!("--user-data-dir={}", own.display()),
                    ])
                },
                || panic!("live own process")
            )
            .unwrap()
        );
        for gone in [false, true] {
            let refreshes = Cell::new(0);
            let result = macos_snapshot_user_data_is_absent_with(
                &snapshot,
                executable,
                &own,
                None,
                |_| anyhow::bail!("process arguments unreadable"),
                || {
                    refreshes.set(refreshes.get() + 1);
                    if gone {
                        Ok(Default::default())
                    } else {
                        Ok(snapshot.clone())
                    }
                },
            );
            assert_eq!(refreshes.get(), 1);
            if gone {
                assert!(result.unwrap());
            } else {
                assert!(result.is_err());
            }
        }
        assert!(
            macos_snapshot_user_data_is_absent_with(
                &snapshot,
                executable,
                &own,
                None,
                |_| { anyhow::bail!("unreadable") },
                || anyhow::bail!("snapshot failed")
            )
            .is_err()
        );
    }

    #[test]
    fn macos_migrated_images_with_the_same_user_data_are_not_ignored() {
        use std::cell::Cell;
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().canonicalize().unwrap();
        let old_image = temp.path().join("Old Codex.app/Contents/MacOS/Codex");
        assert!(old_image.canonicalize().is_err());
        let moved_image = Path::new("/Applications/New Codex.app/Contents/MacOS/ChatGPT");
        let moved_helper = Path::new("/Applications/New Codex.app/Contents/Frameworks/Helper");
        let other_image = Path::new("/Applications/Other.app/Contents/MacOS/Other");
        let current_uid = 501;
        let snapshot = [
            (42, MacosSnapshotProcess { uid: current_uid, image: moved_image.into() }),
            (43, MacosSnapshotProcess { uid: current_uid, image: moved_helper.into() }),
            (44, MacosSnapshotProcess { uid: 502, image: other_image.into() }),
        ]
        .into_iter()
        .collect();
        assert!(macos_snapshot_user_data_is_absent_with(
            &snapshot,
            &old_image,
            &own,
            None,
            |_| panic!("normal scans skip unrelated same-user processes"),
            || panic!("successful reads do not require a refresh"),
        )
        .unwrap());
        for holder in [42, 43] {
            let read_other_user = Cell::new(false);
            assert!(!macos_snapshot_user_data_is_absent_with(
                &snapshot,
                &old_image,
                &own,
                Some(current_uid),
                |pid| {
                    if pid == 44 {
                        read_other_user.set(true);
                    }
                    Ok(if pid == holder {
                        vec!["Codex".into(), format!("--user-data-dir={}", own.display())]
                    } else {
                        vec!["unrelated".into()]
                    })
                },
                || panic!("successful reads do not require a refresh"),
            )
            .unwrap());
            assert!(!read_other_user.get());
        }
        assert!(macos_snapshot_user_data_is_absent_with(
            &snapshot,
            &old_image,
            &own,
            Some(current_uid),
            |pid| {
                assert_ne!(pid, 44);
                Ok(vec!["unrelated".into()])
            },
            || panic!("successful reads do not require a refresh"),
        )
        .unwrap());
        let mut original_image_other_user = snapshot.clone();
        original_image_other_user.insert(45, MacosSnapshotProcess {
            uid: 502,
            image: old_image.clone(),
        });
        assert!(!macos_snapshot_user_data_is_absent_with(
            &original_image_other_user,
            &old_image,
            &own,
            Some(current_uid),
            |pid| {
                Ok(if pid == 45 {
                    vec!["Codex".into(), format!("--user-data-dir={}", own.display())]
                } else {
                    vec!["unrelated".into()]
                })
            },
            || panic!("successful reads do not require a refresh"),
        )
        .unwrap());
        assert!(macos_snapshot_user_data_is_absent_with(
            &snapshot,
            &old_image,
            &own,
            Some(current_uid),
            |pid| {
                assert_ne!(pid, 44);
                if pid == 42 {
                    anyhow::bail!("migrated process arguments unavailable")
                }
                Ok(vec!["unrelated".into()])
            },
            || Ok(snapshot.clone()),
        )
        .is_err());
    }

    #[test]
    fn generation_queries_keep_permission_failures_uncertain_and_compare_current_pid() {
        let pid = std::process::id();
        assert!(!process_generation_exited_with(pid, Some(41), |_| Ok(Some((41, false)))).unwrap());
        assert!(process_generation_exited_with(pid, Some(40), |_| Ok(Some((41, false)))).unwrap());
        assert!(process_generation_exited_with(pid, Some(41), |_| Ok(None)).unwrap());
        assert!(process_generation_exited_with(pid, Some(41), |_| Ok(Some((41, true)))).unwrap());
        assert!(!process_generation_exited_with(pid, None, |_| Ok(Some((41, false)))).unwrap());
        assert!(
            process_generation_exited_with(pid, Some(41), |_| {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into())
            })
            .is_err()
        );
        assert!(process_generation_exited_with(pid, Some(41), |_| Ok(Some((0, false)))).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn limited_query_checks_current_process_birth_without_vm_read() {
        let pid = std::process::id();
        let birth = crate::windows_integration::process_birth_id(pid).unwrap();
        assert!(!codex_process_generation_exited(pid, Some(birth)).unwrap());
        assert!(codex_process_generation_exited(pid, Some(birth + 1)).unwrap());
        assert!(codex_process_generation_exited(u32::MAX, Some(birth)).unwrap());
        assert!(!codex_process_generation_exited(0, Some(birth)).unwrap());
    }

    #[test]
    fn failed_process_reads_require_independent_exit_confirmation() {
        for stage in ["capture", "image", "arguments"] {
            let read = || Err::<(), _>(anyhow::anyhow!("{stage} failed"));
            assert!(
                process_read_or_exited(read(), || Ok(true))
                    .unwrap()
                    .is_none()
            );
            assert!(process_read_or_exited(read(), || Ok(false)).is_err());
            assert!(process_read_or_exited(read(), || anyhow::bail!("unknown")).is_err());
            assert_eq!(
                process_read_or_exited(Ok(41), || panic!("successful read")).unwrap(),
                Some(41)
            );
        }
    }

    #[test]
    fn absence_ports_tolerate_only_unavailable_ipv6_and_verified_neighbors() {
        use std::io::{Error, ErrorKind};
        for code in [47, 49, 10047, 10049] {
            assert!(
                loopback_ports_are_free_with(
                    9341,
                    9342,
                    |host, _| {
                        if host == "::1" {
                            Err(Error::from_raw_os_error(code))
                        } else {
                            Ok(())
                        }
                    },
                    |_| panic!("no occupied ports")
                )
                .unwrap()
            );
        }
        for unrelated in [false, true] {
            assert_eq!(
                loopback_ports_are_free_with(
                    9341,
                    9342,
                    |_, _| { Err::<(), _>(Error::from(ErrorKind::AddrInUse)) },
                    |_| Ok(unrelated)
                )
                .unwrap(),
                unrelated
            );
        }
        assert!(
            loopback_ports_are_free_with(
                9341,
                9342,
                |_, _| { Err::<(), _>(Error::from(ErrorKind::PermissionDenied)) },
                |_| Ok(true)
            )
            .is_err()
        );
        assert!(
            loopback_ports_are_free_with(
                9341,
                9342,
                |_, _| { Err::<(), _>(Error::from(ErrorKind::AddrInUse)) },
                |_| Err(Error::from(ErrorKind::PermissionDenied).into())
            )
            .is_err()
        );
        assert!(loopback_ports_are_free_with(0, 9342, |_, _| Ok(()), |_| Ok(true)).is_err());
    }

    #[test]
    fn lifecycle_lock_serializes_generation_and_receipt_writers() {
        let temp = tempfile::tempdir().unwrap();
        let first = lock_instance_lifecycle(temp.path()).unwrap();
        let second = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(temp.path().join("lifecycle.lock"))
            .unwrap();
        assert!(fs2::FileExt::try_lock_exclusive(&second).is_err());
        drop(first);
        fs2::FileExt::try_lock_exclusive(&second).unwrap();
    }

    #[test]
    fn lifecycle_user_data_scan_requires_one_exact_directory() {
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().join("own data");
        let neighbor = temp.path().join("own data other");
        std::fs::create_dir(&own).unwrap();
        std::fs::create_dir(&neighbor).unwrap();
        let expected = own.canonicalize().unwrap();
        let mut args = vec!["Codex".into(), format!("--user-data-dir={}", own.display())];
        assert!(arguments_user_data_matches(&args, &expected).unwrap());
        assert!(!arguments_user_data_matches(&args, &neighbor.canonicalize().unwrap()).unwrap());
        args.push(format!("--user-data-dir={}", own.display()));
        assert!(arguments_user_data_matches(&args, &expected).is_err());
        assert!(
            arguments_user_data_matches(&["Codex".into(), "--user-data-dir".into()], &expected)
                .is_err()
        );
    }

    #[test]
    fn relative_user_data_never_uses_the_managers_working_directory() {
        let manager_cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let other_data = temp.path().canonicalize().unwrap();
        for path in [".", "missing-relative-data", r"C:relative-data", r"\relative-data"] {
            for args in [
                vec!["Codex".into(), format!("--user-data-dir={path}")],
                vec!["Codex".into(), "--user-data-dir".into(), path.into()],
            ] {
                assert!(arguments_user_data_matches(&args, &manager_cwd).is_err());
                assert!(arguments_user_data_matches(&args, &other_data).is_err());
                let mut identity_args = args;
                identity_args.push("--remote-debugging-port=9341".into());
                assert!(!codex_arguments_match(&identity_args, &manager_cwd, 9341));
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_raw_process_arguments_match_own_test_process_argv() {
        assert_eq!(
            macos_process_arguments(std::process::id()).unwrap(),
            std::env::args().collect::<Vec<_>>()
        );
    }

    #[test]
    fn codex_identity_requires_exact_data_directory_and_single_debug_port() {
        let temp = tempfile::tempdir().unwrap();
        let own = temp.path().join("instance one");
        let neighbor = temp.path().join("instance one other");
        std::fs::create_dir(&own).unwrap();
        std::fs::create_dir(&neighbor).unwrap();
        let mut args = vec![
            "ChatGPT.exe".into(),
            "--remote-debugging-port=9341".into(),
            format!("--user-data-dir={}", own.display()),
        ];
        assert!(codex_arguments_match(&args, &own, 9341));
        assert!(!codex_arguments_match(&args, &neighbor, 9341));
        assert!(!codex_arguments_match(&args, &own, 934));
        args.push("--remote-debugging-port=9341".into());
        assert!(!codex_arguments_match(&args, &own, 9341));
        let args = vec![
            "ChatGPT.exe".into(),
            "--remote-debugging-port".into(),
            "9341".into(),
            "--user-data-dir".into(),
            own.to_string_lossy().into_owned(),
        ];
        assert!(codex_arguments_match(&args, &own, 9341));
        assert!(!codex_arguments_match(&args[..4], &own, 9341));
    }

    #[test]
    fn empty_user_data_is_rejected_for_every_instance() {
        let temp = tempfile::tempdir().unwrap();
        let mut identity = CodexInstanceIdentity {
            instance_id: "main".into(),
            generation: 1,
            debug_port: 9341,
            helper_port: 9342,
            user_data_dir: PathBuf::new(),
            browser_id: "main-browser".into(),
            process_id: 42,
            process_birth_id: 1,
            executable: temp.path().join("ChatGPT.exe"),
        };
        let args = vec!["ChatGPT.exe".into(), "--remote-debugging-port=9341".into()];
        assert!(!valid_stop_identity(&identity));
        assert!(!codex_identity_arguments_match(&args, &identity));
        assert!(!codex_arguments_match(&args, Path::new(""), 9341));
        let mut explicit = args.clone();
        explicit.push(format!("--user-data-dir={}", temp.path().display()));
        assert!(!codex_identity_arguments_match(&explicit, &identity));
        let mut duplicate = args.clone();
        duplicate.push("--remote-debugging-port=9341".into());
        assert!(!codex_identity_arguments_match(&duplicate, &identity));
        identity.instance_id = "multi".into();
        assert!(!valid_stop_identity(&identity));
        assert!(!codex_identity_arguments_match(&args, &identity));
        identity.user_data_dir = temp.path().canonicalize().unwrap();
        assert!(codex_identity_arguments_match(&explicit, &identity));
        assert!(!codex_identity_arguments_match(&args, &identity));
    }

    fn arguments(options: &Path) -> Vec<String> {
        vec![
            "CodexPlusPlus.exe".into(),
            "--codex-plus-multi-worker".into(),
            options.to_string_lossy().into_owned(),
        ]
    }

    #[test]
    fn worker_identity_requires_birth_marker_and_exact_options_file() {
        let temp = tempfile::tempdir().unwrap();
        let options = temp.path().join("instance A worker-options.json");
        std::fs::write(&options, "{}").unwrap();
        let args = arguments(&options);
        assert!(worker_identity_matches(Some(41), Some(41), &args, &options));
        assert!(!worker_identity_matches(None, Some(41), &args, &options));
        assert!(!worker_identity_matches(Some(41), None, &args, &options));
        assert!(!worker_identity_matches(
            Some(41),
            Some(42),
            &args,
            &options
        ));

        let mut wrong_marker = args.clone();
        wrong_marker[1] = "--codex-plus-multi-worker-other".into();
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &wrong_marker,
            &options
        ));
        let mut extra_arg = args.clone();
        extra_arg.push("--unexpected".into());
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &extra_arg,
            &options
        ));
    }

    #[test]
    fn worker_identity_rejects_neighbor_missing_and_prefix_only_paths() {
        let temp = tempfile::tempdir().unwrap();
        let options = temp.path().join("worker-options.json");
        let neighbor = temp.path().join("worker-options.json.backup");
        std::fs::write(&options, "{}").unwrap();
        std::fs::write(&neighbor, "{}").unwrap();
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&neighbor),
            &options
        ));
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&options),
            &neighbor
        ));
        let missing = temp.path().join("missing.json");
        assert!(!worker_identity_matches(
            Some(41),
            Some(41),
            &arguments(&missing),
            &missing
        ));
    }

    #[test]
    fn worker_image_uses_its_own_argv_not_the_new_manager_location() {
        let temp = tempfile::tempdir().unwrap();
        let old_dir = temp.path().join("old version");
        let new_dir = temp.path().join("new version");
        std::fs::create_dir(&old_dir).unwrap();
        std::fs::create_dir(&new_dir).unwrap();
        let old_worker = old_dir.join("manager.exe");
        let new_manager = new_dir.join("manager.exe");
        let neighbor = old_dir.join("neighbor.exe");
        let options = temp.path().join("worker-options.json");
        std::fs::write(&old_worker, "").unwrap();
        std::fs::write(&new_manager, "").unwrap();
        std::fs::write(&neighbor, "").unwrap();
        std::fs::write(&options, "{}").unwrap();
        let mut args = arguments(&options);
        args[0] = old_worker.to_string_lossy().into_owned();
        assert!(worker_identity_matches(Some(41), Some(41), &args, &options));
        assert!(worker_image_matches(&old_worker, &args));
        assert!(!worker_image_matches(&new_manager, &args));
        assert!(!worker_image_matches(&neighbor, &args));
        assert!(!worker_image_matches(
            &old_worker,
            &[new_manager.to_string_lossy().into_owned()]
        ));
        assert!(!worker_image_matches(
            &old_worker,
            &["old-worker.exe".into()]
        ));
        assert!(!worker_image_matches(&old_worker, &[]));
    }

    #[tokio::test]
    async fn readable_browser_conflict_is_rejected_without_a_page_list() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        Mock::given(path("/json/version"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Browser": "fixture",
                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{}/devtools/browser/neighbor",
                    server.address().port())
            })))
            .mount(&server)
            .await;
        let temp = tempfile::tempdir().unwrap();
        let identity = CodexInstanceIdentity {
            instance_id: "own".into(),
            generation: 1,
            debug_port: server.address().port(),
            helper_port: 9342,
            user_data_dir: temp.path().into(),
            browser_id: "own".into(),
            process_id: 42,
            process_birth_id: 1,
            executable: temp.path().join("ChatGPT.exe"),
        };
        assert!(check_stop_browser_identity(&identity).await.is_err());
        let mut matching = identity;
        matching.browser_id = "neighbor".into();
        assert!(check_stop_browser_identity(&matching).await.is_ok());
    }

    #[tokio::test]
    async fn browser_transport_loss_allows_saved_identity_but_invalid_response_does_not() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        let temp = tempfile::tempdir().unwrap();
        let mut identity = CodexInstanceIdentity {
            instance_id: "own".into(),
            generation: 1,
            debug_port: server.address().port(),
            helper_port: 9342,
            user_data_dir: temp.path().into(),
            browser_id: "own".into(),
            process_id: 42,
            process_birth_id: 1,
            executable: temp.path().join("ChatGPT.exe"),
        };
        Mock::given(path("/json/version"))
            .respond_with(ResponseTemplate::new(200).set_body_string("invalid browser"))
            .mount(&server)
            .await;
        assert!(check_stop_browser_identity(&identity).await.is_err());
        drop(server);
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        identity.debug_port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(check_stop_browser_identity(&identity).await.is_ok());
    }
}
