pub mod app_server;
pub mod session_store;
pub mod weixin;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

use self::app_server::{AppServerConfig, AppServerTurnResult, CodexAppServer};
use self::session_store::{ConnectSessionStore, ConnectState};
use self::weixin::{WeixinClient, WeixinMessage};

pub const DEFAULT_WEIXIN_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const MAX_INBOUND_MESSAGE_AGE_MS: u64 = 5 * 60 * 1_000;

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
    pub route_tag: String,
    #[serde(default)]
    pub work_dir: String,
    #[serde(default)]
    pub model: String,
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
            route_tag: String::new(),
            work_dir: String::new(),
            model: String::new(),
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

pub async fn run_weixin_connect(
    config: WeixinConnectConfig,
    stop: Arc<AtomicBool>,
    status: SharedWeixinConnectStatus,
) -> anyhow::Result<()> {
    let config = config.normalized();
    if config.token.is_empty() {
        bail!("请先扫码登录微信");
    }
    let work_dir = if config.work_dir.is_empty() {
        std::env::current_dir().context("无法读取当前工作目录")?
    } else {
        PathBuf::from(&config.work_dir)
    };
    if !work_dir.is_dir() {
        bail!("工作目录不存在：{}", work_dir.display());
    }

    update_status(&status, |current| {
        current.state = "starting".to_string();
        current.message = "正在连接微信和 Codex app-server...".to_string();
        current.account_id = config.account_id.clone();
        current.has_token = true;
    });

    let client = WeixinClient::new(&config.base_url, &config.token, &config.route_tag)?;
    let store = ConnectSessionStore::default_for_account(&config.account_id);
    let mut state = store.load().unwrap_or_default();
    let app_config = AppServerConfig {
        executable: config.codex_path.clone(),
        work_dir,
        model: config.model.clone(),
        sandbox: config.sandbox.clone(),
    };
    let mut app_server: Option<CodexAppServer> = None;
    let mut long_poll_timeout_ms = 35_000;

    update_status(&status, |current| {
        current.state = "running".to_string();
        current.message = "微信连接正在运行。".to_string();
    });

    while !stop.load(Ordering::SeqCst) {
        let updates = match client
            .get_updates(&state.get_updates_buf, long_poll_timeout_ms)
            .await
        {
            Ok(updates) => updates,
            Err(error) => {
                update_status(&status, |current| {
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
                || !is_allowed_peer(&config.allow_from, &message.from_user_id)
            {
                continue;
            }
            let Some(text) = message.text() else {
                state.mark_processed(&message_key);
                continue;
            };
            if message.context_token.trim().is_empty() {
                update_status(&status, |current| {
                    current.state = "running".to_string();
                    current.message = "收到缺少 context_token 的微信消息，已忽略。".to_string();
                });
                state.mark_processed(&message_key);
                continue;
            }

            state
                .context_tokens
                .insert(message.from_user_id.clone(), message.context_token.clone());
            let result = process_weixin_message(
                &client,
                &app_config,
                &mut app_server,
                &mut state,
                &message,
                &text,
                &stop,
            )
            .await;

            if stop.load(Ordering::SeqCst) {
                app_server = None;
                break;
            }

            match result {
                Ok(()) => {
                    state.mark_processed(&message_key);
                    update_status(&status, |current| {
                        current.state = "running".to_string();
                        current.message = "最近一条微信消息已处理。".to_string();
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
                            "Codex 处理失败，请在 Codex++ 管理器中查看连接状态。",
                            &message.context_token,
                        )
                        .await;
                    update_status(&status, |current| {
                        current.state = "error".to_string();
                        current.message = format!("处理微信消息失败：{error}");
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
    update_status(&status, |current| {
        current.state = "stopped".to_string();
        current.message = "微信连接已停止。".to_string();
    });
    Ok(())
}

async fn process_weixin_message(
    client: &WeixinClient,
    app_config: &AppServerConfig,
    app_server: &mut Option<CodexAppServer>,
    state: &mut ConnectState,
    message: &WeixinMessage,
    text: &str,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    if app_server
        .as_ref()
        .map(|server| !server.is_running())
        .unwrap_or(true)
    {
        *app_server = Some(CodexAppServer::start(app_config.clone()).await?);
    }
    let server = app_server.as_mut().context("Codex app-server 未启动")?;
    let saved_thread_id = state.thread_ids.get(&message.from_user_id).cloned();
    let thread_id = match server.prepare_thread(saved_thread_id.as_deref()).await {
        Ok(thread_id) => thread_id,
        Err(error) if saved_thread_id.is_some() => {
            state.thread_ids.remove(&message.from_user_id);
            server
                .prepare_thread(None)
                .await
                .with_context(|| format!("恢复原会话失败（{error}），新建会话也失败"))?
        }
        Err(error) => return Err(error),
    };
    state
        .thread_ids
        .insert(message.from_user_id.clone(), thread_id.clone());

    let turn = server.run_turn(&thread_id, text);
    tokio::pin!(turn);
    let turn_result = loop {
        tokio::select! {
            reply = &mut turn => break reply?,
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                if stop.load(Ordering::SeqCst) {
                    bail!("微信连接已停止");
                }
            }
        }
    };
    let reply = if turn_result.reply.trim().is_empty() {
        "Codex 已完成处理，但没有返回文字内容。"
    } else {
        turn_result.reply.trim()
    };
    let reply_with_footer = append_reply_footer(reply, &turn_result, &app_config.work_dir);
    client
        .send_text_chunks(
            &message.from_user_id,
            &reply_with_footer,
            &message.context_token,
        )
        .await
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

fn is_allowed_peer(allow_from: &str, peer: &str) -> bool {
    let allow_from = allow_from.trim();
    allow_from.is_empty()
        || allow_from == "*"
        || allow_from
            .split(',')
            .map(str::trim)
            .any(|allowed| !allowed.is_empty() && allowed == peer)
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
    update: impl FnOnce(&mut WeixinConnectStatus),
) {
    if let Ok(mut current) = status.lock() {
        update(&mut current);
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
    fn allow_from_supports_wildcard_and_comma_separated_ids() {
        assert!(is_allowed_peer("", "a@im.wechat"));
        assert!(is_allowed_peer("*", "a@im.wechat"));
        assert!(is_allowed_peer("a@im.wechat, b@im.wechat", "b@im.wechat"));
        assert!(!is_allowed_peer("a@im.wechat", "b@im.wechat"));
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
