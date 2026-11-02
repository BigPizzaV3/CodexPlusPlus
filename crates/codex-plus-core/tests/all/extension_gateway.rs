//! 拓展的网关请求中间件：清单校验与用真实 Node 跑通的串联改写。

use codex_plus_core::extension_gateway::apply_request_with;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, gateway: serde_json::Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(dir.join("gateway")).unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "gateway": gateway}).to_string()).unwrap();
    for (path, content) in files {
        fs::write(dir.join(path), content).unwrap();
    }
}

#[test]
fn gateway_declarations_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-kind", json!({"stream": []}), &[]);
    write_package(&temp, "bad-timeout", json!({"request": [{"script": "gateway/a.mjs", "timeout": 60}]}), &[("gateway/a.mjs", "export default () => null")]);
    write_package(&temp, "ok", json!({"request": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", "export default () => null")]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-kind")["error"].as_str().unwrap().contains("只支持 request"));
    assert!(find("user:bad-timeout")["error"].as_str().unwrap().contains("timeout"));
    assert_eq!(find("user:ok")["error"], "");
    assert_eq!(find("user:ok")["package"]["gatewayRequest"], 1);
}

#[tokio::test]
async fn request_middlewares_chain_in_order_and_failures_keep_the_request() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-prefix", json!({"request": [{"script": "gateway/a.mjs"}]}),
        &[("gateway/a.mjs", r#"export default (body, { path }) => ({ ...body, instructions: `${body.instructions}|A:${path}` });"#)]);
    write_package(&temp, "b-broken", json!({"request": [{"script": "gateway/b.mjs"}]}),
        &[("gateway/b.mjs", "export default () => { throw new Error('坏中间件'); }")]);
    write_package(&temp, "c-suffix", json!({"request": [{"script": "gateway/c.mjs"}]}),
        &[("gateway/c.mjs", r#"export default async (body) => ({ ...body, instructions: `${body.instructions}|C` });"#)]);

    let mut request = json!({"model": "gpt", "instructions": "原始", "input": []});
    apply_request_with(&manager, &mut request, "/v1/responses").await;
    assert_eq!(request["instructions"], "原始|A:/v1/responses|C", "按 key 顺序串联，坏中间件被跳过");
    assert_eq!(request["model"], "gpt");

    // 停用后不再改写；全部停用时零开销、原样返回。
    manager.set_script_enabled("user:a-prefix", false).unwrap();
    manager.set_script_enabled("user:c-suffix", false).unwrap();
    let mut untouched = json!({"instructions": "原始"});
    apply_request_with(&manager, &mut untouched, "/v1/responses").await;
    assert_eq!(untouched["instructions"], "原始");
}

fn sse(events: &[serde_json::Value]) -> Vec<u8> {
    events
        .iter()
        .map(|event| format!("event: {}\ndata: {}\n\n", event["type"].as_str().unwrap(), event))
        .collect::<String>()
        .into_bytes()
}

fn parse_sse(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let data = block.lines().find_map(|line| line.strip_prefix("data: ")).unwrap_or(block);
            match serde_json::from_str::<serde_json::Value>(data) {
                Ok(event) => format!("{}:{}", event["type"].as_str().unwrap_or(""), event["delta"].as_str().or(event["text"].as_str()).unwrap_or("")),
                Err(_) => block.to_string(),
            }
        })
        .collect()
}

async fn run_filter(manager: &UserScriptManager, chunks: &[&[u8]]) -> Vec<u8> {
    // 响应 worker 槽位是进程级的，并行的用例会互相换掉它：整段持锁，到 release_worker 才放。
    let _state = crate::common::global_state_lock();
    let middlewares = manager.enabled_gateway_response_middlewares().unwrap();
    let request = json!({"model": "gpt", "input": [{"role": "user", "content": [{"type": "input_text", "text": "最后一句"}]}]});
    let mut filter = codex_plus_core::extension_gateway::ResponseFilter::start_with(&middlewares, "/v1/responses", Some(&request))
        .await
        .expect("有中间件时应建立会话");
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(filter.push(chunk).await);
    }
    out.extend(filter.finish().await);
    // 撤下全局 worker 槽位，免得 Windows 上运行时销毁卡在子进程管道的阻塞读上。
    filter.release_worker().await;
    out
}

#[tokio::test]
async fn response_middlewares_wrap_the_stream_in_order() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    // a 在外层：把文字增量转大写（也作用于 b 插入的事件）。
    write_package(&temp, "a-upper", json!({"response": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", r#"
export default (context, next) => (async function* () {
  for await (const event of next()) {
    if (event.type === "response.output_text.delta") yield { ...event, delta: event.delta.toUpperCase() };
    else if (event.type === "x.note") yield { ...event, text: event.text.toUpperCase() };
    else yield event;
  }
})();"#)]);
    // b 在内层：完成前插入一条带用户最后一句话的事件，流尾再追加一条。
    write_package(&temp, "b-insert", json!({"response": [{"script": "gateway/b.mjs"}]}), &[("gateway/b.mjs", r#"
export default async function* (context, next) {
  for await (const event of next()) {
    if (event.type === "response.completed") yield { type: "x.note", text: `${context.extension}:${context.request.lastUserText}` };
    yield event;
  }
  yield { type: "x.tail", text: context.path };
}"#)]);
    let head = sse(&[json!({"type": "response.created"}), json!({"type": "response.output_text.delta", "delta": "hello"})]);
    let tail = sse(&[json!({"type": "response.completed"})]);
    // 第二个事件被切成两半送来；心跳注释夹在事件边界上。
    let (first, second) = head.split_at(head.len() - 10);
    let ping = b": ping\n\n".to_vec();
    let output = parse_sse(&run_filter(&manager, &[first, second, &ping, &tail]).await);
    assert_eq!(output, [
        "response.created:",
        "response.output_text.delta:HELLO",
        ": ping",
        "x.note:B-INSERT:最后一句",
        "response.completed:",
        "x.tail:/v1/responses",
    ], "按 key 嵌套（先声明的在外层），跨块切分的事件也能拼回；注释块原样放行");
}

#[tokio::test]
async fn response_middleware_failures_pass_the_rest_through() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "broken", json!({"response": [{"script": "gateway/r.mjs", "timeout": 1}]}), &[("gateway/r.mjs", r#"
export default async function* (context, next) {
  let count = 0;
  for await (const event of next()) {
    count += 1;
    if (count === 2) throw new Error("坏中间件");
    yield { ...event, seen: true };
  }
}"#)]);
    let source = sse(&[
        json!({"type": "response.created"}),
        json!({"type": "response.output_text.delta", "delta": "a"}),
        json!({"type": "response.completed"}),
    ]);
    let raw = String::from_utf8(run_filter(&manager, &[&source]).await).unwrap();
    assert!(raw.contains("\"seen\":true"), "出错前的事件经过了中间件：{raw}");
    assert!(raw.contains("response.output_text.delta") && raw.contains("response.completed"), "出错后的事件原样放行：{raw}");

    // 超时：中间件卡住不交回事件，超时后原样放行，不挡住回复。
    manager.set_script_enabled("user:broken", false).unwrap();
    write_package(&temp, "stuck", json!({"response": [{"script": "gateway/s.mjs", "timeout": 1}]}), &[("gateway/s.mjs", r#"
export default async function* (context, next) {
  for await (const event of next()) {
    await new Promise(() => {});
    yield event;
  }
}"#)]);
    let started = std::time::Instant::now();
    let raw = String::from_utf8(run_filter(&manager, &[&source]).await).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(parse_sse(raw.as_bytes()), ["response.created:", "response.output_text.delta:a", "response.completed:"]);
}

#[tokio::test]
async fn releasing_a_filter_leaves_a_newer_worker_in_the_slot() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    use codex_plus_core::extension_gateway::ResponseFilter;
    let _state = crate::common::global_state_lock();
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let pass = "export default (context, next) => next();";
    write_package(&temp, "a-pass", json!({"response": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", pass)]);
    let first = manager.enabled_gateway_response_middlewares().unwrap();
    let older = ResponseFilter::start_with(&first, "/v1/responses", None).await.expect("应建立会话");
    // 多一个中间件，worker 的指纹变了：槽位换成新的 worker。
    write_package(&temp, "b-pass", json!({"response": [{"script": "gateway/b.mjs"}]}), &[("gateway/b.mjs", pass)]);
    let second = manager.enabled_gateway_response_middlewares().unwrap();
    let newer = ResponseFilter::start_with(&second, "/v1/responses", None).await.expect("应建立会话");

    older.release_worker().await;
    assert!(ResponseFilter::worker_installed().await, "槽位里是更新的 worker，旧过滤器收尾不能把它撤掉");
    newer.release_worker().await;
    assert!(!ResponseFilter::worker_installed().await, "自己的 worker 收尾时应撤下");
}

#[tokio::test]
async fn middlewares_run_once_when_the_launcher_forwards_to_the_local_gateway() {
    use codex_plus_core::extension_gateway::{EXTENSIONS_ALREADY_APPLIED, EXTENSIONS_APPLIED_HEADER, ResponseFilter, apply_request, headers_mark_applied};
    // 启动器转给本机网关时带的标记头，大小写不敏感；请求行本身不算。
    assert!(headers_mark_applied(&format!("POST /gw/codex/v1/responses HTTP/1.1\r\nHost: x\r\n{}: 1\r\n", EXTENSIONS_APPLIED_HEADER.to_uppercase())));
    assert!(!headers_mark_applied("POST /gw/codex/v1/responses HTTP/1.1\r\nHost: x\r\n"));
    assert!(!headers_mark_applied(&format!("GET /{EXTENSIONS_APPLIED_HEADER}: HTTP/1.1\r\n")));
    // 标记过的请求：网关这一层的请求、响应中间件都不跑（不读拓展目录，原样返回）。
    EXTENSIONS_ALREADY_APPLIED
        .scope(true, async {
            let mut request = json!({"instructions": "原始"});
            apply_request(&mut request, "/v1/responses").await;
            assert_eq!(request, json!({"instructions": "原始"}));
            assert!(ResponseFilter::start("/v1/responses", None).await.is_none());
        })
        .await;
}

#[tokio::test]
async fn error_middlewares_decide_whether_to_retry() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    use codex_plus_core::extension_gateway::{RetryDecision, decide_retry_for};
    let wait = |ms: u64| Some(RetryDecision { delay: std::time::Duration::from_millis(ms), model: None });
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-broken", json!({"error": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", "export default () => { throw new Error('坏'); }")]);
    write_package(&temp, "b-retry", json!({"error": [{"script": "gateway/b.mjs"}]}), &[("gateway/b.mjs", r#"
export default (error, { path }) => {
  if (error.status === 429 && error.attempt < 2) return { kind: "retry", delayMs: error.json?.retryAfterMs ?? 50 };
  if (error.status === 503 && path === "/v1/responses") return { retry: true, delayMs: 999999 };
  if (error.status === 404) return { kind: "fallback", model: " gpt-5-mini " };
  return null;
};"#)]);
    let list = manager.enabled_gateway_error_middlewares().unwrap();
    assert_eq!(list.len(), 2);
    let retry = decide_retry_for(&list, 429, br#"{"retryAfterMs": 120}"#, "/v1/responses", 0).await;
    assert_eq!(retry, wait(120), "坏中间件被跳过，后面的照常决定");
    assert_eq!(decide_retry_for(&list, 429, b"{}", "/v1/responses", 2).await, None, "拓展自己限制次数");
    assert_eq!(decide_retry_for(&list, 400, b"bad request", "/v1/responses", 0).await, None);
    assert_eq!(decide_retry_for(&list, 503, b"", "/v1/responses", 0).await, wait(10_000), "间隔最多 10 秒");
    assert_eq!(
        decide_retry_for(&list, 404, b"", "/v1/responses", 0).await,
        Some(RetryDecision { delay: std::time::Duration::ZERO, model: Some("gpt-5-mini".into()) }),
        "降级：换模型重发"
    );
    assert_eq!(decide_retry_for(&[], 503, b"", "/v1/responses", 0).await, None, "没有中间件时不重试");
}

#[test]
fn request_step_counts_tool_results_since_the_last_user_message() {
    use codex_plus_core::extension_gateway::request_step;
    let first = request_step(&json!({"input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "旧问题"}]},
        {"type": "function_call_output", "call_id": "old", "output": "x"},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "新问题"}]},
    ]}));
    assert_eq!(first, json!({"index": 1, "toolResults": 0, "userText": "新问题", "firstStep": true}));
    let third = request_step(&json!({"input": [
        {"role": "user", "content": "新问题"},
        {"type": "function_call", "call_id": "a", "name": "shell", "arguments": "{}"},
        {"type": "function_call_output", "call_id": "a", "output": "1"},
        {"type": "custom_tool_call_output", "call_id": "b", "output": "2"},
    ]}));
    assert_eq!(third["index"], 3);
    assert_eq!(third["firstStep"], false);
    assert_eq!(third["userText"], "新问题");
}

#[tokio::test]
async fn request_middlewares_see_the_step_and_can_reject_it() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-gate", json!({"request": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", r#"
export default (body, { step }) => {
  if (step.index > 2) return { reject: `第 ${step.index} 步超过上限` };
  return { ...body, instructions: `step=${step.index}` };
};"#)]);
    write_package(&temp, "b-after", json!({"request": [{"script": "gateway/b.mjs"}]}), &[("gateway/b.mjs", r#"
export default (body) => ({ ...body, instructions: body.instructions + "|b" });"#)]);
    let mut first = json!({"input": [{"role": "user", "content": "hi"}]});
    apply_request_with(&manager, &mut first, "/v1/responses").await.unwrap();
    assert_eq!(first["instructions"], "step=1|b");
    let mut third = json!({"input": [
        {"role": "user", "content": "hi"},
        {"type": "function_call_output", "call_id": "a", "output": "1"},
        {"type": "function_call_output", "call_id": "b", "output": "2"},
    ]});
    let rejected = apply_request_with(&manager, &mut third, "/v1/responses").await.unwrap_err();
    assert_eq!(rejected, "第 3 步超过上限");
    assert!(third.get("instructions").is_none(), "拒绝时不改请求，后面的中间件也不跑");
}

/// gateway.request 的 stage / order 与 gateway.replaces：清单校验，request 段按 order 排序，
/// upstream 段单独跑并拿到上游的上下文，replaces 列出被关掉的内置环节。
#[tokio::test]
async fn request_stages_order_and_builtin_replaces() {
    use codex_plus_core::extension_gateway::apply_upstream_request_with;
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-stage", json!({"response": [{"script": "gateway/a.mjs", "stage": "upstream"}]}), &[("gateway/a.mjs", "export default () => null")]);
    write_package(&temp, "bad-replace", json!({"replaces": ["builtin:routing"]}), &[]);
    write_package(&temp, "a-late", json!({"request": [{"script": "gateway/a.mjs", "order": 10}]}),
        &[("gateway/a.mjs", r#"export default (body) => ({ ...body, instructions: `${body.instructions}|late` });"#)]);
    write_package(&temp, "b-early", json!({
        "request": [
            {"script": "gateway/b.mjs", "order": -5},
            {"script": "gateway/up.mjs", "stage": "upstream"}
        ],
        "replaces": ["builtin:local-compaction"]
    }), &[
        ("gateway/b.mjs", r#"export default (body) => ({ ...body, instructions: `${body.instructions}|early` });"#),
        ("gateway/up.mjs", r#"export default (body, context) => ({ ...body, seen: `${context.stage}:${context.wireApi}:${context.relay.id}` });"#),
    ]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-stage")["error"].as_str().unwrap().contains("stage"), "{}", find("user:bad-stage"));
    assert!(find("user:bad-replace")["error"].as_str().unwrap().contains("builtin:local-compaction"));
    assert_eq!(find("user:b-early")["package"]["gatewayReplaces"], json!(["builtin:local-compaction"]));
    assert_eq!(manager.enabled_gateway_replaces().unwrap(), vec![("b-early".to_string(), "builtin:local-compaction".to_string())]);

    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    // request 段：order 小的先跑，upstream 段的中间件不在这里。
    let mut request = json!({"model": "gpt", "instructions": "原始", "input": []});
    apply_request_with(&manager, &mut request, "/v1/responses").await.unwrap();
    assert_eq!(request["instructions"], "原始|early|late");
    assert!(request.get("seen").is_none());
    // upstream 段：拿到 Codex++ 转换后的请求体与上游信息。
    let mut upstream = json!({"model": "deepseek-chat", "messages": []});
    apply_upstream_request_with(&manager, &mut upstream, "/v1/responses", json!({"stage": "upstream", "wireApi": "chat", "relay": {"id": "r1"}})).await.unwrap();
    assert_eq!(upstream["seen"], "upstream:chat:r1");
}

/// 本地代理与网关模式的内置环节都能写进 gateway.replaces；不认识的名字报错并列出可用的。
#[test]
fn replaces_accepts_proxy_and_gateway_builtin_segments() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let opened = [
        "builtin:auto-retry",
        "builtin:foreign-reasoning",
        "builtin:unlisted-model-mapping",
        "builtin:key-cooldown",
        "builtin:key-breaker",
        "builtin:local-rate-limit",
        "builtin:title-model",
        "builtin:request-conversion",
        "builtin:auth-headers",
        "builtin:official-shaping",
        "builtin:request-log",
    ];
    write_package(&temp, "takeover", json!({ "request": [{ "script": "gateway/a.mjs" }], "replaces": opened }),
        &[("gateway/a.mjs", "export default () => null")]);
    write_package(&temp, "bad-auth", json!({ "replaces": ["builtin:auth-header"] }), &[]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert_eq!(find("user:takeover")["error"], "", "{}", find("user:takeover"));
    assert_eq!(find("user:takeover")["package"]["gatewayReplaces"], json!(opened));
    assert!(find("user:bad-auth")["error"].as_str().unwrap().contains("builtin:key-cooldown"));
    let replaces = manager.enabled_gateway_replaces().unwrap();
    assert_eq!(replaces.len(), opened.len());
    assert!(replaces.iter().all(|(key, _)| key == "takeover"));
}

/// route 段：拓展给的候选供应商顺序（可删减），认不得的 id 丢掉，一个都不认得时照原顺序。
#[tokio::test]
async fn route_middlewares_reorder_candidate_relays() {
    use codex_plus_core::extension_gateway::route_order_with_manager;
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "router", json!({ "request": [{ "script": "gateway/route.mjs", "stage": "route" }], "replaces": ["builtin:relay-rotation"] }),
        &[("gateway/route.mjs", r#"export default (body, context) => (context.stage === "route" && body.model === "m-cheap" ? { order: ["b", "zzz"] } : { ...body, candidates: [...body.candidates].reverse() });"#)]);
    let candidates = vec![json!({"id": "a"}), json!({"id": "b"}), json!({"id": "c"})];
    assert_eq!(route_order_with_manager(&manager, &candidates, json!({"model": "m-cheap"})).await, Some(vec!["b".to_string()]));
    assert_eq!(route_order_with_manager(&manager, &candidates, json!({"model": "m"})).await, Some(vec!["c".to_string(), "b".to_string(), "a".to_string()]));
    assert_eq!(manager.enabled_gateway_replaces().unwrap(), vec![("router".to_string(), "builtin:relay-rotation".to_string())]);
    manager.set_script_enabled("user:router", false).unwrap();
    assert_eq!(route_order_with_manager(&manager, &candidates, json!({"model": "m"})).await, None);
}

/// 关掉 builtin:auth-headers 的拓展在 upstream 段拿到这个候选的 Key，能设鉴权头；别的拓展拿不到 Key，
/// 设的鉴权头也被丢掉。
#[tokio::test]
async fn auth_headers_takeover_gets_the_credential_and_may_set_auth_headers() {
    use codex_plus_core::extension_gateway::{apply_upstream_request_with, extension_headers, EXTENSION_HEADERS};
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-auth", json!({ "request": [{ "script": "gateway/up.mjs", "stage": "upstream" }], "replaces": ["builtin:auth-headers"] }),
        &[("gateway/up.mjs", r#"export default (body, context) => { context.headers.authorization = `Bearer ${context.credential.apiKey}`; context.headers["x-seen"] = "1"; return { ...body, authSaw: context.credential?.apiKey ?? null }; };"#)]);
    write_package(&temp, "b-other", json!({ "request": [{ "script": "gateway/up.mjs", "stage": "upstream" }] }),
        &[("gateway/up.mjs", r#"export default (body, context) => { context.headers["x-api-key"] = "stolen"; return { ...body, otherSaw: context.credential?.apiKey ?? null }; };"#)]);
    let mut body = json!({ "model": "m", "messages": [] });
    let context = json!({ "stage": "upstream", "wireApi": "chat", "relay": { "id": "r1" }, "__credential": { "apiKey": "sk-test" } });
    let headers = EXTENSION_HEADERS
        .scope(std::cell::RefCell::new(Vec::new()), async {
            apply_upstream_request_with(&manager, &mut body, "/v1/responses", context).await.unwrap();
            extension_headers()
        })
        .await;
    assert_eq!(body["authSaw"], "sk-test");
    assert_eq!(body["otherSaw"], serde_json::Value::Null, "没关掉鉴权环节的拓展拿不到 Key");
    assert!(headers.contains(&("authorization".to_string(), "Bearer sk-test".to_string())), "{headers:?}");
    assert!(headers.contains(&("x-seen".to_string(), "1".to_string())));
    assert!(!headers.iter().any(|(name, _)| name == "x-api-key"), "别的拓展设的鉴权头被丢掉：{headers:?}");
}
