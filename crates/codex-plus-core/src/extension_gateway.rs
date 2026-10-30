//! 拓展的网关请求中间件：Codex 经 Codex++ 本地代理 / 网关发出的模型请求，在发往上游前
//! 交给拓展按顺序改写（例如往 `instructions` 追加段落、过滤 `input`、改 `reasoning`）。
//!
//! 只在有启用中的拓展声明了 `gateway.request` 时才起一个 Node 进程；没有时零开销。中间件
//! 拿到的是 Responses 请求体的副本，返回新对象即替换、返回 null 即不改；任何出错、超时
//! 都保留原请求，绝不能挡住发往模型的请求。官方直连（不经 Codex++ 代理）的请求不经过这里。

use std::time::Duration;

use serde_json::{Value, json};

use crate::user_scripts::{ExtensionGatewayMiddleware, UserScriptManager};

/// 整条中间件链的超时上限：逐个超时之和与此取小。
const CHAIN_LIMIT: Duration = Duration::from_secs(15);

/// stdin：`{ path, body, middlewares: [{ id, script, timeoutMs }] }`；按顺序调用默认导出，
/// 返回对象就作为下一个的输入。stdout：`{ body, changed, errors: [{ id, error }] }`。
const RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
let body = input.body;
let changed = false;
const errors = [];
for (const item of input.middlewares) {
  let timer;
  try {
    const module = await import(pathToFileURL(item.script).href);
    const fn = typeof module.default === "function" ? module.default : module.request;
    if (typeof fn !== "function") { errors.push({ id: item.id, error: `${item.script} 没有默认导出函数` }); continue; }
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${item.timeoutMs / 1000} 秒）`)), item.timeoutMs); });
    const next = await Promise.race([Promise.resolve(fn(structuredClone(body), { path: input.path, extension: item.id, config: item.config ?? {} })), timeout]);
    if (next && typeof next === "object" && !Array.isArray(next)) { body = next; changed = true; }
  } catch (error) {
    errors.push({ id: item.id, error: String(error?.stack || error?.message || error) });
  } finally {
    clearTimeout(timer);
  }
}
process.stdout.write(JSON.stringify({ body, changed, errors }));
"#;

/// 启用中的中间件清单缓存。每次模型请求都要查一遍，扫目录、读清单不能每次都做；
/// 3 秒足够让拓展启停后很快生效。
const MIDDLEWARE_CACHE_TTL: Duration = Duration::from_secs(3);
type MiddlewareList = Vec<(String, ExtensionGatewayMiddleware)>;
static MIDDLEWARE_CACHE: std::sync::Mutex<Option<(std::time::Instant, MiddlewareList)>> = std::sync::Mutex::new(None);

fn cached_middlewares() -> MiddlewareList {
    let mut cache = MIDDLEWARE_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < MIDDLEWARE_CACHE_TTL {
            return list.clone();
        }
    }
    let list = crate::user_scripts::default_user_script_manager()
        .enabled_gateway_middlewares()
        .unwrap_or_default();
    *cache = Some((std::time::Instant::now(), list.clone()));
    list
}

/// Codex++ 自己的启动器把请求转给本机网关时带上的请求头：说明拓展中间件已在外层跑过。
pub const EXTENSIONS_APPLIED_HEADER: &str = "x-codex-plus-extensions";

tokio::task_local! {
    /// 当前请求的拓展中间件已在上一跳（启动器）跑过：本层的请求、响应中间件都跳过，避免同一条流改写两遍。
    pub static EXTENSIONS_ALREADY_APPLIED: bool;
}

fn already_applied() -> bool {
    EXTENSIONS_ALREADY_APPLIED.try_with(|applied| *applied).unwrap_or(false)
}

/// 原始请求头里是否带了「已跑过拓展中间件」的标记。
pub fn headers_mark_applied(raw_headers: &str) -> bool {
    raw_headers.lines().skip(1).any(|line| {
        line.split_once(':')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(EXTENSIONS_APPLIED_HEADER))
    })
}

/// 用默认的用户脚本位置改写请求。没有中间件、或任何环节失败时不改 `request`。
pub async fn apply_request(request: &mut Value, request_path: &str) {
    if already_applied() {
        return;
    }
    let middlewares = cached_middlewares();
    if middlewares.is_empty() {
        return;
    }
    apply_middlewares(&middlewares, request, request_path).await;
}

pub async fn apply_request_with(manager: &UserScriptManager, request: &mut Value, request_path: &str) {
    let middlewares = match manager.enabled_gateway_middlewares() {
        Ok(middlewares) if !middlewares.is_empty() => middlewares,
        _ => return,
    };
    apply_middlewares(&middlewares, request, request_path).await;
}

async fn apply_middlewares(middlewares: &[(String, ExtensionGatewayMiddleware)], request: &mut Value, request_path: &str) {
    match run_chain(middlewares, request, request_path).await {
        Ok(result) => {
            for error in result["errors"].as_array().into_iter().flatten() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "extension_gateway.middleware_failed",
                    json!({ "path": request_path, "extension": error.get("id"), "error": error.get("error") }),
                );
            }
            if result["changed"] == true && result["body"].is_object() {
                *request = result["body"].clone();
            }
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_gateway.chain_failed",
                json!({ "path": request_path, "error": format!("{error:#}") }),
            );
        }
    }
}

async fn run_chain(
    middlewares: &[(String, ExtensionGatewayMiddleware)],
    request: &Value,
    request_path: &str,
) -> anyhow::Result<Value> {
    let node = crate::extension_node::require_node()?;
    let input = json!({
        "path": request_path,
        "body": request,
        "middlewares": middlewares.iter().map(|(id, item)| json!({
            "id": id,
            "script": item.script,
            "timeoutMs": item.timeout_secs * 1000,
            "config": item.config,
        })).collect::<Vec<_>>(),
    });
    let limit = Duration::from_secs(middlewares.iter().map(|(_, item)| item.timeout_secs).sum::<u64>() + 2).min(CHAIN_LIMIT);
    crate::extension_node::run_runner(&node, RUNNER, &input, limit).await
}


// ---------------------------------------------------------------------------------------------
// 网关响应流中间件
//
// 写法与语义对齐「整条流外面包一层」的中间件：脚本默认导出 `(context, next) => 异步可迭代对象`，
// `next()` 给出下游（上游模型方向）的事件流，返回的流就是交给 Codex 的事件。可以改写、丢弃、
// 插入事件，也可以不调用 `next()` 自己给出整条流；`next()` 只能调用一次。多个拓展按拓展 key
// 顺序嵌套，先声明的在最外层。事件是 Codex 实际收到的 Responses SSE 事件对象（带 `type`）。
//
// 所有中间件跑在一个常驻 Node 工作进程里（每个响应一次 start、每个事件一次往返，不为每个
// 事件起进程）。Rust 侧逐个事件「一问一答」：推一个源事件，等链路处理完并再次向源要事件时，
// 拿回这期间产出的事件。任何中间件出错、超时或工作进程退出，这个响应剩下的部分原样放行，
// 绝不让 Codex 收不到回复。
// ---------------------------------------------------------------------------------------------

/// 响应流工作进程：stdin 每行一个操作，stdout 每行一个回复（按 id 对应）。
const RESPONSE_RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
import readline from "node:readline";
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const modules = new Map();
const load = (script) => {
  if (!modules.has(script)) {
    modules.set(script, import(pathToFileURL(script).href).then((module) => {
      const fn = typeof module.default === "function" ? module.default : module.response;
      if (typeof fn !== "function") throw new Error(`${script} 没有默认导出函数`);
      return fn;
    }));
  }
  return modules.get(script);
};
const sessions = new Map();
const describe = (error) => String(error?.stack || error?.message || error);

function createSession(rid) {
  const queue = [];
  let wake = null;
  let closed = false;
  const session = { rid, outputs: [], pending: null, done: false, failed: null };
  // 链路要下一个源事件而队列已空：这就是一个事件处理完的边界，把攒下的输出交回去。
  const idle = () => {
    if (session.pending !== null) {
      write({ id: session.pending, events: session.outputs.splice(0) });
      session.pending = null;
    }
  };
  session.source = (async function* () {
    while (true) {
      if (queue.length) { yield queue.shift(); continue; }
      if (closed) return;
      idle();
      await new Promise((resolve) => { wake = resolve; });
    }
  })();
  session.push = (event) => { queue.push(event); const resume = wake; wake = null; resume?.(); };
  session.close = () => { closed = true; const resume = wake; wake = null; resume?.(); };
  session.finish = () => {
    session.done = true;
    if (session.pending !== null) {
      write(session.failed ? { id: session.pending, error: session.failed } : { id: session.pending, events: session.outputs.splice(0), done: true });
      session.pending = null;
    }
  };
  return session;
}

async function start(message) {
  const session = createSession(message.rid);
  sessions.set(message.rid, session);
  let next = () => session.source;
  for (const item of [...message.middlewares].reverse()) {
    const fn = await load(item.script);
    const inner = next;
    let called = false;
    const guarded = () => {
      if (called) throw new Error(`${item.id}：next() 只能调用一次`);
      called = true;
      return inner();
    };
    next = () => {
      const result = fn({ ...message.context, extension: item.id, config: item.config ?? {} }, guarded);
      if (!result || typeof result[Symbol.asyncIterator] !== "function") {
        if (result && typeof result[Symbol.iterator] === "function") return (async function* () { yield* result; })();
        throw new Error(`${item.id}：中间件必须返回异步可迭代对象`);
      }
      return result;
    };
  }
  const chain = next();
  (async () => {
    try {
      for await (const event of chain) session.outputs.push(event);
    } catch (error) {
      session.failed = describe(error);
    }
    session.finish();
  })();
}

{
  const lines = readline.createInterface({ input: process.stdin });
  lines.on("line", async (line) => {
    let message;
    try { message = JSON.parse(line); } catch { return; }
    const session = sessions.get(message.rid);
    try {
      if (message.op === "start") {
        await start(message);
        write({ id: message.id, ok: true });
      } else if (message.op === "event" || message.op === "end") {
        if (!session) { write({ id: message.id, error: "响应会话不存在" }); return; }
        if (session.failed) { write({ id: message.id, error: session.failed }); return; }
        if (session.done) { write({ id: message.id, events: session.outputs.splice(0), done: true }); return; }
        session.pending = message.id;
        if (message.op === "event") session.push(message.event); else session.close();
      } else if (message.op === "abort") {
        session?.close();
        sessions.delete(message.rid);
      }
      if (message.op === "end" || message.op === "abort") setTimeout(() => sessions.delete(message.rid), 1000);
    } catch (error) {
      if (message.id !== undefined) write({ id: message.id, error: describe(error) });
    }
  });
  lines.on("close", () => process.exit(0));
}
"#;

struct ResponseWorker {
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _child: tokio::sync::Mutex<tokio::process::Child>,
}

static RESPONSE_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static RESPONSE_WORKER: tokio::sync::Mutex<Option<(String, std::sync::Arc<ResponseWorker>)>> = tokio::sync::Mutex::const_new(None);
static RESPONSE_CACHE: std::sync::Mutex<Option<(std::time::Instant, MiddlewareList)>> = std::sync::Mutex::new(None);

fn cached_response_middlewares() -> MiddlewareList {
    let mut cache = RESPONSE_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < MIDDLEWARE_CACHE_TTL {
            return list.clone();
        }
    }
    let list = crate::user_scripts::default_user_script_manager()
        .enabled_gateway_response_middlewares()
        .unwrap_or_default();
    *cache = Some((std::time::Instant::now(), list.clone()));
    list
}

/// 中间件清单的指纹：脚本、修改时间、配置任一变化都要换一个工作进程（模块缓存在进程里）。
fn response_fingerprint(middlewares: &[(String, ExtensionGatewayMiddleware)]) -> String {
    middlewares
        .iter()
        .map(|(id, item)| {
            let modified = std::fs::metadata(&item.script)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|time| time.as_nanos())
                .unwrap_or_default();
            format!("{id}|{}|{modified}|{}", item.script.display(), item.config)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn spawn_response_worker() -> anyhow::Result<ResponseWorker> {
    use tokio::io::AsyncBufReadExt;
    let node = crate::extension_node::require_node()?;
    let mut command = tokio::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", RESPONSE_RUNNER])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(crate::windows_create_no_window());
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("响应流工作进程没有 stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("响应流工作进程没有 stdout"))?;
    let pending: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>> = Default::default();
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let pending = pending.clone();
        let alive = alive.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Some(sender) = pending.lock().await.remove(&id) {
                        let _ = sender.send(message);
                    }
                }
            }
            alive.store(false, std::sync::atomic::Ordering::SeqCst);
            for (_, sender) in pending.lock().await.drain() {
                let _ = sender.send(json!({ "error": "响应流工作进程已退出" }));
            }
        });
    }
    Ok(ResponseWorker {
        stdin: tokio::sync::Mutex::new(stdin),
        pending,
        alive,
        _child: tokio::sync::Mutex::new(child),
    })
}

async fn response_worker(fingerprint: &str) -> anyhow::Result<std::sync::Arc<ResponseWorker>> {
    let mut slot = RESPONSE_WORKER.lock().await;
    if let Some((current, worker)) = slot.as_ref() {
        if current == fingerprint && worker.alive.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(worker.clone());
        }
    }
    let worker = std::sync::Arc::new(spawn_response_worker().await?);
    *slot = Some((fingerprint.to_string(), worker.clone()));
    Ok(worker)
}

impl ResponseWorker {
    async fn send(&self, message: Value) -> anyhow::Result<()> {
        use tokio::io::AsyncWriteExt;
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(format!("{message}\n").as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn request(&self, mut message: Value, timeout: Duration) -> anyhow::Result<Value> {
        let id = RESPONSE_IDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        message["id"] = json!(id);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(id, sender);
        if let Err(error) = self.send(message).await {
            self.pending.lock().await.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(reply)) => match reply.get("error").and_then(Value::as_str) {
                Some(error) => anyhow::bail!("{error}"),
                None => Ok(reply),
            },
            Ok(Err(_)) => anyhow::bail!("响应流工作进程没有回应"),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                anyhow::bail!("响应流中间件超时（{} 秒）", timeout.as_secs())
            }
        }
    }
}

/// 交给中间件的请求摘要。完整请求体可能有几 MB（整段对话），每个响应都序列化一遍不划算。
fn request_summary(request: Option<&Value>) -> Value {
    let Some(request) = request else {
        return Value::Null;
    };
    let input = request.get("input").and_then(Value::as_array);
    let last_user_text = input
        .into_iter()
        .flatten()
        .rev()
        .find(|item| item.get("role").and_then(Value::as_str) == Some("user"))
        .map(|item| match item.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        })
        .unwrap_or_default();
    json!({
        "model": request.get("model").cloned().unwrap_or(Value::Null),
        "instructions": request.get("instructions").cloned().unwrap_or(Value::Null),
        "reasoning": request.get("reasoning").cloned().unwrap_or(Value::Null),
        "stream": request.get("stream").cloned().unwrap_or(Value::Null),
        "tools": request.get("tools").and_then(Value::as_array).map(|tools| tools
            .iter()
            .filter_map(|tool| tool.get("name").or_else(|| tool.get("type")).cloned())
            .collect::<Vec<_>>()).unwrap_or_default(),
        "inputItems": input.map(Vec::len).unwrap_or(0),
        "lastUserText": last_user_text,
    })
}

/// 一个流式响应上的中间件会话。没有启用中的响应中间件时 `start` 返回 None，调用方零开销直通。
pub struct ResponseFilter {
    worker: std::sync::Arc<ResponseWorker>,
    rid: u64,
    timeout: Duration,
    buffer: Vec<u8>,
    /// 源流是否用 `event:` 行标注事件名；输出时沿用同一格式。
    event_lines: bool,
    /// 出错后进入直通：剩余字节原样转发。
    passthrough: bool,
    /// 链路已经结束（中间件不再要源事件）：之后的源事件丢弃。
    done: bool,
}

impl ResponseFilter {
    pub async fn start(request_path: &str, request: Option<&Value>) -> Option<Self> {
        if already_applied() {
            return None;
        }
        let middlewares = cached_response_middlewares();
        if middlewares.is_empty() {
            return None;
        }
        Self::start_with(&middlewares, request_path, request).await
    }

    pub async fn start_with(middlewares: &[(String, ExtensionGatewayMiddleware)], request_path: &str, request: Option<&Value>) -> Option<Self> {
        if middlewares.is_empty() {
            return None;
        }
        let timeout = Duration::from_secs(middlewares.iter().map(|(_, item)| item.timeout_secs).max().unwrap_or(5));
        let started = async {
            let worker = response_worker(&response_fingerprint(middlewares)).await?;
            let rid = RESPONSE_IDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            worker
                .request(
                    json!({
                        "op": "start",
                        "rid": rid,
                        "context": { "path": request_path, "request": request_summary(request) },
                        "middlewares": middlewares.iter().map(|(id, item)| json!({ "id": id, "script": item.script, "config": item.config })).collect::<Vec<_>>(),
                    }),
                    timeout,
                )
                .await?;
            anyhow::Ok((worker, rid))
        }
        .await;
        match started {
            Ok((worker, rid)) => Some(Self { worker, rid, timeout, buffer: Vec::new(), event_lines: false, passthrough: false, done: false }),
            Err(error) => {
                log_response_failure(request_path, &error);
                None
            }
        }
    }

    /// 喂入上游字节，返回此刻应写给 Codex 的字节。
    pub async fn push(&mut self, bytes: &[u8]) -> Vec<u8> {
        if self.passthrough {
            return bytes.to_vec();
        }
        self.buffer.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some((end, separator)) = find_event_end(&self.buffer) {
            let block = self.buffer.drain(..end + separator).collect::<Vec<_>>();
            if self.passthrough {
                out.extend_from_slice(&block);
                continue;
            }
            out.extend(self.process_block(&block[..end]).await);
        }
        if self.passthrough && !self.buffer.is_empty() {
            out.extend(self.buffer.drain(..));
        }
        out
    }

    /// 上游结束：把残余字节与中间件在流尾追加的事件交出去。
    pub async fn finish(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            if self.passthrough || self.done {
                out.extend(rest);
            } else {
                out.extend(self.process_block(&rest).await);
            }
        }
        if self.passthrough || self.done {
            return out;
        }
        match self.worker.request(json!({ "op": "end", "rid": self.rid }), self.timeout).await {
            Ok(reply) => out.extend(self.serialize(&reply["events"])),
            Err(error) => {
                log_response_failure("end", &error);
                let _ = self.worker.send(json!({ "op": "abort", "rid": self.rid })).await;
            }
        }
        self.done = true;
        out
    }

    async fn process_block(&mut self, block: &[u8]) -> Vec<u8> {
        let text = String::from_utf8_lossy(block).replace("\r\n", "\n");
        let mut event_name = None;
        let mut data = Vec::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                event_name = Some(value.trim().to_string());
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
            }
        }
        let raw = || {
            let mut bytes = block.to_vec();
            bytes.extend_from_slice(b"\n\n");
            bytes
        };
        // 注释、心跳、`[DONE]` 这类不是 JSON 事件的块原样放行。
        let Ok(mut event) = serde_json::from_str::<Value>(&data.join("\n")) else {
            return raw();
        };
        if !event.is_object() {
            return raw();
        }
        if let Some(name) = &event_name {
            self.event_lines = true;
            if event.get("type").is_none() {
                event["type"] = json!(name);
            }
        }
        if self.done {
            return Vec::new();
        }
        match self.worker.request(json!({ "op": "event", "rid": self.rid, "event": event }), self.timeout).await {
            Ok(reply) => {
                if reply.get("done").and_then(Value::as_bool) == Some(true) {
                    self.done = true;
                }
                self.serialize(&reply["events"])
            }
            Err(error) => {
                log_response_failure("event", &error);
                self.passthrough = true;
                let _ = self.worker.send(json!({ "op": "abort", "rid": self.rid })).await;
                raw()
            }
        }
    }

    fn serialize(&self, events: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        for event in events.as_array().into_iter().flatten() {
            if !event.is_object() {
                continue;
            }
            match event.get("type").and_then(Value::as_str) {
                Some(name) if self.event_lines => out.extend(format!("event: {name}\ndata: {event}\n\n").into_bytes()),
                _ => out.extend(format!("data: {event}\n\n").into_bytes()),
            }
        }
        out
    }
}

/// 找下一个完整 SSE 事件块的结束位置与分隔符长度（`\n\n` 或 `\r\n\r\n`）。
fn find_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|window| window == b"\n\n").map(|index| (index, 2));
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n").map(|index| (index, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

fn log_response_failure(stage: &str, error: &anyhow::Error) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "extension_gateway.response_middleware_failed",
        json!({ "stage": stage, "error": format!("{error:#}") }),
    );
}
