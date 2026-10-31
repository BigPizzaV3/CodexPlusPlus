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
    if user_paths.len() != 1 || user_data_dir.as_os_str().is_empty() {
        return false;
    }
    let Ok(expected) = user_data_dir.canonicalize() else {
        return false;
    };
    let Ok(actual) = Path::new(user_paths[0]).canonicalize() else {
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
    Some(birth.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x100000001b3)
    }))
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
pub fn wait_for_codex_instance_exit(identity: &CodexInstanceIdentity) -> anyhow::Result<()> {
    use anyhow::Context;
    anyhow::ensure!(valid_stop_identity(identity), "多开实例身份字段不完整");
    let process = match crate::windows_integration::TrackedWindowsProcess::capture(
        identity.process_id,
        &identity.executable,
    ) {
        Ok(process) => process.context("多开进程映像身份冲突")?,
        Err(_) if crate::watcher::process_id_is_running(identity.process_id) == Some(false) => {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    anyhow::ensure!(
        process.birth_id == identity.process_birth_id,
        "多开进程创建身份冲突"
    );
    // 页面导航和 CDP 暂时失联不等于桌面退出；始终观察已核验的同一代进程。
    while process.is_alive()? {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    Ok(())
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

/// 直接从仍由父进程持有的 Child 句柄读取创建身份，避免按 PID 重新打开另一进程。
pub fn worker_birth_id(child: &std::process::Child) -> Option<u64> {
    #[cfg(windows)]
    {
        crate::windows_integration::child_process_birth_id(child)
    }
    #[cfg(not(windows))]
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

    #[cfg(windows)]
    #[test]
    fn worker_waits_for_its_process_without_cdp_and_rejects_another_generation() {
        use std::os::windows::process::CommandExt;
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = ChildGuard(
            std::process::Command::new(&executable)
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "Start-Sleep -Seconds 2",
                ])
                .creation_flags(crate::windows_integration::CREATE_NO_WINDOW)
                .spawn()
                .unwrap(),
        );
        let port = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let home = tempfile::tempdir().unwrap();
        let birth = crate::windows_integration::process_birth_id(child.0.id()).unwrap();
        let mut identity = CodexInstanceIdentity {
            instance_id: "worker-lifetime".into(),
            generation: 1,
            debug_port: port.local_addr().unwrap().port(),
            helper_port: 9342,
            user_data_dir: home.path().into(),
            browser_id: "saved-browser".into(),
            process_id: child.0.id(),
            process_birth_id: birth ^ 1,
            executable,
        };
        assert!(wait_for_codex_instance_exit(&identity).is_err());
        assert!(child.0.try_wait().unwrap().is_none());
        identity.process_birth_id = birth;
        wait_for_codex_instance_exit(&identity).unwrap();
        assert!(child.0.try_wait().unwrap().is_some());
        // 已确认退出的同一代进程允许正常收尾。
        wait_for_codex_instance_exit(&identity).unwrap();
    }
}
