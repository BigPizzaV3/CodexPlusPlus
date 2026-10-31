use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::media::MAX_ATTACHMENT_BYTES;
use crate::settings::{BackendSettings, SettingsStore};
pub const ATTACHMENT_TOOL: &str = "weixin_send_attachment";

#[derive(Clone, Debug, PartialEq, Eq)]
struct PolicyState {
    enabled: bool,
    peers: String,
    workspace: PathBuf,
    revision: u64,
    outbound_revision: Option<String>,
}

#[derive(Clone)]
pub struct OutboundPolicy(Arc<LivePolicy>);

struct LivePolicy {
    state: watch::Sender<PolicyState>,
    stop: Option<Arc<AtomicBool>>,
    settings: Option<SettingsPolicyBinding>,
}

struct SettingsPolicyBinding {
    store: SettingsStore,
    account: [String; 4],
}

impl std::fmt::Debug for OutboundPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 绑定的账号凭据不能进入 permit、错误或调试日志。
        f.debug_struct("OutboundPolicy").field("state", &self.0.state).finish_non_exhaustive()
    }
}

fn account_identity(account: &str, token: &str, base_url: &str, route: &str) -> [String; 4] {
    let base_url = base_url.trim().trim_end_matches('/');
    [
        account.trim().to_string(),
        token.trim().to_string(),
        if base_url.is_empty() { super::DEFAULT_WEIXIN_BASE_URL } else { base_url }.to_string(),
        route.trim().to_string(),
    ]
}

#[derive(Clone, Debug)]
pub(super) struct OutboundPermit {
    policy: OutboundPolicy,
    state: PolicyState,
    pub workspace: PathBuf,
    workspace_identity: (u64, u64),
    peer: String,
}

impl OutboundPolicy {
    pub fn new(enabled: bool, peers: &str, workspace: &str) -> Self {
        Self(Arc::new(LivePolicy {
            state: watch::Sender::new(PolicyState {
                enabled,
                peers: peers.trim().to_string(),
                workspace: PathBuf::from(workspace.trim()),
                revision: 0,
                outbound_revision: None,
            }),
            stop: None,
            settings: None,
        }))
    }

    pub(crate) fn bind_settings(
        store: SettingsStore,
        config: &super::WeixinConnectConfig,
        settings: &BackendSettings,
        outbound_revision: &str,
        stop: Arc<AtomicBool>,
    ) -> Self {
        let policy = Self(Arc::new(LivePolicy {
            state: watch::Sender::new(PolicyState {
                enabled: false,
                peers: String::new(),
                workspace: PathBuf::new(),
                revision: 0,
                outbound_revision: None,
            }),
            stop: Some(stop),
            settings: Some(SettingsPolicyBinding {
                store,
                account: account_identity(&config.account_id, &config.token, &config.base_url, &config.route_tag),
            }),
        }));
        policy.apply_settings(settings, outbound_revision);
        policy
    }

    fn apply_settings(&self, settings: &BackendSettings, outbound_revision: &str) {
        let Some(binding) = &self.0.settings else { return; };
        let same_account = binding.account == account_identity(
            &settings.weixin_connect_account_id,
            &settings.weixin_connect_token,
            &settings.weixin_connect_base_url,
            &settings.weixin_connect_route_tag,
        );
        self.update(
            settings.weixin_connect_enabled && settings.weixin_connect_allow_outbound_attachments && same_account,
            &settings.weixin_connect_allow_from,
            &settings.weixin_connect_work_dir,
            Some(outbound_revision),
        );
    }

    fn refresh(&self) -> anyhow::Result<()> {
        if let Some(binding) = &self.0.settings {
            match binding.store.try_load_committed() {
                Ok(Some((settings, revision))) => self.apply_settings(&settings, &revision),
                Ok(None) => {} // 写事务尚未提交，保留上一个已提交的授权。
                Err(error) => {
                    self.set(false, "", "");
                    return Err(error).context("无法核验微信附件回传权限");
                }
            }
        }
        Ok(())
    }

    pub fn set(&self, enabled: bool, peers: &str, workspace: &str) {
        self.update(enabled, peers, workspace, None);
    }

    fn update(&self, enabled: bool, peers: &str, workspace: &str, outbound_revision: Option<&str>) {
        self.0.state.send_if_modified(|state| {
            let enabled = enabled && !self.stopped();
            if state.enabled == enabled
                && state.peers == peers.trim()
                && state.workspace == Path::new(workspace.trim())
                && outbound_revision.is_none_or(|revision| state.outbound_revision.as_deref() == Some(revision))
            {
                return false;
            }
            state.enabled = enabled;
            state.peers = peers.trim().to_string();
            state.workspace = PathBuf::from(workspace.trim());
            if let Some(revision) = outbound_revision {
                state.outbound_revision = Some(revision.to_string());
            }
            state.revision = state.revision.wrapping_add(1);
            true
        });
    }

    fn stopped(&self) -> bool {
        self.0.stop.as_ref().is_some_and(|stop| stop.load(Ordering::SeqCst))
    }

    pub(super) fn status_message(&self, message: &str) -> String {
        let result = self.refresh().and_then(|()| {
            let state = self.0.state.borrow();
            validate_settings(state.enabled, &state.peers, &state.workspace.to_string_lossy())
        });
        match result {
            Ok(()) => message.to_string(),
            Err(error) => format!("{message} 附件回传暂不可用：{error}；文字连接不受影响。"),
        }
    }

    pub(super) fn permit(&self, peer: &str) -> anyhow::Result<OutboundPermit> {
        self.refresh()?;
        let state = self.0.state.borrow().clone();
        if !state.enabled || self.stopped() {
            bail!("微信附件回传未开启");
        }
        validate_allow_from(&state.peers)?;
        if peer.trim().is_empty() || !super::is_allowed_peer(&state.peers, peer) {
            bail!("附件接收方不在允许发送方名单中");
        }
        let workspace = outbound_workspace(&state.workspace)?;
        let workspace_identity = file_identity(&open_directory(&workspace)?)?;
        Ok(OutboundPermit {
            policy: self.clone(),
            state,
            workspace,
            workspace_identity,
            peer: peer.to_string(),
        })
    }
}

pub fn validate_allow_from(peers: &str) -> anyhow::Result<()> {
    if !peers.split(',').any(|peer| !peer.trim().is_empty())
        || peers.split(',').any(|peer| peer.trim() == "*")
    {
        bail!("开启微信附件回传前必须填写允许的微信用户 ID");
    }
    Ok(())
}

pub fn validate_settings(enabled: bool, peers: &str, workspace: &str) -> anyhow::Result<()> {
    if enabled {
        validate_allow_from(peers)?;
        outbound_workspace(Path::new(workspace.trim()))?;
    }
    Ok(())
}

pub fn validate_settings_change(previous: &BackendSettings, next: &BackendSettings) -> anyhow::Result<()> {
    if next.weixin_connect_allow_outbound_attachments
        && (!previous.weixin_connect_allow_outbound_attachments
            || previous.weixin_connect_allow_from != next.weixin_connect_allow_from
            || previous.weixin_connect_work_dir != next.weixin_connect_work_dir)
    {
        validate_settings(true, &next.weixin_connect_allow_from, &next.weixin_connect_work_dir)?;
    }
    Ok(())
}

pub(super) fn outbound_workspace(workspace: &Path) -> anyhow::Result<PathBuf> {
    if workspace.as_os_str().is_empty() {
        bail!("外发附件必须显式设置工作目录");
    }
    reject_sensitive_path(workspace)?;
    let root = workspace.canonicalize().context("无法解析附件工作区")?;
    if !root.is_dir() || root.parent().is_none() {
        bail!("附件工作区必须是非根目录");
    }
    let mut homes = directories::BaseDirs::new()
        .map(|dirs| vec![dirs.home_dir().to_path_buf()])
        .unwrap_or_default();
    homes.extend(
        ["USERPROFILE", "HOME"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from),
    );
    for home in homes {
        if let Ok(home) = home.canonicalize()
            && home.starts_with(&root)
        {
            bail!("附件工作区不能是 home 或其祖先目录");
        }
    }
    reject_sensitive_path(&root)?;
    reject_state_path(&root)?;
    Ok(root)
}

fn reject_state_path(path: &Path) -> anyhow::Result<()> {
    let mut states = vec![
        crate::paths::default_app_state_dir(),
        crate::relay_config::default_codex_home_dir(),
    ];
    states.extend(
        [
            "CODEX_HOME",
            "CODEX_SQLITE_HOME",
            "CODEX_PLUS_STATE_DIR",
            "CODEX_ELECTRON_USER_DATA_PATH",
        ]
        .into_iter()
        .filter_map(std::env::var_os)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from),
    );
    if let Some(appdata) = std::env::var_os("APPDATA") {
        states.extend([
            PathBuf::from(&appdata).join("Codex++"),
            PathBuf::from(appdata).join("Codex"),
        ]);
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        states.extend([
            PathBuf::from(&local).join("OpenAI").join("Codex"),
            PathBuf::from(local).join("Codex"),
        ]);
    }
    require_outside_states(path, states)
}

fn require_outside_states(
    path: &Path,
    states: impl IntoIterator<Item = PathBuf>,
) -> anyhow::Result<()> {
    for state in states {
        if let Ok(state) = state.canonicalize()
            && path.starts_with(state)
        {
            bail!("不能外发 Codex 或管理器状态目录中的文件");
        }
    }
    Ok(())
}

fn reject_sensitive_path(path: &Path) -> anyhow::Result<()> {
    for component in path.components() {
        if let Component::Normal(name) = component {
            let name = name.to_string_lossy().to_ascii_lowercase();
            let name = name.trim_end_matches([' ', '.']);
            if name.contains(':')
                || name.starts_with(".env")
                || name.ends_with(".pem")
                || name.ends_with(".key")
                || name.ends_with(".p12")
                || name.ends_with(".pfx")
                || [
                    "auth.json",
                    "config.toml",
                    "settings.json",
                    "credentials",
                    "credentials.json",
                    "secrets.json",
                    ".codex",
                    ".ssh",
                    ".aws",
                    ".gnupg",
                    ".azure",
                    ".kube",
                    "id_rsa",
                    "id_ed25519",
                    "id_ecdsa",
                ]
                .iter()
                .any(|sensitive| {
                    name.strip_prefix(sensitive)
                        .is_some_and(|suffix| suffix.starts_with(['.', '-', '_']))
                })
                || matches!(
                    name,
                    ".git"
                        | ".gitconfig"
                        | ".git-credentials"
                        | ".docker"
                        | ".ssh"
                        | ".gnupg"
                        | ".aws"
                        | ".azure"
                        | ".kube"
                        | ".config"
                        | ".codex"
                        | ".codex-session-delete"
                        | "codex++"
                        | "auth.json"
                        | "config.toml"
                        | "settings.json"
                        | "credentials"
                        | "credentials.json"
                        | "secrets.json"
                        | ".npmrc"
                        | ".netrc"
                        | ".pypirc"
                        | "id_rsa"
                        | "id_ed25519"
                        | "id_ecdsa"
                        | "login.keychain-db"
                )
            {
                bail!("不能外发敏感配置、凭据或状态目录中的文件");
            }
        }
    }
    Ok(())
}

fn open_directory(path: &Path) -> anyhow::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        return Ok(std::fs::OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)?);
    }
    #[cfg(not(windows))]
    open_verified(path)
}

fn require_path(path: &Path, root: &Path) -> anyhow::Result<()> {
    if path == root || !path.starts_with(root) {
        bail!("附件必须位于当前工作区内");
    }
    reject_sensitive_path(path)?;
    reject_state_path(path)
}

pub(super) fn read_workspace_file(path: &Path, root: &Path) -> anyhow::Result<(PathBuf, Vec<u8>)> {
    let root = outbound_workspace(root)?;
    let root_identity = file_identity(&open_directory(&root)?)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    reject_sensitive_path(&candidate)?;
    let canonical = candidate.canonicalize().context("无法解析附件路径")?;
    require_path(&canonical, &root)?;
    let mut file = open_verified(&canonical)?;
    let before = file.metadata()?;
    if !before.is_file() || before.len() > MAX_ATTACHMENT_BYTES as u64 {
        bail!("附件必须是不超过 100 MiB 的普通文件");
    }
    verify_file(&file, &canonical, &root)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_ATTACHMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (before.ctime(), before.ctime_nsec()) != (after.ctime(), after.ctime_nsec()) {
            bail!("附件内容在读取期间发生变化");
        }
    }
    if bytes.len() > MAX_ATTACHMENT_BYTES
        || bytes.len() as u64 != before.len()
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        bail!("附件内容或大小在读取期间发生变化");
    }
    verify_file(&file, &canonical, &root)?;
    if candidate.canonicalize()? != canonical
        || file_identity(&open_directory(&root)?)? != root_identity
    {
        bail!("附件路径在读取期间发生变化");
    }
    Ok((canonical, bytes))
}

#[cfg(windows)]
fn open_verified(path: &Path) -> anyhow::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    Ok(std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(0x00200000)
        .open(path)?)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn open_verified(path: &Path) -> anyhow::Result<File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };
    const FLAGS: i32 = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    let mut directory = File::open("/")?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let name = std::ffi::CString::new(name.as_bytes())?;
        let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), FLAGS) };
        if descriptor < 0 {
            bail!("附件路径无法访问或在核验期间变为链接");
        }
        directory = unsafe { File::from_raw_fd(descriptor) };
    }
    Ok(directory)
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn open_verified(_path: &Path) -> anyhow::Result<File> {
    bail!("当前平台不支持附件句柄核验")
}

#[cfg(windows)]
fn file_identity(file: &File) -> anyhow::Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)?;
    }
    if file.metadata()?.is_file() && info.nNumberOfLinks != 1 {
        bail!("不能外发具有多个硬链接的文件");
    }
    Ok((
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ))
}

#[cfg(unix)]
fn file_identity(file: &File) -> anyhow::Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    if metadata.is_file() && metadata.nlink() != 1 {
        bail!("不能外发具有多个硬链接的文件");
    }
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(not(any(windows, unix)))]
fn file_identity(_file: &File) -> anyhow::Result<(u64, u64)> {
    bail!("当前平台不支持附件文件身份核验")
}

fn verify_file(file: &File, path: &Path, root: &Path) -> anyhow::Result<()> {
    require_path(&path.canonicalize()?, root)?;
    let target = open_verified(path)?;
    if !target.metadata()?.is_file() || file_identity(file)? != file_identity(&target)? {
        bail!("附件文件身份已改变");
    }
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStringExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW},
        };
        let mut buffer = vec![0u16; 32768];
        let length = unsafe {
            GetFinalPathNameByHandleW(
                HANDLE(file.as_raw_handle()),
                &mut buffer,
                FILE_NAME_NORMALIZED,
            )
        } as usize;
        if length == 0 || length >= buffer.len() {
            bail!("无法核验附件句柄的实际路径");
        }
        let actual = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length]));
        require_path(&actual, root)?;
        if actual != path {
            bail!("附件句柄路径与核准路径不同");
        }
    }
    Ok(())
}

impl OutboundPermit {
    pub fn check_peer(&self, peer: &str) -> anyhow::Result<()> {
        self.check()?;
        if self.peer != peer {
            bail!("附件回传授权不属于当前联系人");
        }
        Ok(())
    }
    pub fn check(&self) -> anyhow::Result<()> {
        self.policy.refresh()?;
        if self.policy.stopped() || *self.policy.0.state.borrow() != self.state {
            bail!("附件回传权限已改变，请在新回合重试");
        }
        if outbound_workspace(&self.state.workspace)? != self.workspace
            || file_identity(&open_directory(&self.workspace)?)? != self.workspace_identity
        {
            bail!("附件工作区身份已改变");
        }
        Ok(())
    }

    pub async fn guard<T>(
        &self,
        future: impl std::future::Future<Output = anyhow::Result<T>>,
    ) -> anyhow::Result<T> {
        let mut changes = self.policy.0.state.subscribe();
        self.check_async().await?;
        let mut refresh = tokio::time::interval(Duration::from_millis(100));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::pin!(future);
        loop {
            tokio::select! {
                biased;
                _ = changes.changed() => bail!("附件回传权限已改变，已取消发送"),
                _ = refresh.tick() => self.check_async().await?,
                result = &mut future => { self.check_async().await?; return result; }
            }
        }
    }

    async fn check_async(&self) -> anyhow::Result<()> {
        let permit = self.clone();
        tokio::task::spawn_blocking(move || permit.check()).await?
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct AttachmentArguments {
    pub path: String,
    pub kind: String,
}

pub(super) struct AttachmentCalls {
    pub permit: Option<OutboundPermit>,
    peer: String,
    thread: String,
    pub turn: Option<String>,
    results: HashMap<String, (AttachmentArguments, Value)>,
    attempts: usize,
}

impl AttachmentCalls {
    pub fn new(peer: &str, thread: &str, permit: Option<OutboundPermit>) -> Self {
        Self {
            permit,
            peer: peer.to_string(),
            thread: thread.to_string(),
            turn: None,
            results: HashMap::new(),
            attempts: 0,
        }
    }

    pub fn validate(
        &self,
        message: &Value,
    ) -> anyhow::Result<(String, AttachmentArguments, Option<Value>)> {
        self.permit
            .as_ref()
            .context("微信附件回传未开启")?
            .check_peer(&self.peer)?;
        let params = &message["params"];
        if message["method"] != "item/tool/call"
            || params["tool"] != ATTACHMENT_TOOL
            || !params["namespace"].is_null()
        {
            bail!("未知附件工具");
        }
        if params["threadId"].as_str() != Some(self.thread.as_str())
            || self.turn.is_none()
            || params["turnId"].as_str() != self.turn.as_deref()
        {
            bail!("附件调用不属于当前微信线程或回合");
        }
        let call = params["callId"]
            .as_str()
            .filter(|call| !call.trim().is_empty() && call.len() <= 128)
            .context("附件调用缺少有效的调用标识")?
            .to_string();
        let arguments: AttachmentArguments =
            serde_json::from_value(params["arguments"].clone()).context("附件参数无效")?;
        if arguments.path.trim().is_empty()
            || arguments.path.len() > 32768
            || !matches!(arguments.kind.as_str(), "image" | "file")
        {
            bail!("附件必须提供有效路径及 image 或 file 类型");
        }
        let cached = if let Some((original, result)) = self.results.get(&call) {
            if original != &arguments {
                bail!("同一附件调用标识不能用于不同参数");
            }
            Some(result.clone())
        } else {
            None
        };
        Ok((call, arguments, cached))
    }

    pub fn begin_send(&mut self) -> anyhow::Result<()> {
        if self.attempts >= 10 {
            bail!("每回合最多回传 10 个附件");
        }
        self.attempts += 1;
        Ok(())
    }

    pub fn record(&mut self, call: String, arguments: AttachmentArguments, result: Value) {
        if self.results.len() < 32 {
            self.results.insert(call, (arguments, result));
        }
    }
}

pub(super) fn attachment_tool_spec() -> Value {
    json!({
        "name": ATTACHMENT_TOOL,
        "description": "将当前工作区中的图片或文件回传给当前微信用户。用户请求接收文件时实际调用本工具；不得只回复本地路径。kind=image 显示图片，kind=file 发送文件。",
        "inputSchema": {
            "type": "object", "properties": {
                "path": {"type": "string"}, "kind": {"type": "string", "enum": ["image", "file"]}
            }, "required": ["path", "kind"], "additionalProperties": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connect::WeixinConnectConfig;

    struct SettingsFixture {
        dir: tempfile::TempDir,
        store: SettingsStore,
        config: WeixinConnectConfig,
        stop: Arc<AtomicBool>,
        policy: OutboundPolicy,
    }

    impl SettingsFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let workspace = dir.path().join("workspace");
            std::fs::create_dir(&workspace).unwrap();
            let store = SettingsStore::new(dir.path().join("settings.json"));
            let mut settings = store.load().unwrap();
            settings.weixin_connect_enabled = true;
            settings.weixin_connect_allow_outbound_attachments = true;
            settings.weixin_connect_allow_from = "peer".into();
            settings.weixin_connect_work_dir = workspace.to_str().unwrap().into();
            settings.weixin_connect_account_id = "old-account".into();
            settings.weixin_connect_token = "fake-test-token".into();
            store.save(&settings).unwrap();
            let config = WeixinConnectConfig {
                account_id: settings.weixin_connect_account_id,
                token: settings.weixin_connect_token,
                base_url: settings.weixin_connect_base_url,
                route_tag: settings.weixin_connect_route_tag,
                allow_outbound_attachments: true,
                allow_from: settings.weixin_connect_allow_from,
                work_dir: settings.weixin_connect_work_dir,
                ..WeixinConnectConfig::default()
            }.normalized();
            let stop = Arc::new(AtomicBool::new(false));
            let policy = store.weixin_outbound_policy(&config, Arc::clone(&stop)).unwrap();
            Self { dir, store, config, stop, policy }
        }
    }

    async fn assert_settings_revoke_cancels(permit: OutboundPermit, revoke: impl FnOnce()) {
        let old = permit.clone();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            permit.guard(async {
                let _ = entered.send(());
                std::future::pending::<anyhow::Result<()>>().await
            }).await
        });
        ready.await.unwrap();
        revoke();
        assert!(old.check().is_err(), "committed settings must revoke the old permit immediately");
        assert!(tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await.expect("inflight attachment was not cancelled").unwrap().is_err());
    }

    #[tokio::test]
    async fn outbound_settings_qr_save_revokes_old_account_and_cancels_inflight() {
        // 与扫码确认一致：load -> 更新账号字段 -> save，不经过 manager save_settings。
        for field in ["account", "token", "base", "route"] {
            let fixture = SettingsFixture::new();
            let mut settings = fixture.store.load().unwrap();
            let mut original = settings.clone();
            match field {
                "account" => settings.weixin_connect_account_id = "new-account".into(),
                "token" => settings.weixin_connect_token = "new-fake-token".into(),
                "base" => settings.weixin_connect_base_url = "https://example.invalid".into(),
                _ => settings.weixin_connect_route_tag = "new-route".into(),
            }
            let permit = fixture.policy.permit("peer").unwrap();
            let old = permit.clone();
            assert_settings_revoke_cancels(permit, || fixture.store.save(&settings).unwrap()).await;
            assert!(fixture.policy.permit("peer").is_err());
            let current = fixture.store.load().unwrap();
            fixture.store.save(&current).unwrap();
            assert!(fixture.policy.permit("peer").is_err());
            original.settings_revision = fixture.store.revision().unwrap();
            fixture.store.save(&original).unwrap();
            assert!(fixture.policy.permit("peer").is_ok());
            assert!(old.check().is_err(), "restoring the account resurrected an old permit");
        }
    }

    #[tokio::test]
    async fn outbound_settings_bridge_set_revokes_permits_and_cancels_inflight() {
        use crate::routes::BridgeContext;

        for field in ["disabled", "connection-disabled", "peers", "wildcard", "workspace", "account"] {
            let fixture = SettingsFixture::new();
            let other_workspace = fixture.dir.path().join("other");
            std::fs::create_dir(&other_workspace).unwrap();
            let payload = match field {
                "disabled" => json!({"weixinConnectAllowOutboundAttachments": false}),
                "connection-disabled" => json!({"weixinConnectEnabled": false}),
                "peers" => json!({"weixinConnectAllowFrom": "other"}),
                "wildcard" => json!({"weixinConnectAllowFrom": "*"}),
                "workspace" => json!({"weixinConnectWorkDir": other_workspace}),
                _ => json!({"weixinConnectAccountId": "new-account"}),
            };
            let ctx = BridgeContext::with_test_settings(fixture.store.clone(), fixture.dir.path().join("app"));
            let permit = fixture.policy.permit("peer").unwrap();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let old = permit.clone();
            let task = tokio::spawn(async move {
                permit.guard(async {
                    let _ = entered.send(());
                    std::future::pending::<anyhow::Result<()>>().await
                }).await
            });
            ready.await.unwrap();
            let response = crate::routes::handle_bridge_request(ctx, "/settings/set", payload.clone()).await;
            for (key, expected) in payload.as_object().unwrap() {
                assert_eq!(&response[key], expected, "bridge settings write failed for {key}");
            }
            assert!(old.check().is_err(), "bridge field {field} did not revoke the permit");
            assert!(tokio::time::timeout(std::time::Duration::from_secs(2), task)
                .await.expect("bridge did not cancel inflight attachment").unwrap().is_err());
        }
    }

    #[tokio::test]
    async fn outbound_settings_writer_process_fixture() {
        let Some(path) = std::env::var_os("CODEX_PLUS_OUTBOUND_TEST_SETTINGS") else { return; };
        let path = PathBuf::from(path);
        let store = SettingsStore::new(path.clone());
        match std::env::var("CODEX_PLUS_OUTBOUND_TEST_WRITE").unwrap().as_str() {
            "fast-off-on" => {
                store.update(json!({"weixinConnectAllowOutboundAttachments": false})).unwrap();
                store.update(json!({"weixinConnectAllowOutboundAttachments": true})).unwrap();
            }
            "qr-account" => {
                let mut settings = store.load().unwrap();
                settings.weixin_connect_account_id = "new-account".into();
                settings.weixin_connect_token = "new-fake-token".into();
                store.save(&settings).unwrap();
            }
            "unrelated" => {
                let ctx = crate::routes::BridgeContext::with_test_settings(store, path.parent().unwrap().join("app"));
                let result = crate::routes::handle_bridge_request(ctx, "/settings/set", json!({
                    "weixinConnectModel": "next-turn-model",
                    "codexAppDreamSkinTheme": "pink",
                })).await;
                assert_eq!(result["weixinConnectModel"], "next-turn-model");
                assert_eq!(result["codexAppDreamSkinTheme"], "pink");
            }
            mode => {
                let payload = if mode == "connection-disabled" {
                    json!({"weixinConnectEnabled": false})
                } else {
                    json!({"weixinConnectAllowOutboundAttachments": false})
                };
                let ctx = crate::routes::BridgeContext::with_test_settings(store, path.parent().unwrap().join("app"));
                let result = crate::routes::handle_bridge_request(ctx, "/settings/set", payload.clone()).await;
                for (key, expected) in payload.as_object().unwrap() {
                    assert_eq!(&result[key], expected);
                }
            }
        }
    }

    async fn run_settings_writer(fixture: &SettingsFixture, mode: &str) {
        let mut writer = tokio::process::Command::new(std::env::current_exe().unwrap());
        writer.args(["--exact", "connect::outbound::tests::outbound_settings_writer_process_fixture"])
            .env("CODEX_PLUS_OUTBOUND_TEST_SETTINGS", fixture.dir.path().join("settings.json"))
            .env("CODEX_PLUS_OUTBOUND_TEST_WRITE", mode)
            .kill_on_drop(true);
        #[cfg(windows)]
        writer.creation_flags(0x08000000);
        let result = tokio::time::timeout(Duration::from_secs(10), writer.output()).await.unwrap().unwrap();
        assert!(result.status.success(), "settings writer fixture failed for {mode}");
    }

    #[tokio::test]
    async fn outbound_settings_another_process_revokes_new_and_inflight_permits() {
        for mode in ["qr-account", "connection-disabled", "attachments-disabled"] {
            let fixture = SettingsFixture::new();
            let idle_policy = fixture.store.weixin_outbound_policy(&fixture.config, Arc::clone(&fixture.stop)).unwrap();
            let permit = fixture.policy.permit("peer").unwrap();
            let old = permit.clone();
            let (entered, ready) = tokio::sync::oneshot::channel();
            let task = tokio::spawn(async move {
                permit.guard(async {
                    let _ = entered.send(());
                    std::future::pending::<anyhow::Result<()>>().await
                }).await
            });
            ready.await.unwrap();
            run_settings_writer(&fixture, mode).await;
            // 不先调用 check/permit，必须由 guard 的只读轮询主动取消在途操作。
            assert!(tokio::time::timeout(Duration::from_secs(1), task)
                .await.expect("cross-process revocation exceeded the polling bound").unwrap().is_err());
            assert!(old.check().is_err());
            assert!(idle_policy.permit("peer").is_err(), "new permit used a stale process-local snapshot");
        }
    }

    #[test]
    fn outbound_settings_failed_saves_leave_current_policy_unchanged() {
        let fixture = SettingsFixture::new();
        let permit = fixture.policy.permit("peer").unwrap();
        let mut settings = fixture.store.load().unwrap();
        settings.weixin_connect_allow_outbound_attachments = false;
        assert!(fixture.store.save_with_commit(&settings, |_| {
            assert!(permit.check().is_ok(), "an uncommitted candidate changed the active policy");
            anyhow::bail!("test publish failure")
        }).is_err());
        assert!(fixture.store.load().unwrap().weixin_connect_allow_outbound_attachments);
        assert!(permit.check().is_ok());
        settings.settings_revision = "stale".into();
        assert!(fixture.store.save(&settings).is_err());
        assert!(fixture.store.update(json!({"revision": "stale", "weixinConnectAllowOutboundAttachments": false})).is_err());
        assert!(permit.check().is_ok());

        fixture.store.update(json!({"weixinConnectAllowOutboundAttachments": false})).unwrap();
        let mut enable = fixture.store.load().unwrap();
        enable.weixin_connect_allow_outbound_attachments = true;
        assert!(fixture.store.save_with_commit(&enable, |_| anyhow::bail!("test publish failure")).is_err());
        assert!(fixture.policy.permit("peer").is_err());
    }

    #[test]
    fn outbound_settings_full_save_and_reenable_do_not_restore_old_permits_or_cache() {
        let fixture = SettingsFixture::new();
        let permit = fixture.policy.permit("peer").unwrap();
        let mut calls = AttachmentCalls::new("peer", "thread", Some(permit.clone()));
        calls.turn = Some("turn".into());
        let request = call("thread", "turn", "call");
        let (id, arguments, _) = calls.validate(&request).unwrap();
        calls.record(id, arguments, json!({"success": true}));
        let mut settings = fixture.store.load().unwrap();
        settings.weixin_connect_allow_outbound_attachments = false;
        fixture.store.save_with_commit(&settings, |_| Ok(())).unwrap();
        assert!(permit.check().is_err());
        let mut settings = fixture.store.load().unwrap();
        settings.weixin_connect_allow_outbound_attachments = true;
        fixture.store.save(&settings).unwrap();
        assert!(fixture.policy.permit("peer").is_ok());
        assert!(permit.check().is_err());
        assert!(calls.validate(&request).is_err());
    }

    #[tokio::test]
    async fn outbound_settings_fast_off_on_invalidates_old_permit_and_success_cache() {
        for another_process in [false, true] {
            let fixture = SettingsFixture::new();
            let permit = fixture.policy.permit("peer").unwrap();
            let mut calls = AttachmentCalls::new("peer", "thread", Some(permit.clone()));
            calls.turn = Some("turn".into());
            let request = call("thread", "turn", "call");
            let (id, arguments, _) = calls.validate(&request).unwrap();
            calls.record(id, arguments, json!({"success": true}));
            if another_process {
                run_settings_writer(&fixture, "fast-off-on").await;
            } else {
                fixture.store.update(json!({"weixinConnectAllowOutboundAttachments": false})).unwrap();
                fixture.store.update(json!({"weixinConnectAllowOutboundAttachments": true})).unwrap();
            }
            // 两次提交之间没有任何权限刷新，当前权限值已经完全恢复。
            assert!(permit.check().is_err(), "fast off/on resurrected an old permit");
            assert!(calls.validate(&request).is_err(), "fast off/on restored a cached success");
            assert!(fixture.policy.permit("peer").is_ok());
        }
    }

    #[tokio::test]
    async fn outbound_settings_unrelated_saves_preserve_permit_cache_and_inflight() {
        let fixture = SettingsFixture::new();
        let permit = fixture.policy.permit("peer").unwrap();
        let mut calls = AttachmentCalls::new("peer", "thread", Some(permit.clone()));
        calls.turn = Some("turn".into());
        let request = call("thread", "turn", "call");
        let (id, arguments, _) = calls.validate(&request).unwrap();
        calls.record(id, arguments, json!({"success": true}));
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (finish, finished) = tokio::sync::oneshot::channel();
        let active = permit.clone();
        let task = tokio::spawn(async move {
            active.guard(async {
                let _ = entered.send(());
                finished.await.unwrap();
                Ok(())
            }).await
        });
        ready.await.unwrap();
        let mut settings = fixture.store.load().unwrap();
        settings.codex_app_dream_skin_theme = "miku".into();
        fixture.store.save(&settings).unwrap();
        assert_eq!(fixture.store.load().unwrap().codex_app_dream_skin_theme, "miku");
        assert!(permit.check().is_ok(), "full unrelated save revoked the permit");
        run_settings_writer(&fixture, "unrelated").await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(permit.check().is_ok(), "cross-process unrelated patch revoked the permit");
        assert_eq!(calls.validate(&request).unwrap().2, Some(json!({"success": true})));
        assert!(!task.is_finished(), "unrelated save cancelled the inflight attachment");
        finish.send(()).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().is_ok());
    }

    #[test]
    fn outbound_settings_authorization_roundtrips_invalidate_old_permits() {
        for (field, replacement) in [
            ("weixinConnectEnabled", json!(false)),
            ("weixinConnectAllowOutboundAttachments", json!(false)),
            ("weixinConnectAllowFrom", json!("other")),
            ("weixinConnectWorkDir", json!("other-workspace")),
            ("weixinConnectAccountId", json!("other-account")),
            ("weixinConnectToken", json!("other-fake-token")),
            ("weixinConnectBaseUrl", json!("https://example.invalid")),
            ("weixinConnectRouteTag", json!("other-route")),
        ] {
            for full_save in [false, true] {
                let fixture = SettingsFixture::new();
                let permit = fixture.policy.permit("peer").unwrap();
                let mut original = fixture.store.load().unwrap();
                let mut changed = serde_json::to_value(&original).unwrap();
                changed[field] = replacement.clone();
                if full_save {
                    fixture.store.save(&serde_json::from_value(changed).unwrap()).unwrap();
                    original.settings_revision = fixture.store.revision().unwrap();
                    fixture.store.save(&original).unwrap();
                } else {
                    let mut patch = json!({});
                    patch[field] = replacement.clone();
                    fixture.store.update(patch).unwrap();
                    let mut patch = json!({});
                    patch[field] = serde_json::to_value(&original).unwrap()[field].clone();
                    fixture.store.update(patch).unwrap();
                }
                assert!(permit.check().is_err(), "{field} roundtrip resurrected an old permit (full_save={full_save})");
                assert!(fixture.policy.permit("peer").is_ok());
            }
        }
    }

    #[test]
    fn outbound_settings_binding_is_current_isolated_and_stop_is_sticky() {
        let fixture = SettingsFixture::new();
        let permit = fixture.policy.permit("peer").unwrap();
        let other = SettingsFixture::new();
        other.store.update(json!({"weixinConnectAllowOutboundAttachments": false})).unwrap();
        assert!(permit.check().is_ok(), "unrelated store changed policy");
        fixture.store.update(json!({"weixinConnectToken": " fake-test-token ", "weixinConnectBaseUrl": ""})).unwrap();
        assert!(permit.check().is_err(), "a committed settings write must invalidate old permits");
        assert!(fixture.policy.permit("peer").is_ok(), "normalized account identity should still allow new permits");

        fixture.store.update(json!({"weixinConnectAllowOutboundAttachments": false})).unwrap();
        let rebound = fixture.store.weixin_outbound_policy(&fixture.config, Arc::clone(&fixture.stop)).unwrap();
        assert!(rebound.permit("peer").is_err(), "stale startup config restored revoked permission");
        fixture.store.update(json!({"weixinConnectAllowOutboundAttachments": true})).unwrap();
        let permit = fixture.policy.permit("peer").unwrap();
        fixture.stop.store(true, Ordering::SeqCst);
        fixture.policy.set(false, "", "");
        fixture.store.save(&fixture.store.load().unwrap()).unwrap();
        assert!(permit.check().is_err());
        assert!(fixture.policy.permit("peer").is_err());
        assert!(rebound.permit("peer").is_err());
    }

    #[test]
    fn outbound_settings_snapshot_restore_and_missing_file_revoke_permits() {
        let fixture = SettingsFixture::new();
        let original = fixture.store.snapshot_raw_bytes().unwrap();
        fixture.store.update(json!({"weixinConnectAllowFrom": "other"})).unwrap();
        let permit = fixture.policy.permit("other").unwrap();
        fixture.store.restore_raw_snapshot(original.as_deref(), &fixture.store.revision().unwrap()).unwrap();
        assert!(permit.check().is_err());
        let permit = fixture.policy.permit("peer").unwrap();
        fixture.store.restore_raw_snapshot(None, &fixture.store.revision().unwrap()).unwrap();
        assert!(permit.check().is_err());
        assert!(fixture.policy.permit("peer").is_err());
    }

    fn call(thread: &str, turn: &str, id: &str) -> Value {
        json!({ "id": 1, "method": "item/tool/call", "params": {
            "threadId": thread, "turnId": turn, "callId": id, "tool": ATTACHMENT_TOOL,
            "namespace": null, "arguments": { "path": "result.txt", "kind": "file" }
        }})
    }

    #[test]
    fn outbound_cache_requires_same_peer_thread_turn_call_and_arguments() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().to_str().unwrap();
        let policy = OutboundPolicy::new(true, "peer,other", root);
        let permit = policy.permit("peer").unwrap();
        let mut calls = AttachmentCalls::new("peer", "thread", Some(permit.clone()));
        let request = call("thread", "turn", "call");
        assert!(calls.validate(&request).is_err());
        calls.turn = Some("turn".to_string());
        let (id, arguments, cached) = calls.validate(&request).unwrap();
        assert!(cached.is_none());
        calls.record(id, arguments, json!({"success": true}));
        assert_eq!(
            calls.validate(&request).unwrap().2,
            Some(json!({"success": true}))
        );
        for invalid in [
            call("other", "turn", "call"),
            call("thread", "other", "call"),
            call("thread", "turn", ""),
        ] {
            assert!(calls.validate(&invalid).is_err());
        }
        let mut invalid = request.clone();
        invalid["params"]["arguments"]["path"] = json!("other.txt");
        assert!(calls.validate(&invalid).is_err());
        invalid = request.clone();
        invalid["params"]["tool"] = json!("other");
        assert!(calls.validate(&invalid).is_err());
        invalid = request.clone();
        invalid["params"]["namespace"] = json!("other");
        assert!(calls.validate(&invalid).is_err());
        invalid = request.clone();
        invalid["params"]["arguments"]["to_user_id"] = json!("other");
        assert!(calls.validate(&invalid).is_err());
        let mut wrong_peer = AttachmentCalls::new("other", "thread", Some(permit));
        wrong_peer.turn = Some("turn".to_string());
        assert!(wrong_peer.validate(&request).is_err());
        policy.set(false, "peer,other", root);
        assert!(calls.validate(&request).is_err());
        policy.set(true, "peer,other", root);
        assert!(calls.validate(&request).is_err());
        let mut next = AttachmentCalls::new("peer", "thread", policy.permit("peer").ok());
        next.turn = Some("next".to_string());
        assert!(
            next.validate(&call("thread", "next", "call"))
                .unwrap()
                .2
                .is_none()
        );
    }

    #[test]
    fn outbound_replaced_workspace_invalidates_existing_permit() {
        let parent = tempfile::tempdir().unwrap();
        let workspace = parent.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let policy = OutboundPolicy::new(true, "peer", workspace.to_str().unwrap());
        let permit = policy.permit("peer").unwrap();
        std::fs::rename(&workspace, parent.path().join("old-workspace")).unwrap();
        std::fs::create_dir(&workspace).unwrap();
        assert!(permit.check().is_err());
    }

    #[test]
    fn outbound_defaults_and_empty_allowlist_are_denied() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().to_str().unwrap();
        assert!(
            OutboundPolicy::new(false, "peer", root)
                .permit("peer")
                .is_err()
        );
        for peers in ["", " , \n", "other", "*", "peer,*"] {
            assert!(
                OutboundPolicy::new(true, peers, root)
                    .permit("peer")
                    .is_err()
            );
        }
        assert!(OutboundPolicy::new(true, "peer", root).permit("").is_err());
        assert!(validate_settings(false, "", "").is_ok());
        assert!(validate_settings(true, "", root).is_err());
        assert!(validate_settings(true, "peer", "").is_err());
        assert!(validate_settings(true, "peer", root).is_ok());
    }

    #[test]
    fn outbound_invalid_existing_workspace_does_not_block_unrelated_settings() {
        let fixture = SettingsFixture::new();
        let previous = fixture.store.load().unwrap();
        std::fs::rename(&previous.weixin_connect_work_dir, fixture.dir.path().join("moved")).unwrap();
        let mut next = previous.clone();
        next.weixin_connect_model = "next-model".into();
        next.enhancements_enabled = !next.enhancements_enabled;
        assert!(validate_settings_change(&previous, &next).is_ok());
        next.weixin_connect_allow_from = "other-peer".into();
        assert!(validate_settings_change(&previous, &next).is_err());
        next.weixin_connect_allow_outbound_attachments = false;
        assert!(validate_settings_change(&previous, &next).is_ok());
        assert!(validate_settings_change(&next, &previous).is_err());
        next = previous.clone();
        next.weixin_connect_work_dir = fixture.dir.path().join("missing").to_str().unwrap().into();
        assert!(validate_settings_change(&previous, &next).is_err());
    }

    #[tokio::test]
    async fn outbound_workspace_disappearing_cancels_only_attachment_and_reports_warning() {
        let fixture = SettingsFixture::new();
        let permit = fixture.policy.permit("peer").unwrap();
        assert_settings_revoke_cancels(permit, || {
            std::fs::rename(&fixture.config.work_dir, fixture.dir.path().join("moved")).unwrap();
        }).await;
        assert!(fixture.policy.permit("peer").is_err());
        let warning = fixture.policy.status_message("text-ready");
        assert!(warning.starts_with("text-ready"));
        assert!(warning.contains("文字连接不受影响"));
        fixture.store.update(json!({"weixinConnectAllowOutboundAttachments":false})).unwrap();
        assert_eq!(fixture.policy.status_message("text-ready"), "text-ready");
    }

    #[tokio::test]
    async fn outbound_revoke_cancels_inflight_and_invalidates_old_permit() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().to_str().unwrap();
        let policy = OutboundPolicy::new(true, "peer", root);
        let permit = policy.permit("peer").unwrap();
        let old = permit.clone();
        let (entered, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            permit
                .guard(async {
                    let _ = entered.send(());
                    std::future::pending::<anyhow::Result<()>>().await
                })
                .await
        });
        ready.await.unwrap();
        policy.set(false, "peer", root);
        assert!(task.await.unwrap().is_err());
        policy.set(true, "peer", root);
        assert!(old.check().is_err());
        assert!(policy.permit("peer").is_ok());
    }

    #[test]
    fn outbound_changed_recipient_and_workspace_revoke_permits() {
        let workspace = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let root = workspace.path().to_str().unwrap();
        let policy = OutboundPolicy::new(true, "peer", root);
        let permit = policy.permit("peer").unwrap();
        policy.set(true, "other", root);
        assert!(permit.check().is_err());
        assert!(policy.permit("peer").is_err());
        let permit = policy.permit("other").unwrap();
        policy.set(true, "other", other.path().to_str().unwrap());
        assert!(permit.check().is_err());
    }

    #[test]
    fn outbound_rejects_broad_roots_and_sensitive_components() {
        assert!(outbound_workspace(Path::new("")).is_err());
        if let Some(dirs) = directories::BaseDirs::new() {
            let home = dirs.home_dir().canonicalize().unwrap();
            for ancestor in home.ancestors() {
                assert!(outbound_workspace(ancestor).is_err(), "{ancestor:?}");
            }
        }
        for name in [
            ".codex/result.txt",
            ".ssh/key",
            ".aws/credentials",
            ".env.local",
            "AUTH.JSON",
            "config.toml",
            "key.pem",
            "settings.json",
            "settings.json.bak",
            ".ssh-backup/key",
            ".codex.bak/result.txt",
            "file:stream",
            ".git/config",
        ] {
            assert!(reject_sensitive_path(Path::new(name)).is_err(), "{name}");
        }
    }

    #[test]
    fn outbound_reads_only_the_verified_workspace_file() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let path = workspace.path().join("result.txt");
        std::fs::write(&path, b"result").unwrap();
        let (actual, bytes) =
            read_workspace_file(Path::new("result.txt"), workspace.path()).unwrap();
        assert_eq!(actual, path.canonicalize().unwrap());
        assert_eq!(bytes, b"result");
        assert!(read_workspace_file(outside.path(), workspace.path()).is_err());
        assert!(read_workspace_file(Path::new("."), workspace.path()).is_err());
        assert!(read_workspace_file(Path::new("missing"), workspace.path()).is_err());
        let wrong_handle = File::open(outside.path()).unwrap();
        assert!(
            verify_file(
                &wrong_handle,
                &actual,
                &workspace.path().canonicalize().unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn outbound_rejects_hardlink_aliases_and_oversized_files() {
        let workspace = tempfile::tempdir().unwrap();
        let original = workspace.path().join("original.txt");
        let alias = workspace.path().join("alias.txt");
        std::fs::write(&original, b"secret").unwrap();
        std::fs::hard_link(&original, &alias).unwrap();
        assert!(read_workspace_file(&alias, workspace.path()).is_err());
        let large = workspace.path().join("large.bin");
        File::create(&large)
            .unwrap()
            .set_len(MAX_ATTACHMENT_BYTES as u64 + 1)
            .unwrap();
        assert!(read_workspace_file(&large, workspace.path()).is_err());
    }

    #[test]
    fn outbound_rejects_sensitive_files_inside_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        for name in [".codex", ".ssh", ".aws", "Codex++", ".docker"] {
            let directory = workspace.path().join(name);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("result.txt"), b"private").unwrap();
            assert!(outbound_workspace(&directory).is_err());
            assert!(read_workspace_file(&directory.join("result.txt"), workspace.path()).is_err());
        }
        for name in [".gitconfig", ".git-credentials"] {
            let file = workspace.path().join(name);
            std::fs::write(&file, b"private").unwrap();
            assert!(read_workspace_file(&file, workspace.path()).is_err());
        }
    }

    #[test]
    fn outbound_rejects_custom_state_directories_without_global_environment_changes() {
        let parent = tempfile::tempdir().unwrap();
        let state = parent.path().join("custom-state");
        std::fs::create_dir(&state).unwrap();
        let path = state.join("result.txt");
        std::fs::write(&path, b"private").unwrap();
        let canonical = path.canonicalize().unwrap();
        assert!(require_outside_states(&canonical, [state.clone()]).is_err());
        assert!(require_outside_states(&state.canonicalize().unwrap(), [state]).is_err());
        assert!(
            require_outside_states(&parent.path().canonicalize().unwrap(), [canonical]).is_ok()
        );
    }

    #[cfg(unix)]
    #[test]
    fn outbound_rejects_symlink_escape_and_replaced_file_identity() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let alias = workspace.path().join("alias");
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        assert!(read_workspace_file(&alias, workspace.path()).is_err());
        let original = workspace.path().join("result.txt");
        std::fs::write(&original, b"original").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let canonical = original.canonicalize().unwrap();
        let handle = open_verified(&canonical).unwrap();
        std::fs::rename(&original, workspace.path().join("old.txt")).unwrap();
        std::fs::write(&original, b"replacement").unwrap();
        assert!(verify_file(&handle, &canonical, &root).is_err());
    }
}
