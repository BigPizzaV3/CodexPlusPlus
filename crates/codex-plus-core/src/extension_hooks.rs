//! 拓展的 Codex 钩子：拓展包在 `codexpp.json` 里声明 `hooks`，由 Codex++ 统一分发。
//!
//! Codex 的钩子要用户逐条信任（按条目哈希记录），新增或改动的钩子在信任前不会运行。所以
//! Codex++ 只往 `~/.codex/config.toml` 写**固定不变**的分发条目：每个用到的事件一条，命令是
//! `CodexPlusPlus --codex-plus-hook <事件>`，超时固定 120 秒。用户在 Codex 里信任一次后，
//! 拓展增删改都不必重新信任；哪些拓展在跑由 Codex++ 的拓展开关决定。
//!
//! 分发时只起一个 Node 进程：它按各拓展的 matcher 过滤、并发调用命中的钩子脚本（各自超时），
//! 把结果交回这里，按 Codex 的协议合并成一个输出。任何拓展出错都只记日志，不挡 Codex。

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::user_scripts::{EXTENSION_HOOK_EVENTS, ExtensionHook, UserScriptManager};

/// 写进 Codex 配置的分发命令参数；也是识别「本工具条目」的标记。
pub const HOOK_FLAG: &str = "--codex-plus-hook";
/// 分发条目的超时（秒）。固定值，避免拓展超时变化改动条目哈希、逼用户重新信任。
const DISPATCH_TIMEOUT_SECS: u64 = 120;
const STATUS_MESSAGE: &str = "Codex++ 拓展钩子";
const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Node 侧运行器：stdin 读 `{ event, subject, payload, hooks: [{ id, script, matcher, timeoutMs }] }`，
/// 按 matcher 过滤后并发 import 并调用默认导出，stdout 输出 `[{ id, output? , error?, skipped? }]`。
const RUNNER: &str = r#"
import { pathToFileURL } from "node:url";
import readline from "node:readline";
const lines = readline.createInterface({ input: process.stdin });
/*HOST_CLIENT*/
let started = false;
lines.on("line", async (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (handleHostReply(message)) return;
  if (started) return;
  started = true;
  const results = await runHooks(message);
  write({ final: results });
  process.exit(0);
});
const runHooks = (input) => Promise.all(input.hooks.map(async (hook) => {
  const matcher = hook.matcher || "";
  if (matcher && matcher !== "*" && input.subject !== null) {
    let re;
    try { re = new RegExp(matcher); } catch (error) { return { id: hook.id, error: `matcher 不是合法正则：${error.message}` }; }
    if (!re.test(String(input.subject ?? ""))) return { id: hook.id, skipped: true };
  }
  let timer;
  try {
    const module = await import(pathToFileURL(hook.script).href);
    const fn = typeof module.default === "function" ? module.default : module[input.event];
    if (typeof fn !== "function") return { id: hook.id, error: `${hook.script} 没有默认导出函数` };
    const timeout = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`超时（${hook.timeoutMs / 1000} 秒）`)), hook.timeoutMs); });
    const race = (value) => Promise.race([Promise.resolve(value), timeout]);
    if (hook.adapter === "preExecute" || hook.adapter === "execute") {
      // 工具管道：参数是工具输入，上下文带工具名、调用 id 与完整负载。
      const payload = structuredClone(input.payload);
      const context = { event: input.event, extension: hook.id, config: hook.config ?? {}, name: payload?.tool_name ?? "", callId: payload?.tool_use_id ?? null, cwd: payload?.cwd ?? null, payload, sessionQuery, sessions, service, llm, ui };
      const result = await race(fn(payload?.tool_input ?? null, context));
      if (result === undefined || result === null) return { id: hook.id, output: null };
      if (hook.adapter === "execute") return { id: hook.id, executed: typeof result === "string" ? result : JSON.stringify(result) };
      if (typeof result === "object" && !Array.isArray(result) && Object.keys(result).length === 1 && "deny" in result) {
        return { id: hook.id, output: { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: String(result.deny) } } };
      }
      return { id: hook.id, output: { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "allow", updatedInput: result } } };
    }
    const output = await race(fn(structuredClone(input.payload), { event: input.event, extension: hook.id, config: hook.config ?? {}, sessionQuery, sessions, service, llm, ui }));
    return { id: hook.id, output: output ?? null };
  } catch (error) {
    return { id: hook.id, error: String(error?.stack || error?.message || error) };
  } finally {
    clearTimeout(timer);
  }
}));
"#;

/// Codex 按事件用哪个字段做 matcher（见 Codex Hooks 文档）；其余事件忽略 matcher。
fn matcher_subject(event: &str, payload: &Value) -> Value {
    let field = match event {
        "PreToolUse" | "PostToolUse" | "PermissionRequest" => "tool_name",
        "SessionStart" => "source",
        "PreCompact" | "PostCompact" => "trigger",
        "SubagentStart" | "SubagentStop" => "agent_type",
        "PreModelRequest" | "PostModelRequest" => "model",
        _ => return Value::Null,
    };
    payload.get(field).cloned().unwrap_or(Value::String(String::new()))
}

/// 分发一次钩子事件，返回要写到 stdout 的 Codex 钩子输出（空对象表示无意见、放行）。
pub async fn dispatch(
    manager: &UserScriptManager,
    event: &str,
    payload: Value,
) -> anyhow::Result<Value> {
    if !EXTENSION_HOOK_EVENTS.contains(&event) {
        anyhow::bail!("不支持的钩子事件 {event}");
    }
    let hooks: Vec<(String, ExtensionHook)> = manager
        .enabled_extension_hooks()?
        .into_iter()
        .filter(|(_, hook)| hook.event == event)
        .collect();
    if hooks.is_empty() {
        return Ok(json!({}));
    }
    let node = crate::extension_node::require_node()?;
    let input = json!({
        "event": event,
        "subject": matcher_subject(event, &payload),
        "payload": payload,
        "hooks": hooks.iter().map(|(id, hook)| json!({
            "id": id,
            "script": hook.script,
            "matcher": hook.matcher,
            "timeoutMs": hook.timeout_secs * 1000,
            "config": hook.config,
            "adapter": hook.adapter,
        })).collect::<Vec<_>>(),
    });
    let longest = hooks.iter().map(|(_, hook)| hook.timeout_secs).max().unwrap_or(30);
    let runner = crate::extension_node::with_host_client(RUNNER);
    let results = crate::extension_node::run_runner_with_host(&node, &runner, &input, Duration::from_secs(longest + 5), manager).await?;
    let results = results.as_array().cloned().unwrap_or_default();
    for item in &results {
        if let Some(error) = item.get("error").and_then(Value::as_str) {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_hooks.hook_failed",
                json!({ "event": event, "extension": item.get("id"), "error": error }),
            );
        }
    }
    let mut outputs = results
        .iter()
        .filter_map(|item| item.get("output").filter(|output| output.is_object()).cloned().map(|output| (item.get("id").and_then(Value::as_str).unwrap_or("").to_string(), output)))
        .collect::<Vec<_>>();
    // 代为执行：结果存成带令牌的文件，原工具调用以「拒绝 + 结果 + 令牌标记」结束。经 Codex++ 代理时，
    // 下一次请求里网关按令牌把这条工具输出换成纯结果；不经代理时模型也能从拒绝理由里读到结果。
    for item in &results {
        if let Some(text) = item.get("executed").and_then(Value::as_str) {
            let id = item.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let token = store_tool_execution(manager, &id, &payload, text)?;
            let reason = format!("（Codex++ 拓展 {id} 已代为执行该工具调用，以下是执行结果）\n{text}\n{}", tool_exec_marker(&token));
            outputs.push((id, json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } })));
        }
    }
    Ok(merge_outputs(event, &outputs))
}


/// 代执行结果在工具输出里的标记：`[[codexpp-exec:令牌]]`。
pub fn tool_exec_marker(token: &str) -> String {
    format!("[[codexpp-exec:{token}]]")
}

/// 存一份代执行结果，返回令牌。顺带清掉 30 天前的旧结果。
fn store_tool_execution(manager: &UserScriptManager, extension: &str, payload: &Value, text: &str) -> anyhow::Result<String> {
    let dir = manager.tool_exec_dir();
    std::fs::create_dir_all(&dir)?;
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_nanos()).unwrap_or_default();
    let token = format!("{nanos:x}{:x}", std::process::id());
    let record = json!({
        "extension": extension,
        "tool": payload.get("tool_name"),
        "callId": payload.get("tool_use_id"),
        "output": text,
    });
    std::fs::write(dir.join(format!("{token}.json")), record.to_string())?;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let cutoff = std::time::SystemTime::now() - Duration::from_secs(30 * 24 * 3600);
        for entry in entries.flatten() {
            if entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|modified| modified < cutoff) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(token)
}

/// 按令牌取回代执行结果。记录里存了调用 id 的，必须与这条工具输出的 `call_id` 一致，
/// 免得别的输出里恰好出现旧令牌（比如 grep 到数据目录）时被整条换成旧结果。
pub fn load_tool_execution(manager: &UserScriptManager, token: &str, call_id: &str) -> Option<String> {
    if token.is_empty() || !token.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let text = std::fs::read_to_string(manager.tool_exec_dir().join(format!("{token}.json"))).ok()?;
    let record = serde_json::from_str::<Value>(&text).ok()?;
    if let Some(expected) = record.get("callId").and_then(Value::as_str).filter(|id| !id.is_empty()) {
        if expected != call_id {
            return None;
        }
    }
    record.get("output")?.as_str().map(str::to_string)
}

/// 按 Codex 钩子协议合并多个拓展的输出。规则（与同一事件多条钩子的语义一致）：
/// - `permissionDecision`：任一 deny 即 deny；否则任一 allow 即 allow（只采用第一个 `updatedInput`）
/// - 旧式顶层 `decision: "block"`：任一即 block
/// - `continue: false`：任一即停止，`stopReason` 取第一个
/// - `additionalContext`、`systemMessage`、各类原因：按拓展顺序拼接并标注来源
/// - `PermissionRequest` 的 `decision.behavior`：deny 优先于 allow
pub fn merge_outputs(event: &str, outputs: &[(String, Value)]) -> Value {
    // 每步前后的模型请求钩子只用来观察，Codex 不采用输出：不回任何字段，免得被当成协议外输出报错。
    if matches!(event, "PreModelRequest" | "PostModelRequest") {
        return json!({});
    }
    let mut merged = Map::new();
    let mut specific = Map::new();
    let mut deny_reasons = Vec::new();
    let mut allow: Option<(String, Value)> = None;
    let mut block_reasons = Vec::new();
    let mut contexts = Vec::new();
    let mut messages = Vec::new();
    let mut request_deny = Vec::new();
    let mut request_allow = false;
    for (id, output) in outputs {
        let hook_specific = output.get("hookSpecificOutput").and_then(Value::as_object);
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        match hook_specific.and_then(|value| value.get("permissionDecision")).and_then(Value::as_str) {
            Some("deny") => deny_reasons.push(format!(
                "[{id}] {}",
                text(hook_specific.and_then(|value| value.get("permissionDecisionReason"))).unwrap_or_default()
            )),
            Some("allow") if allow.is_none() => allow = Some((id.clone(), Value::Object(hook_specific.cloned().unwrap_or_default()))),
            _ => {}
        }
        if let Some(context) = text(hook_specific.and_then(|value| value.get("additionalContext"))) {
            contexts.push(context);
        }
        if let Some(decision) = hook_specific.and_then(|value| value.get("decision")).and_then(Value::as_object) {
            match decision.get("behavior").and_then(Value::as_str) {
                Some("deny") => request_deny.push(format!("[{id}] {}", text(decision.get("message")).unwrap_or_default())),
                Some("allow") => request_allow = true,
                _ => {}
            }
        }
        if output.get("decision").and_then(Value::as_str) == Some("block") {
            block_reasons.push(format!("[{id}] {}", text(output.get("reason")).unwrap_or_default()));
        }
        if output.get("continue").and_then(Value::as_bool) == Some(false) && !merged.contains_key("continue") {
            merged.insert("continue".into(), Value::Bool(false));
            if let Some(reason) = output.get("stopReason") {
                merged.insert("stopReason".into(), reason.clone());
            }
        }
        if let Some(message) = text(output.get("systemMessage")) {
            messages.push(format!("[{id}] {message}"));
        }
    }
    if !deny_reasons.is_empty() {
        specific.insert("permissionDecision".into(), json!("deny"));
        specific.insert("permissionDecisionReason".into(), json!(deny_reasons.join("\n")));
    } else if let Some((_, decision)) = allow {
        specific.insert("permissionDecision".into(), json!("allow"));
        for key in ["permissionDecisionReason", "updatedInput"] {
            if let Some(value) = decision.get(key) {
                specific.insert(key.into(), value.clone());
            }
        }
    }
    if !request_deny.is_empty() {
        specific.insert("decision".into(), json!({ "behavior": "deny", "message": request_deny.join("\n") }));
    } else if request_allow {
        specific.insert("decision".into(), json!({ "behavior": "allow" }));
    }
    if !contexts.is_empty() {
        specific.insert("additionalContext".into(), json!(contexts.join("\n\n")));
    }
    if !specific.is_empty() {
        specific.insert("hookEventName".into(), json!(event));
        merged.insert("hookSpecificOutput".into(), Value::Object(specific));
    }
    if !block_reasons.is_empty() {
        merged.insert("decision".into(), json!("block"));
        merged.insert("reason".into(), json!(block_reasons.join("\n")));
    }
    if !messages.is_empty() {
        merged.insert("systemMessage".into(), json!(messages.join("\n")));
    }
    Value::Object(merged)
}

/// 配置里已有本工具分发条目的事件。
fn owned_events(contents: &str) -> Vec<String> {
    let Ok(document) = contents.parse::<DocumentMut>() else {
        return Vec::new();
    };
    let Some(hooks) = document.get("hooks").and_then(Item::as_table_like) else {
        return Vec::new();
    };
    EXTENSION_HOOK_EVENTS
        .iter()
        .filter(|event| {
            hooks
                .get(event)
                .and_then(Item::as_array_of_tables)
                .is_some_and(|groups| groups.iter().any(is_owned_group))
        })
        .map(|event| event.to_string())
        .collect()
}

/// 本工具写的分发命令。路径按 shell 规则加引号（Codex 经 shell 执行钩子命令）。
fn dispatch_command(executable: &Path, event: &str) -> anyhow::Result<String> {
    let path = executable.to_str().context("Codex++ 程序路径不是有效 UTF-8")?;
    if path.chars().any(char::is_control) {
        anyhow::bail!("Codex++ 程序路径包含控制字符");
    }
    let quoted = if cfg!(windows) {
        format!("\"{path}\"")
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    };
    Ok(format!("{quoted} {HOOK_FLAG} {event}"))
}

fn is_owned_group(group: &Table) -> bool {
    group
        .get("hooks")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Item::as_str)
                    .is_some_and(|command| command.contains(HOOK_FLAG))
            })
        })
}

/// 纯转换：让 config.toml 里本工具的分发条目与 `events` 一致。别的钩子原样保留。
pub fn sync_config_text(contents: &str, executable: &Path, events: &[&str]) -> anyhow::Result<String> {
    let mut document = contents
        .parse::<DocumentMut>()
        .map_err(|_| anyhow::anyhow!("Codex 配置不是有效 TOML，未修改"))?;
    if document.get("hooks").is_none() {
        if events.is_empty() {
            return Ok(contents.to_string());
        }
        let mut hooks = Table::new();
        hooks.set_implicit(true);
        document.insert("hooks", Item::Table(hooks));
    }
    let hooks = document
        .get_mut("hooks")
        .and_then(Item::as_table_like_mut)
        .context("hooks 必须是 TOML 表，未修改配置")?;
    for event in EXTENSION_HOOK_EVENTS {
        let wanted = events.contains(event);
        let existing = hooks.get(event).and_then(Item::as_array_of_tables).cloned();
        let mut groups = ArrayOfTables::new();
        let mut already = false;
        if let Some(existing) = existing {
            for group in existing.iter() {
                if is_owned_group(group) {
                    // 已有且一字不差就保留原样（哈希不变，免得用户重新信任）。
                    let command = group
                        .get("hooks")
                        .and_then(Item::as_array_of_tables)
                        .and_then(|hooks| hooks.get(0))
                        .and_then(|hook| hook.get("command"))
                        .and_then(Item::as_str)
                        .map(str::to_owned);
                    if wanted && !already && command == Some(dispatch_command(executable, event)?) {
                        groups.push(group.clone());
                        already = true;
                    }
                    continue;
                }
                groups.push(group.clone());
            }
        }
        if wanted && !already {
            let mut hook = Table::new();
            hook.insert("type", toml_edit::value("command"));
            hook.insert("command", toml_edit::value(dispatch_command(executable, event)?));
            hook.insert("timeout", toml_edit::value(DISPATCH_TIMEOUT_SECS as i64));
            hook.insert("statusMessage", toml_edit::value(STATUS_MESSAGE));
            let mut inner = ArrayOfTables::new();
            inner.push(hook);
            let mut group = Table::new();
            group.insert("hooks", Item::ArrayOfTables(inner));
            groups.push(group);
        }
        if groups.is_empty() {
            hooks.remove(event);
        } else {
            hooks.insert(event, Item::ArrayOfTables(groups));
        }
    }
    Ok(document.to_string())
}

/// 按启用中的拓展同步 Codex 配置。返回本工具管理的事件，以及这次是否改了文件
/// （改了就意味着有新条目，用户需要在 Codex 里信任一次）。
pub fn sync_codex_config(
    home: &Path,
    executable: &Path,
    manager: &UserScriptManager,
) -> anyhow::Result<Value> {
    let hooks = manager.enabled_extension_hooks()?;
    let mut events = hooks.iter().map(|(_, hook)| hook.event.as_str()).collect::<Vec<_>>();
    events.sort();
    events.dedup();
    let path = home.join("config.toml");
    let before = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("无法读取 Codex 配置"),
    };
    if before.len() > MAX_PAYLOAD_BYTES {
        anyhow::bail!("Codex 配置超过 4 MiB，未修改");
    }
    let updated = sync_config_text(&before, executable, &events)?;
    let changed = updated != before;
    let previous = owned_events(&before);
    // 新增的事件需要用户在 Codex 里信任；移除不需要。
    let added = events
        .iter()
        .filter(|event| !previous.iter().any(|known| known == *event))
        .copied()
        .collect::<Vec<_>>();
    if changed {
        // 其他程序可能同时在改；读到写之间变了就放弃，下次同步再来。
        if std::fs::read_to_string(&path).unwrap_or_default() != before {
            anyhow::bail!("Codex 配置在同步期间被修改，未覆盖");
        }
        crate::settings::atomic_write(&path, updated.as_bytes()).context("无法写入 Codex 配置")?;
    }
    Ok(json!({ "events": events, "added": added, "changed": changed, "configPath": path.to_string_lossy() }))
}

/// 用默认位置同步（Codex home、当前程序）：钩子分发条目，以及拓展的技能、子 agent、MCP 服务器、全局指令。
/// 给拓展增删启停之后调用，失败只记日志。只同步默认的拓展目录：测试等用临时目录的 manager
/// 不能把用户真实的 Codex 配置改成测试里的那一套。
pub fn sync_default(manager: &UserScriptManager) -> Value {
    if manager.config_path() != crate::user_scripts::default_user_script_manager().config_path() {
        return json!({ "skipped": "不是默认的拓展目录，不同步 Codex 配置" });
    }
    // 拓展包带来的 dsh 插件先同步：它们的配套拓展包会带来钩子，下面一并写进 Codex 配置。
    let dsh = crate::dsh_plugins::sync_extension_plugins_default(manager);
    let home = crate::codex_home::default_codex_home_dir();
    let result = std::env::current_exe()
        .context("无法定位 Codex++ 程序")
        .and_then(|executable| sync_codex_config(&home, &executable, manager))
        .map(|mut report| {
            report["codex"] = match manager.enabled_codex_assets().and_then(|enabled| crate::extension_codex::sync(&home, manager, &enabled)) {
                Ok(codex) => codex,
                Err(error) => {
                    let _ = crate::diagnostic_log::append_diagnostic_log("extension_codex.sync_failed", json!({ "error": format!("{error:#}") }));
                    json!({ "error": format!("{error:#}") })
                }
            };
            report
        });
    match result {
        Ok(mut report) => {
            report["dsh"] = dsh;
            report
        }
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "extension_hooks.sync_failed",
                json!({ "error": format!("{error:#}") }),
            );
            json!({ "error": format!("{error:#}") })
        }
    }
}

/// `CodexPlusPlus --codex-plus-hook <事件>` 的入口：stdin 读事件负载，stdout 写合并后的输出。
/// 任何内部错误都以「无意见」退出 0，并把原因写到 stderr，绝不因为 Codex++ 自身问题挡住 Codex。
pub async fn run_cli(manager: &UserScriptManager, event: &str) -> i32 {
    use std::io::Read;
    let mut raw = Vec::new();
    let read = std::io::stdin().take(MAX_PAYLOAD_BYTES as u64 + 1).read_to_end(&mut raw);
    let payload = match read {
        Ok(_) if raw.len() <= MAX_PAYLOAD_BYTES => serde_json::from_slice::<Value>(&raw).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    match dispatch(manager, event, payload).await {
        Ok(output) => {
            if output.as_object().is_some_and(|map| !map.is_empty()) {
                println!("{output}");
            }
            0
        }
        Err(error) => {
            eprintln!("Codex++ 拓展钩子未运行：{error:#}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_wins_and_reasons_are_attributed() {
        let merged = merge_outputs(
            "PreToolUse",
            &[
                ("a".into(), json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "allow", "updatedInput": {"command": "ls"}}})),
                ("b".into(), json!({"hookSpecificOutput": {"permissionDecision": "deny", "permissionDecisionReason": "危险命令"}, "systemMessage": "已拦截"})),
            ],
        );
        assert_eq!(merged["hookSpecificOutput"]["permissionDecision"], "deny");
        assert_eq!(merged["hookSpecificOutput"]["permissionDecisionReason"], "[b] 危险命令");
        assert_eq!(merged["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        assert!(merged["hookSpecificOutput"].get("updatedInput").is_none(), "deny 时不带改写参数");
        assert_eq!(merged["systemMessage"], "[b] 已拦截");
    }

    #[test]
    fn allow_keeps_first_updated_input_and_contexts_concatenate() {
        let merged = merge_outputs(
            "PreToolUse",
            &[
                ("a".into(), json!({"hookSpecificOutput": {"permissionDecision": "allow", "updatedInput": {"command": "ls -la"}, "additionalContext": "甲"}})),
                ("b".into(), json!({"hookSpecificOutput": {"permissionDecision": "allow", "updatedInput": {"command": "pwd"}, "additionalContext": "乙"}})),
            ],
        );
        assert_eq!(merged["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(merged["hookSpecificOutput"]["updatedInput"]["command"], "ls -la");
        assert_eq!(merged["hookSpecificOutput"]["additionalContext"], "甲\n\n乙");
    }

    #[test]
    fn block_continue_and_permission_request_merge() {
        let merged = merge_outputs(
            "Stop",
            &[
                ("a".into(), json!({"continue": false, "stopReason": "够了"})),
                ("b".into(), json!({"decision": "block", "reason": "还没跑测试"})),
            ],
        );
        assert_eq!(merged["continue"], false);
        assert_eq!(merged["stopReason"], "够了");
        assert_eq!(merged["decision"], "block");
        assert_eq!(merged["reason"], "[b] 还没跑测试");
        let request = merge_outputs(
            "PermissionRequest",
            &[
                ("a".into(), json!({"hookSpecificOutput": {"decision": {"behavior": "allow"}}})),
                ("b".into(), json!({"hookSpecificOutput": {"decision": {"behavior": "deny", "message": "不行"}}})),
            ],
        );
        assert_eq!(request["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(merge_outputs("PreToolUse", &[]), json!({}));
    }

    #[test]
    fn sync_adds_removes_and_preserves_foreign_hooks_stably() {
        let exe = Path::new("/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus");
        let original = r#"model = "x"

# 用户自己的钩子
[[hooks.PreToolUse]]
matcher = "^Bash$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "python3 guard.py"
"#;
        let added = sync_config_text(original, exe, &["PreToolUse", "Stop"]).unwrap();
        let document: DocumentMut = added.parse().unwrap();
        let pre = document["hooks"]["PreToolUse"].as_array_of_tables().unwrap();
        assert_eq!(pre.len(), 2, "用户的钩子保留，再加一条分发条目");
        assert_eq!(pre.get(0).unwrap()["hooks"].as_array_of_tables().unwrap().get(0).unwrap()["command"].as_str(), Some("python3 guard.py"));
        let ours = pre.get(1).unwrap()["hooks"].as_array_of_tables().unwrap().get(0).unwrap();
        let expected = if cfg!(windows) {
            "\"/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus\" --codex-plus-hook PreToolUse"
        } else {
            "'/Applications/Codex++.app/Contents/MacOS/CodexPlusPlus' --codex-plus-hook PreToolUse"
        };
        assert_eq!(ours["command"].as_str(), Some(expected));
        assert_eq!(ours["timeout"].as_integer(), Some(120));
        assert!(pre.get(1).unwrap().get("matcher").is_none(), "分发条目匹配全部，由拓展自己的 matcher 过滤");
        assert!(document["hooks"]["Stop"].as_array_of_tables().is_some());
        assert!(added.contains("# 用户自己的钩子"));
        // 再同步一次不变（条目哈希稳定，免得用户重新信任）。
        assert_eq!(sync_config_text(&added, exe, &["PreToolUse", "Stop"]).unwrap(), added);
        // 不再需要时只移除本工具的条目。
        let removed = sync_config_text(&added, exe, &[]).unwrap();
        let document: DocumentMut = removed.parse().unwrap();
        assert_eq!(document["hooks"]["PreToolUse"].as_array_of_tables().unwrap().len(), 1);
        assert!(document["hooks"].get("Stop").is_none());
        // 没有钩子也没有需要时不动文件。
        assert_eq!(sync_config_text("model = \"x\"\n", exe, &[]).unwrap(), "model = \"x\"\n");
    }

    #[test]
    fn command_quotes_paths_with_spaces_and_quotes() {
        let command = dispatch_command(Path::new("/Users/a b/it's/CodexPlusPlus"), "Stop").unwrap();
        // Windows 由 cmd 解析：整段加双引号（路径里不会有双引号）；其他平台按 POSIX 单引号转义。
        if cfg!(windows) {
            assert_eq!(command, r#""/Users/a b/it's/CodexPlusPlus" --codex-plus-hook Stop"#);
        } else {
            assert_eq!(command, r"'/Users/a b/it'\''s/CodexPlusPlus' --codex-plus-hook Stop");
        }
    }
}
