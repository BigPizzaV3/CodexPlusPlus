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

/// stdin：`{ path, body, step, middlewares: [{ id, script, timeoutMs }] }`；按顺序调用默认导出，
/// 返回对象就作为下一个的输入，返回 `{ reject: "原因" }` 即拒绝这一步。
/// stdout：`{ body, changed, errors: [{ id, error }], rejected: { id, reason } | null, headers }`。
const RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
let body = input.body;
let changed = false;
let rejected = null;
const errors = [];
// 中间件可以往 context.headers 里加请求头（发给上游时附上）；鉴权类的头由 Codex++ 过滤掉。
// 关掉了 builtin:auth-headers 的拓展（item.credential）加的头另记一份，鉴权头只认它们给的。
const headers = {};
const authHeaders = {};
for (const item of input.middlewares) {
  let timer;
  const own = {};
  try {
    const module = await import(pathToFileURL(item.script).href);
    const fn = typeof module.default === "function" ? module.default : module.request;
    if (typeof fn !== "function") { errors.push({ id: item.id, error: `${item.script} 没有默认导出函数` }); continue; }
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${item.timeoutMs / 1000} 秒）`)), item.timeoutMs); });
    const extra = item.credential ? { credential: item.credential } : {};
    const next = await Promise.race([Promise.resolve(fn(structuredClone(body), { ...(input.context ?? {}), ...extra, path: input.path, extension: item.id, config: item.config ?? {}, step: input.step, headers: own })), timeout]);
    // { reject: "原因" }：这一步不发给模型，Codex 收到带原因的错误。后面的中间件不再跑。
    if (next && typeof next === "object" && !Array.isArray(next) && typeof next.reject === "string" && Object.keys(next).length === 1) {
      rejected = { id: item.id, reason: next.reject };
      break;
    }
    if (next && typeof next === "object" && !Array.isArray(next)) { body = next; changed = true; }
  } catch (error) {
    errors.push({ id: item.id, error: String(error?.stack || error?.message || error) });
  } finally {
    clearTimeout(timer);
    Object.assign(item.credential ? authHeaders : headers, own);
  }
}
process.stdout.write(JSON.stringify({ body, changed, errors, rejected, headers, authHeaders }));
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

/// 某一段（`request` / `upstream`）的中间件，按 `order` 稳定排序（相同时保持拓展顺序）。
fn stage_middlewares(list: &[(String, ExtensionGatewayMiddleware)], stage: &str) -> MiddlewareList {
    let mut picked: MiddlewareList = list
        .iter()
        .filter(|(_, item)| (if item.stage.is_empty() { "request" } else { item.stage.as_str() }) == stage)
        .cloned()
        .collect();
    picked.sort_by_key(|(_, item)| item.order);
    picked
}

static REPLACES_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<(String, String)>)>> = std::sync::Mutex::new(None);

tokio::task_local! {
    /// 在这个 future 里固定「被关掉的内置环节」清单，不再读启用中的拓展（测试用；生产路径不设）。
    static REPLACED_OVERRIDE: Vec<String>;
}

/// 在 `future` 里把被 `gateway.replaces` 关掉的内置环节固定成 `segments`（只影响这个 future，不碰进程级状态）。
pub async fn with_replaced_segments<F: std::future::Future>(segments: &[&str], future: F) -> F::Output {
    REPLACED_OVERRIDE
        .scope(segments.iter().map(|name| name.to_string()).collect(), future)
        .await
}

/// 同 [`with_replaced_segments`]，用于同步调用（例如解析网关路由计划）。
pub fn with_replaced_segments_sync<R>(segments: &[&str], f: impl FnOnce() -> R) -> R {
    REPLACED_OVERRIDE.sync_scope(segments.iter().map(|name| name.to_string()).collect(), f)
}

/// 内置网关环节（[`crate::user_scripts::BUILTIN_GATEWAY_SEGMENTS`]）是否被启用中的拓展用 `gateway.replaces` 关掉。
pub fn builtin_replaced(segment: &str) -> bool {
    if let Ok(replaced) = REPLACED_OVERRIDE.try_with(|list| list.iter().any(|name| name == segment)) {
        return replaced;
    }
    let mut cache = REPLACES_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let fresh = cache.as_ref().is_some_and(|(at, _)| at.elapsed() < MIDDLEWARE_CACHE_TTL);
    if !fresh {
        let list = crate::user_scripts::default_user_script_manager().enabled_gateway_replaces().unwrap_or_default();
        *cache = Some((std::time::Instant::now(), list));
    }
    cache.as_ref().is_some_and(|(_, list)| list.iter().any(|(_, name)| name == segment))
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

tokio::task_local! {
    /// 请求中间件加的请求头：协议代理在发往上游时附上（只在这一次请求的 future 里可见）。
    pub static EXTENSION_HEADERS: std::cell::RefCell<Vec<(String, String)>>;
}

tokio::task_local! {
    /// 当前请求所属的 Codex 会话（请求体里的 prompt_cache_key）：失败中间件据此告诉拓展是哪个会话出错。
    pub static REQUEST_SESSION: std::cell::RefCell<String>;
}

/// 协议代理解析出请求体后记下会话 id（不在请求的 future 里时什么也不做）。
pub fn set_request_session(session: &str) {
    let _ = REQUEST_SESSION.try_with(|current| *current.borrow_mut() = session.to_string());
}

fn request_session() -> String {
    REQUEST_SESSION.try_with(|current| current.borrow().clone()).unwrap_or_default()
}

/// 当前请求上拓展加的请求头。
pub fn extension_headers() -> Vec<(String, String)> {
    EXTENSION_HEADERS.try_with(|headers| headers.borrow().clone()).unwrap_or_default()
}

fn set_extension_headers(headers: Vec<(String, String)>) {
    let _ = EXTENSION_HEADERS.try_with(|current| *current.borrow_mut() = headers);
}

/// 拓展只能加自定义头：鉴权、Cookie、Host、内容长度 / 类型这类由 Codex++ 自己管。
pub fn allowed_headers(raw: &Value) -> Vec<(String, String)> {
    const BLOCKED: &[&str] = &["authorization", "proxy-authorization", "cookie", "host", "content-length", "content-type", "transfer-encoding", "connection", "x-api-key", "api-key"];
    raw.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            let name = name.trim().to_ascii_lowercase();
            let value = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                _ => return None,
            };
            let valid = !name.is_empty() && name.len() <= 64 && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-') && value.len() <= 4096 && !value.contains(['\r', '\n']);
            (valid && !BLOCKED.contains(&name.as_str()) && name != EXTENSIONS_APPLIED_HEADER).then_some((name, value))
        })
        .take(32)
        .collect()
}

/// 关掉了 `builtin:auth-headers` 的拓展在 upstream 段给的头：鉴权头（Authorization、x-api-key、api-key、Cookie……）
/// 也认，传输头（Host、内容长度 / 类型、连接）与 Codex++ 自己的标记仍不认。
pub fn allowed_auth_headers(raw: &Value) -> Vec<(String, String)> {
    const BLOCKED: &[&str] = &["host", "content-length", "content-type", "transfer-encoding", "connection"];
    raw.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(name, value)| {
            let name = name.trim().to_ascii_lowercase();
            let value = value.as_str()?.to_string();
            let valid = !name.is_empty() && name.len() <= 64 && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-') && value.len() <= 8192 && !value.contains(['\r', '\n']);
            (valid && !BLOCKED.contains(&name.as_str()) && name != EXTENSIONS_APPLIED_HEADER).then_some((name, value))
        })
        .take(32)
        .collect()
}

/// 启用中、关掉了某个内置网关环节的拓展 id（测试固定了被关掉的环节时，按全部拓展都关掉算）。
fn replacing_extensions(segment: &str) -> Option<Vec<String>> {
    if let Ok(replaced) = REPLACED_OVERRIDE.try_with(|list| list.iter().any(|name| name == segment)) {
        return replaced.then(Vec::new);
    }
    builtin_replaced(segment).then(|| {
        REPLACES_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|(_, list)| list.iter().filter(|(_, name)| name == segment).map(|(id, _)| id.clone()).collect())
            .unwrap_or_default()
    })
}

/// 原始请求头里是否带了「已跑过拓展中间件」的标记。
pub fn headers_mark_applied(raw_headers: &str) -> bool {
    raw_headers.lines().skip(1).any(|line| {
        line.split_once(':')
            .is_some_and(|(name, _)| name.trim().eq_ignore_ascii_case(EXTENSIONS_APPLIED_HEADER))
    })
}

/// 用默认的用户脚本位置改写请求。没有中间件、或任何环节失败时不改 `request`。
///
/// 有中间件拒绝这一步时返回 `Err(原因)`，调用方据此直接回错误、不发给模型。
pub async fn apply_request(request: &mut Value, request_path: &str) -> Result<(), String> {
    if already_applied() {
        return Ok(());
    }
    let manager = crate::user_scripts::default_user_script_manager();
    apply_tool_pipeline(&manager, &cached_post_execute(), request).await;
    let middlewares = stage_middlewares(&cached_middlewares(), "request");
    if middlewares.is_empty() {
        return Ok(());
    }
    apply_middlewares(&middlewares, request, request_path, Value::Null, None).await
}

pub async fn apply_request_with(manager: &UserScriptManager, request: &mut Value, request_path: &str) -> Result<(), String> {
    let post_execute = manager.enabled_tool_post_execute().unwrap_or_default();
    apply_tool_pipeline(manager, &post_execute, request).await;
    let middlewares = stage_middlewares(&manager.enabled_gateway_middlewares().unwrap_or_default(), "request");
    if middlewares.is_empty() {
        return Ok(());
    }
    apply_middlewares(&middlewares, request, request_path, Value::Null, None).await
}

/// upstream 段：Codex++ 自己的压缩、路由、协议转换之后，改真正发给上游的请求体（`context` 带上
/// `stage: "upstream"`、`wireApi`、`relay`、`model`）。没有声明这一段的拓展时零开销；拒绝时返回 `Err(原因)`。
pub async fn apply_upstream_request(body: &mut Value, request_path: &str, context: Value) -> Result<(), String> {
    if already_applied() {
        return Ok(());
    }
    let middlewares = stage_middlewares(&cached_middlewares(), "upstream");
    if middlewares.is_empty() {
        return Ok(());
    }
    let owners = replacing_extensions("builtin:auth-headers");
    apply_middlewares(&middlewares, body, request_path, context, owners).await
}

/// route 段：轮换按规则选出候选供应商后，交给拓展重排 / 筛选。中间件收到 `{ model, conversationId, candidates: [{ id, name, protocol }] }`，
/// 返回同形对象（`candidates` 换成想要的顺序、可以删掉一些）或 `{ order: ["id", …] }`。返回认得的 id 顺序；
/// 没有中间件、出错或结果里一个认得的都没有时返回 `None`（照原顺序）。
pub async fn route_order(candidates: &[Value], context: Value) -> Option<Vec<String>> {
    // 不看「上一跳已跑过」的标记：候选只在真正挑上游的那一跳出现（启动器转给网关时那一跳只有网关一个候选）。
    if candidates.len() < 2 {
        return None;
    }
    let middlewares = stage_middlewares(&cached_middlewares(), "route");
    route_order_with(&middlewares, candidates, context).await
}

pub async fn route_order_with_manager(manager: &UserScriptManager, candidates: &[Value], context: Value) -> Option<Vec<String>> {
    let middlewares = stage_middlewares(&manager.enabled_gateway_middlewares().unwrap_or_default(), "route");
    route_order_with(&middlewares, candidates, context).await
}

async fn route_order_with(middlewares: &[(String, ExtensionGatewayMiddleware)], candidates: &[Value], context: Value) -> Option<Vec<String>> {
    if middlewares.is_empty() || candidates.is_empty() {
        return None;
    }
    let mut body = json!({ "candidates": candidates });
    if let (Some(target), Some(extra)) = (body.as_object_mut(), context.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    apply_middlewares(middlewares, &mut body, "route", json!({ "stage": "route" }), None).await.ok()?;
    let known: Vec<&str> = candidates.iter().filter_map(|item| item.get("id").and_then(Value::as_str)).collect();
    let picked: Vec<String> = match body.get("order").and_then(Value::as_array) {
        Some(order) => order.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => body["candidates"].as_array().into_iter().flatten().filter_map(|item| item.get("id").and_then(Value::as_str)).map(str::to_string).collect(),
    };
    let mut seen = std::collections::BTreeSet::new();
    let order: Vec<String> = picked.into_iter().filter(|id| known.contains(&id.as_str()) && seen.insert(id.clone())).collect();
    (!order.is_empty()).then_some(order)
}

pub async fn apply_upstream_request_with(manager: &UserScriptManager, body: &mut Value, request_path: &str, context: Value) -> Result<(), String> {
    let middlewares = stage_middlewares(&manager.enabled_gateway_middlewares().unwrap_or_default(), "upstream");
    if middlewares.is_empty() {
        return Ok(());
    }
    // 鉴权凭据只交给这个清单里关掉了 builtin:auth-headers 的拓展。
    let owners: Vec<String> = manager
        .enabled_gateway_replaces()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, name)| name == "builtin:auth-headers")
        .map(|(id, _)| id)
        .collect();
    apply_middlewares(&middlewares, body, request_path, context, (!owners.is_empty()).then_some(owners)).await
}

/// 这一步在本轮里是第几步：最后一条用户消息之后已有几个工具结果，再加 1。
/// 也带上本轮用户输入的文字，方便中间件按轮判断。
pub fn request_step(request: &Value) -> Value {
    let input = request.get("input").and_then(Value::as_array).cloned().unwrap_or_default();
    let last_user = input.iter().rposition(|item| {
        item.get("role").and_then(Value::as_str) == Some("user")
            && item.get("type").and_then(Value::as_str).is_none_or(|kind| kind == "message")
    });
    let after = last_user.map(|index| &input[index + 1..]).unwrap_or(&input[..]);
    let tool_results = after
        .iter()
        .filter(|item| matches!(item.get("type").and_then(Value::as_str), Some("function_call_output" | "custom_tool_call_output")))
        .count();
    let user_text = last_user
        .and_then(|index| input[index].get("content"))
        .map(|content| match content {
            Value::String(text) => text.clone(),
            Value::Array(blocks) => blocks.iter().filter_map(|block| block.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        })
        .unwrap_or_default();
    json!({ "index": tool_results + 1, "toolResults": tool_results, "userText": user_text, "firstStep": tool_results == 0 })
}

// ---------------------------------------------------------------------------------------------
// 工具管道的网关侧：
// - 代为执行（toolPipeline.execute）的工具调用在 Codex 里以「拒绝 + 结果 + [[codexpp-exec:令牌]]」结束，
//   这里按令牌把整条工具输出换成纯结果；
// - 结果改写（toolPipeline.postExecute）：把请求里的工具输出交给拓展改写，模型看到的是改写后的结果。
//   每次请求都带着整段历史，所以按「拓展 + 脚本 + 调用 + 输出」缓存，同一条输出只改写一次。
// ---------------------------------------------------------------------------------------------

const POST_EXECUTE_RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
const results = {};
const errors = [];
for (const item of input.items) {
  let value = item.output;
  let changed = false;
  for (const handler of input.handlers) {
    if (handler.matcher && handler.matcher !== "*") {
      let re;
      try { re = new RegExp(handler.matcher); } catch (error) { errors.push({ id: handler.id, error: `matcher 不是合法正则：${error.message}` }); continue; }
      if (!re.test(item.name)) continue;
    }
    let timer;
    try {
      const module = await import(pathToFileURL(handler.script).href);
      const fn = typeof module.default === "function" ? module.default : module.postExecute;
      if (typeof fn !== "function") throw new Error(`${handler.script} 没有默认导出函数`);
      const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${handler.timeoutMs / 1000} 秒）`)), handler.timeoutMs); });
      const next = await Promise.race([Promise.resolve(fn(structuredClone(value), { name: item.name, input: item.input, callId: item.callId, extension: handler.id, config: handler.config ?? {} })), timeout]);
      if (next !== undefined && next !== null) { value = next; changed = true; }
    } catch (error) {
      errors.push({ id: handler.id, error: String(error?.stack || error?.message || error) });
    } finally {
      clearTimeout(timer);
    }
  }
  results[item.key] = changed ? value : null;
}
process.stdout.write(JSON.stringify({ results, errors }));
"#;

static POST_EXECUTE_CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<(String, crate::user_scripts::ExtensionHook)>)>> = std::sync::Mutex::new(None);
/// 改写结果缓存：键 → 改写后的输出（None 表示不改）。条目上限防止无限增长。
static POST_EXECUTE_RESULTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Option<Value>>>> =
    std::sync::LazyLock::new(Default::default);
const POST_EXECUTE_RESULT_LIMIT: usize = 4096;

fn cached_post_execute() -> Vec<(String, crate::user_scripts::ExtensionHook)> {
    let mut cache = POST_EXECUTE_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < MIDDLEWARE_CACHE_TTL {
            return list.clone();
        }
    }
    let list = crate::user_scripts::default_user_script_manager()
        .enabled_tool_post_execute()
        .unwrap_or_default();
    *cache = Some((std::time::Instant::now(), list.clone()));
    list
}

fn is_tool_output(item: &Value) -> bool {
    matches!(item.get("type").and_then(Value::as_str), Some("function_call_output" | "custom_tool_call_output"))
}

/// 工具输出里的代执行令牌（输出可能是字符串，也可能是内容块数组）。
fn exec_token(output: &Value) -> Option<String> {
    let text = match output {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    // 标记由钩子追加在拒绝理由末尾：取最后一个，且其后只能剩空白或引号等收尾字符。
    let start = text.rfind("[[codexpp-exec:")? + "[[codexpp-exec:".len();
    let end = text[start..].find("]]")? + start;
    let tail = &text[end + 2..];
    if tail.chars().any(|ch| ch.is_alphanumeric()) {
        return None;
    }
    Some(text[start..end].to_string())
}

async fn apply_tool_pipeline(
    manager: &UserScriptManager,
    post_execute: &[(String, crate::user_scripts::ExtensionHook)],
    request: &mut Value,
) {
    let Some(input) = request.get_mut("input").and_then(Value::as_array_mut) else {
        return;
    };
    // 代执行结果换回。
    for item in input.iter_mut().filter(|item| is_tool_output(item)) {
        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(token) = item.get("output").and_then(exec_token) {
            if let Some(output) = crate::extension_hooks::load_tool_execution(manager, &token, &call_id) {
                item["output"] = Value::String(output);
            }
        }
    }
    if post_execute.is_empty() {
        return;
    }
    // 结果改写：调用 id → (工具名, 参数)。
    let calls = input
        .iter()
        .filter(|item| matches!(item.get("type").and_then(Value::as_str), Some("function_call" | "custom_tool_call")))
        .filter_map(|item| {
            let call_id = item.get("call_id")?.as_str()?.to_string();
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let arguments = item.get("arguments").or_else(|| item.get("input")).cloned().unwrap_or(Value::Null);
            Some((call_id, (name, arguments)))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let signature = post_execute
        .iter()
        .map(|(id, hook)| format!("{id}:{}:{}:{}", hook.script.display(), hook.matcher, hook.config))
        .collect::<Vec<_>>()
        .join("|");
    let key_of = |call_id: &str, output: &Value| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        signature.hash(&mut hasher);
        call_id.hash(&mut hasher);
        output.to_string().hash(&mut hasher);
        format!("{:x}", hasher.finish())
    };
    let mut pending = Vec::new();
    for item in input.iter().filter(|item| is_tool_output(item)) {
        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default();
        let output = item.get("output").cloned().unwrap_or(Value::Null);
        let key = key_of(call_id, &output);
        let known = POST_EXECUTE_RESULTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).contains_key(&key);
        if !known {
            let (name, arguments) = calls.get(call_id).cloned().unwrap_or_default();
            pending.push(json!({ "key": key, "callId": call_id, "name": name, "input": arguments, "output": output }));
        }
    }
    if !pending.is_empty() {
        match run_post_execute(post_execute, pending.clone()).await {
            Ok(reply) => {
                for error in reply["errors"].as_array().into_iter().flatten() {
                    let _ = crate::diagnostic_log::append_diagnostic_log(
                        "extension_gateway.post_execute_failed",
                        json!({ "extension": error.get("id"), "error": error.get("error") }),
                    );
                }
                let mut cache = POST_EXECUTE_RESULTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                if cache.len() > POST_EXECUTE_RESULT_LIMIT {
                    cache.clear();
                }
                for (key, value) in reply["results"].as_object().into_iter().flatten() {
                    cache.insert(key.clone(), (!value.is_null()).then(|| value.clone()));
                }
            }
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "extension_gateway.post_execute_failed",
                    json!({ "error": format!("{error:#}") }),
                );
                // 整条链超时 / 失败：这批输出记成「不改」，免得之后每个请求都再等一遍。
                let mut cache = POST_EXECUTE_RESULTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                for item in &pending {
                    if let Some(key) = item["key"].as_str() {
                        cache.insert(key.to_string(), None);
                    }
                }
                return;
            }
        }
    }
    let cache = POST_EXECUTE_RESULTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for item in input.iter_mut().filter(|item| is_tool_output(item)) {
        let call_id = item.get("call_id").and_then(Value::as_str).unwrap_or_default().to_string();
        let output = item.get("output").cloned().unwrap_or(Value::Null);
        if let Some(Some(next)) = cache.get(&key_of(&call_id, &output)) {
            item["output"] = next.clone();
        }
    }
}

async fn run_post_execute(handlers: &[(String, crate::user_scripts::ExtensionHook)], items: Vec<Value>) -> anyhow::Result<Value> {
    let node = crate::extension_node::require_node()?;
    let input = json!({
        "items": items,
        "handlers": handlers.iter().map(|(id, hook)| json!({
            "id": id, "script": hook.script, "matcher": hook.matcher,
            "timeoutMs": hook.timeout_secs * 1000, "config": hook.config,
        })).collect::<Vec<_>>(),
    });
    let total = handlers.iter().map(|(_, hook)| hook.timeout_secs).sum::<u64>() * (items_len(&input) as u64).max(1) + 2;
    crate::extension_node::run_runner(&node, POST_EXECUTE_RUNNER, &input, Duration::from_secs(total).min(CHAIN_LIMIT)).await
}

fn items_len(input: &Value) -> usize {
    input["items"].as_array().map(Vec::len).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// 请求失败重试（gateway.error）：上游返回错误状态时，把 { status, body, path, attempt } 交给拓展，
// 返回 { retry: true, delayMs } 或 { kind: "retry", delayMs } 即重发（最多 3 次，间隔最多 10 秒）。
// ---------------------------------------------------------------------------------------------

const ERROR_RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
const errors = [];
let decision = null;
for (const item of input.middlewares) {
  let timer;
  try {
    const module = await import(pathToFileURL(item.script).href);
    const fn = typeof module.default === "function" ? module.default : module.error;
    if (typeof fn !== "function") throw new Error(`${item.script} 没有默认导出函数`);
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${item.timeoutMs / 1000} 秒）`)), item.timeoutMs); });
    const result = await Promise.race([Promise.resolve(fn(structuredClone(input.error), { path: input.error.path, extension: item.id, config: item.config ?? {} })), timeout]);
    if (result && (result.retry === true || result.kind === "retry")) { decision = { id: item.id, delayMs: Number(result.delayMs) || 0 }; break; }
    if (result && result.kind === "fallback" && typeof result.model === "string" && result.model.trim()) { decision = { id: item.id, delayMs: Number(result.delayMs) || 0, model: result.model.trim() }; break; }
  } catch (error) {
    errors.push({ id: item.id, error: String(error?.stack || error?.message || error) });
  } finally {
    clearTimeout(timer);
  }
}
process.stdout.write(JSON.stringify({ decision, errors }));
"#;

static ERROR_CACHE: std::sync::Mutex<Option<(std::time::Instant, MiddlewareList)>> = std::sync::Mutex::new(None);
/// 交给拓展的错误响应体最多多少字节。
const ERROR_BODY_LIMIT: usize = 64 * 1024;
/// 拓展要求的重试间隔上限。
const RETRY_DELAY_LIMIT: Duration = Duration::from_secs(10);

fn cached_error_middlewares() -> MiddlewareList {
    let mut cache = ERROR_CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < MIDDLEWARE_CACHE_TTL {
            return list.clone();
        }
    }
    let list = crate::user_scripts::default_user_script_manager()
        .enabled_gateway_error_middlewares()
        .unwrap_or_default();
    *cache = Some((std::time::Instant::now(), list.clone()));
    list
}

/// 失败中间件的决定：等多久重发；`model` 非空时换这个模型重发（降级）。
#[derive(Debug, Clone, PartialEq)]
pub struct RetryDecision {
    pub delay: Duration,
    pub model: Option<String>,
}

/// 上游返回错误时问拓展要不要重试；要就返回等待间隔。读过的响应体会放回 `buffered_body`，
/// 不重试时调用方照常拿到原错误。
pub async fn decide_retry(
    response: &mut crate::protocol_proxy::UpstreamProxyResponse,
    request_path: &str,
    attempt: usize,
) -> Option<RetryDecision> {
    if already_applied() {
        return None;
    }
    let middlewares = cached_error_middlewares();
    decide_retry_with(&middlewares, response, request_path, attempt).await
}

pub async fn decide_retry_with(
    middlewares: &[(String, ExtensionGatewayMiddleware)],
    response: &mut crate::protocol_proxy::UpstreamProxyResponse,
    request_path: &str,
    attempt: usize,
) -> Option<RetryDecision> {
    if middlewares.is_empty() {
        return None;
    }
    let body = match response.read_body().await {
        Ok(body) => body,
        Err(_) => return None,
    };
    response.buffered_body = Some(body.clone());
    decide_retry_for(middlewares, response.status_code, &body, request_path, attempt).await
}

/// 按状态码与响应体问拓展要不要重试。
pub async fn decide_retry_for(
    middlewares: &[(String, ExtensionGatewayMiddleware)],
    status: u16,
    body: &[u8],
    request_path: &str,
    attempt: usize,
) -> Option<RetryDecision> {
    if middlewares.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&body[..body.len().min(ERROR_BODY_LIMIT)]).to_string();
    let error = json!({
        "status": status,
        "body": text,
        "json": serde_json::from_slice::<Value>(body).unwrap_or(Value::Null),
        "path": request_path,
        "attempt": attempt,
        "session": request_session(),
    });
    let node = crate::extension_node::require_node().ok()?;
    let input = json!({
        "error": error,
        "middlewares": middlewares.iter().map(|(id, item)| json!({
            "id": id, "script": item.script, "timeoutMs": item.timeout_secs * 1000, "config": item.config,
        })).collect::<Vec<_>>(),
    });
    let limit = Duration::from_secs(middlewares.iter().map(|(_, item)| item.timeout_secs).sum::<u64>() + 2).min(CHAIN_LIMIT);
    match crate::extension_node::run_runner(&node, ERROR_RUNNER, &input, limit).await {
        Ok(reply) => {
            for failure in reply["errors"].as_array().into_iter().flatten() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "extension_gateway.error_middleware_failed",
                    json!({ "extension": failure.get("id"), "error": failure.get("error") }),
                );
            }
            let decision = reply.get("decision").filter(|decision| decision.is_object())?;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_gateway.retry",
                json!({ "extension": decision.get("id"), "status": status, "attempt": attempt, "model": decision.get("model") }),
            );
            Some(RetryDecision {
                delay: Duration::from_millis(decision["delayMs"].as_u64().unwrap_or(0)).min(RETRY_DELAY_LIMIT),
                model: decision.get("model").and_then(Value::as_str).map(str::to_string),
            })
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_gateway.error_chain_failed",
                json!({ "error": format!("{error:#}") }),
            );
            None
        }
    }
}

/// `credential_owners`：upstream 段带来的这个候选的 Key（上下文里的 `__credential`）能交给哪些拓展——
/// 关掉了 builtin:auth-headers 的那些（空表示全部，只在测试固定被关掉的环节时）；`None` 时谁都不给。
async fn apply_middlewares(middlewares: &[(String, ExtensionGatewayMiddleware)], request: &mut Value, request_path: &str, mut context: Value, credential_owners: Option<Vec<String>>) -> Result<(), String> {
    let upstream_stage = !context.is_null();
    let credential = context
        .as_object_mut()
        .and_then(|map| map.remove("__credential"))
        .filter(|_| credential_owners.is_some());
    match run_chain(middlewares, request, request_path, context, credential.as_ref().zip(credential_owners.as_deref())).await {
        Ok(result) => {
            if let Some(rejected) = result.get("rejected").filter(|value| value.is_object()) {
                let reason = rejected.get("reason").and_then(Value::as_str).unwrap_or("拓展拒绝了这一步").to_string();
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "extension_gateway.request_rejected",
                    json!({ "path": request_path, "extension": rejected.get("id"), "reason": reason }),
                );
                return Err(reason);
            }
            for error in result["errors"].as_array().into_iter().flatten() {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "extension_gateway.middleware_failed",
                    json!({ "path": request_path, "extension": error.get("id"), "error": error.get("error") }),
                );
            }
            if result["changed"] == true && result["body"].is_object() {
                *request = result["body"].clone();
            }
            // upstream 段加的请求头接在 request 段的后面。
            let mut headers = if upstream_stage { extension_headers() } else { Vec::new() };
            headers.extend(allowed_headers(&result["headers"]));
            if credential.is_some() {
                headers.extend(allowed_auth_headers(&result["authHeaders"]));
            }
            set_extension_headers(headers);
            Ok(())
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_gateway.chain_failed",
                json!({ "path": request_path, "error": format!("{error:#}") }),
            );
            Ok(())
        }
    }
}

async fn run_chain(
    middlewares: &[(String, ExtensionGatewayMiddleware)],
    request: &Value,
    request_path: &str,
    context: Value,
    // (凭据, 能拿到它的拓展 id；空表示全部，测试固定被关掉的环节时)
    credential: Option<(&Value, &[String])>,
) -> anyhow::Result<Value> {
    let node = crate::extension_node::require_node()?;
    let input = json!({
        "path": request_path,
        "body": request,
        "context": context,
        "step": request_step(request),
        "middlewares": middlewares.iter().map(|(id, item)| {
            let mut entry = json!({
                "id": id,
                "script": item.script,
                "timeoutMs": item.timeout_secs * 1000,
                "config": item.config,
            });
            if let Some((credential, owners)) = credential
                && (owners.is_empty() || owners.iter().any(|owner| owner == id))
            {
                entry["credential"] = credential.clone();
            }
            entry
        }).collect::<Vec<_>>(),
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

/// 常驻的响应流工作进程。进程与读回复的线程都不挂在任何 tokio 运行时上：
/// 工作进程是全进程共用的，第一次用到它的运行时可能先结束（测试的每个用例、临时运行时）。
/// 挂在那个运行时上时，Windows 的管道读取在阻塞线程池里永远等不到 EOF，运行时关停会一直
/// 卡住；Unix 上读回复的任务随运行时消失，之后的请求只会超时。
struct ResponseWorker {
    stdin: std::sync::Arc<std::sync::Mutex<std::process::ChildStdin>>,
    pending: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>>,
    alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    child: std::sync::Mutex<std::process::Child>,
}

impl Drop for ResponseWorker {
    fn drop(&mut self) {
        // 换掉旧工作进程（中间件变了）时结束它；读线程随之读到 EOF 退出。
        let mut child = self.child.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
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
    use std::io::BufRead;
    let node = crate::extension_node::require_node()?;
    let mut command = std::process::Command::new(node);
    command
        .args(["--input-type=module", "-e", RESPONSE_RUNNER])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Ok(exe) = std::env::current_exe() {
        command.env(crate::extension_backend::HOST_EXE_ENV, exe);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_create_no_window());
    }
    let mut child = command.spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("响应流工作进程没有 stdin"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("响应流工作进程没有 stdout"))?;
    let pending: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<u64, tokio::sync::oneshot::Sender<Value>>>> = Default::default();
    let alive = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    {
        let pending = pending.clone();
        let alive = alive.clone();
        std::thread::Builder::new()
            .name("codex-plus-response-worker".into())
            .spawn(move || {
                for line in std::io::BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(message) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = message.get("id").and_then(Value::as_u64) {
                        let sender = pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&id);
                        if let Some(sender) = sender {
                            let _ = sender.send(message);
                        }
                    }
                }
                alive.store(false, std::sync::atomic::Ordering::SeqCst);
                for (_, sender) in pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain() {
                    let _ = sender.send(json!({ "error": "响应流工作进程已退出" }));
                }
            })?;
    }
    Ok(ResponseWorker {
        stdin: std::sync::Arc::new(std::sync::Mutex::new(stdin)),
        pending,
        alive,
        child: std::sync::Mutex::new(child),
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
        use std::io::Write;
        // 写管道可能在对方读得慢时阻塞：放到阻塞线程里写，写完即返回（不会长时间占着线程）。
        let stdin = self.stdin.clone();
        let line = format!("{message}\n");
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut stdin = stdin.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            stdin.write_all(line.as_bytes())?;
            stdin.flush()
        })
        .await??;
        Ok(())
    }

    async fn request(&self, mut message: Value, timeout: Duration) -> anyhow::Result<Value> {
        let id = RESPONSE_IDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        message["id"] = json!(id);
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).insert(id, sender);
        if let Err(error) = self.send(message).await {
            self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(reply)) => match reply.get("error").and_then(Value::as_str) {
                Some(error) => anyhow::bail!("{error}"),
                None => Ok(reply),
            },
            Ok(Err(_)) => anyhow::bail!("响应流工作进程没有回应"),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&id);
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
        // Codex 把会话 id 放在 prompt_cache_key：中间件据此认出是哪个会话。
        "sessionId": request.get("prompt_cache_key").cloned().unwrap_or(Value::Null),
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
    /// 已向 worker 发过 end / abort，会话已在 worker 侧清理。
    closed: bool,
}

/// 回复中途断开（Codex 取消、连接中断）时 filter 被直接丢弃，不会走到 finish：
/// 补发 abort，免得 worker 里的中间件会话一直挂着。
impl Drop for ResponseFilter {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let worker = self.worker.clone();
        let rid = self.rid;
        handle.spawn(async move {
            let _ = worker.send(json!({ "op": "abort", "rid": rid })).await;
        });
    }
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
            Ok((worker, rid)) => Some(Self { worker, rid, timeout, buffer: Vec::new(), event_lines: false, passthrough: false, done: false, closed: false }),
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
        self.closed = true;
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

    /// 把本会话用的常驻 worker 从全局槽位撤下（槽位里已是别的 worker 时不动）。
    /// 生产里 worker 随进程常驻；测试每个用例各有一个 tokio 运行时，Windows 上子进程 stdout 的读走阻塞线程池，
    /// 运行时销毁要等那次读返回，读又要等子进程退出，所以收尾时撤下槽位，让最后一个引用丢掉后被 kill_on_drop 杀掉。
    #[doc(hidden)]
    pub async fn release_worker(&self) {
        let mut slot = RESPONSE_WORKER.lock().await;
        if slot.as_ref().is_some_and(|(_, worker)| std::sync::Arc::ptr_eq(worker, &self.worker)) {
            *slot = None;
        }
    }

    /// 全局槽位里现在有没有常驻 worker，供测试断言 `release_worker` 的行为。
    #[doc(hidden)]
    pub async fn worker_installed() -> bool {
        RESPONSE_WORKER.lock().await.is_some()
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
                self.closed = true;
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
