pub mod app_server;
pub mod controls;
pub mod desktop_sync;
mod inbox;
pub mod media;
pub mod outbound;
pub mod projects;
pub mod session_store;
pub mod weixin;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use self::app_server::{
    AppServerConfig, AppServerTurnResult, CodexAppServer, SessionControlChannel,
};
use self::inbox::Inbox;
use self::media::{AttachmentKind, DownloadedAttachments};
use self::session_store::{ConnectSessionStore, ConnectState};
use self::weixin::{WeixinClient, WeixinMessage};

pub const DEFAULT_WEIXIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const MAX_INBOUND_MESSAGE_AGE_MS: u64 = 5 * 60 * 1_000;
const FALLBACK_WORKSPACE_NOTICE: &str =
    " 原项目目录不可用，当前为状态目录中的备用空目录，无法访问原项目文件。";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WeixinConnectConfig {
    #[serde(default = "default_weixin_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub allow_from: String,
    #[serde(default)]
    pub allow_outbound_attachments: bool,
    #[serde(default)]
    pub route_tag: String,
    #[serde(default)]
    pub work_dir: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default = "default_sandbox")]
    pub sandbox: String,
    #[serde(default)]
    pub codex_path: String,
}

impl Default for WeixinConnectConfig {
    fn default() -> Self {
        Self {
            base_url: default_weixin_base_url(),
            token: String::new(),
            account_id: String::new(),
            allow_from: String::new(),
            allow_outbound_attachments: false,
            route_tag: String::new(),
            work_dir: String::new(),
            model: String::new(),
            reasoning_effort: String::new(),
            project_id: String::new(),
            sandbox: default_sandbox(),
            codex_path: String::new(),
        }
    }
}

impl WeixinConnectConfig {
    pub fn normalized(mut self) -> Self {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        if self.base_url.is_empty() {
            self.base_url = default_weixin_base_url();
        }
        self.token = self.token.trim().to_string();
        self.account_id = self.account_id.trim().to_string();
        self.allow_from = self.allow_from.trim().to_string();
        self.route_tag = self.route_tag.trim().to_string();
        self.work_dir = self.work_dir.trim().to_string();
        self.model = self.model.trim().to_string();
        self.reasoning_effort = self.reasoning_effort.trim().to_string();
        self.project_id = self.project_id.trim().to_string();
        self.sandbox = normalize_sandbox(&self.sandbox);
        self.codex_path = self.codex_path.trim().to_string();
        self
    }

}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WeixinConnectStatus {
    pub state: String,
    pub message: String,
    pub account_id: String,
    pub has_token: bool,
    pub last_peer_id: String,
    pub last_message_at_ms: u64,
    pub processed_messages: u64,
}

impl Default for WeixinConnectStatus {
    fn default() -> Self {
        Self {
            state: "stopped".to_string(),
            message: "微信连接未启动。".to_string(),
            account_id: String::new(),
            has_token: false,
            last_peer_id: String::new(),
            last_message_at_ms: 0,
            processed_messages: 0,
        }
    }
}

pub type SharedWeixinConnectStatus = Arc<Mutex<WeixinConnectStatus>>;

/// 只在消息边界切换 CLI，避免中断正在执行的回合。
#[derive(Clone)]
pub struct WeixinCodexPath(Arc<Mutex<(String, Option<SessionDefaults>)>>);

#[derive(Clone)]
pub struct SessionDefaults {
    pub model: String,
    pub reasoning_effort: String,
    pub project_id: String,
    pub work_dir: String,
    pub sandbox: String,
}

impl WeixinCodexPath {
    pub fn new(path: &str) -> Self {
        Self(Arc::new(Mutex::new((path.trim().to_string(), None))))
    }

    pub fn set(&self, path: &str) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).0 = path.trim().to_string();
    }

    pub fn set_defaults(&self, defaults: SessionDefaults) {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).1 = Some(defaults);
    }

    fn apply_defaults(&self, config: &mut WeixinConnectConfig) {
        let saved = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(defaults) = &saved.1 {
            config.model.clone_from(&defaults.model);
            config
                .reasoning_effort
                .clone_from(&defaults.reasoning_effort);
            config.project_id.clone_from(&defaults.project_id);
            config.work_dir.clone_from(&defaults.work_dir);
            config.sandbox = normalize_sandbox(&defaults.sandbox);
        }
    }

    fn apply(&self, config: &mut AppServerConfig) -> bool {
        let path = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if config.executable == path.0 {
            return false;
        }
        config.executable.clone_from(&path.0);
        true
    }
}

pub async fn run_weixin_connect(
    config: WeixinConnectConfig,
    stop: Arc<AtomicBool>,
    status: SharedWeixinConnectStatus,
) -> anyhow::Result<()> {
    let codex_path = WeixinCodexPath::new(&config.codex_path);
    run_weixin_connect_with_codex_path(config, stop, status, codex_path).await
}

pub async fn run_weixin_connect_with_codex_path(
    config: WeixinConnectConfig,
    stop: Arc<AtomicBool>,
    status: SharedWeixinConnectStatus,
    codex_path: WeixinCodexPath,
) -> anyhow::Result<()> {
    let outbound = outbound::OutboundPolicy::new(
        config.allow_outbound_attachments,
        &config.allow_from,
        &config.work_dir,
    );
    run_weixin_connect_with_outbound(config, stop, status, codex_path, outbound).await
}

pub async fn run_weixin_connect_with_outbound(
    config: WeixinConnectConfig,
    stop: Arc<AtomicBool>,
    status: SharedWeixinConnectStatus,
    codex_path: WeixinCodexPath,
    outbound: outbound::OutboundPolicy,
) -> anyhow::Result<()> {
    let mut config = config.normalized();
    if config.token.is_empty() {
        bail!("请先扫码登录微信");
    }
    let work_dir = connection_work_dir(&config, &crate::paths::default_app_state_dir())?;

    update_status(&status, &config, |current| {
        current.state = "starting".to_string();
        current.message = "正在连接微信和 Codex app-server...".to_string();
        current.account_id = config.account_id.clone();
        current.has_token = true;
    });

    let client = WeixinClient::new(&config.base_url, &config.token, &config.route_tag)?;
    let store = ConnectSessionStore::default_for_account(&config.account_id);
    let mut state = store.load()?;
    let mut app_config = AppServerConfig {
        executable: config.codex_path.clone(),
        work_dir,
        model: config.model.clone(),
        reasoning_effort: config.reasoning_effort.clone(),
        sandbox: config.sandbox.clone(),
    };
    let mut app_server: Option<CodexAppServer> = None;
    let mut long_poll_timeout_ms = 35_000;

    update_status(&status, &config, |current| {
        current.state = "running".to_string();
        current.message = outbound.status_message("微信连接正在运行。");
    });

    while !stop.load(Ordering::SeqCst) {
        update_status(&status, &config, |_| {});
        let updates = match client
            .get_updates(&state.get_updates_buf, long_poll_timeout_ms)
            .await
        {
            Ok(updates) => updates,
            Err(error) => {
                update_status(&status, &config, |current| {
                    current.state = "retrying".to_string();
                    current.message = format!("微信长轮询失败，稍后重试：{error}");
                });
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }
        };

        if !updates.get_updates_buf.is_empty() {
            state.get_updates_buf = updates.get_updates_buf;
        }
        if updates.longpolling_timeout_ms > 0 {
            long_poll_timeout_ms = updates.longpolling_timeout_ms;
        }

        for message in updates.messages {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let message_key = message.dedup_key();
            if !message.is_finished_user_message()
                || message.is_older_than(now_ms(), MAX_INBOUND_MESSAGE_AGE_MS)
                || state.is_processed(&message_key)
            {
                continue;
            }
            // 白名单外的消息以前完全静默：不回复也没有任何提示，
            // 用户无法区分「消息没到」和「被过滤了」。
            if !is_allowed_peer(&config.allow_from, &message.from_user_id) {
                update_status(&status, &config, |current| {
                    current.state = "running".to_string();
                    current.message =
                        format!("已忽略来自非白名单发送方的消息：{}", message.from_user_id);
                });
                continue;
            }
            let text = message.text().unwrap_or_default();
            if text.is_empty() && !message.has_attachments() {
                state.mark_processed(&message_key);
                continue;
            }
            if message.context_token.trim().is_empty() {
                update_status(&status, &config, |current| {
                    current.state = "running".to_string();
                    current.message = "收到缺少 context_token 的微信消息，已忽略。".to_string();
                });
                state.mark_processed(&message_key);
                continue;
            }

            state
                .context_tokens
                .insert(message.from_user_id.clone(), message.context_token.clone());
            codex_path.apply_defaults(&mut config);
            app_config.model.clone_from(&config.model);
            app_config
                .reasoning_effort
                .clone_from(&config.reasoning_effort);
            app_config.sandbox.clone_from(&config.sandbox);
            let work_dir = connection_work_dir(&config, &crate::paths::default_app_state_dir())?;
            let changed_work_dir = app_config.work_dir != work_dir;
            app_config.work_dir = work_dir;
            if codex_path.apply(&mut app_config) || changed_work_dir {
                if let Some(mut server) = app_server.take() {
                    server.close().await;
                }
            }
            let result = process_weixin_message(
                &client,
                &app_config,
                &mut app_server,
                &mut state,
                &message,
                &text,
                &stop,
                &outbound,
                &config.account_id,
                &config.project_id,
                !config.work_dir.is_empty(),
            )
            .await;

            if let Some(mut server) = app_server.take() {
                server.close().await;
            }

            if stop.load(Ordering::SeqCst) {
                app_server = None;
                break;
            }

            match result {
                Ok(()) => {
                    state.mark_processed(&message_key);
                    update_status(&status, &config, |current| {
                        current.state = "running".to_string();
                        current.message = outbound.status_message("最近一条微信消息已处理。");
                        current.last_peer_id = message.from_user_id.clone();
                        current.last_message_at_ms = now_ms();
                        current.processed_messages = current.processed_messages.saturating_add(1);
                    });
                }
                Err(error) => {
                    app_server = None;
                    state.mark_processed(&message_key);
                    let _ = client
                        .send_text_chunks(
                            &message.from_user_id,
                            &weixin_failure_reply(&error.to_string()),
                            &message.context_token,
                        )
                        .await;
                    update_status(&status, &config, |current| {
                        current.state = "retrying".to_string();
                        current.message = format!("处理微信消息失败，等待下一条消息重试：{error}");
                        current.last_peer_id = message.from_user_id.clone();
                        current.last_message_at_ms = now_ms();
                    });
                }
            }
            store.save(&state)?;
        }
        store.save(&state)?;
    }

    if let Some(mut server) = app_server {
        server.close().await;
    }
    update_status(&status, &config, |current| {
        current.state = "stopped".to_string();
        current.message = "微信连接已停止。".to_string();
    });
    Ok(())
}

fn connection_work_dir(config: &WeixinConnectConfig, state_dir: &std::path::Path) -> anyhow::Result<PathBuf> {
    let work_dir = if config.work_dir.is_empty() {
        std::env::current_dir().context("无法读取当前工作目录")?
    } else {
        PathBuf::from(&config.work_dir)
    };
    if work_dir.is_dir() {
        return Ok(work_dir);
    }
    // 设置可能在连接启动后开启回传；目录降级不能依赖启动时的开关快照。
    let fallback = state_dir.join("weixin-text-workspace");
    std::fs::create_dir_all(&fallback).context("无法创建微信文字连接的临时工作目录")?;
    Ok(fallback)
}

struct PreparedWeixinInput {
    prompt: String,
    image_paths: Vec<PathBuf>,
    warnings: Vec<String>,
    _inbox: Option<Inbox>,
}

fn attachment_turn_input(
    text: &str,
    downloaded: &DownloadedAttachments,
) -> anyhow::Result<(String, Vec<PathBuf>)> {
    if downloaded.attachments.is_empty() {
        if text.trim().is_empty() {
            bail!(
                "微信附件接收失败：{}",
                downloaded
                    .warnings
                    .first()
                    .map(String::as_str)
                    .unwrap_or("没有可用附件")
            );
        }
        return Ok((text.to_owned(), Vec::new()));
    }
    let mut prompt = if text.trim().is_empty() {
        "请查看本条微信消息中的附件。".to_string()
    } else {
        text.to_owned()
    };
    prompt.push_str("\n\n微信附件已保存到本地，附件路径如下：");
    let mut image_paths = Vec::new();
    for attachment in &downloaded.attachments {
        let label = match attachment.kind {
            AttachmentKind::Image => {
                image_paths.push(attachment.path.clone());
                "图片"
            }
            AttachmentKind::File => "文件",
        };
        // 引号和控制字符按 JSON 转义，文件名不能伪造额外的提示行。
        let path = serde_json::to_string(&attachment.path.to_string_lossy())?;
        prompt.push_str(&format!("\n- {label}：{path}"));
    }
    Ok((prompt, image_paths))
}

async fn prepare_weixin_input(
    client: &WeixinClient,
    work_dir: &std::path::Path,
    message: &WeixinMessage,
    text: &str,
) -> anyhow::Result<PreparedWeixinInput> {
    let (inbox, downloaded) = if message.has_attachments() {
        match Inbox::create(work_dir) {
            Ok(inbox) => {
                let downloaded = client.download_attachments(message, &inbox).await;
                (Some(inbox), downloaded)
            }
            Err(_) => (
                None,
                DownloadedAttachments {
                    warnings: vec!["无法准备微信附件收件目录".into()],
                    ..Default::default()
                },
            ),
        }
    } else {
        (None, DownloadedAttachments::default())
    };
    let (prompt, image_paths) = attachment_turn_input(text, &downloaded)?;
    Ok(PreparedWeixinInput {
        prompt,
        image_paths,
        warnings: downloaded.warnings,
        _inbox: inbox,
    })
}

async fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

async fn process_weixin_message(
    client: &WeixinClient,
    app_config: &AppServerConfig,
    app_server: &mut Option<CodexAppServer>,
    state: &mut ConnectState,
    message: &WeixinMessage,
    text: &str,
    stop: &AtomicBool,
    outbound: &outbound::OutboundPolicy,
    account_id: &str,
    project_id: &str,
    explicit_work_dir: bool,
) -> anyhow::Result<()> {
    let options_store = controls::SessionOptionsStore::for_peer(account_id, &message.from_user_id);
    if matches!(text.trim(), "/reset" | "/reset-session") {
        options_store.request_reset()?;
        state.thread_ids.remove(&message.from_user_id);
        state.completed_resets.remove(&message.from_user_id);
        ConnectSessionStore::default_for_account(account_id).save(state)?;
        if let Some(mut server) = app_server.take() {
            server.close().await;
        }
        return client
            .send_text_chunks(
                &message.from_user_id,
                "已重置当前微信会话并清除模型、推理强度覆盖值。下一条消息将按当前连接默认设置创建新会话，原会话历史保留。",
                &message.context_token,
            )
            .await;
    }
    let permit = outbound
        .permit(&message.from_user_id)
        .ok()
        .filter(|permit| {
            app_config.work_dir.canonicalize().ok().as_ref() == Some(&permit.workspace)
        });
    if text.trim() == "/reset-attachments" {
        let reply = if permit.is_some() {
            state.thread_ids.remove(&message.from_user_id);
            "已重置当前联系人的微信会话。下一条消息将创建带附件工具的新会话；原会话历史保留。"
        } else {
            "请先开启附件回传并配置允许的微信用户和工作区，再发送 /reset-attachments。"
        };
        return client
            .send_text_chunks(&message.from_user_id, reply, &message.context_token)
            .await;
    }
    let options = options_store.load()?;
    let mut app_config = app_config.clone();
    if let Some(reason) = local_model_endpoint_unreachable_reason().await {
        bail!("{reason}");
    }
    let input = tokio::select! {
        input = prepare_weixin_input(client, &app_config.work_dir, message, text) => input?,
        _ = wait_for_stop(stop) => bail!("微信连接已停止"),
    };
    if app_server
        .as_ref()
        .map(|server| !server.is_running())
        .unwrap_or(true)
    {
        *app_server = Some(CodexAppServer::start(app_config.clone()).await?);
    }
    let server = app_server.as_mut().context("Codex app-server 未启动")?;
    let allow_attachments = permit.as_ref().is_some_and(|permit| permit.check().is_ok());
    let reset_pending = options
        .reset_request
        .as_ref()
        .is_some_and(|request| state.completed_resets.get(&message.from_user_id) != Some(request));
    let saved_thread_id = (!reset_pending)
        .then(|| saved_thread_id(state, &message.from_user_id))
        .flatten();
    let project = if saved_thread_id.is_none() {
        projects::resolve_project(server, project_id)
            .await
            .context("读取所选工作项目失败，未创建会话；请检查项目选择后重试")?
    } else {
        None
    };
    if !explicit_work_dir
        && let Some(path) = project.as_ref().and_then(|project| {
            project["roots"]
                .as_array()
                .and_then(|roots| roots.first())
                .and_then(|root| root["path"].as_str())
        })
        && std::path::Path::new(path).is_dir()
    {
        app_config.work_dir = PathBuf::from(path);
    }
    if !app_config.work_dir.is_dir() {
        bail!("工作项目目录不可用");
    }
    app_config.work_dir = std::path::absolute(&app_config.work_dir)?;
    let config_response = server
        .api_request(
            "config/read",
            serde_json::json!({"includeLayers":false,"cwd":app_config.work_dir}),
        )
        .await
        .unwrap_or_default();
    let native = &config_response["config"];
    let mut defaults = controls::SessionOptions {
        model: if app_config.model.is_empty() {
            native["model"].as_str().unwrap_or_default().to_string()
        } else {
            app_config.model.clone()
        },
        reasoning_effort: if app_config.reasoning_effort.is_empty() {
            native["model_reasoning_effort"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        } else {
            app_config.reasoning_effort.clone()
        },
        reset_request: None,
    };
    let mut catalog = crate::model_catalog::read_codex_model_catalog().await;
    let unresolved = options.resolve(&defaults, &catalog);
    if defaults.model.is_empty() || unresolved.model.is_empty() || unresolved.reasoning_effort.is_empty() {
        let models = server
            .api_request("model/list", serde_json::json!({"includeHidden":true}))
            .await
            .unwrap_or_default();
        controls::apply_native_model_defaults(&mut defaults, &mut catalog, &models);
    }
    let (effective, recovered_options) = options.resolve_for_turn(&defaults, &catalog);
    if recovered_options {
        client
            .send_text_chunks(
                &message.from_user_id,
                "已保存的模型或推理强度在当前配置下不可用，本条消息改用连接默认设置。发送 /reset 可清除覆盖值并重置当前微信会话。",
                &message.context_token,
            )
            .await?;
    }
    app_config.model = effective.model;
    app_config.reasoning_effort = effective.reasoning_effort;
    server.set_session_config(&app_config);
    let thread_id = server
        .prepare_thread_in_project_with_attachments(
            saved_thread_id.as_deref(),
            project.as_ref().and_then(|value| value["id"].as_str()),
            allow_attachments,
        )
        .await
        .map_err(|error| anyhow::anyhow!("无法准备微信会话，原映射已保留：{error}；可发送 /reset 重置当前微信会话后重试"))?;
    state
        .thread_ids
        .insert(message.from_user_id.clone(), thread_id.clone());
    if let Some(request) = &options.reset_request {
        state
            .completed_resets
            .insert(message.from_user_id.clone(), request.clone());
    }
    ConnectSessionStore::default_for_account(account_id).save(state)?;
    if let Some(request) = &options.reset_request {
        options_store.complete_reset(request)?;
    }
    if let Some(project) = &project {
        let _ = server
            .api_request(
                "thread/metadata/update",
                serde_json::json!({
                    "threadId":thread_id,"projectId":project["id"]
                }),
            )
            .await;
    }
    let _ = desktop_sync::record(&thread_id, project.as_ref());

    let turn_result = {
        let turn = server.run_turn_with_images_and_release(
            &thread_id,
            &input.prompt,
            &input.image_paths,
            Some(app_server::WeixinReplyChannel {
                client,
                peer: &message.from_user_id,
                context_token: &message.context_token,
                permit,
            }),
            Some(SessionControlChannel {
                options_store: &options_store,
                defaults: &defaults,
            }),
        );
        tokio::pin!(turn);
        loop {
            tokio::select! {
                reply = &mut turn => break reply?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                    if stop.load(Ordering::SeqCst) {
                        bail!("微信连接已停止");
                    }
                }
            }
        }
    };
    let reply = if turn_result.reply.trim().is_empty() {
        "Codex 已完成处理，但没有返回文字内容。"
    } else {
        turn_result.reply.trim()
    };
    let _ = desktop_sync::record(&thread_id, project.as_ref());
    let reply = if input.warnings.is_empty() {
        reply.to_owned()
    } else {
        format!(
            "微信附件接收提示：\n{}\n\n{reply}",
            input.warnings.join("\n")
        )
    };
    let reply_with_footer = append_reply_footer(&reply, &turn_result, &app_config.work_dir);
    client
        .send_text_chunks(
            &message.from_user_id,
            &reply_with_footer,
            &message.context_token,
        )
        .await
}

fn saved_thread_id(state: &ConnectState, peer: &str) -> Option<String> {
    state.thread_ids.get(peer).cloned()
}

fn append_reply_footer(
    reply: &str,
    turn: &AppServerTurnResult,
    work_dir: &std::path::Path,
) -> String {
    let model = if turn.model.trim().is_empty() {
        "Codex"
    } else {
        turn.model.trim()
    };
    let context = match (turn.usage.context_used, turn.usage.context_window) {
        (Some(used), Some(window)) if window > 0 => {
            let percent = ((used as f64 / window as f64) * 100.0).round() as u64;
            format!("ctx {}%", percent.min(100))
        }
        _ => "ctx --".to_string(),
    };
    format!(
        "{}\n\n{} · {}\n{}",
        reply.trim(),
        model,
        context,
        compact_work_dir(work_dir)
    )
}

fn compact_work_dir(work_dir: &std::path::Path) -> String {
    let path = work_dir.to_string_lossy();
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::Path::new(&home).to_string_lossy();
        if path == home {
            return "~".to_string();
        }
        if let Some(relative) = path
            .strip_prefix(home.as_ref())
            .and_then(|value| value.strip_prefix('/').filter(|value| !value.is_empty()))
        {
            return format!("~/{relative}");
        }
    }
    path.into_owned()
}

/// 微信端收到的失败回复要带上原因：过去只有一句「处理失败」，用户必须回到电脑上
/// 打开管理器才知道是 CLI 路径、登录态还是上游错误（#2349）。
/// 只取首行、隐去用户目录并截断，避免把长堆栈或本机路径整段发到聊天里。
fn weixin_failure_reply(error: &str) -> String {
    const MAX_REASON_CHARS: usize = 160;
    let mut reason = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();
    for key in ["USERPROFILE", "HOME"] {
        if let Some(home) = std::env::var_os(key) {
            let home = home.to_string_lossy().to_string();
            if home.len() > 1 {
                reason = reason.replace(&home, "~");
            }
        }
    }
    if reason.chars().count() > MAX_REASON_CHARS {
        reason = reason.chars().take(MAX_REASON_CHARS).collect::<String>() + "…";
    }
    if reason.is_empty() {
        "Codex 处理失败，请在 Codex++ 管理器中查看连接状态。".to_string()
    } else {
        format!("Codex 处理失败：{reason}\n详情请在 Codex++ 管理器中查看连接状态。")
    }
}

fn is_allowed_peer(allow_from: &str, peer: &str) -> bool {
    let allow_from = allow_from.trim();
    allow_from.is_empty()
        || allow_from == "*"
        || allow_from
            .split(',')
            .map(str::trim)
            .any(|allowed| !allowed.is_empty() && allowed == peer)
}

/// 模型流量若走本地代理（config.toml 的 openai_base_url 指向 127.0.0.1，
/// 由 Codex++ 的单模型路由写入），该代理只随从 Codex++ 启动的桌面版存在。
/// 提前探测，避免 turn 失败后只剩一句模糊的「处理失败」。
async fn local_model_endpoint_unreachable_reason() -> Option<String> {
    let base_url =
        std::fs::read_to_string(crate::relay_config::default_codex_home_dir().join("config.toml"))
            .ok()
            .and_then(|contents| {
                crate::relay_config::root_key_string(&contents, "openai_base_url")
            })?;
    let (host, port) = localhost_endpoint(&base_url)?;
    let connect = tokio::net::TcpStream::connect((host.as_str(), port));
    if tokio::time::timeout(std::time::Duration::from_millis(300), connect)
        .await
        .is_ok_and(|result| result.is_ok())
    {
        return None;
    }
    Some(
        "Codex 桌面版当前未运行，微信连接暂时无法调用模型。\
         请先从 Codex++ 启动 Codex 桌面版后重试。"
            .to_string(),
    )
}

/// 只对「本地回环 + 显式端口」的 base_url 预检；
/// 直连中转站或未带端口的地址不属于桌面版代理，返回 None 表示跳过。
fn localhost_endpoint(base_url: &str) -> Option<(String, u16)> {
    let url = reqwest::Url::parse(base_url.trim()).ok()?;
    let host = url.host_str()?;
    if host != "127.0.0.1" && host != "localhost" {
        return None;
    }
    Some((host.to_string(), url.port()?))
}

fn normalize_sandbox(value: &str) -> String {
    match value.trim() {
        "workspace-write" => "workspace-write",
        "danger-full-access" => "danger-full-access",
        _ => "read-only",
    }
    .to_string()
}

fn default_weixin_base_url() -> String {
    DEFAULT_WEIXIN_BASE_URL.to_string()
}

fn default_sandbox() -> String {
    "read-only".to_string()
}

fn update_status(
    status: &SharedWeixinConnectStatus,
    config: &WeixinConnectConfig,
    update: impl FnOnce(&mut WeixinConnectStatus),
) {
    if let Ok(mut current) = status.lock() {
        update(&mut current);
        current.message = current
            .message
            .trim_end_matches(FALLBACK_WORKSPACE_NOTICE)
            .to_string();
        if current.state != "stopped"
            && !config.work_dir.is_empty()
            && !std::path::Path::new(&config.work_dir).is_dir()
        {
            current.message.push_str(FALLBACK_WORKSPACE_NOTICE);
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_input_keeps_caption_and_file_paths_with_only_images_in_local_image_input() {
        let image = PathBuf::from("workspace/inbox/image.png");
        let file = PathBuf::from("workspace/inbox/report.txt");
        let downloaded = DownloadedAttachments {
            attachments: vec![
                media::LocalAttachment {
                    path: image.clone(),
                    kind: AttachmentKind::Image,
                },
                media::LocalAttachment {
                    path: file.clone(),
                    kind: AttachmentKind::File,
                },
            ],
            warnings: vec!["one attachment failed".into()],
        };
        let (prompt, images) = attachment_turn_input("keep caption", &downloaded).unwrap();
        assert!(prompt.starts_with("keep caption\n"));
        assert!(prompt.contains(&serde_json::to_string(&file.to_string_lossy()).unwrap()));
        assert!(prompt.contains(&serde_json::to_string(&image.to_string_lossy()).unwrap()));
        assert!(!prompt.contains("one attachment failed"));
        assert_eq!(images, [image]);
        assert!(
            attachment_turn_input("", &downloaded)
                .unwrap()
                .0
                .contains("请查看")
        );
        let failed = DownloadedAttachments {
            warnings: vec!["附件 1：下载失败".into()],
            ..Default::default()
        };
        assert_eq!(
            attachment_turn_input("original text", &failed).unwrap(),
            ("original text".into(), vec![])
        );
        assert!(
            attachment_turn_input("", &failed)
                .unwrap_err()
                .to_string()
                .contains("下载失败")
        );
    }

    #[tokio::test]
    async fn prepare_inbound_handles_media_only_partial_failure_and_text_without_starting_a_server()
    {
        use serde_json::json;
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

        let cdn = MockServer::start().await;
        Mock::given(path("/image"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\nfixture"))
            .mount(&cdn)
            .await;
        let client = WeixinClient::new(&cdn.uri(), "test-token", "test-route")
            .unwrap()
            .with_media_base_url(&cdn.uri())
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let message: WeixinMessage = serde_json::from_value(json!({
            "item_list":[{"type":2,"image_item":{"url":format!("{}/image",cdn.uri())}}]
        }))
        .unwrap();
        assert!(message.text().is_none() && message.has_attachments());
        let input = prepare_weixin_input(&client, root.path(), &message, "")
            .await
            .unwrap();
        assert_eq!(input.image_paths.len(), 1);
        assert_eq!(
            std::fs::read(&input.image_paths[0]).unwrap(),
            b"\x89PNG\r\n\x1a\nfixture"
        );
        assert!(input.warnings.is_empty());
        assert!(input._inbox.is_some());

        let message: WeixinMessage = serde_json::from_value(json!({
            "item_list":[
                {"type":2,"image_item":{"url":format!("{}/image",cdn.uri())}},
                {"type":4,"file_item":{}}
            ]
        }))
        .unwrap();
        let partial = prepare_weixin_input(&client, root.path(), &message, "caption")
            .await
            .unwrap();
        assert!(partial.prompt.starts_with("caption\n"));
        assert_eq!(partial.image_paths.len(), 1);
        assert_eq!(partial.warnings.len(), 1);

        let invalid = root.path().join("not-a-directory");
        std::fs::write(&invalid, b"keep").unwrap();
        let failed = prepare_weixin_input(&client, &invalid, &message, "keep text")
            .await
            .unwrap();
        assert_eq!(failed.prompt, "keep text");
        assert_eq!(failed.warnings, ["无法准备微信附件收件目录"]);
        assert!(failed.image_paths.is_empty());
        assert!(
            prepare_weixin_input(&client, &invalid, &message, "")
                .await
                .is_err()
        );

        let plain: WeixinMessage = serde_json::from_value(json!({"item_list":[]})).unwrap();
        let text_only = prepare_weixin_input(&client, root.path(), &plain, "plain text")
            .await
            .unwrap();
        assert_eq!(text_only.prompt, "plain text");
        assert!(text_only._inbox.is_none());
        assert!(text_only.image_paths.is_empty());
    }

    #[test]
    fn defaults_are_reloaded_only_at_message_boundaries_and_original_thread_wins() {
        let runtime = WeixinCodexPath::new("codex");
        let mut config = WeixinConnectConfig::default();
        runtime.set_defaults(SessionDefaults {
            model: "next-model".into(),
            reasoning_effort: "low".into(),
            project_id: "project-2".into(),
            work_dir: "next-workspace".into(),
            sandbox: "workspace-write".into(),
        });
        assert!(config.model.is_empty());
        runtime.apply_defaults(&mut config);
        assert_eq!(config.model, "next-model");
        assert_eq!(config.reasoning_effort, "low");
        assert_eq!(config.project_id, "project-2");
        assert_eq!(config.work_dir, "next-workspace");
        assert_eq!(config.sandbox, "workspace-write");
        let mut state = ConnectState::default();
        state.thread_ids.insert("peer".into(), "original".into());
        assert_eq!(saved_thread_id(&state, "peer").as_deref(), Some("original"));
        assert_eq!(saved_thread_id(&state, "other"), None);
    }

    #[test]
    fn saved_cli_path_replaces_startup_path_at_next_message() {
        let path = WeixinCodexPath::new("missing-codex");
        let runtime_path = path.clone();
        let mut config = AppServerConfig {
            executable: "missing-codex".to_string(),
            work_dir: PathBuf::from("."),
            model: "test-model".to_string(),
            reasoning_effort: String::new(),
            sandbox: "read-only".to_string(),
        };
        assert!(!runtime_path.apply(&mut config));
        path.set(" C:/new/codex.exe ");
        assert_eq!(config.executable, "missing-codex");
        assert!(runtime_path.apply(&mut config));
        assert_eq!(config.executable, "C:/new/codex.exe");
        assert_eq!(config.model, "test-model");
        assert!(!runtime_path.apply(&mut config));
        path.set("   ");
        assert!(runtime_path.apply(&mut config));
        assert_eq!(config.executable, "");
    }

    #[tokio::test]
    async fn unavailable_attachment_workspace_keeps_text_input_and_recovers_when_restored() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let config = WeixinConnectConfig {
            allow_outbound_attachments: false,
            allow_from: "peer".into(),
            work_dir: workspace.to_str().unwrap().into(),
            ..Default::default()
        };
        let state_dir = root.path().join("state");
        assert_eq!(connection_work_dir(&config, &state_dir).unwrap(), workspace);
        let policy = outbound::OutboundPolicy::new(false, "peer", &config.work_dir);
        assert!(policy.permit("peer").is_err());
        policy.set(true, "peer", &config.work_dir);
        assert!(policy.permit("peer").is_ok());
        std::fs::rename(&workspace, root.path().join("moved-workspace")).unwrap();
        let fallback = connection_work_dir(&config, &state_dir).unwrap();
        assert_eq!(fallback, state_dir.join("weixin-text-workspace"));
        assert!(fallback.is_dir());
        assert!(!workspace.exists(), "the unavailable workspace must not be recreated");
        assert!(!config.allow_outbound_attachments, "runtime permission must not rewrite the startup snapshot");
        assert!(policy.permit("peer").is_err());
        assert!(policy.status_message("running").contains("附件回传暂不可用"));
        let client = WeixinClient::new("http://127.0.0.1:1", "test", "").unwrap();
        let message: WeixinMessage = serde_json::from_value(serde_json::json!({"item_list":[]})).unwrap();
        let input = prepare_weixin_input(&client, &fallback, &message, "still text").await.unwrap();
        assert_eq!(input.prompt, "still text");
        assert!(input._inbox.is_none());
        std::fs::create_dir(&workspace).unwrap();
        assert_eq!(connection_work_dir(&config, &state_dir).unwrap(), workspace);
        assert!(policy.permit("peer").is_ok());
        assert_eq!(policy.status_message("running"), "running");
    }

    #[test]
    fn fallback_workspace_status_without_attachments_clears_when_directory_recovers() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("unavailable-workspace");
        let config = WeixinConnectConfig {
            work_dir: workspace.to_str().unwrap().into(),
            ..Default::default()
        };
        assert!(!config.allow_outbound_attachments);
        let fallback = connection_work_dir(&config, &root.path().join("state")).unwrap();
        assert!(fallback.is_dir());
        assert!(!workspace.exists());
        let status = Arc::new(Mutex::new(WeixinConnectStatus::default()));
        for state in ["starting", "running", "retrying"] {
            update_status(&status, &config, |current| {
                current.state = state.into();
                current.message = "text-ready".into();
            });
            let message = status.lock().unwrap().message.clone();
            assert!(message.starts_with("text-ready"));
            assert!(message.contains("原项目目录不可用"));
            assert!(message.contains("备用空目录"));
            update_status(&status, &config, |_| {});
            assert_eq!(status.lock().unwrap().message, message);
        }
        std::fs::create_dir(&workspace).unwrap();
        assert_eq!(
            connection_work_dir(&config, &root.path().join("state")).unwrap(),
            workspace,
        );
        update_status(&status, &config, |_| {});
        assert_eq!(status.lock().unwrap().message, "text-ready");
    }

    #[tokio::test]
    async fn outbound_reset_is_explicit_authorized_and_keeps_other_contact_history() {
        use serde_json::json;
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let api = MockServer::start().await;
        Mock::given(path("/ilink/bot/sendmessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
            .mount(&api)
            .await;
        let client = WeixinClient::new(&api.uri(), "fixture", "").unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let config = AppServerConfig {
            executable: "unused".into(),
            work_dir: workspace.path().to_path_buf(),
            model: String::new(),
            reasoning_effort: String::new(),
            sandbox: "read-only".into(),
        };
        let policy =
            outbound::OutboundPolicy::new(false, "peer", workspace.path().to_str().unwrap());
        let mut state = ConnectState::default();
        state.thread_ids.insert("peer".into(), "original".into());
        state
            .thread_ids
            .insert("other".into(), "other-history".into());
        let message: WeixinMessage = serde_json::from_value(json!({
            "from_user_id":"peer", "context_token":"context"
        }))
        .unwrap();
        let stop = AtomicBool::new(false);
        let mut server = None;
        process_weixin_message(
            &client,
            &config,
            &mut server,
            &mut state,
            &message,
            "/reset-attachments",
            &stop,
            &policy,
            "fixture",
            "",
            true,
        )
        .await
        .unwrap();
        assert_eq!(state.thread_ids["peer"], "original");
        policy.set(true, "peer", workspace.path().to_str().unwrap());
        process_weixin_message(
            &client,
            &config,
            &mut server,
            &mut state,
            &message,
            "/reset-attachments",
            &stop,
            &policy,
            "fixture",
            "",
            true,
        )
        .await
        .unwrap();
        assert!(!state.thread_ids.contains_key("peer"));
        assert_eq!(state.thread_ids["other"], "other-history");
        assert!(server.is_none());
        let requests = api.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let denied: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        let accepted: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert!(
            denied["msg"]["item_list"][0]["text_item"]["text"]
                .as_str()
                .unwrap()
                .contains("请先开启")
        );
        assert!(
            accepted["msg"]["item_list"][0]["text_item"]["text"]
                .as_str()
                .unwrap()
                .contains("原会话历史保留")
        );
    }

    #[test]
    fn weixin_failure_reply_carries_a_short_first_line_reason() {
        let reply =
            weixin_failure_reply("app-server 启动失败：拒绝访问\nstack line 1\nstack line 2");
        assert!(reply.contains("app-server 启动失败：拒绝访问"), "{reply}");
        assert!(!reply.contains("stack line"), "{reply}");
        let long = "x".repeat(500);
        let reply = weixin_failure_reply(&long);
        assert!(reply.chars().count() < 260, "{reply}");
        assert!(reply.contains('…'));
        assert_eq!(
            weixin_failure_reply("  \n "),
            "Codex 处理失败，请在 Codex++ 管理器中查看连接状态。"
        );
    }

    #[test]
    fn allow_from_supports_wildcard_and_comma_separated_ids() {
        assert!(is_allowed_peer("", "a@im.wechat"));
        assert!(is_allowed_peer("*", "a@im.wechat"));
        assert!(is_allowed_peer("a@im.wechat, b@im.wechat", "b@im.wechat"));
        assert!(!is_allowed_peer("a@im.wechat", "b@im.wechat"));
    }

    /// 白名单外的消息不再无声消失：状态栏要能看到「已忽略」，
    /// 否则用户无法区分「消息没到」和「被过滤」（群内真实发生过）。
    /// 用 wiremock 假微信网关驱动真实消息循环验证。
    #[tokio::test]
    async fn non_allowlisted_message_updates_status_and_is_dropped() {
        let server = wiremock::MockServer::start().await;
        let message = serde_json::json!({
            "seq": 1,
            "message_id": (now_ms() % 1_000_000) as i64,
            "from_user_id": "stranger@im.wechat",
            "client_id": "unit-test",
            "create_time_ms": now_ms() as i64,
            "message_type": 1,
            "message_state": 2,
            "item_list": [{ "type": 1, "text_item": { "text": "hi" } }],
            "context_token": "ctx"
        });
        let updates = serde_json::json!({
            "ret": 0,
            "errcode": 0,
            "errmsg": "",
            "msgs": [message],
            "get_updates_buf": "YnVm",
            "longpolling_timeout_ms": 1000
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/ilink/bot/getupdates"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(updates)
                    .set_delay(std::time::Duration::from_millis(300)),
            )
            .mount(&server)
            .await;

        let work_dir = tempfile::tempdir().unwrap();
        let config = WeixinConnectConfig {
            base_url: server.uri(),
            token: "tok".to_string(),
            account_id: "unit-test-ignore".to_string(),
            allow_from: "only-me@im.wechat".to_string(),
            allow_outbound_attachments: false,
            route_tag: String::new(),
            work_dir: work_dir.path().display().to_string(),
            model: String::new(),
            reasoning_effort: String::new(),
            project_id: String::new(),
            sandbox: "read-only".to_string(),
            codex_path: String::new(),
        };
        let status: SharedWeixinConnectStatus = Arc::new(Mutex::new(Default::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run_weixin_connect_with_codex_path(
            config,
            Arc::clone(&stop),
            Arc::clone(&status),
            WeixinCodexPath::new(""),
        ));
        // 轮询等待「已忽略」状态出现：固定 sleep 在并行测试负载下会抖动（CI/本地实测偶发）。
        // 循环正常退出时会写「微信连接已停止」，所以要在 stop 之前拿到快照。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        let mut message = String::new();
        while std::time::Instant::now() < deadline {
            message = status
                .lock()
                .map(|current| current.message.clone())
                .unwrap_or_default();
            if message.contains("已忽略来自非白名单发送方的消息") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        stop.store(true, Ordering::SeqCst);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;

        assert!(
            message.contains("已忽略来自非白名单发送方的消息"),
            "状态栏应提示已忽略，实际：{message}"
        );
        // 清理循环写入真实 app state 目录的测试状态文件
        let _ = std::fs::remove_file(
            crate::paths::default_app_state_dir()
                .join("weixin-connect-state-unit-test-ignore.json"),
        );
    }

    #[test]
    fn localhost_endpoint_only_matches_explicit_local_ports() {
        assert_eq!(
            localhost_endpoint("http://127.0.0.1:57321/v1"),
            Some(("127.0.0.1".to_string(), 57321))
        );
        assert_eq!(
            localhost_endpoint("http://localhost:8080/v1"),
            Some(("localhost".to_string(), 8080))
        );
        // 直连中转站不属于桌面版代理，不预检
        assert_eq!(localhost_endpoint("https://api.example.com/v1"), None);
        // 无显式端口的本地地址也不预检，避免误判成「桌面版未运行」
        assert_eq!(localhost_endpoint("http://127.0.0.1/v1"), None);
    }

    #[test]
    fn config_normalizes_base_url_and_sandbox() {
        let config = WeixinConnectConfig {
            base_url: " https://example.test/ ".to_string(),
            sandbox: "unknown".to_string(),
            ..WeixinConnectConfig::default()
        }
        .normalized();
        assert_eq!(config.base_url, "https://example.test");
        assert_eq!(config.sandbox, "read-only");
    }

    #[test]
    fn reply_footer_contains_model_context_and_compact_work_dir() {
        let turn = AppServerTurnResult {
            reply: "完成了".to_string(),
            model: "gpt-5.5".to_string(),
            usage: app_server::TurnUsage {
                context_used: Some(41772),
                context_window: Some(1_000_000),
            },
        };
        let footer = append_reply_footer(
            "完成了",
            &turn,
            std::path::Path::new("/Users/tester/project"),
        );
        assert!(footer.contains("gpt-5.5 · ctx 4%"));
        assert!(footer.ends_with("/Users/tester/project"));
    }

    #[test]
    fn reply_footer_does_not_invent_context_when_usage_is_missing() {
        let turn = AppServerTurnResult {
            reply: "done".to_string(),
            model: String::new(),
            usage: app_server::TurnUsage::default(),
        };
        let footer = append_reply_footer("done", &turn, std::path::Path::new("/tmp/work"));
        assert!(footer.contains("Codex · ctx --"));
        assert!(footer.ends_with("/tmp/work"));
    }
}
