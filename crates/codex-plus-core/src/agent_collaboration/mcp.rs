//! 专用的本机 MCP stdio 服务。协议数据是 stdout 的唯一输出。

use std::io::{BufRead, Write};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Store, TaskRecord, TaskRequest, executor, is_terminal};

pub const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_WAIT_MS: u64 = 30_000;
const MAX_CONCURRENT_WAITS: usize = 8;

const INSTRUCTIONS: &str = "在当前 Codex 对话中承接用户的自然语言委派。用户明确要求 DeepSeek/DeepSeek Harness 时调用 delegate_to_deepseek；明确要求 Claude/Claude Code 时调用 delegate_to_claude_code；明确要求另一个 Codex 时调用 delegate_to_codex，不能用默认 Agent 替换用户点名的对象。用户泛指委派给其他 Agent、未指定对象时使用 delegate_task，省略 agentId 会采用 Codex++ 配置的默认 Agent；显式 agentId 始终优先。用户的委派请求已构成执行该任务的授权，不要再要求每次确认，也不要让用户去管理面板手工提交任务。提供完整、自包含的 prompt 和已存在的绝对 cwd。工具可能编辑该目录并消耗原生 CLI 账户额度，权限仍由各 CLI 控制。返回 queued/taskId 只代表任务已接收；先继续独立工作，再用 wait_task（每次最多 30 秒）或 get_task 收取结果，必须把结果带回当前对话。pending=true 时可继续工作并稍后查询；只有 task.status=completed 才能报告子任务成功，failed/cancelled/interrupted 必须如实说明，结果不确定时禁止自动重发。同一 Agent 可以同时运行多个任务（并发上限在 Codex++ 中按 Agent 设置，默认 1），互不依赖的子任务可以连续派发、各自用 taskId 收取；运行中的任务达到上限时新任务会排队（返回 queued 和排队进度），任一任务结束后按提交顺序自动启动，不同 Agent 可并行。并发任务各自独立进程和会话，但会在各自 cwd 中直接改文件：可能同时运行的任务应使用不同的 cwd（例如各自的 git worktree），返回 cwdWarning 时说明同目录有其他任务可能同时运行；只有排队已满、目标未启用或不可用时才报错，此时报告该目标的问题，不擅自改派。continueTaskId 只续本工具先前创建的同 Agent、同目录已完成 CLI 会话。这些是外部 harness/CLI 协作任务，不是原生 spawn_agent 子树，也不能接管或唤醒同窗口中已有桌面会话。";

enum InputLine {
    Data(Vec<u8>),
    TooLong,
    Failed,
}

/// 不启用 tokio io-std；有界读取放到专用 blocking 线程，不阻塞 CLI worker。
pub async fn serve(store: Store) -> anyhow::Result<()> {
    let result = serve_loop(store).await;
    // EOF、协议输出失败和输入读取失败均收口到这里，只停止此进程拥有的任务。
    executor::shutdown_owned_tasks().await;
    result
}

async fn serve_loop(store: Store) -> anyhow::Result<()> {
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    tokio::task::spawn_blocking(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            match read_bounded_line(&mut reader) {
                Ok(None) => break,
                Ok(Some(line)) => {
                    if sender.blocking_send(line).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = sender.blocking_send(InputLine::Failed);
                    break;
                }
            }
        }
    });
    let mut waits = tokio::task::JoinSet::new();
    loop {
        // 等待某个任务时仍可处理取消、派发和其他查询；EOF 不等 30 秒，直接清理自有任务。
        let response = tokio::select! {
            line = receiver.recv() => match line {
                None => break,
                Some(InputLine::Data(line)) if line.iter().all(u8::is_ascii_whitespace) => None,
                Some(InputLine::Data(line)) => {
                    let request = serde_json::from_slice::<Value>(&line).ok();
                    if let Some(request) = request.filter(is_wait_request) {
                        if waits.len() >= MAX_CONCURRENT_WAITS {
                            let mut response = handle_request(&store, request);
                            if let Some(response) = response.as_mut() {
                                if response.get("result").is_some() && response["result"]["isError"] == false {
                                    response["result"] = tool_result(Err(anyhow::anyhow!("同时等待的请求过多，请稍后使用原 taskId 查询；不要重发任务")));
                                }
                            }
                            response
                        } else {
                            let store = store.clone();
                            waits.spawn(async move { handle_request_async(&store, request).await });
                            None
                        }
                    } else {
                        process_line(&store, &line)
                    }
                }
                Some(InputLine::TooLong) => Some(rpc_error(Value::Null, -32600, "请求行超过 128 KiB 上限")),
                Some(InputLine::Failed) => anyhow::bail!("无法读取 MCP stdin"),
            },
            completed = waits.join_next(), if !waits.is_empty() => {
                completed.context("等待请求状态不可用")?.context("等待请求执行失败")?
            }
        };
        if let Some(response) = response {
            write_response(&response)?;
        }
    }
    Ok(())
}

fn write_response(response: &Value) -> anyhow::Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer(&mut output, response).context("无法写入 MCP 响应")?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

fn is_wait_request(request: &Value) -> bool {
    request["method"] == "tools/call" && request["params"]["name"] == "wait_task"
}

async fn handle_request_async(store: &Store, request: Value) -> Option<Value> {
    // 先走同一套同步校验：通知、非法 RPC 和无效参数不会变成实际等待请求。
    let mut response = handle_request(store, request.clone())?;
    if is_wait_request(&request)
        && response["result"]["isError"] == false
        && response.get("error").is_none()
    {
        let arguments = request["params"]
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        response["result"] = tool_result(wait_task(store, arguments).await);
    }
    Some(response)
}

fn read_bounded_line(reader: &mut impl BufRead) -> std::io::Result<Option<InputLine>> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut consumed_any = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(if !consumed_any {
                None
            } else if too_long {
                Some(InputLine::TooLong)
            } else {
                Some(InputLine::Data(line))
            });
        }
        consumed_any = true;
        let take = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| position + 1)
            .unwrap_or(available.len());
        let ended = available[take - 1] == b'\n';
        if !too_long {
            if line.len().saturating_add(take) > MAX_REQUEST_BYTES {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(&available[..take]);
            }
        }
        reader.consume(take);
        if ended {
            return Ok(Some(if too_long {
                InputLine::TooLong
            } else {
                InputLine::Data(line)
            }));
        }
    }
}

/// 测试及嵌入时沿用 stdio 的错误语义；同步 wait_task 返回即时快照，不阻塞线程。
pub fn process_line(store: &Store, line: &[u8]) -> Option<Value> {
    if line.len() > MAX_REQUEST_BYTES {
        return Some(rpc_error(Value::Null, -32600, "请求行超过 128 KiB 上限"));
    }
    match serde_json::from_slice(line) {
        Ok(request) => handle_request(store, request),
        Err(_) => Some(rpc_error(Value::Null, -32700, "JSON 解析失败")),
    }
}

pub fn handle_request(store: &Store, request: Value) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(rpc_error(Value::Null, -32600, "请求必须是 JSON-RPC 对象"));
    };
    // 通知没有响应，包括未知通知；绝不把通知误当成工具执行请求。
    let Some(id) = object.get("id").cloned() else {
        return None;
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || !(id.is_string() || id.is_number() || id.is_null())
    {
        return Some(rpc_error(Value::Null, -32600, "JSON-RPC 请求格式无效"));
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(rpc_error(id, -32600, "缺少请求方法"));
    };
    let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => {
            let requested = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or("2024-11-05");
            let version = if matches!(requested, "2024-11-05" | "2025-03-26" | "2025-06-18") {
                requested
            } else {
                "2025-06-18"
            };
            json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "codex-plus-agent-collaboration", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_definitions()}),
        "tools/call" => match params.get("name").and_then(Value::as_str) {
            Some(name) => tool_result(call_tool(
                store,
                name,
                params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )),
            None => return Some(rpc_error(id, -32602, "tools/call 缺少工具名")),
        },
        _ => return Some(rpc_error(id, -32601, "未实现的 MCP 方法")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_result(result: anyhow::Result<Value>) -> Value {
    match result {
        Ok(value) => {
            json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false})
        }
        Err(error) => {
            json!({"content": [{"type": "text", "text": error.to_string()}], "isError": true})
        }
    }
}

fn required_id(arguments: &Value) -> anyhow::Result<&str> {
    arguments
        .get("taskId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .context("taskId 必须是任务返回的 ID")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DelegateArguments {
    #[serde(default)]
    agent_id: Option<String>,
    prompt: String,
    cwd: String,
    #[serde(default)]
    continue_task_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixedDelegateArguments {
    prompt: String,
    cwd: String,
    #[serde(default)]
    continue_task_id: Option<String>,
}

fn delegation_request(store: &Store, name: &str, arguments: Value) -> anyhow::Result<TaskRequest> {
    if name == "delegate_task" {
        let arguments: DelegateArguments =
            serde_json::from_value(arguments).context("delegate_task 参数无效")?;
        let agent_id = match arguments.agent_id {
            Some(agent_id) => agent_id,
            None => store.config()?.default_agent_id,
        };
        return Ok(TaskRequest {
            agent_id,
            prompt: arguments.prompt,
            cwd: arguments.cwd,
            continue_task_id: arguments.continue_task_id,
        });
    }
    let agent_id = match name {
        "delegate_to_deepseek" => "deepseek",
        "delegate_to_claude_code" => "claude",
        "delegate_to_codex" => "codex",
        _ => anyhow::bail!("未知委派工具"),
    };
    let arguments: FixedDelegateArguments = serde_json::from_value(arguments)
        .context("固定 Agent 委派仅接受 prompt、cwd、continueTaskId；不能通过 agentId 改派")?;
    Ok(TaskRequest {
        agent_id: agent_id.to_owned(),
        prompt: arguments.prompt,
        cwd: arguments.cwd,
        continue_task_id: arguments.continue_task_id,
    })
}

fn dispatch_task(store: &Store, request: TaskRequest) -> anyhow::Result<Value> {
    let task = store.submit(request)?;
    executor::spawn_task(store.clone(), task.id.clone());
    let mut result = json!({"taskId": task.id, "task": task, "pending": true,
        "next": "任务已接收，queued 不代表成功；同一 Agent 运行中的任务未达并发上限时直接启动，达到上限时排队，任一任务结束后按提交顺序自动启动（见 task.progress）。继续独立工作，随后使用 wait_task/get_task 查询这个 taskId，将终态和答案带回当前对话；不要重复派发。"});
    // 不强制分目录：同一 cwd 有其他可能同时运行的任务时只给一句提示（#2800）。
    if let Ok(count) = store.overlapping_cwd_tasks(&task.id)
        && count > 0
    {
        result["cwdWarning"] = json!(super::same_cwd_hint(count));
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitArguments {
    task_id: String,
    #[serde(default = "default_wait_ms")]
    timeout_ms: u64,
}

fn default_wait_ms() -> u64 {
    MAX_WAIT_MS
}

fn wait_arguments(arguments: Value) -> anyhow::Result<WaitArguments> {
    let arguments: WaitArguments =
        serde_json::from_value(arguments).context("wait_task 参数无效")?;
    if arguments.task_id.is_empty() || arguments.task_id.len() > 128 {
        anyhow::bail!("taskId 必须是任务返回的 ID");
    }
    if arguments.timeout_ms > MAX_WAIT_MS {
        anyhow::bail!("timeoutMs 必须在 0 到 30000 之间");
    }
    Ok(arguments)
}

fn wait_snapshot(task: TaskRecord, timed_out: bool) -> Value {
    let pending = !is_terminal(&task.status);
    let succeeded = task.status == "completed";
    let queued = task.status == "queued";
    json!({"taskId": task.id, "task": task, "pending": pending, "timedOut": timed_out && pending, "succeeded": succeeded,
        "next": if queued { "任务仍在排队，轮到后自动启动（见 task.progress）。继续独立工作或稍后用同一 taskId 调用 wait_task/get_task；不要重发任务。" } else if pending { "任务仍在运行。继续独立工作或稍后用同一 taskId 调用 wait_task/get_task；不要重发任务。" } else if succeeded { "任务已完成，将答案带回当前对话。" } else { "任务未成功完成，向用户如实汇报 task.status 和 task.error，不要自动重发或报告成功。" }})
}

async fn wait_task(store: &Store, arguments: Value) -> anyhow::Result<Value> {
    let arguments = wait_arguments(arguments)?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(arguments.timeout_ms);
    loop {
        let task = store.task(&arguments.task_id)?;
        if is_terminal(&task.status) {
            return Ok(wait_snapshot(task, false));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(wait_snapshot(task, true));
        }
        tokio::time::sleep_until(std::cmp::min(deadline, now + Duration::from_millis(200))).await;
    }
}

fn call_tool(store: &Store, name: &str, arguments: Value) -> anyhow::Result<Value> {
    if !arguments.is_object() {
        anyhow::bail!("工具参数必须是对象");
    }
    match name {
        "list_agents" => {
            let snapshot = store.snapshot()?;
            let enabled = snapshot["config"]["enabled"].as_bool().unwrap_or(false);
            let agents: Vec<Value> = snapshot["config"]["agents"].as_array().into_iter().flatten().map(|agent| {
                let entry = snapshot["availability"].as_array().into_iter().flatten()
                    .find(|entry| entry["id"] == agent["id"]);
                let flag = |key: &str| entry.and_then(|entry| entry[key].as_bool()).unwrap_or(false);
                // 已保存路径失效时如实告知，便于区分「没装」和「桌面版更新后路径变了」（#2751）。
                json!({"id": agent["id"], "name": agent["name"], "enabled": agent["enabled"], "available": flag("available"),
                "configuredPathMissing": flag("configuredMissing"), "usingFallbackPath": flag("recovered"),
                // 同一 Agent 最多同时运行的任务数；默认值不写进配置，缺省即 1（#2800）。
                "maxConcurrentTasks": agent["maxConcurrentTasks"].as_u64().unwrap_or(1)})
            }).collect();
            Ok(
                json!({"enabled": enabled, "defaultAgentId": snapshot["config"]["defaultAgentId"], "agents": agents, "desktopSessionWake": false,
                "availabilityMeaning": "available 仅表示找到可执行文件，未验证登录或协议兼容"}),
            )
        }
        "delegate_task"
        | "delegate_to_deepseek"
        | "delegate_to_claude_code"
        | "delegate_to_codex" => dispatch_task(store, delegation_request(store, name, arguments)?),
        "send_task" => {
            let request: TaskRequest =
                serde_json::from_value(arguments).context("send_task 参数无效")?;
            dispatch_task(store, request)
        }
        "get_task" => Ok(serde_json::to_value(store.task(required_id(&arguments)?)?)?),
        "wait_task" => {
            let arguments = wait_arguments(arguments)?;
            let mut snapshot =
                wait_snapshot(store.task(&arguments.task_id)?, arguments.timeout_ms == 0);
            snapshot["waitMode"] = json!("snapshot");
            Ok(snapshot)
        }
        "cancel_task" => Ok(serde_json::to_value(
            store.cancel(required_id(&arguments)?)?,
        )?),
        "list_tasks" => {
            let limit = arguments
                .get("limit")
                .map(|value| {
                    value
                        .as_u64()
                        .filter(|limit| *limit > 0 && *limit <= 50)
                        .context("limit 必须在 1 到 50 之间")
                })
                .transpose()?
                .unwrap_or(20) as usize;
            let mut snapshot = store.snapshot()?;
            let tasks = snapshot["tasks"]
                .as_array_mut()
                .context("协作任务列表不可用")?;
            tasks.truncate(limit);
            Ok(json!({"tasks": tasks, "limit": limit}))
        }
        _ => anyhow::bail!("未知协作工具"),
    }
}

fn tool_definitions() -> Vec<Value> {
    let id_schema = json!({"type": "object", "properties": {"taskId": {"type": "string"}}, "required": ["taskId"], "additionalProperties": false});
    let fixed_schema = json!({"type": "object", "properties": {
        "prompt": {"type": "string", "maxLength": 65536, "description": "自包含的子任务、必要背景与验收标准"},
        "cwd": {"type": "string", "description": "已存在的绝对工作目录"},
        "continueTaskId": {"type": ["string", "null"], "description": "可选：继续本工具先前创建的同 Agent、同目录已完成任务"}
    }, "required": ["prompt", "cwd"], "additionalProperties": false});
    let mut delegate_schema = fixed_schema.clone();
    delegate_schema["properties"]["agentId"] = json!({"type": "string", "enum": ["codex", "deepseek", "claude"], "description": "可选；省略时使用 Codex++ 默认 Agent，明确指定时优先使用该目标"});
    let mut send_schema = delegate_schema.clone();
    send_schema["required"] = json!(["agentId", "prompt", "cwd"]);
    let dispatch_annotations =
        json!({"readOnlyHint": false, "destructiveHint": true, "idempotentHint": false});
    vec![
        json!({"name": "list_agents", "description": "只读：列出默认 Agent、固定 CLI worker 与启用状态。available 只验证本机文件，不代表登录成功；配置在 Codex++ 中管理。", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}, "annotations": {"readOnlyHint": true}}),
        json!({"name": "delegate_to_deepseek", "description": "把用户指定的子任务交给 DeepSeek Harness。用户点名 DeepSeek 时使用本工具，固定目标不受默认 Agent 设置影响。异步启动真实外部 CLI，可能编辑 cwd 并消耗账户额度；返回 taskId 后先做独立工作，再 wait_task/get_task 收取结果带回当前对话。不是原生 spawn_agent 子树，不接管已有桌面会话。同一 Agent 可并发执行多个任务（上限按 Agent 配置，默认 1），达到上限时排队、按提交顺序自动启动，排队已满会报错；并发任务请使用不同 cwd 避免互相修改相同文件。失败不自动重发。", "inputSchema": fixed_schema, "annotations": dispatch_annotations}),
        json!({"name": "delegate_to_claude_code", "description": "把用户指定的子任务交给 Claude Code。用户点名 Claude/Claude Code 时使用本工具，固定目标不受默认 Agent 设置影响。异步启动真实外部 CLI，可能编辑 cwd 并消耗账户额度；返回 taskId 后先做独立工作，再 wait_task/get_task 收取结果带回当前对话。调用的是 Claude Code harness，不是 Claude Desktop 普通 Chat，也不接管已有会话。同一 Agent 可并发执行多个任务（上限按 Agent 配置，默认 1），达到上限时排队、按提交顺序自动启动，排队已满会报错；并发任务请使用不同 cwd 避免互相修改相同文件。失败不自动重发。", "inputSchema": fixed_schema, "annotations": dispatch_annotations}),
        json!({"name": "delegate_to_codex", "description": "把子任务交给另一个 Codex CLI 会话。用户明确要求另一个 Codex 时使用本工具，固定目标不受默认 Agent 设置影响。可能编辑 cwd 并消耗账户额度；异步返回 taskId 后先做独立工作，再 wait_task/get_task 收结果。不是当前对话原生 spawn_agent 子树，也不接管已有桌面会话。同一 Agent 可并发执行多个任务（上限按 Agent 配置，默认 1），达到上限时排队、按提交顺序自动启动，排队已满会报错；并发任务请使用不同 cwd 避免互相修改相同文件。失败不自动重发。", "inputSchema": fixed_schema, "annotations": dispatch_annotations}),
        json!({"name": "delegate_task", "description": "用户泛指委派给其他 Agent 时使用：省略 agentId 会路由到 Codex++ 配置的默认 Agent，显式 agentId 始终优先。目标未启用或不可用时直接报错，不改派别家。可能编辑 cwd 并消耗原生 CLI 账户额度；异步返回 taskId 后先做独立工作，再 wait_task/get_task 收取终态及答案并带回当前对话。不是原生 spawn_agent 子树，不接管已有桌面会话。", "inputSchema": delegate_schema, "annotations": dispatch_annotations}),
        json!({"name": "send_task", "description": "兼容入口：向显式 agentId 异步派发真实 CLI 任务。可能编辑 cwd 并消耗账户额度；continueTaskId 只续本工具创建的同 Agent、同目录已完成会话。返回 taskId 后先做独立工作，再 wait_task/get_task 收结果；queued 不代表成功；同一 Agent 运行中的任务达到并发上限时排队，失败不自动重发；并发任务请使用不同 cwd。不唤醒已有桌面会话。", "inputSchema": send_schema, "annotations": dispatch_annotations}),
        json!({"name": "wait_task", "description": "只读：用 taskId 等待先前委派任务，timeoutMs 为 0..30000，默认最多等待 30 秒；终态立即返回。返回 task、pending、timedOut、succeeded。pending=true 时继续独立工作并稍后查询同一 taskId；只有 task.status=completed 才成功，failed/cancelled/interrupted 必须如实报告，绝不因此自动重发。", "inputSchema": {"type": "object", "properties": {"taskId": {"type": "string"}, "timeoutMs": {"type": "integer", "minimum": 0, "maximum": 30000, "default": 30000}}, "required": ["taskId"], "additionalProperties": false}, "annotations": {"readOnlyHint": true}}),
        json!({"name": "get_task", "description": "只读：立即查询任务持久化状态与输出。completed/failed/cancelled/interrupted 为终态；只有 completed 代表成功，interrupted 表示结果不确定，不应自动重发。把子任务结果带回当前对话。", "inputSchema": id_schema, "annotations": {"readOnlyHint": true}}),
        json!({"name": "list_tasks", "description": "只读：活跃任务优先，随后列出最近的持久化协作任务，最多 50 条；不会启动 Agent。", "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 50}}, "additionalProperties": false}, "annotations": {"readOnlyHint": true}}),
        json!({"name": "cancel_task", "description": "请求停止指定任务。排队任务立即取消并移出队列；运行任务等待 worker 停止 CLI 后结束。取消不会回滚已经发生的文件或外部操作。", "inputSchema": id_schema, "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true}}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured_store() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("collaboration.sqlite3"));
        let mut config = super::super::CollaborationConfig::default();
        config.enabled = true;
        for agent in &mut config.agents {
            agent.enabled = true;
            // 仅用作文件可用性检查；这些测试从不派生真实 agent 或本测试进程。
            agent.executable = std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned();
        }
        store.save_config(config).unwrap();
        (directory, store)
    }

    fn delegate_arguments(directory: &tempfile::TempDir) -> Value {
        json!({"prompt": "检查给定实现并返回结论", "cwd": directory.path().to_str().unwrap()})
    }

    #[test]
    fn fixed_delegation_routes_cannot_be_changed_by_default_or_arguments() {
        let (directory, store) = configured_store();
        for (tool, expected) in [
            ("delegate_to_deepseek", "deepseek"),
            ("delegate_to_claude_code", "claude"),
            ("delegate_to_codex", "codex"),
        ] {
            let mut config = store.config().unwrap();
            config.default_agent_id = if expected == "codex" {
                "deepseek"
            } else {
                "codex"
            }
            .to_owned();
            store.save_config(config).unwrap();
            let request = delegation_request(&store, tool, delegate_arguments(&directory)).unwrap();
            let task = store.submit(request).unwrap();
            assert_eq!(task.agent_id, expected);
            let mut bad_arguments = delegate_arguments(&directory);
            bad_arguments["agentId"] = json!("claude");
            assert!(delegation_request(&store, tool, bad_arguments).is_err());
            let definitions = tool_definitions();
            let schema = &definitions
                .iter()
                .find(|definition| definition["name"] == tool)
                .unwrap()["inputSchema"];
            assert!(schema["properties"].get("agentId").is_none());
            assert_eq!(schema["additionalProperties"], false);
        }
    }

    #[test]
    fn generic_delegation_uses_default_only_when_agent_id_is_absent() {
        let (directory, store) = configured_store();
        let mut config = store.config().unwrap();
        config.default_agent_id = "deepseek".to_owned();
        store.save_config(config).unwrap();
        let request =
            delegation_request(&store, "delegate_task", delegate_arguments(&directory)).unwrap();
        assert_eq!(request.agent_id, "deepseek");

        let mut arguments = delegate_arguments(&directory);
        arguments["agentId"] = json!("claude");
        let task = store
            .submit(delegation_request(&store, "delegate_task", arguments).unwrap())
            .unwrap();
        assert_eq!(task.agent_id, "claude");
        let agents = call_tool(&store, "list_agents", json!({})).unwrap();
        assert_eq!(agents["defaultAgentId"], "deepseek");
    }

    #[test]
    fn disabled_default_is_an_error_without_fallback_or_creating_a_task() {
        let (directory, store) = configured_store();
        let mut config = store.config().unwrap();
        config.default_agent_id = "deepseek".to_owned();
        config
            .agents
            .iter_mut()
            .find(|agent| agent.id == "deepseek")
            .unwrap()
            .enabled = false;
        store.save_config(config).unwrap();
        let error = call_tool(&store, "delegate_task", delegate_arguments(&directory)).unwrap_err();
        assert!(error.to_string().contains("启用"));
        assert_eq!(store.snapshot().unwrap()["tasks"], json!([]));
    }

    #[tokio::test]
    async fn wait_is_bounded_and_terminal_failure_is_never_reported_as_success() {
        let (directory, store) = configured_store();
        let task = store
            .submit(
                delegation_request(&store, "delegate_to_codex", delegate_arguments(&directory))
                    .unwrap(),
            )
            .unwrap();
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            wait_task(&store, json!({"taskId": task.id, "timeoutMs": 25})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response["pending"], true);
        assert_eq!(response["timedOut"], true);
        assert_eq!(response["succeeded"], false);
        assert_eq!(response["task"]["status"], "queued");
        assert!(
            wait_task(&store, json!({"taskId": task.id, "timeoutMs": 30001}))
                .await
                .is_err()
        );

        store.claim(&task.id).unwrap();
        let finish_store = store.clone();
        let finish_id = task.id.clone();
        let finish = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            finish_store
                .finish(&finish_id, "failed", "", "原生 CLI 登录失败", "")
                .unwrap();
        });
        let waited = tokio::time::timeout(Duration::from_secs(2), handle_request_async(&store, json!({"jsonrpc":"2.0","id":"wait-failed","method":"tools/call","params":{"name":"wait_task","arguments":{"taskId":task.id,"timeoutMs":1000}}}))).await.unwrap().unwrap();
        finish.await.unwrap();
        let result: Value =
            serde_json::from_str(waited["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(waited["id"], "wait-failed");
        assert_eq!(result["pending"], false);
        assert_eq!(result["timedOut"], false);
        assert_eq!(result["succeeded"], false);
        assert_eq!(result["task"]["status"], "failed");
        assert_eq!(result["task"]["error"], "原生 CLI 登录失败");

        // 已有失败终态即使请求 30 秒，也必须立即收回；同步入口仍可返回同形快照。
        let terminal = tokio::time::timeout(
            Duration::from_secs(2),
            wait_task(&store, json!({"taskId":task.id})),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(terminal["pending"], false);
        let sync = call_tool(&store, "wait_task", json!({"taskId":task.id})).unwrap();
        assert_eq!(sync["task"], terminal["task"]);
        assert_eq!(sync["waitMode"], "snapshot");
    }

    #[test]
    fn instructions_connect_natural_language_delegation_to_results_and_honest_boundaries() {
        let (_directory, store) = configured_store();
        let response = handle_request(
            &store,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .unwrap();
        let instructions = response["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("delegate_to_deepseek"));
        assert!(instructions.contains("delegate_to_claude_code"));
        assert!(instructions.contains("wait_task"));
        assert!(instructions.contains("task.status=completed"));
        assert!(instructions.contains("不是原生 spawn_agent"));
        assert!(!instructions.contains("每次确认后"));
    }

    #[test]
    fn oversized_line_is_drained_without_losing_next_request() {
        let mut bytes = vec![b'x'; MAX_REQUEST_BYTES + 32];
        bytes.extend_from_slice(b"\n{}\n");
        let mut reader = std::io::Cursor::new(bytes);
        assert!(matches!(
            read_bounded_line(&mut reader).unwrap(),
            Some(InputLine::TooLong)
        ));
        assert!(
            matches!(read_bounded_line(&mut reader).unwrap(), Some(InputLine::Data(line)) if line == b"{}\n")
        );
        assert!(read_bounded_line(&mut reader).unwrap().is_none());
    }
}
