//! 侧车 app-server：和桌面共用 CODEX_HOME。两种用法：
//! - 只读：会话列表、历史、建会话（不发起推理）；
//! - 后台执行（[`super::headless`]）：`thread/resume` + `turn/start` 自己跑一轮，审批经 [`Sidecar::events`]
//!   交给调用方，回答用 [`Sidecar::respond_server_request`]。
//!
//! 写锁：一个会话同一时间只能有一个 app-server 持有写入权。侧车 `thread/start`（或 `thread/resume`）之后就持有了，
//! 桌面再 `thread/resume` 会报「thread … already has an active writer」，打不开这个会话。
//! 所以侧车以 `thread_unload_delay_secs=0` 启动，跑完一轮就 `thread/unsubscribe` 并等它卸载；
//! `thread/read` / `thread/list` 不会占锁（P0 实测）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, broadcast, oneshot};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Sidecar {
    child: std::sync::Mutex<Option<Child>>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<Value>>>>,
    /// 所有服务端推来的消息（通知 + 审批等请求）。后台执行方在这里认领自己会话的消息。
    events: broadcast::Sender<Value>,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
}

impl Sidecar {
    pub async fn start(codex_bin: &Path, codex_home: &Path) -> anyhow::Result<Arc<Self>> {
        let mut command = Command::new(codex_bin);
        command
            .args(["-c", "thread_unload_delay_secs=0", "app-server"])
            .env("CODEX_HOME", codex_home)
            .env("RUST_LOG", "error")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = command
            .spawn()
            .with_context(|| format!("启动侧车 app-server 失败：{}", codex_bin.display()))?;
        let stdin = child.stdin.take().context("侧车没有 stdin")?;
        let stdout = child.stdout.take().context("侧车没有 stdout")?;
        let sidecar = Arc::new(Self {
            child: std::sync::Mutex::new(Some(child)),
            stdin: Mutex::new(stdin),
            pending: Arc::new(std::sync::Mutex::new(HashMap::new())),
            events: broadcast::channel(1024).0,
            next_id: AtomicU64::new(1),
            alive: Arc::new(AtomicBool::new(true)),
        });
        tokio::spawn(read_loop(BufReader::new(stdout), Arc::downgrade(&sidecar)));
        sidecar
            .call(
                "initialize",
                json!({
                    "clientInfo": { "name": "codex-plus-remote", "version": crate::version::VERSION },
                    "capabilities": { "experimentalApi": true },
                }),
            )
            .await
            .context("侧车 initialize 失败")?;
        sidecar.notify("initialized", json!({})).await?;
        Ok(sidecar)
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// 订阅服务端推送：`{"method","params"}` 通知，或 `{"id","method","params"}` 请求（审批、提问等）。
    pub fn events(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    /// 回答一个服务端请求（审批、提问）。`id` 取自请求里的 `id`。
    pub async fn respond_server_request(&self, id: &Value, result: Value) -> anyhow::Result<()> {
        self.write_line(json!({ "jsonrpc": "2.0", "id": id, "result": result })).await
    }

    /// 回绝一个没人处理的服务端请求，免得那一轮一直等。
    pub async fn reject_server_request(&self, id: &Value, message: &str) -> anyhow::Result<()> {
        self.write_line(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": message } })).await
    }

    async fn write_line(&self, message: Value) -> anyhow::Result<()> {
        let line = format!("{message}\n");
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let line = format!("{}\n", json!({ "jsonrpc": "2.0", "method": method, "params": params }));
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// 发请求，返回 `result`；app-server 报错时返回错误。
    pub async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        if !self.is_alive() {
            bail!("侧车 app-server 已退出");
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, sender);
        let line = format!("{}\n", json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await?;
        }
        let message = match tokio::time::timeout(REQUEST_TIMEOUT, receiver).await {
            Ok(Ok(message)) => message,
            Ok(Err(_)) => bail!("侧车 app-server 已退出"),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                bail!("侧车 {method} 超时");
            }
        };
        if let Some(error) = message.get("error") {
            let text = error.get("message").and_then(Value::as_str).unwrap_or("未知错误");
            bail!("{method} 失败：{text}");
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn list_threads(&self, cursor: Option<&str>, limit: u32) -> anyhow::Result<Value> {
        let mut params = json!({ "limit": limit });
        if let Some(cursor) = cursor {
            params["cursor"] = json!(cursor);
        }
        self.call("thread/list", params).await
    }

    pub async fn read_thread(&self, thread_id: &str) -> anyhow::Result<Value> {
        let result = self.call("thread/read", json!({ "threadId": thread_id, "includeTurns": true })).await?;
        Ok(result.get("thread").cloned().unwrap_or(Value::Null))
    }

    pub async fn is_loaded(&self, thread_id: &str) -> anyhow::Result<bool> {
        let loaded = self.call("thread/loaded/list", json!({})).await?;
        Ok(loaded
            .get("data")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(thread_id))))
    }

    /// 放掉写锁：unsubscribe 后等会话卸载。
    pub async fn release(&self, thread_id: &str) -> anyhow::Result<()> {
        self.call("thread/unsubscribe", json!({ "threadId": thread_id })).await?;
        for _ in 0..50 {
            if !self.is_loaded(thread_id).await? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        bail!("侧车 5 秒内没有卸载会话 {thread_id}")
    }

    /// 新建会话并落盘（`thread/name/set` 会写出 rollout，且不往历史塞假消息），最后放掉写锁。
    pub async fn create_thread(&self, cwd: &str, name: &str) -> anyhow::Result<String> {
        let started = self.call("thread/start", json!({ "cwd": cwd })).await?;
        let thread_id = started
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .context("thread/start 没有返回会话 ID")?
            .to_string();
        self.call("thread/name/set", json!({ "threadId": thread_id, "name": name })).await?;
        self.release(&thread_id).await?;
        Ok(thread_id)
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::SeqCst);
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn read_loop(stdout: BufReader<tokio::process::ChildStdout>, sidecar: std::sync::Weak<Sidecar>) {
    let mut lines = stdout.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(sidecar) = sidecar.upgrade() else { return };
        let id = message.get("id").and_then(Value::as_u64);
        let is_server_request = message.get("method").is_some();
        match (id, is_server_request) {
            (Some(id), false) => {
                let sender = sidecar.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                if let Some(sender) = sender {
                    let _ = sender.send(message);
                }
            }
            // 通知与请求都交给订阅方（后台执行）。没人认领的请求由订阅方统一回绝，避免这一轮挂住。
            (_, true) => {
                let _ = sidecar.events.send(message);
            }
            _ => {}
        }
    }
    if let Some(sidecar) = sidecar.upgrade() {
        sidecar.alive.store(false, Ordering::SeqCst);
        sidecar.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// 找一个能跑 `app-server` 的 Codex CLI：优先已安装桌面版里自带的那份。
pub fn resolve_codex_cli(app_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(windows) {
        candidates.extend(crate::app_paths::find_desktop_managed_codex_cli());
    }
    if let Some(app_dir) = app_dir {
        if app_dir.extension().is_some_and(|extension| extension == "app") {
            let resources = app_dir.join("Contents").join("Resources");
            candidates.push(resources.join("codex-cli/bin/codex"));
            candidates.push(resources.join("codex-cli/CodexCLI.app/Contents/MacOS/codex"));
            candidates.push(resources.join("codex"));
        } else {
            candidates.extend(crate::app_paths::find_bundled_codex_cli(app_dir));
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}
