//! 拓展的常驻后台进程：拓展包在 `codexpp.json` 里声明 `backend.script`，页面侧经 `ctx.backend`
//! 调用它导出的方法、接收它推送的事件。
//!
//! 进程由处理桥接请求的 Codex++ 进程（启动器）托管：首次调用时用 Node 启动，之后常驻。
//! 热重载、启停拓展多半发生在别的进程（管理器、拓展 MCP 直接经 CDP 重载），所以停后台靠两条路：
//! 页面上的拓展实例卸载时经桥接发 `stop`（带实例标识，不误停新实例刚拉起的后台）；每次调用前
//! 再核对脚本、配置、启用状态，过期就重起、停用就停掉。页面整体重新加载时启动器直接全部停掉。
//! 与后台之间是 stdin / stdout 上的行式 JSON：
//! 请求 `{ id, method, params }`，回应 `{ id, result }` / `{ id, error }`，事件 `{ event, payload }`。
//! 事件进有界队列，页面以长轮询取走，core 不需要持有页面连接。

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Notify, oneshot};

use crate::user_scripts::UserScriptManager;

/// 每个后台保留的未取走事件上限，超出时丢最旧的。
const EVENT_QUEUE_LIMIT: usize = 200;
/// 方法调用的默认超时；略短于桥接自身的 26 秒。
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(20);
/// 事件长轮询最长等待。
pub const MAX_EVENT_WAIT: Duration = Duration::from_secs(20);

/// 运行器：第一行读初始化参数，import 后台脚本并调用默认导出拿到方法表，之后逐行处理请求。
const RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
import readline from "node:readline";
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const lines = readline.createInterface({ input: process.stdin });
let methods = null;
let ready = null;
// 向 Codex++ 发起宿主调用（例如只读会话查询），回复按 hostReply 对应。
const hostPending = new Map();
let hostSeq = 0;
const host = (service, method, params) => new Promise((resolve, reject) => {
  const id = ++hostSeq;
  hostPending.set(id, { resolve, reject });
  write({ host: id, service, method, params: params ?? null });
});
const withId = (method) => (id, options) => host("sessionQuery", method, { id, options: options ?? {} });
const sessionQuery = Object.freeze({
  listSessions: (options) => host("sessionQuery", "listSessions", options ?? {}),
  filterSessions: (filters) => host("sessionQuery", "filterSessions", filters ?? {}),
  readSession: (id) => host("sessionQuery", "readSession", { id }),
  readTitle: (id) => host("sessionQuery", "readTitle", { id }),
  listEvents: withId("listEvents"),
  filterEvents: withId("filterEvents"),
  readEvent: (query) => host("sessionQuery", "readEvent", query ?? {}),
  searchSessions: (query) => host("sessionQuery", "searchSessions", query ?? {}),
  searchEvents: (query) => host("sessionQuery", "searchEvents", query ?? {}),
});
lines.on("line", async (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.hostReply !== undefined) {
    const waiter = hostPending.get(message.hostReply);
    hostPending.delete(message.hostReply);
    if (message.error !== undefined) waiter?.reject(new Error(message.error)); else waiter?.resolve(message.result);
    return;
  }
  if (!ready) {
    ready = (async () => {
      try {
        const module = await import(pathToFileURL(message.script).href);
        const create = typeof module.default === "function" ? module.default : null;
        if (!create) throw new Error(`${message.script} 没有默认导出函数`);
        methods = await create({
          extension: message.extension,
          config: message.config ?? {},
          dataDir: message.dataDir,
          emit: (event, payload) => write({ event: String(event), payload: payload ?? null }),
          sessionQuery,
          log: (...args) => process.stderr.write(args.map(String).join(" ") + "\n"),
        });
        write({ ready: true, methods: Object.keys(methods ?? {}).filter((name) => typeof methods[name] === "function") });
      } catch (error) {
        write({ fatal: String(error?.stack || error?.message || error) });
        process.exit(1);
      }
    })();
    return;
  }
  await ready;
  const { id, method, params } = message;
  try {
    const fn = methods?.[method];
    if (typeof fn !== "function") throw new Error(`后台没有方法 ${method}`);
    const result = await fn(params ?? null);
    write({ id, result: result === undefined ? null : result });
  } catch (error) {
    write({ id, error: String(error?.stack || error?.message || error) });
  }
});
lines.on("close", () => process.exit(0));
"#;

struct Backend {
    /// 与读取任务共享：宿主调用的回复也从这里写回后台。
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    events: Arc<Mutex<VecDeque<(u64, Value)>>>,
    notify: Arc<Notify>,
    next_id: u64,
    /// 启动时的脚本、配置指纹；变了说明拓展被改过，要重起。
    fingerprint: String,
    /// 最近一次调用来自哪个页面实例；旧实例卸载时发来的 stop 只在它仍是最近调用方时生效。
    instance: Option<String>,
    _child: tokio::process::Child,
}

static BACKENDS: std::sync::LazyLock<Mutex<HashMap<String, Backend>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static EVENT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 拓展后台的可写数据目录：`<用户脚本配置目录>/extension-data/<拓展 id>`。
fn data_dir(manager: &UserScriptManager, extension: &str) -> PathBuf {
    manager
        .user_dir()
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| manager.user_dir().to_path_buf())
        .join("extension-data")
        .join(extension)
}

/// 后台脚本路径、修改时间与生效配置的指纹。
fn fingerprint(script: &std::path::Path, config: &Value) -> String {
    let modified = std::fs::metadata(script)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_nanos())
        .unwrap_or_default();
    format!("{}|{modified}|{config}", script.display())
}

async fn spawn(manager: &UserScriptManager, (extension, script, config): (String, PathBuf, Value)) -> anyhow::Result<Backend> {
    let fingerprint = fingerprint(&script, &config);
    let node = crate::extension_node::require_node()?;
    let data = data_dir(manager, &extension);
    std::fs::create_dir_all(&data).with_context(|| format!("无法创建拓展数据目录 {}", data.display()))?;
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", RUNNER])
        .current_dir(&data)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // Windows 上不能闪出黑色终端窗口。
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().context("无法启动拓展后台")?;
    let stdin = Arc::new(Mutex::new(child.stdin.take().context("拓展后台没有 stdin")?));
    let stdout = child.stdout.take().context("拓展后台没有 stdout")?;
    let init = json!({ "script": script, "extension": extension, "config": config, "dataDir": data });
    {
        let mut writer = stdin.lock().await;
        writer.write_all(format!("{init}\n").as_bytes()).await?;
        writer.flush().await?;
    }

    let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> = Arc::default();
    let events: Arc<Mutex<VecDeque<(u64, Value)>>> = Arc::default();
    let notify = Arc::new(Notify::new());
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();
    {
        let pending = pending.clone();
        let events = events.clone();
        let notify = notify.clone();
        let host_stdin = stdin.clone();
        let mut ready_tx = Some(ready_tx);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(host_id) = message.get("host").and_then(Value::as_u64) {
                    // 宿主调用另起任务处理，不挡住后台的其它输出。
                    let host_stdin = host_stdin.clone();
                    tokio::spawn(async move {
                        let reply = match host_call(&message).await {
                            Ok(result) => json!({ "hostReply": host_id, "result": result }),
                            Err(error) => json!({ "hostReply": host_id, "error": format!("{error:#}") }),
                        };
                        let mut writer = host_stdin.lock().await;
                        let _ = writer.write_all(format!("{reply}\n").as_bytes()).await;
                        let _ = writer.flush().await;
                    });
                } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Some(sender) = pending.lock().await.remove(&id) {
                        let outcome = match message.get("error").and_then(Value::as_str) {
                            Some(error) => Err(error.to_string()),
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = sender.send(outcome);
                    }
                } else if let Some(event) = message.get("event").and_then(Value::as_str) {
                    let seq = EVENT_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut queue = events.lock().await;
                    queue.push_back((seq, json!({ "seq": seq, "event": event, "payload": message.get("payload").cloned().unwrap_or(Value::Null) })));
                    while queue.len() > EVENT_QUEUE_LIMIT {
                        queue.pop_front();
                    }
                    drop(queue);
                    notify.notify_waiters();
                } else if message.get("ready").is_some() {
                    if let Some(sender) = ready_tx.take() {
                        let _ = sender.send(Ok(()));
                    }
                } else if let Some(fatal) = message.get("fatal").and_then(Value::as_str) {
                    if let Some(sender) = ready_tx.take() {
                        let _ = sender.send(Err(fatal.to_string()));
                    }
                }
            }
            // 进程退出：让还在等的调用立即失败，而不是等到超时。
            for (_, sender) in pending.lock().await.drain() {
                let _ = sender.send(Err("拓展后台已退出".to_string()));
            }
        });
    }
    match tokio::time::timeout(Duration::from_secs(15), ready_rx).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(fatal))) => anyhow::bail!("拓展后台启动失败：{fatal}"),
        Ok(Err(_)) => anyhow::bail!("拓展后台启动后立即退出"),
        Err(_) => anyhow::bail!("拓展后台 15 秒内没有就绪"),
    }
    Ok(Backend { stdin, pending, events, notify, next_id: 1, fingerprint, instance: None, _child: child })
}

/// 后台发起的宿主调用。目前只开放只读会话查询（sessionQuery）。
pub(crate) async fn host_call(message: &Value) -> anyhow::Result<Value> {
    let service = message.get("service").and_then(Value::as_str).unwrap_or_default();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match service {
        "sessionQuery" => {
            let home = crate::codex_sqlite::default_codex_home_dir();
            tokio::task::spawn_blocking(move || crate::extension_history::call(&home, &method, &params)).await?
        }
        other => anyhow::bail!("后台不能调用服务 {other}"),
    }
}

/// 调用一个拓展后台的方法；后台没起就先起。后台退出过、或拓展改过的会被重新拉起，
/// 拓展已停用的会被停掉。`instance` 是发起调用的页面实例标识（可省略）。
pub async fn call(
    manager: &UserScriptManager,
    key: &str,
    method: &str,
    params: Value,
    timeout: Duration,
    instance: Option<&str>,
) -> anyhow::Result<Value> {
    if method.is_empty() || method.len() > 128 {
        anyhow::bail!("方法名无效");
    }
    let receiver = {
        let mut backends = BACKENDS.lock().await;
        let declared = match manager.enabled_extension_backend(key) {
            Ok(declared) => declared,
            Err(error) => {
                backends.remove(key);
                return Err(error);
            }
        };
        if backends.get(key).is_some_and(|backend| backend.fingerprint != fingerprint(&declared.1, &declared.2)) {
            backends.remove(key);
        }
        if !backends.contains_key(key) {
            let backend = spawn(manager, declared).await?;
            backends.insert(key.to_string(), backend);
        }
        let backend = backends.get_mut(key).expect("刚插入");
        if let Some(instance) = instance {
            backend.instance = Some(instance.to_string());
        }
        let id = backend.next_id;
        backend.next_id += 1;
        let (sender, receiver) = oneshot::channel();
        backend.pending.lock().await.insert(id, sender);
        let request = json!({ "id": id, "method": method, "params": params });
        let written = {
            let mut writer = backend.stdin.lock().await;
            writer.write_all(format!("{request}\n").as_bytes()).await.is_ok() && writer.flush().await.is_ok()
        };
        if !written {
            // 进程已经没了：移除，下次调用重新拉起。
            backends.remove(key);
            anyhow::bail!("拓展后台已退出，请重试");
        }
        receiver
    };
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(error))) => {
            if error == "拓展后台已退出" {
                BACKENDS.lock().await.remove(key);
            }
            anyhow::bail!("{error}")
        }
        Ok(Err(_)) => anyhow::bail!("拓展后台没有回应"),
        Err(_) => anyhow::bail!("调用 {method} 超时"),
    }
}

/// 取走 `since` 之后的事件；没有就最多等 `wait`。后台没起时直接返回空（不为取事件而启动后台）。
pub async fn events(key: &str, since: u64, wait: Duration) -> Value {
    let wait = wait.min(MAX_EVENT_WAIT);
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let (queue, notify) = {
            let backends = BACKENDS.lock().await;
            let Some(backend) = backends.get(key) else {
                return json!({ "events": [], "next": since, "running": false });
            };
            (backend.events.clone(), backend.notify.clone())
        };
        let notified = notify.notified();
        let ready = queue
            .lock()
            .await
            .iter()
            .filter(|(seq, _)| *seq > since)
            .map(|(_, event)| event.clone())
            .collect::<Vec<_>>();
        if !ready.is_empty() {
            let next = ready.last().and_then(|event| event["seq"].as_u64()).unwrap_or(since);
            return json!({ "events": ready, "next": next, "running": true });
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            return json!({ "events": [], "next": since, "running": true });
        }
    }
}

/// 停掉全部后台（页面加载、热重载拓展时调用）。进程随 Child 释放被杀掉。
pub async fn stop_all() {
    BACKENDS.lock().await.clear();
}

/// 停掉一个拓展的后台。给了 `instance` 时，只在最近一次调用也来自它时才停：
/// 热重载时旧实例的 stop 可能晚于新实例的首次调用到达，不能把新后台停了。返回是否停掉。
pub async fn stop(key: &str, instance: Option<&str>) -> bool {
    let mut backends = BACKENDS.lock().await;
    let matches = match (backends.get(key), instance) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(backend), Some(instance)) => backend.instance.as_deref().is_none_or(|last| last == instance),
    };
    if matches {
        backends.remove(key);
    }
    matches
}

/// 桥接路由 `/extensions/backend/call`：`{ key, method, params, timeoutMs? }`。
pub async fn route_call(payload: &Value) -> anyhow::Result<Value> {
    let key = payload.get("key").and_then(Value::as_str).context("缺少拓展 key")?;
    let method = payload.get("method").and_then(Value::as_str).context("缺少方法名")?;
    let timeout = payload
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_CALL_TIMEOUT)
        .min(Duration::from_secs(24));
    let instance = payload.get("instance").and_then(Value::as_str);
    let manager = crate::user_scripts::default_user_script_manager();
    let result = call(&manager, key, method, payload.get("params").cloned().unwrap_or(Value::Null), timeout, instance).await?;
    Ok(json!({ "status": "ok", "result": result }))
}

/// 桥接路由 `/extensions/backend/stop`：`{ key, instance? }`，页面上的拓展实例卸载时发。
pub async fn route_stop(payload: &Value) -> anyhow::Result<Value> {
    let key = payload.get("key").and_then(Value::as_str).context("缺少拓展 key")?;
    let stopped = stop(key, payload.get("instance").and_then(Value::as_str)).await;
    Ok(json!({ "status": "ok", "stopped": stopped }))
}

/// 桥接路由 `/extensions/backend/events`：`{ key, since, waitMs? }`。
pub async fn route_events(payload: &Value) -> anyhow::Result<Value> {
    let key = payload.get("key").and_then(Value::as_str).context("缺少拓展 key")?;
    let since = payload.get("since").and_then(Value::as_u64).unwrap_or(0);
    let wait = payload
        .get("waitMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(MAX_EVENT_WAIT);
    let mut result = events(key, since, wait).await;
    result["status"] = json!("ok");
    Ok(result)
}
