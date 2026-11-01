use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};

use super::outbound::{ATTACHMENT_TOOL, AttachmentCalls, OutboundPermit, attachment_tool_spec};
use super::weixin::WeixinClient;

pub(super) struct WeixinReplyChannel<'a> {
    pub client: &'a WeixinClient,
    pub peer: &'a str,
    pub context_token: &'a str,
    pub permit: Option<OutboundPermit>,
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
/// app-server 起不来时保留多少行 stderr 用于报错。
const STDERR_TAIL_LINES: usize = 8;
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_PENDING_TURN_MESSAGES: usize = 256;

#[derive(Debug)]
struct RpcFailure {
    code: i64,
    message: String,
}

impl std::fmt::Display for RpcFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RPC {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcFailure {}

pub struct SessionControlChannel<'a> {
    pub options_store: &'a super::controls::SessionOptionsStore,
    pub defaults: &'a super::controls::SessionOptions,
}

#[derive(Debug, Clone)]
pub struct AppServerConfig {
    pub executable: String,
    pub work_dir: PathBuf,
    pub model: String,
    pub reasoning_effort: String,
    pub sandbox: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnUsage {
    /// The active context size reported by app-server for the latest turn.
    pub context_used: Option<u64>,
    pub context_window: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppServerTurnResult {
    pub reply: String,
    pub model: String,
    pub usage: TurnUsage,
}

pub struct CodexAppServer {
    child: Child,
    stdin: Option<tokio::process::ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    running: bool,
    named_permissions: bool,
    config: AppServerConfig,
    #[cfg(test)]
    test_catalog: Option<Value>,
}

fn turn_params(
    config: &AppServerConfig,
    thread_id: &str,
    prompt: &str,
    image_paths: &[PathBuf],
) -> Value {
    let mut input = vec![json!({"type": "text", "text": prompt, "text_elements": []})];
    input.extend(
        image_paths
            .iter()
            .map(|path| json!({"type": "localImage", "path": path.to_string_lossy()})),
    );
    let mut params = json!({
        "threadId": thread_id,
        "input": input,
        "approvalPolicy": "never"
    });
    if !config.model.trim().is_empty() {
        params["model"] = Value::String(config.model.trim().to_string());
    }
    params
}

fn collect_stderr_tail(sink: &Arc<Mutex<VecDeque<String>>>) -> String {
    sink.lock()
        .map(|lines| lines.iter().cloned().collect::<Vec<_>>().join("；"))
        .unwrap_or_default()
}

/// 留空或托管路径失效时，Windows 选用桌面版维护的标准 CLI，
/// 其余平台沿用 PATH 里的 codex（macOS 桌面版内置路径本来就可执行）。
fn resolve_startup_executable(configured: &str) -> String {
    #[cfg(windows)]
    {
        return resolve_windows_startup_executable(
            configured,
            std::env::var_os("LOCALAPPDATA").as_deref().map(Path::new),
        );
    }
    #[cfg(not(windows))]
    {
        if !configured.is_empty() {
            return configured.to_string();
        }
        "codex".to_string()
    }
}

#[cfg(windows)]
fn resolve_windows_startup_executable(configured: &str, local_appdata: Option<&Path>) -> String {
    // 桌面版更新会替换哈希目录；仅恢复已失效的托管路径，保留自定义路径的校验。
    let missing_managed = !configured.is_empty()
        && !Path::new(configured).exists()
        && local_appdata.is_some_and(|local| {
            let bin = local.join("OpenAI").join("Codex").join("bin");
            let prefix = format!(
                "{}\\",
                bin.to_string_lossy()
                    .replace('/', "\\")
                    .to_ascii_lowercase()
            );
            let normalized = configured.replace('/', "\\").to_ascii_lowercase();
            normalized.strip_prefix(&prefix).is_some_and(|suffix| {
                let parts: Vec<_> = suffix.split('\\').collect();
                parts == ["codex.exe"]
                    || (parts.len() == 2
                        && !parts[0].is_empty()
                        && parts[0].bytes().all(|byte| byte.is_ascii_hexdigit())
                        && parts[1] == "codex.exe")
            })
        });
    if configured.is_empty() || missing_managed {
        if let Some(path) = crate::app_paths::find_desktop_managed_codex_cli_from_var(local_appdata)
        {
            return path.to_string_lossy().into_owned();
        }
        if configured.is_empty() {
            return "codex".to_string();
        }
    }
    configured.to_string()
}

/// 启动前先把「路径本身就不对」的情况挑出来。
///
/// 之前不管什么原因失败，用户只会看到一句「请检查 Codex CLI 路径」——填的是目录、
/// 文件不存在、没有执行权限、甚至 app-server 协议不兼容，全都是这一句，
/// 于是 #1879 下面积了十几条「到底该填什么路径」。
///
/// 裸命令名（codex）交给 PATH 解析，这里只校验看起来像路径的输入。
fn validate_codex_executable(executable: &str) -> anyhow::Result<()> {
    let looks_like_path = executable.contains('/')
        || executable.contains('\\')
        || Path::new(executable).is_absolute();
    if !looks_like_path {
        return Ok(());
    }
    if crate::app_paths::is_windows_store_cli_path(executable) {
        bail!(
            "Codex CLI 路径位于系统保护的安装目录内，Codex++ 无法直接运行它：{executable}\n\
             请清空「Codex CLI 路径」保存（留空时自动查找），\
             或点击「使用桌面版内置 CLI」重新选择。"
        );
    }
    let path = Path::new(executable);
    if !path.exists() {
        bail!(
            "Codex CLI 路径不存在：{executable}\n请填 Codex CLI 可执行文件的完整路径。\
             macOS 桌面版通常在 /Applications/ChatGPT.app/Contents/Resources/codex；\
             npm 全局安装可用 `which codex` / `where codex` 查看。"
        );
    }
    if path.is_dir() {
        bail!(
            "Codex CLI 路径指向的是目录而不是可执行文件：{executable}\n\
             请去掉结尾的路径分隔符，或补上具体的可执行文件名。"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path)
            && metadata.permissions().mode() & 0o111 == 0
        {
            bail!("Codex CLI 没有执行权限：{executable}");
        }
    }
    Ok(())
}

/// spawn 失败按错误类别给出对应提示，不再一律归成「请检查路径」。
fn spawn_failure_hint(executable: &str, kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => {
            if cfg!(windows) && !executable.contains('/') && !executable.contains('\\') {
                // CreateProcess 解析裸命令名只补 .exe，npm 生成的 codex.cmd 不在其列（#1879）
                "：找不到可运行的 Codex CLI；请点击「使用桌面版内置 CLI」自动填入，\
                 或将「Codex CLI 路径」留空自动查找"
            } else {
                "：找不到该文件；填 Codex CLI 可执行文件的完整路径，或确保 codex 在 PATH 里"
            }
        }
        std::io::ErrorKind::PermissionDenied
            if crate::app_paths::is_windows_store_cli_path(executable) =>
        {
            "：此路径位于系统保护的安装目录，无法直接运行；\
             请清空「Codex CLI 路径」自动查找，或点击「使用桌面版内置 CLI」重新选择"
        }
        std::io::ErrorKind::PermissionDenied => "：没有执行权限",
        _ => "",
    }
}

impl CodexAppServer {
    pub async fn start(mut config: AppServerConfig) -> anyhow::Result<Self> {
        config.work_dir = std::path::absolute(&config.work_dir).context("无法解析微信工作目录")?;
        let executable = resolve_startup_executable(config.executable.trim());
        validate_codex_executable(&executable)?;
        let mut command = Command::new(&executable);
        command
            .arg("app-server")
            .env_remove("CODEX_APP_TOOLS_PIPE_PATH")
            .current_dir(&config.work_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Windows 下不设 CREATE_NO_WINDOW 时，app-server 会拿到可见控制台并弹出
        // CMD/Windows Terminal 窗口，它派生的 MCP 子进程（memory 等）也会各起一个
        // conhost。仓库其它 spawn 路径都已设该标志，仅此处遗漏（issue #2048）。
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        }
        let mut child = command.spawn().map_err(|error| {
            let hint = spawn_failure_hint(&executable, error.kind());
            anyhow::anyhow!("无法启动 Codex app-server（{executable}）{hint}（{error}）")
        })?;
        let stdin = child
            .stdin
            .take()
            .context("无法连接 Codex app-server stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("无法连接 Codex app-server stdout")?;
        // 别把 stderr 读了就扔——app-server 起不来时，真正的原因只在这里。
        // 留最近几行，初始化失败时附到错误信息上。
        let stderr_tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
        if let Some(stderr) = child.stderr.take() {
            let sink = Arc::clone(&stderr_tail);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(mut sink) = sink.lock() {
                        if sink.len() == STDERR_TAIL_LINES {
                            sink.pop_front();
                        }
                        sink.push_back(line);
                    }
                }
            });
        }

        let mut server = Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
            running: true,
            named_permissions: false,
            config,
            #[cfg(test)]
            test_catalog: None,
        };
        if let Err(error) = server.initialize().await {
            let detail = collect_stderr_tail(&stderr_tail);
            return Err(if detail.is_empty() {
                error
            } else {
                error.context(format!("Codex app-server 输出：{detail}"))
            });
        }
        Ok(server)
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_session_config(&mut self, config: &AppServerConfig) {
        self.config = config.clone();
    }

    pub async fn api_request(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        self.request(method, params, Duration::from_secs(3)).await
    }

    pub async fn prepare_thread(&mut self, thread_id: Option<&str>) -> anyhow::Result<String> {
        self.prepare_thread_in_project_with_attachments(thread_id, None, false)
            .await
    }

    pub(super) async fn prepare_thread_with_attachments(
        &mut self,
        thread_id: Option<&str>,
        enabled: bool,
    ) -> anyhow::Result<String> {
        self.prepare_thread_in_project_with_attachments(thread_id, None, enabled)
            .await
    }

    pub async fn prepare_thread_in_project(
        &mut self,
        thread_id: Option<&str>,
        project_id: Option<&str>,
    ) -> anyhow::Result<String> {
        self.prepare_thread_in_project_with_attachments(thread_id, project_id, false)
            .await
    }

    pub(super) async fn prepare_thread_in_project_with_attachments(
        &mut self,
        thread_id: Option<&str>,
        project_id: Option<&str>,
        allow_attachments: bool,
    ) -> anyhow::Result<String> {
        self.negotiate_permissions().await?;
        let mut params = self.thread_params();
        let method = if let Some(thread_id) = thread_id.filter(|id| !id.trim().is_empty()) {
            params["threadId"] = Value::String(thread_id.trim().to_string());
            params.as_object_mut().unwrap().remove("dynamicTools");
            "thread/resume"
        } else {
            if allow_attachments {
                params["dynamicTools"]
                    .as_array_mut()
                    .unwrap()
                    .push(attachment_tool_spec());
            }
            if let Some(project_id) = project_id {
                params["projectId"] = json!(project_id);
            }
            "thread/start"
        };
        let result = match self.request(method, params.clone(), REQUEST_TIMEOUT).await {
            Err(error)
                if method == "thread/start"
                    && error
                        .downcast_ref::<RpcFailure>()
                        .is_some_and(|failure| failure.code == -32602) =>
            {
                params.as_object_mut().unwrap().remove("projectId");
                params.as_object_mut().unwrap().remove("dynamicTools");
                self.request(method, params, REQUEST_TIMEOUT).await?
            }
            Err(error)
                if method == "thread/resume"
                    && error.downcast_ref::<RpcFailure>().is_some_and(|failure| {
                        failure.message.contains("already has an active writer")
                    }) =>
            {
                bail!(
                    "原会话正由其他应用占用，请先在该应用中关闭会话；若仍占用则退出该应用后重新发送。\
                     原会话映射已保留，未启动新回合"
                );
            }
            result => result?,
        };
        verify_sandbox_policy(&result, &self.config.sandbox)?;
        let result_id = extract_thread_id(&result)
            .with_context(|| format!("Codex app-server {method} 未返回 thread id"))?;
        if let Some(expected) = thread_id.filter(|id| !id.trim().is_empty())
            && result_id != expected.trim()
        {
            bail!("Codex app-server 恢复了不同的线程，原会话映射已保留");
        }
        Ok(result_id)
    }

    pub async fn run_turn(
        &mut self,
        thread_id: &str,
        prompt: &str,
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_images(thread_id, prompt, &[]).await
    }

    pub async fn run_turn_with_images(
        &mut self,
        thread_id: &str,
        prompt: &str,
        image_paths: &[PathBuf],
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_reply(thread_id, prompt, image_paths, None)
            .await
    }

    pub(super) async fn run_turn_with_reply(
        &mut self,
        thread_id: &str,
        prompt: &str,
        image_paths: &[PathBuf],
        channel: Option<WeixinReplyChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_reply_and_controls(thread_id, prompt, image_paths, channel, None)
            .await
    }

    pub async fn run_turn_with_controls(
        &mut self,
        thread_id: &str,
        prompt: &str,
        channel: Option<SessionControlChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_images_and_controls(thread_id, prompt, &[], channel)
            .await
    }

    pub async fn run_turn_with_images_and_controls(
        &mut self,
        thread_id: &str,
        prompt: &str,
        image_paths: &[PathBuf],
        channel: Option<SessionControlChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_reply_and_controls(thread_id, prompt, image_paths, None, channel)
            .await
    }

    pub(super) async fn run_turn_with_reply_and_controls(
        &mut self,
        thread_id: &str,
        prompt: &str,
        image_paths: &[PathBuf],
        reply_channel: Option<WeixinReplyChannel<'_>>,
        session_channel: Option<SessionControlChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        let mut calls = AttachmentCalls::new(
            reply_channel
                .as_ref()
                .map(|channel| channel.peer)
                .unwrap_or_default(),
            thread_id,
            reply_channel
                .as_ref()
                .and_then(|channel| channel.permit.clone()),
        );
        let id = self.take_request_id();
        let mut params = turn_params(&self.config, thread_id, prompt, image_paths);
        params["cwd"] = json!(self.config.work_dir);
        if !self.config.reasoning_effort.is_empty() {
            params["effort"] = json!(self.config.reasoning_effort);
        }
        if self.named_permissions {
            params["permissions"] = json!(permission_profile(&self.config.sandbox));
        }
        self.write_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "turn/start",
            "params": params
        }))
        .await?;

        let mut response_received = false;
        let mut turn_id = String::new();
        let mut pending_messages = VecDeque::new();
        let mut session_results = HashMap::new();
        let mut turn_completed = false;
        let mut reply_parts = Vec::new();
        let mut model = self.config.model.trim().to_string();
        let mut usage = TurnUsage::default();
        let deadline = tokio::time::Instant::now() + TURN_TIMEOUT;
        while !response_received || !turn_completed {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                bail!("等待 Codex 回复超时");
            }
            let message = if response_received {
                match pending_messages.pop_front() {
                    Some(message) => message,
                    None => self.read_message(remaining).await?,
                }
            } else {
                self.read_message(remaining).await?
            };
            if response_id(&message) == Some(id) {
                if let Some(error) = rpc_error(&message) {
                    bail!("Codex turn/start 失败：{error}");
                }
                let result = message.get("result").cloned().unwrap_or(Value::Null);
                calls.turn = extract_turn_id(&result);
                turn_id = calls.turn.clone().unwrap_or_default();
                if calls.turn.is_none() {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "weixin.turn_start.missing_turn_id",
                        json!({ "request_id": id }),
                    );
                }
                if model.is_empty() {
                    model = extract_model(&result).unwrap_or_default();
                }
                if let Some(reported_usage) = extract_turn_usage(&result) {
                    usage = reported_usage;
                }
                response_received = true;
                continue;
            }
            if !response_received {
                if pending_messages.len() >= MAX_PENDING_TURN_MESSAGES {
                    bail!("Codex turn/start 响应前的消息数量超过限制");
                }
                pending_messages.push_back(message);
                continue;
            }
            if is_server_request(&message) {
                if message["method"] == "item/tool/call"
                    && !turn_id.is_empty()
                    && message_belongs_to_turn(&message, thread_id, &turn_id)
                {
                    match (
                        message["params"]["tool"].as_str(),
                        reply_channel.as_ref(),
                        session_channel.as_ref(),
                    ) {
                        (Some(ATTACHMENT_TOOL), Some(channel), _) => {
                            self.handle_attachment_tool(&message, channel, &mut calls)
                                .await?;
                        }
                        (Some("weixin_session_settings"), _, Some(channel)) => {
                            self.handle_session_tool(
                                &message,
                                thread_id,
                                channel,
                                &mut session_results,
                            )
                            .await?;
                        }
                        _ => self.reject_server_request(&message).await?,
                    }
                } else {
                    self.reject_server_request(&message).await?;
                }
                continue;
            }
            if !notification_matches_turn(&message, thread_id, calls.turn.as_deref()) {
                continue;
            }

            if model.is_empty() {
                model = extract_model(&message).unwrap_or_default();
            }
            if let Some(reported_usage) = extract_turn_usage(&message) {
                usage = reported_usage;
            }

            match message.get("method").and_then(Value::as_str) {
                Some("item/completed") => {
                    if let Some(text) = extract_completed_agent_text(&message) {
                        if !reply_parts.iter().any(|part| part == &text) {
                            reply_parts.push(text);
                        }
                    }
                }
                Some("turn/completed") => {
                    if matches!(
                        message["params"]["turn"]["status"].as_str(),
                        Some("failed" | "interrupted")
                    ) {
                        bail!("Codex 处理失败：{}", message["params"]["turn"]["error"]);
                    }
                    turn_completed = true;
                }
                Some("error") => {
                    let error = deep_string(message.get("params"), &["message", "error"])
                        .unwrap_or_else(|| "Codex app-server 返回未知错误".to_string());
                    bail!("{error}");
                }
                _ => {}
            }
        }
        Ok(AppServerTurnResult {
            reply: reply_parts.join("\n\n"),
            model,
            usage,
        })
    }

    async fn handle_attachment_tool(
        &mut self,
        message: &Value,
        channel: &WeixinReplyChannel<'_>,
        calls: &mut AttachmentCalls,
    ) -> anyhow::Result<()> {
        let result = match calls.validate(message) {
            Err(error) => attachment_result(Err(error)),
            Ok((_, _, Some(cached))) => cached,
            Ok((call, arguments, None)) => {
                let result = async {
                    calls.begin_send()?;
                    let permit = calls.permit.as_ref().context("附件回传未开启")?;
                    let kind = if arguments.kind == "image" {
                        super::media::AttachmentKind::Image
                    } else {
                        super::media::AttachmentKind::File
                    };
                    channel
                        .client
                        .send_workspace_attachment(
                            channel.peer,
                            Path::new(&arguments.path),
                            kind,
                            channel.context_token,
                            permit,
                        )
                        .await?;
                    Ok(())
                }
                .await;
                let result = attachment_result(result);
                calls.record(call, arguments, result.clone());
                result
            }
        };
        self.write_json(&json!({"jsonrpc": "2.0", "id": message["id"], "result": result}))
            .await
    }

    pub async fn run_turn_and_release(
        &mut self,
        thread_id: &str,
        prompt: &str,
        channel: Option<SessionControlChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        self.run_turn_with_images_and_release(thread_id, prompt, &[], None, channel)
            .await
    }

    pub(super) async fn run_turn_with_images_and_release(
        &mut self,
        thread_id: &str,
        prompt: &str,
        image_paths: &[PathBuf],
        reply_channel: Option<WeixinReplyChannel<'_>>,
        channel: Option<SessionControlChannel<'_>>,
    ) -> anyhow::Result<AppServerTurnResult> {
        let result = self
            .run_turn_with_reply_and_controls(
                thread_id,
                prompt,
                image_paths,
                reply_channel,
                channel,
            )
            .await;
        // unsubscribe 不保证释放跨进程写租约；仅退出本连接创建的独立 peer。
        self.close().await;
        result
    }

    pub async fn close(&mut self) {
        self.running = false;
        // ChildStdin::shutdown 在 Windows 上不关闭句柄，必须 drop 才能送达 EOF。
        drop(self.stdin.take());
        if !matches!(
            tokio::time::timeout(CLOSE_TIMEOUT, self.child.wait()).await,
            Ok(Ok(_))
        ) {
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
        }
    }

    async fn initialize(&mut self) -> anyhow::Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "codex-plus-weixin",
                    "title": "Codex++ Weixin Connect",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "experimentalApi": true,
                    "optOutNotificationMethods": [
                        "command/exec/outputDelta",
                        "item/agentMessage/delta",
                        "item/plan/delta",
                        "item/fileChange/outputDelta",
                        "item/reasoning/summaryTextDelta",
                        "item/reasoning/textDelta"
                    ]
                }
            }),
            REQUEST_TIMEOUT,
        )
        .await
        .context("初始化 Codex app-server 失败")?;
        self.write_json(&json!({
            "jsonrpc": "2.0",
            "method": "initialized"
        }))
        .await
    }

    async fn negotiate_permissions(&mut self) -> anyhow::Result<()> {
        self.named_permissions = false;
        if let Some(result) = self
            .request_optional(
                "permissionProfile/list",
                json!({"cwd":self.config.work_dir}),
                REQUEST_TIMEOUT,
            )
            .await?
        {
            let expected = permission_profile(&self.config.sandbox);
            let profiles = result["data"]
                .as_array()
                .context("未返回可确认的权限配置列表")?;
            if !profiles
                .iter()
                .any(|profile| profile["id"] == expected && profile["allowed"] == true)
            {
                bail!("Codex app-server 不允许微信连接请求的权限配置：{expected}");
            }
            self.named_permissions = true;
        }
        Ok(())
    }

    fn thread_params(&self) -> Value {
        let mut params = json!({
            "cwd": self.config.work_dir.to_string_lossy(),
            "experimentalRawEvents": false,
            "historyMode": "legacy",
            "ephemeral": false,
            "approvalPolicy": "never",
            "sandbox": normalize_sandbox(&self.config.sandbox),
            "dynamicTools": [super::controls::session_tool_spec()]
        });
        if !self.config.model.trim().is_empty() {
            params["model"] = Value::String(self.config.model.trim().to_string());
        }
        if !self.config.reasoning_effort.is_empty() {
            params["config"] = json!({"model_reasoning_effort":self.config.reasoning_effort});
        }
        if self.named_permissions {
            params.as_object_mut().unwrap().remove("sandbox");
            params["permissions"] = json!(permission_profile(&self.config.sandbox));
            params["runtimeWorkspaceRoots"] = json!([self.config.work_dir]);
        }
        params
    }

    async fn handle_session_tool(
        &mut self,
        message: &Value,
        thread_id: &str,
        channel: &SessionControlChannel<'_>,
        results: &mut HashMap<String, Value>,
    ) -> anyhow::Result<()> {
        let call_id = message["params"]["callId"]
            .as_str()
            .filter(|id| !id.is_empty());
        let valid = message["params"]["tool"] == "weixin_session_settings"
            && message["params"]["namespace"].is_null()
            && message["params"]["threadId"] == thread_id;
        if valid && let Some(cached) = call_id.and_then(|id| results.get(id)) {
            return self
                .write_json(&json!({"jsonrpc":"2.0","id":message["id"],"result":cached}))
                .await;
        }
        let result = async {
            if !valid || call_id.is_none() {
                bail!("设置工具不属于当前微信会话");
            }
            let args = message["params"]["arguments"].as_object().context("设置参数必须是对象")?;
            if args.keys().any(|key| !["action","model","reasoningEffort"].contains(&key.as_str())) {
                bail!("不支持的设置参数");
            }
            #[cfg(test)]
            let catalog = match &self.test_catalog {
                Some(catalog) => catalog.clone(),
                None => crate::model_catalog::read_codex_model_catalog().await,
            };
            #[cfg(not(test))]
            let catalog = crate::model_catalog::read_codex_model_catalog().await;
            let mut next = channel.options_store.load()?;
            match args.get("action").and_then(Value::as_str) {
                Some("set") => {
                    next = super::controls::updated_options(&catalog, channel.defaults, &next, args)?;
                    channel.options_store.save(next.clone())?;
                }
                Some("get") => {}
                Some("reset") => {
                    channel.options_store.request_reset()?;
                    next = channel.options_store.load()?;
                }
                _ => bail!("未知设置操作"),
            }
            Ok::<_, anyhow::Error>(json!({
                "currentTurn":{"model":self.config.model,"reasoningEffort":self.config.reasoning_effort},
                "savedOverrides":next,"appliesFrom":"next-message",
                "connectionDefaults":channel.defaults,
                "models":catalog["models"],"modelMetadata":catalog["modelMetadata"]
            }))
        }.await;
        let result = match result {
            Ok(value) => {
                json!({"success":true,"contentItems":[{"type":"inputText","text":value.to_string()}]})
            }
            Err(error) => {
                json!({"success":false,"contentItems":[{"type":"inputText","text":error.to_string()}]})
            }
        };
        if valid && let Some(id) = call_id {
            results.insert(id.to_string(), result.clone());
        }
        self.write_json(&json!({"jsonrpc":"2.0","id":message["id"],"result":result}))
            .await
    }

    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Value> {
        self.request_optional(method, params, timeout)
            .await?
            .with_context(|| format!("Codex app-server 不支持 {method}"))
    }

    async fn request_optional(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> anyhow::Result<Option<Value>> {
        let id = self.take_request_id();
        self.write_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                bail!("Codex app-server {method} 请求超时");
            }
            let message = self.read_message(remaining).await?;
            if is_server_request(&message) {
                self.reject_server_request(&message).await?;
                continue;
            }
            if response_id(&message) != Some(id) {
                continue;
            }
            let code = message["error"]["code"].as_i64();
            let unknown_method = code == Some(-32600)
                && message["error"]["message"].as_str().is_some_and(|error| {
                    let error = error.strip_prefix("Invalid request: ").unwrap_or(error);
                    ["unknown variant", "unknown method"].iter().any(|kind| {
                        ['`', '\'', '"'].iter().any(|quote| {
                            error.starts_with(&format!("{kind} {quote}{method}{quote}"))
                        })
                    })
                });
            if code == Some(-32601) || unknown_method {
                return Ok(None);
            }
            if let Some(error) = rpc_error(&message) {
                return Err(RpcFailure {
                    code: message["error"]["code"].as_i64().unwrap_or_default(),
                    message: format!("Codex app-server {method} 失败：{error}"),
                }
                .into());
            }
            return Ok(Some(message.get("result").cloned().unwrap_or(Value::Null)));
        }
    }

    async fn read_message(&mut self, timeout: Duration) -> anyhow::Result<Value> {
        loop {
            let line = tokio::time::timeout(timeout, self.stdout.next_line())
                .await
                .context("等待 Codex app-server 响应超时")??;
            let Some(line) = line else {
                self.running = false;
                let exit = self.child.try_wait().ok().flatten();
                bail!(
                    "Codex app-server 已关闭{}",
                    exit.map(|status| format!("：{status}")).unwrap_or_default()
                );
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(message) => return Ok(message),
                Err(_) => continue,
            }
        }
    }

    async fn reject_server_request(&mut self, message: &Value) -> anyhow::Result<()> {
        let Some(id) = message.get("id") else {
            return Ok(());
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                json!({ "decision": "decline" })
            }
            "item/permissions/requestApproval" => json!({ "permissions": {} }),
            "item/tool/requestUserInput" => json!({ "answers": {} }),
            "item/tool/call" => json!({
                "success": false,
                "contentItems": [{
                    "type": "inputText",
                    "text": "tool not available on this client"
                }]
            }),
            _ => {
                return self
                    .write_json(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "method not found" }
                    }))
                    .await;
            }
        };
        self.write_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        }))
        .await
    }

    async fn write_json(&mut self, value: &Value) -> anyhow::Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        let stdin = self
            .stdin
            .as_mut()
            .context("Codex app-server stdin 已关闭")?;
        stdin
            .write_all(&bytes)
            .await
            .context("写入 Codex app-server 失败")?;
        stdin.flush().await?;
        Ok(())
    }

    fn take_request_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }
}

fn attachment_result(result: anyhow::Result<()>) -> Value {
    let (success, text) = match result {
        Ok(()) => (true, "附件已回传给当前微信联系人。".to_string()),
        Err(error) => (false, format!("附件未发送：{error}")),
    };
    json!({"success": success, "contentItems": [{"type": "inputText", "text": text}]})
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        if self.running {
            let _ = self.child.start_kill();
        }
    }
}

fn normalize_sandbox(value: &str) -> &'static str {
    match value.trim() {
        "workspace-write" => "workspace-write",
        "danger-full-access" => "danger-full-access",
        _ => "read-only",
    }
}

fn permission_profile(sandbox: &str) -> &'static str {
    match normalize_sandbox(sandbox) {
        "workspace-write" => ":workspace",
        "danger-full-access" => ":danger-full-access",
        _ => ":read-only",
    }
}

fn verify_sandbox_policy(result: &Value, expected: &str) -> anyhow::Result<()> {
    let actual = match result["sandbox"]["type"].as_str() {
        Some("readOnly" | "read-only") => "read-only",
        Some("workspaceWrite" | "workspace-write") => "workspace-write",
        Some("dangerFullAccess" | "danger-full-access") => "danger-full-access",
        _ => bail!("Codex app-server 未确认微信会话权限"),
    };
    if actual != normalize_sandbox(expected) {
        bail!("Codex app-server 返回的权限与微信连接设置不符");
    }
    Ok(())
}

fn message_belongs_to_turn(message: &Value, thread_id: &str, turn_id: &str) -> bool {
    let params = &message["params"];
    params["threadId"].as_str() == Some(thread_id)
        && params["turnId"]
            .as_str()
            .or_else(|| params["turn"]["id"].as_str())
            == Some(turn_id)
}

fn notification_matches_turn(message: &Value, thread_id: &str, turn_id: Option<&str>) -> bool {
    match turn_id {
        Some(turn_id) => message_belongs_to_turn(message, thread_id, turn_id),
        None => message["params"]["threadId"].as_str() == Some(thread_id),
    }
}

fn response_id(message: &Value) -> Option<u64> {
    message.get("id").and_then(Value::as_u64)
}

fn rpc_error(message: &Value) -> Option<String> {
    let error = message.get("error")?;
    deep_string(Some(error), &["message", "error"]).or_else(|| Some(error.to_string()))
}

fn is_server_request(message: &Value) -> bool {
    message.get("id").is_some() && message.get("method").and_then(Value::as_str).is_some()
}

fn extract_thread_id(result: &Value) -> Option<String> {
    deep_string(Some(result), &["threadId", "id"]).or_else(|| {
        result
            .get("thread")
            .and_then(|thread| deep_string(Some(thread), &["id", "threadId"]))
    })
}

fn extract_turn_id(result: &Value) -> Option<String> {
    deep_string(Some(result), &["turnId", "id"]).or_else(|| {
        result
            .get("turn")
            .and_then(|turn| deep_string(Some(turn), &["id", "turnId"]))
    })
}

fn extract_model(value: &Value) -> Option<String> {
    deep_string(
        Some(value),
        &["model", "modelSlug", "model_slug", "modelId", "model_id"],
    )
    .map(|model| model.trim().to_string())
    .filter(|model| !model.is_empty())
}

fn extract_turn_usage(value: &Value) -> Option<TurnUsage> {
    if let Some(usage) = value.get("tokenUsage").or_else(|| value.get("token_usage")) {
        if let Some(parsed) = parse_turn_usage(usage) {
            return Some(parsed);
        }
    }
    if value.get("method").and_then(Value::as_str) == Some("thread/tokenUsage/updated") {
        return value.get("params").and_then(extract_turn_usage);
    }
    value
        .as_object()
        .and_then(|object| object.values().find_map(extract_turn_usage))
}

fn parse_turn_usage(value: &Value) -> Option<TurnUsage> {
    let object = value.as_object()?;
    let window = object
        .get("modelContextWindow")
        .or_else(|| object.get("model_context_window"))
        .and_then(value_as_u64);
    let last = object
        .get("last")
        .or_else(|| object.get("last_token_usage"));
    let context_used = last
        .and_then(|last| {
            last.get("totalTokens")
                .or_else(|| last.get("total_tokens"))
                .and_then(value_as_u64)
        })
        .or_else(|| {
            object
                .get("contextUsed")
                .or_else(|| object.get("context_used"))
                .and_then(value_as_u64)
        });
    if context_used.is_none() && window.is_none() {
        return None;
    }
    Some(TurnUsage {
        context_used,
        context_window: window,
    })
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
}

fn extract_completed_agent_text(message: &Value) -> Option<String> {
    let item = message.get("params")?.get("item")?;
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
    if !matches!(
        item_type,
        "agentMessage" | "assistantMessage" | "output_text"
    ) {
        return None;
    }
    deep_string(Some(item), &["text", "content", "output_text"])
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn deep_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let value = value?;
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    if let Some(items) = value.as_array() {
        let parts = items
            .iter()
            .filter_map(|item| deep_string(Some(item), keys))
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Some(parts.join("\n"));
        }
    }
    let object = value.as_object()?;
    for key in keys {
        if let Some(text) = object
            .get(*key)
            .and_then(|value| deep_string(Some(value), keys))
        {
            return Some(text);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_params_preserve_text_and_encode_local_images_without_tools() {
        let config = AppServerConfig {
            executable: "unused".into(),
            work_dir: PathBuf::from("workspace"),
            model: " existing-model ".into(),
            reasoning_effort: String::new(),
            sandbox: "read-only".into(),
        };
        let images = [
            PathBuf::from("workspace/image one.png"),
            PathBuf::from("workspace/two.jpg"),
        ];
        let params = turn_params(&config, "saved-thread", "caption and file path", &images);
        assert_eq!(
            params,
            json!({
                "threadId": "saved-thread",
                "input": [
                    {"type":"text", "text":"caption and file path", "text_elements":[]},
                    {"type":"localImage", "path":images[0]},
                    {"type":"localImage", "path":images[1]}
                ],
                "approvalPolicy":"never",
                "model":"existing-model"
            })
        );
        let mut default_config = config;
        default_config.model.clear();
        let text_only = turn_params(&default_config, "saved-thread", "original text", &[]);
        assert_eq!(
            text_only,
            json!({
                "threadId":"saved-thread",
                "input":[{"type":"text", "text":"original text", "text_elements":[]}],
                "approvalPolicy":"never"
            })
        );
    }

    #[test]
    fn outbound_stdio_fixture() {
        use std::io::BufRead;
        let Ok(mode) = std::env::var("CODEX_PLUS_W2_STDIO_FIXTURE_MODE") else {
            return;
        };
        let missing_turn = mode == "missing-turn" || mode == "missing-turn-id";
        let enabled = mode == "enabled" || mode == "combined" || missing_turn;
        let with_controls = mode == "combined" || mode == "missing-turn";
        let emit = |value: Value| {
            println!("{value}");
            std::io::Write::flush(&mut std::io::stdout()).unwrap();
        };
        let tool = |id: &str, turn: &str| {
            json!({
                "id": id, "method": "item/tool/call", "params": {
                    "threadId": "thread", "turnId": turn, "callId": "call",
                    "tool": super::super::outbound::ATTACHMENT_TOOL, "namespace": null,
                    "arguments": {"path": "result.txt", "kind": "file"}
                }
            })
        };
        for line in std::io::stdin().lock().lines() {
            let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
            match message["method"].as_str() {
                Some("initialize") => emit(json!({"id": message["id"], "result": {}})),
                Some("initialized") => {}
                Some("permissionProfile/list") => emit(json!({
                    "id": message["id"],
                    "error": {"code": -32601, "message": "unsupported"}
                })),
                Some("thread/start") => {
                    let tools = message["params"]["dynamicTools"].as_array().unwrap();
                    assert_eq!(tools.len(), if enabled { 2 } else { 1 });
                    assert!(
                        tools
                            .iter()
                            .any(|tool| tool["name"] == "weixin_session_settings")
                    );
                    assert_eq!(
                        tools.iter().any(|tool| tool["name"] == ATTACHMENT_TOOL),
                        enabled
                    );
                    if with_controls {
                        assert_eq!(message["params"]["projectId"], "project-1");
                    }
                    emit(json!({"id": message["id"], "result": {
                        "thread": {"id": "thread"}, "sandbox": {"type": "readOnly"}
                    }}));
                }
                Some("thread/resume") => {
                    assert!(message["params"].get("dynamicTools").is_none());
                    emit(json!({"id": message["id"], "result": {
                        "thread": {"id": "thread"}, "sandbox": {"type": "readOnly"}
                    }}));
                }
                Some("turn/start") => {
                    if with_controls {
                        assert_eq!(message["params"]["model"], "fixture");
                        assert_eq!(message["params"]["effort"], "high");
                        emit(
                            json!({"id": "settings", "method": "item/tool/call", "params": {
                                "threadId": "thread", "turnId": "turn", "callId": "call",
                                "tool": "weixin_session_settings", "namespace": null,
                                "arguments": {"action": "set", "reasoningEffort": "low"}
                            }}),
                        );
                    }
                    emit(
                        json!({"id": message["id"], "result": if missing_turn { json!({}) } else { json!({"turn": {"id": "turn"}}) }}),
                    );
                    if !missing_turn {
                        emit(
                            json!({"method": "turn/completed", "params": {"threadId": "thread", "turn": {"id": "old-turn", "status": "completed"}}}),
                        );
                    }
                    emit(tool("stale", "old-turn"));
                }
                None => match message["id"].as_str().unwrap() {
                    "settings" => {
                        assert!(with_controls);
                        assert_eq!(message["result"]["success"], !missing_turn);
                        if missing_turn {
                            continue;
                        }
                        let result: Value = serde_json::from_str(
                            message["result"]["contentItems"][0]["text"]
                                .as_str()
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(result["currentTurn"]["reasoningEffort"], "high");
                        assert_eq!(result["savedOverrides"]["reasoningEffort"], "low");
                    }
                    "stale" => {
                        assert_eq!(message["result"]["success"], false);
                        emit(tool("first", "turn"));
                    }
                    "first" => {
                        assert_eq!(message["result"]["success"], enabled && !missing_turn);
                        emit(tool("repeat", "turn"));
                    }
                    "repeat" => {
                        assert_eq!(message["result"]["success"], enabled && !missing_turn);
                        emit(
                            json!({"method": "item/completed", "params": {"threadId": "thread", "turnId": "turn", "item": {"type": "agentMessage", "text": "done"}}}),
                        );
                        emit(
                            json!({"method": "turn/completed", "params": {"threadId": "thread", "turn": {"id": "turn", "status": "completed"}}}),
                        );
                    }
                    other => panic!("unexpected tool result: {other}"),
                },
                other => panic!("unexpected request: {other:?}"),
            }
        }
    }

    fn fixture_server(root: &Path, mode: &str) -> CodexAppServer {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "connect::app_server::tests::session_stdio_fixture",
                "--nocapture",
            ])
            .env("CODEX_PLUS_SESSION_FIXTURE", mode)
            .env("CODEX_PLUS_SESSION_FIXTURE_ROOT", root)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
        let mut child = command.spawn().unwrap();
        CodexAppServer {
            stdin: child.stdin.take(),
            stdout: BufReader::new(child.stdout.take().unwrap()).lines(),
            child,
            next_id: 1,
            running: true,
            named_permissions: false,
            config: AppServerConfig {
                executable: String::new(),
                work_dir: root.to_owned(),
                model: "fixture".into(),
                reasoning_effort: "high".into(),
                sandbox: "read-only".into(),
            },
            test_catalog: Some(json!({"models":["fixture"],"modelMetadata":{
                "fixture":{"supportedReasoningEfforts":["low","high"]}
            }})),
        }
    }

    #[test]
    fn session_stdio_fixture() {
        use std::io::{BufRead, Write};
        let Ok(mode) = std::env::var("CODEX_PLUS_SESSION_FIXTURE") else {
            return;
        };
        let emit = |value: Value| {
            println!("{value}");
            std::io::stdout().flush().unwrap();
        };
        let tool = |id: &str, turn: &str, namespace: Option<&str>, args: Value| {
            emit(json!({"id":id,"method":"item/tool/call","params":{
                "threadId":"original","turnId":turn,"callId":id,
                "tool":"weixin_session_settings","namespace":namespace,"arguments":args
            }}));
        };
        let complete = |turn: &str| {
            emit(json!({"method":"item/completed","params":{
                "threadId":"other","turnId":turn,"item":{"type":"agentMessage","text":"FOREIGN"}
            }}));
            emit(json!({"method":"item/completed","params":{
                "threadId":"original","turnId":turn,"item":{"type":"agentMessage","text":"checked"}
            }}));
            emit(json!({"method":"turn/completed","params":{
                "threadId":"original","turn":{"id":turn,"status":"completed"}
            }}));
        };
        let mut turns = 0;
        let mut seen_set = false;
        for line in std::io::stdin().lock().lines() {
            let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let id = message["id"].clone();
            let params = &message["params"];
            match message["method"].as_str() {
                Some("initialize") => emit(json!({"id":id,"result":{}})),
                Some("initialized") => {}
                Some("permissionProfile/list") => {
                    if let Some(case) = mode.strip_prefix("optional-permissions-") {
                        let error = match case {
                            "unknown-variant" => "Invalid request: unknown variant `permissionProfile/list`, expected one of `initialize`, `thread/start`, `thread/resume`, `turn/start`",
                            "unknown-method" => "Invalid request: unknown method `permissionProfile/list`",
                            "unknown-method-unprefixed" => "unknown method \"permissionProfile/list\"",
                            "invalid-request" => "Invalid request: missing field `cwd`",
                            "invalid-variant" => "Invalid request: unknown variant `invalid`, expected `read-only` or `workspace-write`",
                            "invalid-method" => "Invalid request: unknown method `another/method`",
                            "invalid-nested-message" => "Invalid request: invalid cwd: unknown method `permissionProfile/list`",
                            "invalid-active-writer" => "thread original already has an active writer",
                            _ => panic!("Unexpected permission fixture: {case}"),
                        };
                        emit(json!({"id":id,"error":{"code":-32600,"message":error}}));
                    } else if mode == "legacy" || mode.starts_with("optional-") {
                        emit(json!({"id":id,"error":{"code":-32601,"message":"unsupported"}}));
                    } else {
                        emit(json!({"id":id,"result":{"data":[
                            {"id":":read-only","allowed":true},
                            {"id":":workspace","allowed":true}
                        ]}}));
                    }
                }
                Some("thread/start") => {
                    assert_ne!(mode, "resume-failed", "恢复失败不能新建线程");
                    if mode == "optional-reject-start" && params.get("projectId").is_some() {
                        emit(
                            json!({"id":id,"error":{"code":-32602,"message":"projectId unsupported"}}),
                        );
                        continue;
                    }
                    if mode.starts_with("optional-") {
                        assert!(params.get("projectId").is_none());
                    } else {
                        assert_eq!(params["projectId"], "project-1");
                    }
                    if mode != "optional-reject-start" {
                        assert_eq!(params["dynamicTools"].as_array().unwrap().len(), 1);
                        assert_eq!(params["dynamicTools"][0]["name"], "weixin_session_settings");
                    }
                    assert_eq!(params["config"]["model_reasoning_effort"], "high");
                    emit(json!({"id":id,"result":{"thread":{"id":"original"},
                        "sandbox":{"type":"readOnly"}}}));
                }
                Some("thread/resume") => {
                    if mode == "release-busy" {
                        emit(json!({"id":id,"error":{"code":-32600,
                            "message":"thread original already has an active writer"}}));
                        continue;
                    }
                    if mode == "resume-failed" {
                        emit(
                            json!({"id":id,"error":{"code":-32000,"message":"thread unavailable"}}),
                        );
                        continue;
                    }
                    assert_eq!(params["threadId"], "original");
                    assert!(params.get("dynamicTools").is_none());
                    if mode != "legacy" && !mode.starts_with("optional-") {
                        assert_eq!(params["runtimeWorkspaceRoots"].as_array().unwrap().len(), 1);
                    }
                    let result_id = if mode == "wrong-id" {
                        "replacement"
                    } else {
                        "original"
                    };
                    let sandbox = if mode == "wrong-scope" {
                        "dangerFullAccess"
                    } else if turns > 0 {
                        "workspaceWrite"
                    } else {
                        "readOnly"
                    };
                    emit(
                        json!({"id":id,"result":{"thread":{"id":result_id},"sandbox":{"type":sandbox}}}),
                    );
                }
                Some("turn/start") => {
                    assert_ne!(mode, "release-busy", "占用中的原线程不能开始回合");
                    if mode == "release-images" {
                        assert_eq!(params["input"][1]["type"], "localImage");
                        assert!(
                            params["input"][1]["path"]
                                .as_str()
                                .unwrap()
                                .ends_with("fixture.png")
                        );
                    }
                    turns += 1;
                    let turn = format!("t{turns}");
                    assert_eq!(params["model"], "fixture");
                    assert_eq!(params["effort"], if turns == 1 { "high" } else { "low" });
                    if mode != "legacy" && !mode.starts_with("optional-") {
                        assert_eq!(
                            params["permissions"],
                            if turns == 1 {
                                ":read-only"
                            } else {
                                ":workspace"
                            }
                        );
                    }
                    // 工具可能先于 turn/start 响应抵达，必须等待身份确认再执行。
                    if turns == 1 && mode == "settings" {
                        tool(
                            "set",
                            &turn,
                            None,
                            json!({"action":"set","reasoningEffort":"low"}),
                        );
                    }
                    emit(json!({"id":id,"result":{"turn":{"id":turn}}}));
                    if mode.starts_with("release-") {
                        let root = std::env::var_os("CODEX_PLUS_SESSION_FIXTURE_ROOT").unwrap();
                        std::fs::write(Path::new(&root).join("active"), "active").unwrap();
                        std::thread::sleep(Duration::from_millis(200));
                        if mode == "release-failed" {
                            emit(json!({"method":"turn/completed","params":{
                                "threadId":"original","turn":{"id":turn,"status":"failed",
                                "error":{"message":"fixture failure"}}
                            }}));
                            continue;
                        }
                    }
                    if turns > 1 || mode != "settings" {
                        complete(&turn);
                    }
                }
                Some(
                    "project/read"
                    | "project/list"
                    | "thread/metadata/update"
                    | "config/read"
                    | "model/list",
                ) if mode == "optional-unavailable" => {
                    emit(json!({"id":id,"error":{"code":-32601,"message":"unsupported"}}));
                }
                None => match id.as_str() {
                    Some("set") => {
                        assert_eq!(message["result"]["success"], true);
                        let result: Value = serde_json::from_str(
                            message["result"]["contentItems"][0]["text"]
                                .as_str()
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(result["currentTurn"]["reasoningEffort"], "high");
                        assert_eq!(result["savedOverrides"]["reasoningEffort"], "low");
                        if !seen_set {
                            seen_set = true;
                            tool(
                                "foreign",
                                "old-turn",
                                None,
                                json!({"action":"set","reasoningEffort":"high"}),
                            );
                        }
                    }
                    Some("foreign") => {
                        assert_eq!(message["result"]["success"], false);
                        tool(
                            "namespace",
                            "t1",
                            Some("other"),
                            json!({"action":"set","reasoningEffort":"high"}),
                        );
                    }
                    Some("namespace") => {
                        assert_eq!(message["result"]["success"], false);
                        tool(
                            "invalid",
                            "t1",
                            None,
                            json!({"action":"set","model":"unknown"}),
                        );
                    }
                    Some("invalid") => {
                        assert_eq!(message["result"]["success"], false);
                        // 同 callId 的重放必须返回首个结果，不能再次写入。
                        tool(
                            "set",
                            "t1",
                            None,
                            json!({"action":"set","reasoningEffort":"high"}),
                        );
                        tool("get", "t1", None, json!({"action":"get"}));
                    }
                    Some("get") => {
                        assert_eq!(message["result"]["success"], true);
                        complete("t1");
                    }
                    _ => panic!("Unexpected response: {message}"),
                },
                _ => panic!("Unexpected request: {message}"),
            }
        }
        if mode.starts_with("release-") {
            let root = std::env::var_os("CODEX_PLUS_SESSION_FIXTURE_ROOT").unwrap();
            std::fs::write(Path::new(&root).join("exited"), "stdin EOF").unwrap();
        }
    }

    #[tokio::test]
    async fn owned_peer_exits_after_completed_or_failed_turn_without_unsubscribe_rpc() {
        for mode in ["release-current", "release-failed"] {
            let root = tempfile::tempdir().unwrap();
            let mut server = fixture_server(root.path(), mode);
            server.initialize().await.unwrap();
            let thread = server.prepare_thread(Some("original")).await.unwrap();
            let monitor = async {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                while !root.path().join("active").exists() {
                    assert!(tokio::time::Instant::now() < deadline);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                assert!(!root.path().join("exited").exists());
            };
            let (result, ()) = tokio::join!(
                server.run_turn_and_release(&thread, "local fixture only", None),
                monitor
            );
            assert_eq!(result.is_err(), mode == "release-failed");
            assert!(!server.is_running());
            assert!(server.child.try_wait().unwrap().unwrap().success());
            assert_eq!(
                std::fs::read_to_string(root.path().join("exited")).unwrap(),
                "stdin EOF"
            );
        }
    }

    #[tokio::test]
    async fn legacy_peer_releases_without_requiring_new_rpcs() {
        let root = tempfile::tempdir().unwrap();
        let mut server = fixture_server(root.path(), "legacy");
        server.initialize().await.unwrap();
        let thread = server.prepare_thread(Some("original")).await.unwrap();
        assert_eq!(
            server
                .run_turn_and_release(&thread, "legacy", None)
                .await
                .unwrap()
                .reply,
            "checked"
        );
        assert!(!server.is_running());
        assert!(server.child.try_wait().unwrap().unwrap().success());
    }

    #[tokio::test]
    async fn images_and_session_controls_share_the_releasing_turn() {
        use super::super::controls::{SessionOptions, SessionOptionsStore};
        let root = tempfile::tempdir().unwrap();
        let store = SessionOptionsStore::test_store(root.path().join("settings.json"), "peer");
        let defaults = SessionOptions::default();
        let mut server = fixture_server(root.path(), "release-images");
        server.initialize().await.unwrap();
        let thread = server.prepare_thread(Some("original")).await.unwrap();
        let result = server
            .run_turn_with_images_and_release(
                &thread,
                "caption",
                &[root.path().join("fixture.png")],
                None,
                Some(SessionControlChannel {
                    options_store: &store,
                    defaults: &defaults,
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.reply, "checked");
        assert!(server.child.try_wait().unwrap().unwrap().success());
    }

    #[tokio::test]
    async fn next_owned_peer_resumes_the_original_thread() {
        let root = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let mut server = fixture_server(root.path(), "release-current");
            server.initialize().await.unwrap();
            let thread = server.prepare_thread(Some("original")).await.unwrap();
            assert_eq!(thread, "original");
            assert_eq!(
                server
                    .run_turn_and_release(&thread, "next message", None)
                    .await
                    .unwrap()
                    .reply,
                "checked"
            );
            assert!(server.child.try_wait().unwrap().unwrap().success());
        }
    }

    #[tokio::test]
    async fn external_writer_is_not_reset_replaced_or_started() {
        let root = tempfile::tempdir().unwrap();
        let mut server = fixture_server(root.path(), "release-busy");
        server.initialize().await.unwrap();
        let error = server.prepare_thread(Some("original")).await.unwrap_err();
        assert!(error.to_string().contains("其他应用占用"));
        assert!(error.to_string().contains("原会话映射已保留，未启动新回合"));
        assert!(!root.path().join("active").exists());
        server.close().await;
        assert!(server.child.try_wait().unwrap().unwrap().success());
    }

    #[tokio::test]
    async fn outbound_stdio_registers_only_when_enabled_and_sends_each_call_once() {
        use super::super::controls::{SessionOptions, SessionOptionsStore};
        use super::super::outbound::OutboundPolicy;
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("result.txt"), b"result").unwrap();
        let missing_turn_id = std::env::var_os("CODEX_PLUS_W2_MISSING_TURN_ID").is_some();
        for (mode, enabled, with_controls) in [
            ("disabled", false, false),
            ("enabled", true, false),
            ("combined", true, true),
            ("missing-turn", true, true),
            ("missing-turn-id", true, false),
        ] {
            if missing_turn_id && mode != "missing-turn-id" {
                continue;
            }
            let api = MockServer::start().await;
            let cdn = MockServer::start().await;
            let count = if enabled && !mode.starts_with("missing-turn") { 1 } else { 0 };
            Mock::given(method("POST"))
                .and(path("/ilink/bot/getuploadurl"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"upload_param": "upload"})),
                )
                .expect(count)
                .mount(&api)
                .await;
            Mock::given(method("POST"))
                .and(path("/upload"))
                .respond_with(
                    ResponseTemplate::new(200).insert_header("x-encrypted-param", "download"),
                )
                .expect(count)
                .mount(&cdn)
                .await;
            Mock::given(method("POST"))
                .and(path("/ilink/bot/sendmessage"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret": 0})))
                .expect(count)
                .mount(&api)
                .await;
            let client = WeixinClient::new(&api.uri(), "fixture", "")
                .unwrap()
                .with_media_base_url(&cdn.uri())
                .unwrap();
            let policy = OutboundPolicy::new(enabled, "peer", workspace.path().to_str().unwrap());
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "connect::app_server::tests::outbound_stdio_fixture",
                    "--nocapture",
                ])
                .env("CODEX_PLUS_W2_STDIO_FIXTURE_MODE", mode)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(crate::windows_create_no_window());
            let mut child = command.spawn().unwrap();
            let stdin = child.stdin.take().unwrap();
            let stdout = child.stdout.take().unwrap();
            let mut server = CodexAppServer {
                child,
                stdin: Some(stdin),
                stdout: BufReader::new(stdout).lines(),
                next_id: 1,
                running: true,
                named_permissions: false,
                config: AppServerConfig {
                    executable: String::new(),
                    work_dir: workspace.path().to_path_buf(),
                    model: if with_controls {
                        "fixture".into()
                    } else {
                        String::new()
                    },
                    reasoning_effort: if with_controls {
                        "high".into()
                    } else {
                        String::new()
                    },
                    sandbox: "read-only".into(),
                },
                test_catalog: Some(json!({"models":["fixture"],"modelMetadata":{
                    "fixture":{"supportedReasoningEfforts":["low","high"]}
                }})),
            };
            let options_store = SessionOptionsStore::test_store(
                workspace.path().join(format!("settings-{mode}.json")),
                "peer",
            );
            let defaults = SessionOptions {
                model: "fixture".into(),
                reasoning_effort: "high".into(),
                reset_request: None,
            };
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                server.initialize().await?;
                let thread = if with_controls {
                    server
                        .prepare_thread_in_project_with_attachments(
                            None,
                            Some("project-1"),
                            enabled,
                        )
                        .await?
                } else {
                    server
                        .prepare_thread_with_attachments(None, enabled)
                        .await?
                };
                server
                    .prepare_thread_with_attachments(Some(&thread), enabled)
                    .await?;
                let reply_channel = Some(WeixinReplyChannel {
                    client: &client,
                    peer: "peer",
                    context_token: "private context token",
                    permit: policy.permit("peer").ok(),
                });
                if with_controls {
                    server
                        .run_turn_with_reply_and_controls(
                            &thread,
                            "send private prompt",
                            &[],
                            reply_channel,
                            Some(SessionControlChannel {
                                options_store: &options_store,
                                defaults: &defaults,
                            }),
                        )
                        .await
                } else {
                    server
                        .run_turn_with_reply(&thread, "send private prompt", &[], reply_channel)
                        .await
                }
            })
            .await;
            server.close().await;
            assert_eq!(result.unwrap().unwrap().reply, "done");
            if with_controls {
                assert_eq!(
                    options_store.load().unwrap().reasoning_effort,
                    if mode == "missing-turn" { "" } else { "low" }
                );
                assert_eq!(server.config.reasoning_effort, "high");
            }
        }
    }

    #[tokio::test]
    async fn optional_permissions_unknown_methods_fall_back_to_legacy_sandbox() {
        let root = tempfile::tempdir().unwrap();
        for mode in [
            "optional-unavailable",
            "optional-permissions-unknown-variant",
            "optional-permissions-unknown-method",
            "optional-permissions-unknown-method-unprefixed",
        ] {
            let mut server = fixture_server(root.path(), mode);
            server.initialize().await.unwrap();
            server.named_permissions = true;
            let thread = server.prepare_thread(None).await.unwrap();
            assert_eq!(thread, "original");
            assert!(!server.named_permissions, "{mode}");
            let params = server.thread_params();
            assert_eq!(params["sandbox"], "read-only");
            assert!(params.get("permissions").is_none());
            assert!(params.get("runtimeWorkspaceRoots").is_none());
            assert_eq!(server.prepare_thread(Some(&thread)).await.unwrap(), thread);
            assert_eq!(
                server.run_turn(&thread, "basic text").await.unwrap().reply,
                "checked"
            );
            server.close().await;
            assert!(server.child.try_wait().unwrap().unwrap().success());
        }
    }

    #[tokio::test]
    async fn optional_permissions_invalid_requests_are_not_downgraded() {
        let root = tempfile::tempdir().unwrap();
        for (mode, expected) in [
            ("optional-permissions-invalid-request", "missing field `cwd`"),
            (
                "optional-permissions-invalid-variant",
                "unknown variant `invalid`",
            ),
            (
                "optional-permissions-invalid-method",
                "unknown method `another/method`",
            ),
            (
                "optional-permissions-invalid-nested-message",
                "invalid cwd: unknown method `permissionProfile/list`",
            ),
            (
                "optional-permissions-invalid-active-writer",
                "already has an active writer",
            ),
        ] {
            let mut server = fixture_server(root.path(), mode);
            server.initialize().await.unwrap();
            let error = server.prepare_thread(None).await.unwrap_err();
            let failure = error.downcast_ref::<RpcFailure>().unwrap();
            assert_eq!(failure.code, -32600, "{mode}");
            assert!(failure.message.contains(expected), "{mode}: {error}");
            server.close().await;
            assert!(server.child.try_wait().unwrap().unwrap().success());
        }
    }

    #[tokio::test]
    async fn selected_project_read_failure_preserves_contact_mapping() {
        use std::sync::atomic::AtomicBool;

        use super::super::{
            outbound::OutboundPolicy,
            session_store::{ConnectSessionStore, ConnectState},
            weixin::{WeixinClient, WeixinMessage},
        };

        let root = tempfile::tempdir().unwrap();
        let account = format!(
            "project-read-{}",
            root.path().file_name().unwrap().to_string_lossy()
        );
        let store = ConnectSessionStore::default_for_account(&account);
        let mut state = ConnectState::default();
        state
            .thread_ids
            .insert("other".into(), "other-history".into());
        store.save(&state).unwrap();
        let original = state.clone();
        let api = wiremock::MockServer::start().await;
        let client = WeixinClient::new(&api.uri(), "fixture", "").unwrap();
        let message: WeixinMessage = serde_json::from_value(json!({
            "from_user_id":"peer", "context_token":"context"
        }))
        .unwrap();
        let policy = OutboundPolicy::new(false, "peer", root.path().to_str().unwrap());
        let mut server = fixture_server(root.path(), "optional-unavailable");
        server.initialize().await.unwrap();
        let config = server.config.clone();
        let mut server = Some(server);
        let result = super::super::process_weixin_message(
            &client,
            &config,
            &mut server,
            &mut state,
            &message,
            "new project message",
            &AtomicBool::new(false),
            &policy,
            &account,
            "selected",
            true,
        )
        .await;
        server.as_mut().unwrap().close().await;

        let error = result.unwrap_err();
        assert!(error.to_string().contains("读取所选工作项目失败"), "{error:#}");
        assert_eq!(error.root_cause().to_string(), "Codex app-server 不支持 project/read");
        assert_eq!(state, original);
        assert_eq!(store.load().unwrap(), original);
        assert!(api.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn optional_project_rpcs_do_not_block_basic_threads_or_resume_originals() {
        let root = tempfile::tempdir().unwrap();
        for mode in [
            "optional-unavailable",
            "optional-reject-start",
            "resume-failed",
        ] {
            let mut server = fixture_server(root.path(), mode);
            server.initialize().await.unwrap();
            match mode {
                "optional-unavailable" => {
                    assert!(
                        super::super::projects::resolve_project(&mut server, "")
                            .await
                            .unwrap()
                            .is_none()
                    );
                    assert!(
                        super::super::projects::resolve_project(&mut server, "selected")
                            .await
                            .is_err()
                    );
                    assert!(
                        super::super::projects::read_projects(&mut server)
                            .await
                            .is_err()
                    );
                    assert!(
                        server
                            .api_request("thread/metadata/update", json!({}))
                            .await
                            .is_err()
                    );
                    assert_eq!(server.prepare_thread(None).await.unwrap(), "original");
                    assert_eq!(
                        server
                            .run_turn("original", "basic text")
                            .await
                            .unwrap()
                            .reply,
                        "checked"
                    );
                }
                "optional-reject-start" => {
                    assert_eq!(
                        server
                            .prepare_thread_in_project(None, Some("selected"))
                            .await
                            .unwrap(),
                        "original"
                    );
                }
                _ => {
                    assert!(server.prepare_thread(Some("original")).await.is_err());
                    assert!(server.is_running());
                }
            }
            server.close().await;
        }
    }

    #[tokio::test]
    async fn settings_tool_is_scoped_persistent_and_applies_on_next_turn() {
        use super::super::controls::{SessionOptions, SessionOptionsStore};
        let root = tempfile::tempdir().unwrap();
        let store = SessionOptionsStore::test_store(root.path().join("settings.json"), "peer-a");
        let other = SessionOptionsStore::test_store(root.path().join("settings.json"), "peer-b");
        let mut server = fixture_server(root.path(), "settings");
        let defaults = SessionOptions {
            model: "fixture".into(),
            reasoning_effort: "high".into(),
            reset_request: None,
        };
        let result = async {
            server.initialize().await?;
            let thread = server
                .prepare_thread_in_project(None, Some("project-1"))
                .await?;
            let first = server
                .run_turn_with_controls(
                    &thread,
                    "set low",
                    Some(SessionControlChannel {
                        options_store: &store,
                        defaults: &defaults,
                    }),
                )
                .await?;
            assert_eq!(first.reply, "checked");
            assert_eq!(server.config.reasoning_effort, "high");
            assert_eq!(store.load()?.reasoning_effort, "low");
            assert_eq!(other.load()?, SessionOptions::default());
            let mut next = server.config.clone();
            next.reasoning_effort = store.load()?.with_defaults(&defaults).reasoning_effort;
            next.sandbox = "workspace-write".into();
            server.set_session_config(&next);
            assert_eq!(server.prepare_thread(Some(&thread)).await?, thread);
            assert_eq!(server.run_turn(&thread, "next").await?.reply, "checked");
            Ok::<_, anyhow::Error>(())
        }
        .await;
        server.close().await;
        result.unwrap();
    }

    #[tokio::test]
    async fn resumed_thread_cannot_change_identity_or_sandbox_silently() {
        let root = tempfile::tempdir().unwrap();
        for mode in ["wrong-id", "wrong-scope", "legacy"] {
            let mut server = fixture_server(root.path(), mode);
            server.initialize().await.unwrap();
            let result = server.prepare_thread(Some("original")).await;
            server.close().await;
            if mode == "legacy" {
                assert_eq!(result.unwrap(), "original");
            } else {
                assert!(result.is_err(), "{mode}");
            }
        }
    }

    #[test]
    fn missing_turn_id_is_persistently_logged_without_user_content() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "connect::app_server::tests::outbound_stdio_registers_only_when_enabled_and_sends_each_call_once",
        ]);
        command.env("CODEX_PLUS_W2_MISSING_TURN_ID", "1");
        for (name, directory) in [
            ("CODEX_PLUS_STATE_DIR", "state"),
            ("CODEX_HOME", "home"),
            ("CODEX_SQLITE_HOME", "sqlite"),
            ("APPDATA", "appdata"),
        ] {
            command.env(name, root.path().join(directory));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(crate::windows_create_no_window());
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        let log = std::fs::read_to_string(state.join("codex-plus.log")).unwrap();
        let records: Vec<Value> = log
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let record = records
            .iter()
            .find(|record| record["event"] == "weixin.turn_start.missing_turn_id")
            .unwrap();
        assert!(record["detail"]["request_id"].as_u64().is_some());
        assert_eq!(record["detail"].as_object().unwrap().len(), 1);
        assert!(!log.contains("private prompt"));
        assert!(!log.contains("private context token"));
    }

    #[cfg(windows)]
    #[test]
    fn expired_managed_executable_resolves_after_desktop_update() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("OpenAI").join("Codex").join("bin");
        let old = bin.join("0123456789abcdef").join("codex.exe");
        let current = bin.join("abcdef0123456789").join("codex.exe");
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, "cli").unwrap();
        let resolve = |path: &str| resolve_windows_startup_executable(path, Some(temp.path()));

        assert_eq!(resolve(&old.to_string_lossy()), current.to_string_lossy());
        assert_eq!(
            resolve(&bin.join("codex.exe").to_string_lossy()),
            current.to_string_lossy()
        );
        assert_eq!(
            resolve(
                &old.to_string_lossy()
                    .replace('\\', "/")
                    .to_ascii_uppercase()
            ),
            current.to_string_lossy()
        );
        assert_eq!(resolve(""), current.to_string_lossy());
        assert!(validate_codex_executable(&resolve(&old.to_string_lossy())).is_ok());

        // 仍存在的旧版本必须继续使用，不能强行切换用户选择。
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "old cli").unwrap();
        assert_eq!(resolve(&old.to_string_lossy()), old.to_string_lossy());
    }

    #[cfg(windows)]
    #[test]
    fn managed_recovery_preserves_custom_paths_and_missing_cli_errors() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("OpenAI/Codex/bin");
        let expired = bin.join("0123456789abcdef/codex.exe");
        let expired = expired.to_string_lossy().into_owned();
        assert_eq!(
            resolve_windows_startup_executable(&expired, Some(temp.path())),
            expired
        );
        assert!(validate_codex_executable(&expired).is_err());

        let current = bin.join("codex.exe");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(&current, "cli").unwrap();
        for custom in [
            temp.path()
                .join("custom/codex.exe")
                .to_string_lossy()
                .into_owned(),
            bin.join("custom/codex.exe").to_string_lossy().into_owned(),
            temp.path()
                .join("OpenAI/Codex/bin-other/0123456789abcdef/codex.exe")
                .to_string_lossy()
                .into_owned(),
            bin.join("0123456789abcdef/other.exe")
                .to_string_lossy()
                .into_owned(),
            "codex".to_string(),
        ] {
            assert_eq!(
                resolve_windows_startup_executable(&custom, Some(temp.path())),
                custom
            );
        }
        assert_eq!(resolve_windows_startup_executable(&expired, None), expired);
    }

    #[test]
    fn extracts_ids_from_current_app_server_shapes() {
        assert_eq!(
            extract_thread_id(&json!({"thread": {"id": "thread-1"}})).as_deref(),
            Some("thread-1")
        );
        assert_eq!(
            extract_turn_id(&json!({"turn": {"id": "turn-1"}})).as_deref(),
            Some("turn-1")
        );
    }

    #[test]
    fn extracts_completed_agent_message_only() {
        let message = json!({
            "method": "item/completed",
            "params": {"item": {"type": "agentMessage", "text": "done"}}
        });
        assert_eq!(
            extract_completed_agent_text(&message).as_deref(),
            Some("done")
        );
        let user = json!({
            "method": "item/completed",
            "params": {"item": {"type": "userMessage", "text": "input"}}
        });
        assert_eq!(extract_completed_agent_text(&user), None);
    }

    #[test]
    fn extracts_current_context_usage_event() {
        let message = json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "tokenUsage": {
                    "last": {"totalTokens": 41772},
                    "total": {"totalTokens": 50000},
                    "modelContextWindow": 1000000
                }
            }
        });
        assert_eq!(
            extract_turn_usage(&message),
            Some(TurnUsage {
                context_used: Some(41772),
                context_window: Some(1000000)
            })
        );
    }

    /// #1879：以前不管什么原因失败都只说「请检查 Codex CLI 路径」，
    /// 用户填了目录、填了不存在的路径、权限不对，看到的都是同一句话。
    #[test]
    fn executable_validation_names_the_actual_problem() {
        let temp = tempfile::tempdir().unwrap();

        // 目录而不是可执行文件——issue 里多人贴的正是带尾斜杠的目录路径
        let error = validate_codex_executable(&temp.path().display().to_string()).unwrap_err();
        assert!(error.to_string().contains("目录"), "{error}");

        // 路径不存在
        let missing = temp.path().join("nope").join("codex");
        let error = validate_codex_executable(&missing.display().to_string()).unwrap_err();
        assert!(error.to_string().contains("不存在"), "{error}");

        // 裸命令名交给 PATH 解析，不该在这里被拦下
        assert!(validate_codex_executable("codex").is_ok());
    }

    /// #2028：系统保护目录（WindowsApps）路径在启动前就被拦下，
    /// 并直接告诉用户怎么改，而不是等 spawn 失败后一句「没有执行权限」。
    #[test]
    fn windows_store_cli_path_is_rejected_with_actionable_message() {
        let error = validate_codex_executable(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.915.4065.0_x64__2p2nqsd0c76g0\app\resources\codex.exe",
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("系统保护"), "{message}");
        assert!(message.contains("留空"), "{message}");
        assert!(message.contains("使用桌面版内置 CLI"), "{message}");
    }

    /// 留空即自动选用：显式填写的路径原样透传；空值在 Windows 上优先
    /// 桌面版维护的标准 CLI，取不到时退回 PATH（#2028 的零配置路径）。
    #[test]
    fn empty_executable_resolves_to_standard_cli_or_path() {
        assert_eq!(
            resolve_startup_executable("C:/x/codex.exe"),
            "C:/x/codex.exe"
        );
        let fallback = resolve_startup_executable("");
        assert!(
            fallback == "codex" || fallback.to_ascii_lowercase().ends_with("codex.exe"),
            "{fallback}"
        );
    }

    #[test]
    fn spawn_hints_distinguish_windows_store_and_missing_cli_cases() {
        let store = spawn_failure_hint(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_1.0\app\resources\codex.exe",
            std::io::ErrorKind::PermissionDenied,
        );
        assert!(store.contains("系统保护"), "{store}");
        assert!(store.contains("清空"), "{store}");

        let plain = spawn_failure_hint(r"C:\codex\codex.exe", std::io::ErrorKind::PermissionDenied);
        assert_eq!(plain, "：没有执行权限");

        let missing = spawn_failure_hint("codex", std::io::ErrorKind::NotFound);
        if cfg!(windows) {
            assert!(missing.contains("使用桌面版内置 CLI"), "{missing}");
        } else {
            assert!(missing.contains("PATH"), "{missing}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn executable_validation_flags_a_file_without_exec_permission() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("codex");
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let error = validate_codex_executable(&path.display().to_string()).unwrap_err();
        assert!(error.to_string().contains("执行权限"), "{error}");

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_codex_executable(&path.display().to_string()).is_ok());
    }

    /// app-server 起不来时，真正的原因只在 stderr 里；以前被读了就扔。
    #[test]
    fn stderr_tail_is_kept_and_bounded() {
        let sink = Arc::new(Mutex::new(VecDeque::<String>::new()));
        {
            let mut guard = sink.lock().unwrap();
            for i in 0..(STDERR_TAIL_LINES + 4) {
                if guard.len() == STDERR_TAIL_LINES {
                    guard.pop_front();
                }
                guard.push_back(format!("line-{i}"));
            }
        }
        let tail = collect_stderr_tail(&sink);
        assert!(tail.contains(&format!("line-{}", STDERR_TAIL_LINES + 3)));
        // 只留最近若干行，不能无限涨
        assert!(!tail.contains("line-0"));
        assert_eq!(tail.split('；').count(), STDERR_TAIL_LINES);
    }
}
