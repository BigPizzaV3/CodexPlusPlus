//! 拓展的 Node 侧执行：钩子、网关中间件、拓展工具都以「一段运行器脚本 + stdin JSON」的方式
//! 起一个 Node 进程，运行器再 import 拓展脚本并调用其默认导出。这里只管起进程、喂输入、
//! 读输出与整体超时；各自的运行器脚本与结果语义由调用方定义。

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde_json::Value;

/// 用 `node --input-type=module -e <runner>` 运行，stdin 写 `input`，读 stdout 解析为 JSON。
/// 超过 `limit` 即杀进程并报错。stderr 丢弃（拓展脚本的日志不应混进协议输出）。
pub async fn run_runner(node: &Path, runner: &str, input: &Value, limit: Duration) -> anyhow::Result<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", runner])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::extension_backend::set_host_exe_env(&mut command);
    // Windows 上不能闪出黑色终端窗口。
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().context("无法启动 Node 运行拓展脚本")?;
    let mut stdin = child.stdin.take().context("Node 进程没有 stdin")?;
    stdin.write_all(input.to_string().as_bytes()).await?;
    drop(stdin);
    let mut stdout = child.stdout.take().context("Node 进程没有 stdout")?;
    let read = async {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer).await?;
        child.wait().await?;
        anyhow::Ok(buffer)
    };
    let buffer = tokio::time::timeout(limit, read).await.context("拓展脚本整体超时")??;
    serde_json::from_slice(&buffer).context("拓展脚本运行器输出不是 JSON")
}

/// 运行器里的宿主调用客户端：`write`、`host(service, method, params)`、只读会话查询 `sessionQuery`、会话写入 `sessions`、
/// 按名字调用后台服务的 `service(name)`、以及处理回复的 `handleHostReply(message)`。运行器在 import
/// 之后放一行 `/*HOST_CLIENT*/`，由 [`with_host_client`] 替换成这段代码。
pub const HOST_CLIENT_JS: &str = r#"
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
// 向 Codex++ 发起宿主调用（例如只读会话查询），回复按 hostReply 对应。
const hostPending = new Map();
let hostSeq = 0;
const host = (service, method, params) => new Promise((resolve, reject) => {
  const id = ++hostSeq;
  hostPending.set(id, { resolve, reject });
  write({ host: id, service, method, params: params ?? null });
});
// 调用任一拓展后台提供的服务：service("名字").方法(...参数)。
const service = (name) => new Proxy({}, {
  get: (_, method) => (typeof method !== "string" || method === "then") ? undefined
    : (...args) => host("services", "call", { name: String(name), method, args }),
});
const withId = (method) => (id, options) => host("sessionQuery", method, { id, options: options ?? {} });
const readSession = (id) => host("sessionQuery", "readSession", { id });
const readEventsSince = (query) => host("sessionQuery", "readEventsSince", query ?? {});
// 把会话事件日志折叠成状态；apply(state, event) 返回 undefined 视为不变。
const project = async (id, { init, apply } = {}) => {
  if (typeof apply !== "function") throw new Error("project 需要 apply(state, event)");
  const { events } = await readSession(id);
  let state = typeof init === "function" ? init() : init;
  for (const event of events) { const next = apply(state, event); if (next !== undefined) state = next; }
  return state;
};
const sessionQuery = Object.freeze({
  listSessions: (options) => host("sessionQuery", "listSessions", options ?? {}),
  filterSessions: (filters) => host("sessionQuery", "filterSessions", filters ?? {}),
  readSession,
  readTitle: (id) => host("sessionQuery", "readTitle", { id }),
  listEvents: withId("listEvents"),
  filterEvents: withId("filterEvents"),
  readEvent: (query) => host("sessionQuery", "readEvent", query ?? {}),
  readEventsSince,
  project,
  // 订阅会话新事件（默认从当前末尾开始，每 intervalMs 增量读一次），返回撤销函数。
  observeSession(id, handler, options = {}) {
    let stopped = false;
    let next = Number.isFinite(options.fromSeq) ? Number(options.fromSeq) : null;
    let timer = null;
    const interval = Math.max(500, Number(options.intervalMs) || 2000);
    const tick = async () => {
      if (stopped) return;
      try {
        if (next === null) {
          const tail = await host("sessionQuery", "listEvents", { id, options: { fromEnd: true, limit: 1 } });
          next = tail.length ? tail[0].seq + 1 : 0;
        } else {
          for (const event of await readEventsSince({ sessionId: id, seq: next, limit: 200 })) {
            if (stopped) return;
            next = event.seq + 1;
            try { handler(event); } catch {}
          }
        }
      } catch {}
      if (!stopped) timer = setTimeout(tick, interval);
    };
    void tick();
    return () => { stopped = true; clearTimeout(timer); };
  },
  // 订阅全部会话的新事件：handler(event, { sessionId })；onStatus({ sessionId, status, turn }) 可选。
  // 已有事件不重放，订阅之后新建的会话从头发。返回撤销函数。
  observeAll(handler, options = {}) {
    let stopped = false;
    let primed = false;
    let timer = null;
    const cursors = new Map();
    const interval = Math.max(500, Number(options.intervalMs) || 2000);
    const statusOf = (event) => event.type === "turn/start" ? "running" : event.type === "turn/complete" ? "idle" : event.type === "turn/aborted" ? "aborted" : null;
    const tick = async () => {
      if (stopped) return;
      try {
        const list = await host("sessionQuery", "listSessions", { limit: 20 });
        for (const [index, session] of (list || []).entries()) {
          let cursor = cursors.get(session.id);
          if (!cursor) {
            let next = 0;
            if (!primed) {
              const tail = await host("sessionQuery", "listEvents", { id: session.id, options: { fromEnd: true, limit: 1 } }).catch(() => []);
              next = tail.length ? tail[0].seq + 1 : 0;
            }
            cursor = { updatedAt: primed ? null : session.updatedAt, next };
            cursors.set(session.id, cursor);
            if (!primed) continue;
          }
          if (index >= 3 && cursor.updatedAt === session.updatedAt) continue;
          cursor.updatedAt = session.updatedAt;
          for (const event of await readEventsSince({ sessionId: session.id, seq: cursor.next, limit: 200 }).catch(() => [])) {
            if (stopped) return;
            cursor.next = event.seq + 1;
            try { handler(event, { sessionId: session.id }); } catch {}
            const status = statusOf(event);
            if (status && typeof options.onStatus === "function") {
              try { options.onStatus({ sessionId: session.id, status, turn: event.data?.turn, reason: event.data?.reason ?? null }); } catch {}
            }
          }
        }
        primed = true;
      } catch {}
      if (!stopped) timer = setTimeout(tick, interval);
    };
    void tick();
    return () => { stopped = true; clearTimeout(timer); };
  },
  searchSessions: (query) => host("sessionQuery", "searchSessions", query ?? {}),
  searchEvents: (query) => host("sessionQuery", "searchEvents", query ?? {}),
});
// 会话写入（新建、分叉、改名、追加条目、发起 / 插话 / 中断一轮、压缩），经 Codex 页面里的 app-server 连接完成。
const sessions = Object.freeze({
  list: (options) => host("sessions", "list", options ?? {}),
  create: (input, options) => host("sessions", "create", { input, options }),
  fork: (id, options) => host("sessions", "fork", { id, options }),
  rename: (id, name) => host("sessions", "rename", { id, name }),
  append: (id, items) => host("sessions", "append", { id, items }),
  startTurn: (id, input, options) => host("sessions", "startTurn", { id, input, options }),
  steer: (id, input, turnId) => host("sessions", "steer", { id, input, turnId }),
  interrupt: (id, turnId) => host("sessions", "interrupt", { id, turnId }),
  compact: (id) => host("sessions", "compact", { id }),
});
// 拓展自己调模型：经 Codex++ 协议代理用当前供应商。complete({ input, model?, instructions?, reasoningEffort? })。
const llm = Object.freeze({
  complete: (options) => host("llm", "complete", typeof options === "string" ? { input: options } : options ?? {}),
  listModels: () => host("llm", "listModels", {}),
});
// 界面：ui.ask({ title, message, options?, input? }) → { value, text } | null；ui.toast(文字)。
const ui = Object.freeze({
  ask: (question) => host("ui", "ask", { timeoutMs: 600000, ...(question ?? {}) }),
  toast: (message, options) => host("ui", "toast", { message: String(message ?? ""), options }),
});
// 宿主调用的回复：处理了就返回 true。
const handleHostReply = (message) => {
  if (message?.hostReply === undefined) return false;
  const waiter = hostPending.get(message.hostReply);
  hostPending.delete(message.hostReply);
  if (message.error !== undefined) waiter?.reject(new Error(message.error)); else waiter?.resolve(message.result);
  return true;
};
"#;

/// 把运行器里的 `/*HOST_CLIENT*/` 换成宿主调用客户端。
pub fn with_host_client(runner: &str) -> String {
    runner.replace("/*HOST_CLIENT*/", HOST_CLIENT_JS)
}

/// 带宿主调用的运行方式：stdin 第一行写 `input` 且保持打开；运行器中途可以写
/// `{ host, service, method, params }` 请求 Codex++（目前是只读会话查询），回复以
/// `{ hostReply, result | error }` 写回 stdin；最后一行 `{ final: 结果 }` 即运行结果。
pub async fn run_runner_with_host(
    node: &Path,
    runner: &str,
    input: &Value,
    limit: Duration,
    manager: &crate::user_scripts::UserScriptManager,
) -> anyhow::Result<Value> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", runner])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::extension_backend::set_host_exe_env(&mut command);
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn().context("无法启动 Node 运行拓展脚本")?;
    let mut stdin = child.stdin.take().context("Node 进程没有 stdin")?;
    let stdout = child.stdout.take().context("Node 进程没有 stdout")?;
    stdin.write_all(format!("{input}\n").as_bytes()).await?;
    stdin.flush().await?;
    let run = async {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(id) = message.get("host").and_then(Value::as_u64) {
                let reply = match crate::extension_backend::host_call(&message, manager).await {
                    Ok(result) => serde_json::json!({ "hostReply": id, "result": result }),
                    Err(error) => serde_json::json!({ "hostReply": id, "error": format!("{error:#}") }),
                };
                stdin.write_all(format!("{reply}\n").as_bytes()).await?;
                stdin.flush().await?;
            } else if let Some(result) = message.get("final") {
                return anyhow::Ok(result.clone());
            }
        }
        anyhow::bail!("拓展脚本运行器没有给出结果")
    };
    let result = tokio::time::timeout(limit, run).await.context("拓展脚本整体超时")?;
    let _ = child.kill().await;
    result
}

/// 找 Node；找不到时给出统一的提示。
pub fn require_node() -> anyhow::Result<std::path::PathBuf> {
    crate::dsh_plugins::locate_node().context(
        "没有找到 Node 运行时：请安装 Codex App（自带 Node）或 Node.js，也可用 CODEX_PLUS_NODE 指定",
    )
}
