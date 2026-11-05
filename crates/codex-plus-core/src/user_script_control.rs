//! 用户脚本重载的私有会话通道。接收和查询不经过 renderer，页面执行仍由 launcher owner 协调。

use std::future::Future;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::user_scripts::UserScriptManager;

const PROTOCOL: u32 = 1;
const MAX_REQUEST: usize = 16 * 1024;
const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const IO_DEADLINE: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
#[error("重载 owner 已变化，旧操作结果无法确认；没有向新 owner 重投")]
pub struct OwnerChanged;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlIdentity {
    protocol: u32,
    state_dir: String,
    script_dir: String,
    config_path: String,
    debug_port: u16,
}

impl ControlIdentity {
    pub fn new(state_dir: &Path, debug_port: u16, manager: &UserScriptManager) -> anyhow::Result<Self> {
        anyhow::ensure!(debug_port != 0, "重载会话的调试端口无效");
        Ok(Self {
            protocol: PROTOCOL,
            state_dir: normalized_path(state_dir)?,
            script_dir: normalized_path(manager.user_dir())?,
            config_path: normalized_path(manager.config_path())?,
            debug_port,
        })
    }

    pub(crate) fn key(&self) -> String {
        let hash = Sha256::digest(serde_json::to_vec(self).expect("control identity is serializable"));
        format!("{hash:x}")
    }
}

fn normalized_path(path: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(path.is_absolute(), "重载会话的路径必须是绝对路径");
    // 新安装时配置文件可能还不存在。只解析现有父目录，不创建或修改用户目录。
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(ancestor.file_name().context("无法解析重载会话路径")?.to_owned());
        ancestor = ancestor.parent().context("无法解析重载会话父目录")?;
    }
    let mut resolved = ancestor.canonicalize()?;
    for component in suffix.iter().rev() {
        resolved.push(component);
    }
    let value = resolved.to_str().context("重载会话路径不是有效 UTF-8")?.to_owned();
    #[cfg(windows)]
    let value = value.to_lowercase();
    Ok(value)
}

pub fn new_owner_epoch() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub fn new_operation_id(epoch: &str) -> String {
    format!("{epoch}-{:x}-{}", now_ms(), uuid::Uuid::new_v4().simple())
}

pub fn operation_issued_at(epoch: &str, id: &str) -> Option<u64> {
    let tail = id.strip_prefix(epoch)?.strip_prefix('-')?;
    let (time, nonce) = tail.split_once('-')?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(time, 16).ok()
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub enum ControlAction {
    Submit(Value),
    Status(String),
    StatusWithVerification(String, String),
}

pub type ControlHandler = Arc<dyn Fn(ControlAction) -> anyhow::Result<Value> + Send + Sync>;

pub struct ControlServer {
    task: tokio::task::JoinHandle<()>,
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(unix)]
    _lock: std::fs::File,
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.task.abort();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn start_owner(
    identity: ControlIdentity,
    owner_epoch: String,
    handler: ControlHandler,
) -> anyhow::Result<ControlServer> {
    let endpoint = endpoint(&identity)?;
    let permits = Arc::new(tokio::sync::Semaphore::new(16));
    #[cfg(windows)]
    {
        let mut listener = private_pipe(&endpoint, true)?;
        let task = tokio::spawn(async move {
            loop {
                if listener.connect().await.is_err() {
                    break;
                }
                // 先创建下一实例，避免已连接实例退出时出现可被抢占的管道空档。
                let next = match private_pipe(&endpoint, false) {
                    Ok(next) => next,
                    Err(_) => break,
                };
                let stream = std::mem::replace(&mut listener, next);
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let identity = identity.clone();
                let owner_epoch = owner_epoch.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = bounded(serve_connection(stream, &identity, &owner_epoch, &handler)).await;
                });
            }
        });
        Ok(ControlServer { task })
    }
    #[cfg(unix)]
    {
        use fs2::FileExt;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let path = PathBuf::from(endpoint);
        let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(path.with_extension("lock"))?;
        lock.try_lock_exclusive().context("此实例已有重载协调通道")?;
        // 同名锁已独占；只移除这一条上次异常退出留下的 socket。
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => {
                use std::os::unix::fs::FileTypeExt;
                anyhow::ensure!(metadata.file_type().is_socket(), "重载通道路径不是 socket");
                std::fs::remove_file(&path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let listener = tokio::net::UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let identity = identity.clone();
                let owner_epoch = owner_epoch.clone();
                let handler = handler.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = bounded(serve_connection(stream, &identity, &owner_epoch, &handler)).await;
                });
            }
        });
        Ok(ControlServer { task, path, _lock: lock })
    }
}

async fn bounded<T>(future: impl Future<Output = anyhow::Result<T>>) -> anyhow::Result<T> {
    tokio::time::timeout(IO_DEADLINE, future).await.context("本地重载协调通道响应超时")?
}

async fn serve_connection(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    identity: &ControlIdentity,
    epoch: &str,
    handler: &ControlHandler,
) -> anyhow::Result<()> {
    let request = read_frame(&mut stream, MAX_REQUEST).await?;
    let (identity_for_request, epoch_for_request, handler_for_request) =
        (identity.clone(), epoch.to_owned(), handler.clone());
    // 配置/快照的文件 I/O 不能占住 launcher 单线程 runtime 的接收及查询循环。
    let result = tokio::task::spawn_blocking(move || dispatch_request(
        request, &identity_for_request, &epoch_for_request, &handler_for_request,
    )).await.context("重载协调命令执行任务失败")?;
    let response = match result {
        Ok(value) => json!({ "identity": identity, "ownerEpoch": epoch, "result": value }),
        Err(error) => json!({ "identity": identity, "ownerEpoch": epoch, "error": format!("{error:#}") }),
    };
    write_frame(&mut stream, &response, MAX_RESPONSE).await
}

fn dispatch_request(
    request: Value,
    identity: &ControlIdentity,
    epoch: &str,
    handler: &ControlHandler,
) -> anyhow::Result<Value> {
    anyhow::ensure!(request.get("identity") == Some(&serde_json::to_value(identity)?),
        "重载通道的实例、脚本目录或配置身份不匹配");
    if request["action"] == "hello" {
        return Ok(json!({ "capabilities": { "nativeReload": true, "noPageReload": true } }));
    }
    anyhow::ensure!(request["ownerEpoch"].as_str() == Some(epoch), "重载 owner 已变化；没有向新 owner 重投旧操作");
    let id = request["operationId"].as_str().context("缺少重载操作编号")?;
    anyhow::ensure!(operation_issued_at(epoch, id).is_some(), "重载操作不属于此 owner");
    match request["action"].as_str() {
        Some("submit") => {
            let allow_page = request["allowPage"].as_bool().context("缺少页面刷新策略")?;
            handler(ControlAction::Submit(json!({ "operationId": id, "allowPage": allow_page,
                "verificationId": request["verificationId"] })))
        }
        Some("status") => match request["verificationId"].as_str() {
            Some(verification) => handler(ControlAction::StatusWithVerification(id.to_owned(), verification.to_owned())),
            None => handler(ControlAction::Status(id.to_owned())),
        },
        _ => anyhow::bail!("未知重载协调命令"),
    }
}

async fn read_frame(stream: &mut (impl AsyncRead + Unpin), limit: usize) -> anyhow::Result<Value> {
    let size = stream.read_u32().await? as usize;
    anyhow::ensure!(size > 0 && size <= limit, "重载协调消息超出长度限制");
    let mut data = vec![0_u8; size];
    stream.read_exact(&mut data).await?;
    Ok(serde_json::from_slice(&data)?)
}

async fn write_frame(stream: &mut (impl AsyncWrite + Unpin), value: &Value, limit: usize) -> anyhow::Result<()> {
    let encoded = serde_json::to_vec(value)?;
    anyhow::ensure!(encoded.len() <= limit, "重载协调响应超出长度限制");
    stream.write_u32(encoded.len() as u32).await?;
    stream.write_all(&encoded).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn request(identity: &ControlIdentity, mut request: Value) -> anyhow::Result<Value> {
    request["identity"] = serde_json::to_value(identity)?;
    bounded(async {
        let endpoint = endpoint(identity)?;
        #[cfg(windows)]
        let mut stream = {
            loop {
                match tokio::net::windows::named_pipe::ClientOptions::new().open(&endpoint) {
                    Ok(stream) => {
                        verify_pipe_owner(&stream)?;
                        break stream;
                    }
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => return Err(anyhow::Error::from(error).context(
                        "没有找到匹配的本地重载协调器；请更新或启动对应 Codex++ 实例")),
                }
            }
        };
        #[cfg(unix)]
        let mut stream = tokio::net::UnixStream::connect(endpoint).await.context(
            "没有找到匹配的本地重载协调器；请更新或启动对应 Codex++ 实例")?;
        write_frame(&mut stream, &request, MAX_REQUEST).await?;
        let response = read_frame(&mut stream, MAX_RESPONSE).await?;
        anyhow::ensure!(response.get("identity") == Some(&serde_json::to_value(identity)?),
            "重载协调响应的实例身份不匹配");
        if request["action"] != "hello" {
            if response["ownerEpoch"] != request["ownerEpoch"] {
                return Err(OwnerChanged.into());
            }
        }
        if let Some(error) = response["error"].as_str() {
            anyhow::bail!("{error}");
        }
        Ok(json!({ "ownerEpoch": response["ownerEpoch"], "result": response["result"] }))
    }).await
}

#[cfg(windows)]
fn endpoint(identity: &ControlIdentity) -> anyhow::Result<String> {
    Ok(format!(r"\\.\pipe\codex-plus-reload-{}", identity.key()))
}

#[cfg(any(unix, test))]
fn unix_socket_path(temp_root: &str, uid: u32, key: &str, capacity: usize) -> anyhow::Result<String> {
    for root in [temp_root, "/tmp"] {
        let path = format!("{}/codex-plus-reload-{uid}/{key}.sock", root.trim_end_matches('/'));
        // sun_path 包含结尾 NUL；UTF-8 路径按字节而非字符计长。
        if path.len() < capacity {
            return Ok(path);
        }
    }
    anyhow::bail!("重载 socket 路径超过当前平台的长度限制")
}

#[cfg(unix)]
fn endpoint(identity: &ControlIdentity) -> anyhow::Result<String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let uid = unsafe { libc::geteuid() };
    // sockaddr_un 全零有效；读取实际容量以适配 macOS、Linux 及其他 Unix。
    let address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    let temp_root = std::env::temp_dir();
    let path = unix_socket_path(
        temp_root.to_str().context("重载 socket 路径不是有效 UTF-8")?,
        uid,
        &identity.key()[..40],
        address.sun_path.len(),
    )?;
    let directory = Path::new(&path).parent().context("重载 socket 缺少私有目录")?;
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(&directory)?;
    anyhow::ensure!(metadata.is_dir() && !metadata.file_type().is_symlink()
        && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "重载协调目录不是当前用户的私有目录");
    Ok(path)
}

#[cfg(windows)]
fn process_user_sid(process: windows::Win32::Foundation::HANDLE) -> anyhow::Result<String> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::Foundation::{HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
    use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows::Win32::System::Threading::OpenProcessToken;
    use windows::core::PWSTR;
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }?;
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let mut needed = 0;
    let _ = unsafe { GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, None, 0, &mut needed) };
    anyhow::ensure!(needed > 0 && needed <= 64 * 1024, "无法读取当前用户 SID");
    // Vec<usize> 给 TOKEN_USER/SID 提供原生指针对齐。
    let mut buffer = vec![0_usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    unsafe {
        GetTokenInformation(HANDLE(token.as_raw_handle()), TokenUser, Some(buffer.as_mut_ptr().cast()), needed, &mut needed)
    }?;
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = PWSTR::null();
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) }?;
    let text = unsafe { sid.to_string() };
    unsafe { let _ = LocalFree(HLOCAL(sid.0.cast())); }
    Ok(text?)
}

#[cfg(windows)]
fn verify_pipe_owner(stream: &tokio::net::windows::named_pipe::NamedPipeClient) -> anyhow::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let mut pid = 0;
    unsafe { GetNamedPipeServerProcessId(HANDLE(stream.as_raw_handle()), &mut pid) }?;
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }?;
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    anyhow::ensure!(
        process_user_sid(HANDLE(process.as_raw_handle()))? == process_user_sid(unsafe { GetCurrentProcess() })?,
        "重载协调器不属于当前用户",
    );
    Ok(())
}

#[cfg(windows)]
fn with_user_security<T>(
    inherit_children: bool,
    action: impl FnOnce(*mut std::ffi::c_void) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::core::PCWSTR;
    struct LocalAllocation(*mut std::ffi::c_void);
    impl Drop for LocalAllocation {
        fn drop(&mut self) { unsafe { let _ = LocalFree(HLOCAL(self.0)); } }
    }
    let sid = process_user_sid(unsafe { GetCurrentProcess() })?;
    // 只授予当前用户，禁止继承宽泛的默认 DACL；句柄不继承给子进程。
    let inheritance = if inherit_children { "OICI" } else { "" };
    let sddl: Vec<u16> = format!("D:P(A;{inheritance};GA;;;{sid})").encode_utf16().chain(Some(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), SDDL_REVISION_1, &mut descriptor, None)
    }?;
    let _descriptor_allocation = LocalAllocation(descriptor.0);
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: false.into(),
    };
    action((&mut attributes as *mut SECURITY_ATTRIBUTES).cast())
}

#[cfg(windows)]
fn private_pipe(endpoint: &str, first: bool) -> anyhow::Result<tokio::net::windows::named_pipe::NamedPipeServer> {
    // SECURITY_ATTRIBUTES 和 descriptor 在 CreateNamedPipe 调用期间保持有效。
    with_user_security(false, |attributes| Ok(unsafe {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(first).reject_remote_clients(true)
            .create_with_security_attributes_raw(endpoint, attributes)
    }?))
}

/// 创建新的私有目录。已有路径返回错误，不接受未经检查的旧目录或重解析点。
pub(crate) fn create_private_directory(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_absolute(), "私有快照目录必须是绝对路径");
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Security::SECURITY_ATTRIBUTES;
        use windows::Win32::Storage::FileSystem::CreateDirectoryW;
        use windows::core::PCWSTR;
        let encoded = path.as_os_str().encode_wide().chain(Some(0)).collect::<Vec<_>>();
        with_user_security(true, |attributes| {
            unsafe { CreateDirectoryW(PCWSTR(encoded.as_ptr()), Some(attributes.cast::<SECURITY_ATTRIBUTES>())) }?;
            Ok(())
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        Ok(std::fs::DirBuilder::new().mode(0o700).create(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn unix_socket_path_keeps_short_roots() {
        let key = "a".repeat(40);
        for capacity in [104, 108] {
            for root in ["/tmp", "/var/tmp/"] {
                assert_eq!(
                    unix_socket_path(root, 501, &key, capacity).unwrap(),
                    format!("{}/codex-plus-reload-501/{key}.sock", root.trim_end_matches('/')),
                );
            }
        }
    }

    #[test]
    fn unix_socket_path_falls_back_from_long_temp_root() {
        let key = "a".repeat(40);
        let root = format!("/var/folders/{}/T/", "long-root".repeat(16));
        for capacity in [104, 108] {
            assert_eq!(
                unix_socket_path(&root, 501, &key, capacity).unwrap(),
                format!("/tmp/codex-plus-reload-501/{key}.sock"),
            );
        }
        assert!(unix_socket_path(&root, 501, &key, 16).is_err());
    }

    #[test]
    fn unix_socket_path_reserves_terminating_nul_at_platform_limits() {
        let key = "a".repeat(40);
        let suffix = format!("/codex-plus-reload-501/{key}.sock");
        for capacity in [104, 108] {
            let root = format!("/{}", "x".repeat(capacity - suffix.len() - 2));
            let path = unix_socket_path(&root, 501, &key, capacity).unwrap();
            assert_eq!(path, format!("{root}{suffix}"));
            assert_eq!(path.len(), capacity - 1);
            assert_eq!(
                unix_socket_path(&format!("{root}x"), 501, &key, capacity).unwrap(),
                format!("/tmp{suffix}"),
            );
        }
    }

    #[test]
    fn unix_socket_path_counts_multibyte_roots_as_utf8_bytes() {
        let key = "a".repeat(40);
        let suffix = format!("/codex-plus-reload-501/{key}.sock");
        for capacity in [104, 108] {
            let root_bytes = capacity - suffix.len() - 2;
            let root = format!("/{}{}", "\u{6e29}".repeat(root_bytes / 3), "x".repeat(root_bytes % 3));
            let path = unix_socket_path(&root, 501, &key, capacity).unwrap();
            assert_eq!(path, format!("{root}{suffix}"));
            assert_eq!(path.len(), capacity - 1);
            let too_long = format!("{root}\u{e9}");
            assert!(format!("{too_long}{suffix}").chars().count() < capacity);
            assert_eq!(
                unix_socket_path(&too_long, 501, &key, capacity).unwrap(),
                format!("/tmp{suffix}"),
            );
        }
    }

    fn identity(temp: &tempfile::TempDir, port: u16) -> ControlIdentity {
        ControlIdentity::new(temp.path(), port, &UserScriptManager::new(
            temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"),
        )).unwrap()
    }

    #[tokio::test]
    async fn native_channel_receives_without_a_renderer_and_checks_identity_and_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let identity = identity(&temp, 9321);
        let epoch = new_owner_epoch();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let handler: ControlHandler = Arc::new(move |action| {
            counted.fetch_add(1, Ordering::SeqCst);
            match action {
                ControlAction::Submit(payload) => Ok(json!({ "reloadOperation": payload })),
                ControlAction::Status(id) | ControlAction::StatusWithVerification(id, _) =>
                    Ok(json!({ "reloadOperation": { "operationId": id, "phase": "unknown" } })),
            }
        });
        let _server = start_owner(identity.clone(), epoch.clone(), handler.clone()).unwrap();
        let hello = request(&identity, json!({ "action": "hello" })).await.unwrap();
        assert_eq!(hello["ownerEpoch"], epoch);
        let id = new_operation_id(&epoch);
        let submitted = request(&identity, json!({
            "action": "submit", "ownerEpoch": epoch, "operationId": id, "allowPage": false,
        })).await.unwrap();
        assert_eq!(submitted["result"]["reloadOperation"]["allowPage"], false);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(request(&identity, json!({
            "action": "submit", "ownerEpoch": new_owner_epoch(), "operationId": id, "allowPage": true,
        })).await.is_err());
        let mut wrong = identity.clone();
        wrong.script_dir.push_str("-other");
        assert!(dispatch_request(json!({
            "identity": wrong, "action": "submit", "ownerEpoch": epoch, "operationId": id, "allowPage": true,
        }), &identity, &epoch, &handler).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn distinct_instance_identity_cannot_reuse_an_owner() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let owner = identity(&first, 9322);
        let other = identity(&second, 9322);
        assert_ne!(owner.key(), other.key());
        let _server = start_owner(owner.clone(), new_owner_epoch(), Arc::new(|_| unreachable!())).unwrap();
        assert!(request(&other, json!({ "action": "hello" })).await.is_err());
    }

    #[tokio::test]
    async fn lost_submission_ack_is_resolved_by_the_same_id() {
        use std::sync::Mutex;
        let temp = tempfile::tempdir().unwrap();
        let identity = identity(&temp, 9323);
        let epoch = new_owner_epoch();
        let records = Arc::new(Mutex::new(std::collections::HashMap::<String, Value>::new()));
        let accepted = records.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let handler: ControlHandler = Arc::new(move |action| {
            match action {
                ControlAction::Submit(payload) => {
                    let id = payload["operationId"].as_str().unwrap().to_owned();
                    let mut records = accepted.lock().unwrap();
                    if let Some(record) = records.get(&id) { return Ok(record.clone()) }
                    counted.fetch_add(1, Ordering::SeqCst);
                    let record = json!({ "reloadOperation": { "operationId": id, "phase": "accepted" } });
                    records.insert(id, record);
                    // 已登记后的响应丢失；handler故障不可导致client换ID。
                    anyhow::bail!("模拟接收后的 ACK 丢失")
                }
                ControlAction::Status(id) | ControlAction::StatusWithVerification(id, _) =>
                    Ok(accepted.lock().unwrap().get(&id).unwrap().clone()),
            }
        });
        let _server = start_owner(identity.clone(), epoch.clone(), handler).unwrap();
        let id = new_operation_id(&epoch);
        let submit = json!({ "action": "submit", "ownerEpoch": epoch, "operationId": id, "allowPage": false });
        assert!(request(&identity, submit.clone()).await.is_err());
        let status = request(&identity, json!({ "action": "status", "ownerEpoch": epoch, "operationId": id })).await.unwrap();
        assert_eq!(status["result"]["reloadOperation"]["phase"], "accepted");
        assert_eq!(request(&identity, submit).await.unwrap()["result"]["reloadOperation"]["operationId"], id);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn completed_old_revision_advances_to_current_intent_instead_of_false_success() {
        use std::sync::Mutex;
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let identity = ControlIdentity::new(temp.path(), 9324, &manager).unwrap();
        let epoch = new_owner_epoch();
        let ids = Arc::new(Mutex::new(Vec::new()));
        let captured = ids.clone();
        let handler: ControlHandler = Arc::new(move |action| {
            let ControlAction::Submit(payload) = action else { anyhow::bail!("测试应直接收到新意图") };
            let mut ids = captured.lock().unwrap();
            ids.push(payload["operationId"].clone());
            let revision = if ids.len() == 1 { "old" } else { "current" };
            Ok(json!({
                "reloadOperation": { "operationId": payload["operationId"], "phase": "applied", "revision": revision },
                "desiredRevision": "current",
            }))
        });
        let _server = start_owner(identity, epoch, handler).unwrap();
        let inventory = crate::user_scripts::reload_native_scripts(temp.path(), 9324, &manager, true, None).await.unwrap();
        assert_eq!(inventory["reloadOperation"]["phase"], "applied");
        assert_eq!(inventory["reloadOperation"]["revision"], "current");
        let ids = ids.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }

    #[tokio::test]
    async fn resuming_an_old_epoch_never_submits_to_replacement_owner() {
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let identity = ControlIdentity::new(temp.path(), 9325, &manager).unwrap();
        let old_epoch = new_owner_epoch();
        let old_id = new_operation_id(&old_epoch);
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let _server = start_owner(identity, new_owner_epoch(), Arc::new(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            unreachable!("旧 epoch 不得送入新 owner")
        })).unwrap();
        let inventory = crate::user_scripts::reload_native_scripts(temp.path(), 9325, &manager, true, Some(&json!({
            "operationId": old_id, "ownerEpoch": old_epoch, "allowPage": true, "phase": "unknown",
        }))).await.unwrap();
        assert_eq!(inventory["reloadOperation"]["phase"], "degraded");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn initial_lost_ack_reuses_identical_bundle_and_source() {
        use std::sync::Mutex;
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let expressions = Arc::new(Mutex::new(Vec::<String>::new()));
        let make_evaluator = || {
            let expressions = expressions.clone();
            move |expression: String| {
                let mut expressions = expressions.lock().unwrap();
                let index = expressions.len();
                expressions.push(expression);
                std::future::ready(match index {
                    0 | 2 => Ok(json!({ "result": { "result": { "value": json!({
                        "documentId": "initial-document", "initialized": false,
                    }).to_string() } } })),
                    1 => Err(anyhow::anyhow!("模拟页面已收到初始化，但 ACK 丢失")),
                    3 => Ok(json!({ "result": { "result": { "value": json!({
                        "accepted": true, "scripts": {},
                    }).to_string() } } })),
                    _ => unreachable!(),
                })
            }
        };
        assert!(crate::user_scripts::load_scripts_with_evaluator(&manager, 1, make_evaluator()).await.is_err());
        let generation = crate::extension_backend::backend_generation(&manager);
        crate::user_scripts::load_scripts_with_evaluator(&manager, 2, make_evaluator()).await.unwrap();
        assert_eq!(generation, crate::extension_backend::backend_generation(&manager));
        let expressions = expressions.lock().unwrap();
        assert_eq!(expressions[1], expressions[3]);
    }

    #[tokio::test]
    async fn initialized_document_with_retired_source_is_not_reported_as_loaded() {
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let generation = crate::extension_backend::backend_generation(&manager);
        let old = crate::extension_backend::bind_backend_document(&manager, generation, "old-document").await.unwrap();
        crate::extension_backend::revoke_backend_document(&manager, generation).await.unwrap();
        let calls = AtomicUsize::new(0);
        let error = crate::user_scripts::load_scripts_with_evaluator(&manager, 3, |_| {
            assert_eq!(calls.fetch_add(1, Ordering::SeqCst), 0);
            std::future::ready(Ok(json!({ "result": { "result": { "value": json!({
                "documentId": "old-document", "initialized": true, "backendSource": old, "scripts": {},
            }).to_string() } } })))
        }).await.unwrap_err();
        assert!(error.to_string().contains("旧代"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn guarded_initial_replay_is_exact_and_old_document_cannot_bind_source() {
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let source = json!({ "ownerEpoch": "test-owner", "generation": 1, "documentId": "old-document" });
        let (bundle, _) = manager.build_guarded_initial_bundle("old-document", 12, &source).unwrap();
        let script = format!(r#"
const assert = require("node:assert/strict");
const vm = require("node:vm");
const bundle = {};
function page(documentId) {{
  const window = {{ __codexPlusUserScripts: {{ documentId, scripts: {{}} }}, __codexPlusBackendSource: null }};
  const context = vm.createContext({{ window, document: {{}}, console }});
  return {{ window, context }};
}}
const first = page("old-document");
assert.equal(JSON.parse(vm.runInContext(bundle, first.context)).accepted, true);
assert.equal(JSON.parse(vm.runInContext(bundle, first.context)).replayed, true);
first.window.__codexPlusBackendSource.generation = 2;
assert.equal(JSON.parse(vm.runInContext(bundle, first.context)).accepted, false);
const next = page("new-document");
assert.equal(JSON.parse(vm.runInContext(bundle, next.context)).accepted, false);
assert.equal(next.window.__codexPlusBackendSource, null);
assert.equal(next.window.__codexPlusUserScripts.initialized, undefined);
"#, json!(bundle));
        let output = std::process::Command::new("node").args(["-e", &script]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    }

    #[tokio::test]
    async fn no_page_caller_never_submits_unaccepted_page_pending_and_explicit_page_can_resume() {
        use std::sync::Mutex;
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let identity = ControlIdentity::new(temp.path(), 9341, &manager).unwrap();
        let epoch = new_owner_epoch();
        let old_id = new_operation_id(&epoch);
        let submissions = Arc::new(Mutex::new(Vec::new()));
        let captured = submissions.clone();
        let handler: ControlHandler = Arc::new(move |action| {
            match action {
                ControlAction::Status(id) | ControlAction::StatusWithVerification(id, _) => Ok(json!({ "reloadOperation": {
                    "operationId": id, "phase": "not_found",
                } })),
                ControlAction::Submit(payload) => {
                    captured.lock().unwrap().push(payload.clone());
                    Ok(json!({ "reloadOperation": {
                        "operationId": payload["operationId"], "phase": "applied", "revision": "current",
                    }, "desiredRevision": "current" }))
                }
            }
        });
        let _server = start_owner(identity, epoch.clone(), handler).unwrap();
        let resume = json!({ "operationId": old_id, "ownerEpoch": epoch, "allowPage": true, "needsSubmit": true });
        let blocked = crate::user_scripts::reload_native_scripts(temp.path(), 9341, &manager, false, Some(&resume)).await.unwrap();
        assert_eq!(blocked["reloadOperation"]["phase"], "blocked");
        assert_eq!(blocked["reloadOperation"]["operationId"], old_id);
        assert!(submissions.lock().unwrap().is_empty());
        let resumed = crate::user_scripts::reload_native_scripts(temp.path(), 9341, &manager, true, None).await.unwrap();
        assert_eq!(resumed["reloadOperation"]["phase"], "applied");
        let submissions = submissions.lock().unwrap();
        assert_eq!(submissions.len(), 1);
        assert_eq!(submissions[0]["operationId"], old_id);
        assert_eq!(submissions[0]["allowPage"], true);
    }

    #[tokio::test]
    async fn no_page_caller_queries_accepted_page_operation_then_uses_no_page_for_new_intent() {
        use std::sync::Mutex;
        let temp = tempfile::tempdir().unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("scripts.json"));
        let identity = ControlIdentity::new(temp.path(), 9342, &manager).unwrap();
        let epoch = new_owner_epoch();
        let old_id = new_operation_id(&epoch);
        let old_for_handler = old_id.clone();
        let submissions = Arc::new(Mutex::new(Vec::new()));
        let captured = submissions.clone();
        let handler: ControlHandler = Arc::new(move |action| match action {
            ControlAction::Status(id) | ControlAction::StatusWithVerification(id, _) => {
                assert_eq!(id, old_for_handler);
                Ok(json!({ "reloadOperation": { "operationId": id, "phase": "applied", "revision": "old" },
                    "desiredRevision": "current" }))
            }
            ControlAction::Submit(payload) => {
                assert_ne!(payload["operationId"], old_for_handler);
                assert_eq!(payload["allowPage"], false);
                captured.lock().unwrap().push(payload.clone());
                Ok(json!({ "reloadOperation": { "operationId": payload["operationId"], "phase": "applied", "revision": "current" },
                    "desiredRevision": "current" }))
            }
        });
        let _server = start_owner(identity, epoch.clone(), handler).unwrap();
        let resume = json!({ "operationId": old_id, "ownerEpoch": epoch, "allowPage": true, "phase": "unknown" });
        let applied = crate::user_scripts::reload_native_scripts(temp.path(), 9342, &manager, false, Some(&resume)).await.unwrap();
        assert_eq!(applied["reloadOperation"]["phase"], "applied");
        assert_eq!(applied["reloadOperation"]["allowPage"], false);
        assert_eq!(submissions.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn skipped_initialization_does_not_scan_or_stage_an_unreadable_enabled_package() {
        let temp = tempfile::tempdir().unwrap();
        let user = temp.path().join("user");
        let package = user.join("broken");
        std::fs::create_dir_all(&package).unwrap();
        // 目录占用 manifest 路径：任何正常包扫描都会触及这个损坏的启用包。
        std::fs::create_dir(package.join("codexpp.json")).unwrap();
        std::fs::write(package.join("renderer.js"), b"\xff\xfe").unwrap();
        let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("scripts.json"));
        manager.set_skip_next_start(true).unwrap();
        let calls = AtomicUsize::new(0);
        let inventory = crate::user_scripts::load_scripts_with_evaluator(&manager, 4, |expression| {
            let response = if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                json!({ "documentId": "safe-document", "initialized": false })
            } else {
                assert!(expression.contains("runtime.initialMode = \"skipped\""));
                assert!(!expression.contains("renderer.js"));
                assert!(!expression.contains(".reload-backend-"));
                // 安全启动绑定空来源，精确检查运行结果而非整段脚本中的字段文本。
                let script = format!(r#"
const assert = require("node:assert/strict");
const vm = require("node:vm");
const window = {{ __codexPlusUserScripts: {{ documentId: "safe-document", scripts: {{}} }} }};
const context = vm.createContext({{ window, document: {{}}, console }});
assert.equal(JSON.parse(vm.runInContext({}, context)).accepted, true);
assert.equal(window.__codexPlusBackendSource.documentId, "safe-document");
assert.equal(window.__codexPlusBackendSource.revision, null);
assert.equal(window.__codexPlusUserScripts.appliedRevision, null);
assert.equal(window.__codexPlusUserScripts.initialMode, "skipped");
"#, json!(expression));
                let output = std::process::Command::new("node").args(["-e", &script]).output().unwrap();
                assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
                json!({ "accepted": true, "scripts": {} })
            };
            std::future::ready(Ok(json!({ "result": { "result": { "value": response.to_string() } } })))
        }).await.unwrap();
        assert_eq!(inventory["initialMode"], "skipped");
        assert_eq!(inventory["inventoryDeferred"], true);
        assert_eq!(inventory["skip_next_start"], false);
        assert_eq!(inventory["scripts"], json!([]));
        assert!(!std::fs::read_dir(temp.path()).unwrap().filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().starts_with(".reload-backend-")));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
