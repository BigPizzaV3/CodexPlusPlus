use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 端口只是连接地址；停止实例必须同时持有这一代的进程和 Browser 身份。
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
    if ports.len() != 1
        || ports[0].parse::<u16>().ok() != Some(debug_port)
        || debug_port == 0
        || user_paths.len() != 1
    {
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
        user_data_dir: user_data_dir.canonicalize()?,
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
    if identity.generation == 0
        || identity.instance_id.is_empty()
        || identity.browser_id.is_empty()
        || identity.debug_port == 0
        || identity.helper_port == 0
        || unix_birth_token(identity.process_id) != Some(identity.process_birth_id)
    {
        return Ok(false);
    }
    let args = unix_process_arguments(identity.process_id)?;
    if !codex_arguments_match(&args, &identity.user_data_dir, identity.debug_port)
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
    let user_data_dir = user_data_dir.canonicalize()?;
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.to_path_buf());
    let browser_id = codex_browser_id(debug_port).await?;
    let process = crate::windows_integration::capture_debug_listener(debug_port, &executable)?
        .context("无法确认多开调试端口所属进程")?;
    let arguments = crate::windows_integration::process_arguments(process.process_id)
        .context("无法读取多开进程数据目录")?;
    anyhow::ensure!(
        codex_arguments_match(&arguments, &user_data_dir, debug_port),
        "多开进程参数与实例身份不一致"
    );
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
        process.is_alive()? && codex_instance_identity_matches(&identity).await?,
        "多开进程在确认身份期间已改变"
    );
    Ok(identity)
}

#[cfg(windows)]
pub async fn codex_instance_identity_matches(
    identity: &CodexInstanceIdentity,
) -> anyhow::Result<bool> {
    if identity.instance_id.is_empty()
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
    if !codex_arguments_match(&arguments, &identity.user_data_dir, identity.debug_port) {
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
    if !codex_instance_identity_matches(identity).await? {
        return Ok(false);
    }
    crate::windows_integration::terminate_verified_codex_instance(identity)
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
}
