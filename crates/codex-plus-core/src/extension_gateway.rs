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
    const next = await Promise.race([Promise.resolve(fn(structuredClone(body), { path: input.path, extension: item.id })), timeout]);
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

/// 用默认的用户脚本位置改写请求。没有中间件、或任何环节失败时不改 `request`。
pub async fn apply_request(request: &mut Value, request_path: &str) {
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
        })).collect::<Vec<_>>(),
    });
    let limit = Duration::from_secs(middlewares.iter().map(|(_, item)| item.timeout_secs).sum::<u64>() + 2).min(CHAIN_LIMIT);
    crate::extension_node::run_runner(&node, RUNNER, &input, limit).await
}

