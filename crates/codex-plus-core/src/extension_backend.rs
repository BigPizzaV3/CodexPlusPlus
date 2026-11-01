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
import fs from "node:fs";
const lines = readline.createInterface({ input: process.stdin });
let methods = null;
let ready = null;
let dataDir = null;
const provided = new Map();
/*HOST_CLIENT*/
lines.on("line", async (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (handleHostReply(message)) return;
  if (!ready) {
    dataDir = message.dataDir;
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
          sessions,
          // 调模型：llm.complete({ input, model?, instructions? }) → { text, model, usage }；llm.listModels()。
          llm,
          // 问用户 / 提示：ui.ask({ title, message, options?, input? })、ui.toast(文字)，在 Codex 页面里弹。
          ui,
          // 提供服务：名字要在 codexpp.json 的 backend.provides 里声明，别处才找得到。
          provide: (name, impl) => { provided.set(String(name), impl); },
          // 调用任一拓展后台提供的服务（包括自己的）：service("名字").方法(...参数)。
          service,
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
    // 定时任务：宿主每分钟发一次 { schedules: [{ method, everyMinutes?, at? }] }，到点的就调。
    // 上次运行时间记在数据目录，重启后不会重复补跑；at 按本机时区判断。
    if (method === "__tick") {
      const statePath = `${dataDir}/.schedules.json`;
      let state = {};
      try { state = JSON.parse(fs.readFileSync(statePath, "utf8")); } catch {}
      const now = Date.now();
      const ran = [];
      for (const schedule of params?.schedules ?? []) {
        const key = `${schedule.method}|${schedule.everyMinutes ?? ""}|${schedule.at ?? ""}`;
        const last = Number(state[key]) || 0;
        let due = false;
        if (schedule.everyMinutes) {
          if (!last) state[key] = now;
          // 调度本身每 60 秒一次，会有几毫秒到几秒的抖动：留 5 秒余量，免得「每分钟」变成每两分钟。
          else due = now - last >= schedule.everyMinutes * 60000 - 5000;
        } else if (schedule.at) {
          const [hour, minute] = schedule.at.split(":").map(Number);
          const target = new Date(now);
          target.setHours(hour, minute, 0, 0);
          due = now >= target.getTime() && last < target.getTime() && now - target.getTime() < 6 * 3600000;
          if (!last && !due) state[key] = now;
        }
        if (!due) continue;
        state[key] = now;
        const fn = methods?.[schedule.method];
        try {
          if (typeof fn !== "function") throw new Error(`后台没有方法 ${schedule.method}`);
          await fn({ scheduledAt: now, schedule });
          ran.push({ method: schedule.method });
        } catch (error) {
          ran.push({ method: schedule.method, error: String(error?.stack || error?.message || error) });
        }
      }
      try { fs.writeFileSync(statePath, JSON.stringify(state)); } catch {}
      write({ id, result: { ran } });
      return;
    }
    // 别处按服务名调来的方法：{ name, method, args }，分派到 ctx.provide 登记的实现。
    if (method === "__service") {
      const impl = provided.get(params?.name);
      if (!impl) throw new Error(`后台没有提供服务 ${params?.name}（请在默认导出里调用 ctx.provide）`);
      const target = impl[params?.method];
      if (typeof target !== "function") throw new Error(`服务 ${params?.name} 没有方法 ${params?.method}`);
      const result = await target.apply(impl, Array.isArray(params?.args) ? params.args : []);
      write({ id, result: result === undefined ? null : result });
      return;
    }
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
        .args(["--input-type=module", "-e", &crate::extension_node::with_host_client(RUNNER)])
        .current_dir(&data)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    set_host_exe_env(&mut command);
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
        // 后台的宿主调用（例如调用别的后台服务）沿用启动它的那个拓展目录。
        let host_manager = manager.clone();
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
                    let host_manager = host_manager.clone();
                    tokio::spawn(async move {
                        let reply = match host_call(&message, &host_manager).await {
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

/// 后台、模型工具发起的宿主调用：只读会话查询（sessionQuery），以及按名字调用后台服务（services.call）。
///
/// 返回装箱的 Send future：服务调用会经 `call` 再回到这里（后台调用后台），递归的 async 需要显式装箱。
pub(crate) fn host_call<'a>(
    message: &'a Value,
    manager: &'a UserScriptManager,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<Value>> + Send + 'a>> {
    Box::pin(host_call_inner(message, manager))
}

async fn host_call_inner(message: &Value, manager: &UserScriptManager) -> anyhow::Result<Value> {
    let service = message.get("service").and_then(Value::as_str).unwrap_or_default().to_string();
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    match (service.as_str(), method.as_str()) {
        ("sessionQuery", _) => {
            let home = crate::codex_sqlite::default_codex_home_dir();
            tokio::task::spawn_blocking(move || crate::extension_history::call(&home, &method, &params)).await?
        }
        ("services", "call") => {
            // 后台都托管在处理桥接的那个进程里。本进程不是它（拓展 MCP、钩子进程）时经 Codex 页面的桥接转过去，
            // 否则同一个后台会在两个进程里各起一份，状态对不上。
            if let Some(ports) = REMOTE_BRIDGE_PORTS.get() {
                let result = remote_bridge_call(ports, "/extensions/services/call", &params).await?;
                return Ok(result.get("result").cloned().unwrap_or(Value::Null));
            }
            call_service(manager, &params).await
        }
        ("sessions", "list" | "create" | "fork" | "rename" | "append" | "startTurn" | "steer" | "interrupt" | "compact") => {
            // 会话写入经 Codex 页面里的 app-server 连接完成：在页面里调 __codexPlusSessionsCall。
            let ports = REMOTE_BRIDGE_PORTS.get().cloned().unwrap_or_else(default_debug_ports);
            page_call(&ports, "__codexPlusSessionsCall", &json!(method), &params).await
        }
        ("ui", "ask" | "toast") => {
            // 问用户 / 提示：在 Codex 页面里弹，等用户回答（提问最长等 10 分钟）。
            let ports = REMOTE_BRIDGE_PORTS.get().cloned().unwrap_or_else(default_debug_ports);
            page_call(&ports, "__codexPlusUiCall", &json!(method), &params).await
        }
        // stream 只有命令行入口（[`run_host_call_cli`]）能边收边吐；别的通道一次性返回同样的结果。
        ("llm", "complete" | "stream") => crate::extension_llm::complete(&params).await,
        ("llm", "listModels") => crate::extension_llm::list_models(),
        _ => anyhow::bail!("不能调用 {service}.{method}"),
    }
}

/// Codex 页面的调试端口：最近一次启动记录（本实例与常规安装），再补默认端口。
fn default_debug_ports() -> Vec<u16> {
    let mut ports = Vec::new();
    let mut records = vec![crate::paths::default_app_state_dir().join("latest-status.json")];
    if let Some(home) = std::env::var_os("HOME") {
        records.push(PathBuf::from(home).join(".codex-session-delete/latest-status.json"));
    }
    for path in records {
        if let Ok(Some(status)) = crate::status::StatusStore::new(path).load_latest() {
            if let Some(port) = status.debug_port.filter(|port| *port > 0 && !ports.contains(port)) {
                ports.push(port);
            }
        }
    }
    if !ports.contains(&9229) {
        ports.push(9229);
    }
    ports
}

/// 在 Codex 页面里调用一个全局函数 `window[function](name, params)`，返回它的结果。
async fn page_call(ports: &[u16], function: &str, name: &Value, params: &Value) -> anyhow::Result<Value> {
    let mut websocket = None;
    for port in ports {
        let Ok(targets) = crate::cdp::list_targets(*port).await else {
            continue;
        };
        if let Some(url) = crate::cdp::pick_injectable_codex_page_target(&targets).ok().and_then(|target| target.web_socket_debugger_url) {
            websocket = Some(url);
            break;
        }
    }
    let websocket = websocket.context("没有找到已连接的 Codex 页面")?;
    let expression = format!(
        "(async () => {{ if (typeof window.{function} !== \"function\") throw new Error(\"Codex 页面还没装好 Codex++ 拓展运行时\"); return JSON.stringify(await window.{function}({name}, {params})); }})()"
    );
    // 提问要等用户回答：按它的 timeoutMs 多给 5 秒；其余页面调用（会话写入）给 30 秒。
    let timeout = if function == "__codexPlusUiCall" && name == "ask" {
        Duration::from_millis(params.get("timeoutMs").and_then(Value::as_u64).unwrap_or(600_000).min(600_000) + 5_000)
    } else {
        Duration::from_secs(30)
    };
    let response = crate::bridge::evaluate_script_with_timeout(&websocket, &expression, timeout).await?;
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        let message = exception
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| exception.get("text").and_then(Value::as_str))
            .unwrap_or("未知错误");
        anyhow::bail!("{}", message.lines().next().unwrap_or(message));
    }
    let text = response.pointer("/result/result/value").and_then(Value::as_str).unwrap_or("null");
    Ok(serde_json::from_str(text).unwrap_or(Value::Null))
}

/// 不托管后台的进程（拓展 MCP、钩子）设置：服务调用经这些 CDP 调试端口上的 Codex 页面桥接转发。
static REMOTE_BRIDGE_PORTS: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();

pub fn set_remote_bridge_ports(ports: Vec<u16>) {
    let _ = REMOTE_BRIDGE_PORTS.set(ports);
}

/// 宿主调用的命令行入口参数：`CodexPlusPlus --codex-plus-host-call`，stdin 一个 `{ service, method, params }`，
/// stdout 一行 `{ result }` 或 `{ error }`。给不经运行器起的 Node 进程用（dsh 插件 MCP、钩子常驻进程）。
pub const HOST_CALL_FLAG: &str = "--codex-plus-host-call";
/// Codex++ 起 Node 进程时设置：指向 Codex++ 可执行文件，Node 侧据此发起宿主调用。
pub const HOST_EXE_ENV: &str = "CODEX_PLUS_HOST_EXE";
const MAX_HOST_CALL_BYTES: u64 = 8 * 1024 * 1024;

/// 给 Node 子进程设置 [`HOST_EXE_ENV`]（取当前可执行文件；拿不到就不设，Node 侧报「不可用」）。
pub fn set_host_exe_env(command: &mut tokio::process::Command) {
    if let Ok(exe) = std::env::current_exe() {
        command.env(HOST_EXE_ENV, exe);
    }
}

/// [`HOST_CALL_FLAG`] 的实现：读 stdin、调 [`host_call`]、把结果写到 stdout。总是返回 0，错误写进输出。
/// `llm.stream` 是行式流：每个文字 / 推理增量先写一行 `{ "delta": ... }`，最后一行是 `{ result }` 或 `{ error }`。
pub async fn run_host_call_cli(manager: &UserScriptManager) -> i32 {
    use std::io::Read;
    let mut raw = Vec::new();
    let read = std::io::stdin().take(MAX_HOST_CALL_BYTES + 1).read_to_end(&mut raw);
    let reply = match read {
        Ok(_) if raw.len() as u64 <= MAX_HOST_CALL_BYTES => match serde_json::from_slice::<Value>(&raw) {
            Ok(message) if message.get("service").and_then(Value::as_str) == Some("llm") && message.get("method").and_then(Value::as_str) == Some("stream") => {
                let params = message.get("params").cloned().unwrap_or(Value::Null);
                // Rust 的 stdout 按行刷新：每个增量写完就到了 Node 那边。
                match crate::extension_llm::stream(&params, |delta| println!("{}", json!({ "delta": delta }))).await {
                    Ok(result) => json!({ "result": result }),
                    Err(error) => json!({ "error": format!("{error:#}") }),
                }
            }
            Ok(message) => match host_call(&message, manager).await {
                Ok(result) => json!({ "result": result }),
                Err(error) => json!({ "error": format!("{error:#}") }),
            },
            Err(error) => json!({ "error": format!("宿主调用的输入不是 JSON：{error}") }),
        },
        _ => json!({ "error": "宿主调用的输入过大或读取失败" }),
    };
    println!("{reply}");
    0
}

/// 在 Codex 页面里调用桥接路由，返回路由结果（失败时报错）。
async fn remote_bridge_call(ports: &[u16], path: &str, payload: &Value) -> anyhow::Result<Value> {
    let mut websocket = None;
    for port in ports {
        let Ok(targets) = crate::cdp::list_targets(*port).await else {
            continue;
        };
        if let Some(url) = crate::cdp::pick_injectable_codex_page_target(&targets).ok().and_then(|target| target.web_socket_debugger_url) {
            websocket = Some(url);
            break;
        }
    }
    let websocket = websocket.context("没有找到已连接的 Codex 页面，无法调用拓展服务")?;
    let expression = format!(
        "(async () => JSON.stringify(await window.__codexSessionDeleteBridge({}, {})))()",
        json!(path),
        payload
    );
    let response = crate::bridge::evaluate_script_with_await_promise(&websocket, &expression, true).await?;
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        anyhow::bail!("页面桥接出错：{}", exception.get("text").and_then(Value::as_str).unwrap_or("未知错误"));
    }
    let text = response.pointer("/result/result/value").and_then(Value::as_str).context("页面桥接没有返回结果")?;
    let result: Value = serde_json::from_str(text)?;
    if result.get("status").and_then(Value::as_str) == Some("failed") || result.get("error").is_some() {
        anyhow::bail!("{}", result.get("message").or_else(|| result.get("error")).and_then(Value::as_str).unwrap_or("服务调用失败"));
    }
    Ok(result)
}

/// 扫描拓展目录的短缓存：服务调用、后台调用每次都要知道「谁启用着、脚本与配置是什么」，
/// 一轮对话里可能连续几十次，没必要每次都整轮扫描。改动最多 2 秒后生效。
const SCAN_CACHE_TTL: Duration = Duration::from_secs(2);
type ScanCache<T> = std::sync::LazyLock<std::sync::Mutex<HashMap<String, (std::time::Instant, T)>>>;
static BACKEND_SCAN: ScanCache<Result<(String, PathBuf, Value), String>> = std::sync::LazyLock::new(Default::default);
static SERVICE_SCAN: ScanCache<Vec<(String, String)>> = std::sync::LazyLock::new(Default::default);

fn cached<T: Clone>(cache: &ScanCache<T>, key: &str, load: impl FnOnce() -> T) -> T {
    if let Some((at, value)) = cache.lock().unwrap().get(key) {
        if at.elapsed() < SCAN_CACHE_TTL {
            return value.clone();
        }
    }
    let value = load();
    cache.lock().unwrap().insert(key.to_string(), (std::time::Instant::now(), value.clone()));
    value
}

fn declared_backend(manager: &UserScriptManager, key: &str) -> anyhow::Result<(String, PathBuf, Value)> {
    let cache_key = format!("{}\u{0}{key}", manager.config_path().display());
    cached(&BACKEND_SCAN, &cache_key, || manager.enabled_extension_backend(key).map_err(|error| error.to_string())).map_err(anyhow::Error::msg)
}

fn backend_services(manager: &UserScriptManager) -> anyhow::Result<Vec<(String, String)>> {
    let cache_key = manager.config_path().display().to_string();
    if let Some((at, value)) = SERVICE_SCAN.lock().unwrap().get(&cache_key) {
        if at.elapsed() < SCAN_CACHE_TTL {
            return Ok(value.clone());
        }
    }
    let value = manager.enabled_backend_services()?;
    SERVICE_SCAN.lock().unwrap().insert(cache_key, (std::time::Instant::now(), value.clone()));
    Ok(value)
}

/// 清掉扫描缓存（热重载、启停拓展后立即生效用）。
pub fn invalidate_scan_cache() {
    BACKEND_SCAN.lock().unwrap().clear();
    SERVICE_SCAN.lock().unwrap().clear();
}

/// 按服务名调用提供它的后台：`{ name, method, args, timeoutMs? }`。后台没起就先起。
pub async fn call_service(manager: &UserScriptManager, params: &Value) -> anyhow::Result<Value> {
    let name = params.get("name").and_then(Value::as_str).context("缺少服务名")?;
    let method = params.get("method").and_then(Value::as_str).filter(|method| !method.is_empty()).context("缺少服务方法名")?;
    let (_, key) = backend_services(manager)?
        .into_iter()
        .find(|(service, _)| service == name)
        .with_context(|| format!("没有拓展提供服务 {name}（需要在 codexpp.json 的 backend.provides 里声明）"))?;
    let timeout = params
        .get("timeoutMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_CALL_TIMEOUT)
        .min(Duration::from_secs(24));
    let args = params.get("args").cloned().filter(Value::is_array).unwrap_or_else(|| json!([]));
    call(manager, &key, "__service", json!({ "name": name, "method": method, "args": args }), timeout, None).await
}

/// 桥接路由 `/extensions/services/call`：`{ name, method, args, timeoutMs? }`。
pub async fn route_service_call(payload: &Value) -> anyhow::Result<Value> {
    let manager = crate::user_scripts::default_user_script_manager();
    let result = call_service(&manager, payload).await?;
    Ok(json!({ "status": "ok", "result": result }))
}

/// 正在启动中的后台：key → 启动完成的通知。启动（最多 15 秒等就绪）不占全局锁，
/// 否则一个后台在启动时调用别的后台服务会和全局锁互相等待。
static STARTING: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<Notify>>>> = std::sync::LazyLock::new(Default::default);

/// 启动方的收尾：无论启动成功、失败还是调用方被取消（future 被丢弃），都移除启动中标记并唤醒等待者，
/// 否则被取消后标记留着，之后每次调用都会等满 15 秒报「启动超时」。
struct StartingGuard {
    key: String,
    notify: Arc<Notify>,
}

impl Drop for StartingGuard {
    fn drop(&mut self) {
        STARTING.lock().unwrap().remove(&self.key);
        self.notify.notify_waiters();
    }
}

/// 确保 key 的后台在跑且与当前清单一致：过期的换掉，没起的由第一个调用方启动，其余调用方等它（最多 15 秒）。
async fn ensure_started(
    manager: &UserScriptManager,
    key: &str,
    declared: (String, PathBuf, Value),
    wanted: &str,
) -> anyhow::Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        {
            let mut backends = BACKENDS.lock().await;
            match backends.get(key) {
                Some(backend) if backend.fingerprint == wanted => return Ok(()),
                Some(_) => {
                    backends.remove(key);
                }
                None => {}
            }
        }
        let (notify, starter) = {
            let mut starting = STARTING.lock().unwrap();
            match starting.get(key) {
                Some(notify) => (notify.clone(), false),
                None => {
                    let notify = Arc::new(Notify::new());
                    starting.insert(key.to_string(), notify.clone());
                    (notify, true)
                }
            }
        };
        if starter {
            let _guard = StartingGuard { key: key.to_string(), notify };
            let backend = spawn(manager, declared).await?;
            BACKENDS.lock().await.insert(key.to_string(), backend);
            return Ok(());
        }
        // 别人正在启动：等它结束后再看一次（结束得早于开始等待时，下一轮直接看到结果）。
        let notified = notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if !STARTING.lock().unwrap().contains_key(key) {
            continue;
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            anyhow::bail!("拓展后台启动超时");
        }
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
    // 扫描拓展目录在锁外做；拓展已停用就停掉还在跑的后台。
    let declared = match declared_backend(manager, key) {
        Ok(declared) => declared,
        Err(error) => {
            BACKENDS.lock().await.remove(key);
            return Err(error);
        }
    };
    let wanted = fingerprint(&declared.1, &declared.2);
    ensure_started(manager, key, declared, &wanted).await?;
    let receiver = {
        let mut backends = BACKENDS.lock().await;
        let Some(backend) = backends.get_mut(key) else {
            anyhow::bail!("拓展后台已退出，请重试");
        };
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

/// 后台队列里最新事件的序号；后台没起时为 None。
async fn latest_seq(key: &str) -> Option<u64> {
    let queue = BACKENDS.lock().await.get(key)?.events.clone();
    let last = queue.lock().await.back().map(|(seq, _)| *seq).unwrap_or(0);
    Some(last)
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

/// 拓展的 HTTP 路由：`/ext/<拓展 id>/<路径>?<查询>` 交给该拓展后台导出的 `http(request)`。
/// 拓展要在 codexpp.json 里写 `backend.http: true` 才对外开放。返回 (状态码, Content-Type, 响应体)。
///
/// request：`{ method, path, query, headers, body }`（body 是文本；JSON 请求另给 `json`）。
/// 后台返回字符串当纯文本；返回 `{ status?, headers?: { "content-type" }, body?, json? }` 按字段组装；其余当 JSON。
pub async fn route_http(method: &str, raw_path: &str, raw_headers: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
    route_http_with(&crate::user_scripts::default_user_script_manager(), method, raw_path, raw_headers, body).await
}

pub async fn route_http_with(manager: &UserScriptManager, method: &str, raw_path: &str, raw_headers: &str, body: &[u8]) -> (u16, String, Vec<u8>) {
    let fail = |status: u16, message: String| (status, "application/json; charset=utf-8".to_string(), json!({ "error": message }).to_string().into_bytes());
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    let Some(rest) = path.strip_prefix("/ext/") else {
        return fail(404, "not found".into());
    };
    let (id, sub) = rest.split_once('/').map(|(id, sub)| (id, format!("/{sub}"))).unwrap_or((rest, "/".to_string()));
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return fail(404, "not found".into());
    }
    let key = crate::user_scripts::extension_package_key(id);
    if !manager.extension_backend_http(&key) {
        return fail(404, format!("extension {id} does not serve HTTP"));
    }
    let headers = raw_headers
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), Value::String(value.trim().to_string())))
        .collect::<serde_json::Map<_, _>>();
    let text = String::from_utf8_lossy(body).to_string();
    let mut request = json!({ "method": method, "path": sub, "query": query, "headers": headers, "body": text });
    if let Ok(parsed) = serde_json::from_slice::<Value>(body) {
        request["json"] = parsed;
    }
    match call(manager, &key, "http", request, DEFAULT_CALL_TIMEOUT, None).await {
        Ok(Value::String(text)) => (200, "text/plain; charset=utf-8".into(), text.into_bytes()),
        Ok(Value::Object(response)) if response.contains_key("status") || response.contains_key("body") || response.contains_key("json") => {
            let status = response.get("status").and_then(Value::as_u64).filter(|code| (100..600).contains(code)).unwrap_or(200) as u16;
            let content_type = response
                .get("headers")
                .and_then(|headers| headers.get("content-type").or_else(|| headers.get("Content-Type")))
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(value) = response.get("json") {
                return (status, content_type.unwrap_or_else(|| "application/json; charset=utf-8".into()), value.to_string().into_bytes());
            }
            let body = match response.get("body") {
                Some(Value::String(text)) => text.clone().into_bytes(),
                Some(Value::Null) | None => Vec::new(),
                Some(other) => other.to_string().into_bytes(),
            };
            (status, content_type.unwrap_or_else(|| "text/plain; charset=utf-8".into()), body)
        }
        Ok(value) => (200, "application/json; charset=utf-8".into(), value.to_string().into_bytes()),
        Err(error) => fail(502, format!("{error:#}")),
    }
}

/// 请求是不是 WebSocket 升级（`Connection: Upgrade` + `Upgrade: websocket`）。
pub fn is_websocket_upgrade(raw_headers: &str) -> bool {
    let header = |name: &str| {
        raw_headers.lines().skip(1).filter_map(|line| line.split_once(':')).find(|(key, _)| key.trim().eq_ignore_ascii_case(name)).map(|(_, value)| value.trim().to_ascii_lowercase())
    };
    header("upgrade").is_some_and(|value| value == "websocket")
        && header("connection").is_some_and(|value| value.split(',').any(|item| item.trim() == "upgrade"))
}

/// 拓展 HTTP 路由上的协议升级（dsh 的 `webServer.registerUpgrade`）：后台的 `upgrade(request)` 给出一个本机 socket
/// （常驻进程里的 Node http 服务）后，把原始升级请求原样转过去，之后两边字节对拷，协议协商与帧都由插件自己的处理器负责。
/// 网页发来的升级只接受同源（Origin 与 Host 一致，即页面本身由这条拓展路由提供）。连接结束才返回。
pub async fn route_upgrade(stream: &mut tokio::net::TcpStream, raw_path: &str, raw_headers: &str, raw_head: &[u8]) -> anyhow::Result<()> {
    route_upgrade_with(&crate::user_scripts::default_user_script_manager(), stream, raw_path, raw_headers, raw_head).await
}

pub async fn route_upgrade_with(
    manager: &UserScriptManager,
    stream: &mut tokio::net::TcpStream,
    raw_path: &str,
    raw_headers: &str,
    raw_head: &[u8],
) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    async fn refuse(stream: &mut tokio::net::TcpStream, status: &str, message: &str) -> anyhow::Result<()> {
        let body = json!({ "error": message }).to_string();
        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes()).await?;
        stream.shutdown().await?;
        Ok(())
    }
    let header = |name: &str| raw_headers.lines().skip(1).filter_map(|line| line.split_once(':')).find(|(key, _)| key.trim().eq_ignore_ascii_case(name)).map(|(_, value)| value.trim().to_string());
    // 同源才放行：页面本身由拓展路由提供时 Origin 就是 http://<Host>。
    if let Some(origin) = header("origin").filter(|origin| !origin.is_empty()) {
        let host = header("host").unwrap_or_default();
        let same = reqwest::Url::parse(&origin).ok().is_some_and(|url| url.host_str().is_some_and(|name| format!("{name}{}", url.port().map(|port| format!(":{port}")).unwrap_or_default()) == host));
        if !same {
            return refuse(stream, "403 Forbidden", "extension upgrades only accept same-origin pages").await;
        }
    }
    let (path, query) = raw_path.split_once('?').unwrap_or((raw_path, ""));
    let Some(rest) = path.strip_prefix("/ext/") else {
        return refuse(stream, "404 Not Found", "not found").await;
    };
    let (id, sub) = rest.split_once('/').map(|(id, sub)| (id, format!("/{sub}"))).unwrap_or((rest, "/".to_string()));
    if id.is_empty() || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
        return refuse(stream, "404 Not Found", "not found").await;
    }
    let key = crate::user_scripts::extension_package_key(id);
    if !manager.extension_backend_http(&key) {
        return refuse(stream, "404 Not Found", &format!("extension {id} does not serve HTTP")).await;
    }
    let headers = raw_headers
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), Value::String(value.trim().to_string())))
        .collect::<serde_json::Map<_, _>>();
    let reply = match call(manager, &key, "upgrade", json!({ "path": sub, "query": query, "headers": headers }), DEFAULT_CALL_TIMEOUT, None).await {
        Ok(reply) => reply,
        Err(error) => return refuse(stream, "502 Bad Gateway", &format!("{error:#}")).await,
    };
    let Some(socket) = reply.get("socket").and_then(Value::as_str).filter(|path| !path.is_empty()) else {
        let message = reply.get("error").and_then(Value::as_str).unwrap_or("no upgrade handler for this path");
        return refuse(stream, "404 Not Found", message).await;
    };
    let mut head = raw_head.to_vec();
    head.extend_from_slice(b"\r\n\r\n");
    #[cfg(unix)]
    let mut upstream = tokio::net::UnixStream::connect(socket).await.with_context(|| format!("连不上拓展的升级服务 {socket}"))?;
    #[cfg(windows)]
    let mut upstream = tokio::net::windows::named_pipe::ClientOptions::new().open(socket).with_context(|| format!("连不上拓展的升级服务 {socket}"))?;
    upstream.write_all(&head).await?;
    let _ = tokio::io::copy_bidirectional(stream, &mut upstream).await;
    Ok(())
}

/// 定时任务调度：每分钟给声明了 schedules 的拓展后台发一次 `__tick`（没起就先起），到点与否由后台按本机时区判断。
/// 只在托管后台的进程（启动器）里起一次。
pub fn start_scheduler() {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    handle.spawn(async {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let manager = crate::user_scripts::default_user_script_manager();
            let schedules = tokio::task::spawn_blocking({
                let manager = manager.clone();
                move || manager.enabled_extension_schedules().unwrap_or_default()
            })
            .await
            .unwrap_or_default();
            let mut grouped: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
            for (key, schedule) in schedules {
                grouped.entry(key).or_default().push(serde_json::to_value(schedule).unwrap_or(Value::Null));
            }
            for (key, list) in grouped {
                match call(&manager, &key, "__tick", json!({ "schedules": list }), Duration::from_secs(55), None).await {
                    Ok(result) => {
                        for failure in result["ran"].as_array().into_iter().flatten().filter(|item| item.get("error").is_some()) {
                            let _ = crate::diagnostic_log::append_diagnostic_log(
                                "extension_backend.schedule_failed",
                                json!({ "extension": key, "method": failure.get("method"), "error": failure.get("error") }),
                            );
                        }
                    }
                    Err(error) => {
                        let _ = crate::diagnostic_log::append_diagnostic_log(
                            "extension_backend.schedule_failed",
                            json!({ "extension": key, "error": format!("{error:#}") }),
                        );
                    }
                }
            }
        }
    });
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
/// `since: "latest"` 用于刚开始订阅：后台已在跑时从队尾开始，不把订阅前的旧事件重放给新页面实例。
pub async fn route_events(payload: &Value) -> anyhow::Result<Value> {
    let key = payload.get("key").and_then(Value::as_str).context("缺少拓展 key")?;
    if payload.get("since").and_then(Value::as_str) == Some("latest") {
        return Ok(match latest_seq(key).await {
            Some(next) => json!({ "status": "ok", "events": [], "next": next, "running": true }),
            None => json!({ "status": "ok", "events": [], "next": 0, "running": false }),
        });
    }
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
