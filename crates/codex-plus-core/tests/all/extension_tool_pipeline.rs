//! 拓展工具管道：preExecute / execute 走 Codex 的 PreToolUse 钩子，postExecute 与代执行结果换回走网关请求侧。

use codex_plus_core::extension_gateway::apply_request_with;
use codex_plus_core::extension_hooks::{dispatch, load_tool_execution};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, pipeline: Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(dir.join("tools")).unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "toolPipeline": pipeline}).to_string()).unwrap();
    for (path, content) in files {
        fs::write(dir.join(path), content).unwrap();
    }
}

#[test]
fn pipeline_declarations_are_validated_and_summarized() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-stage", json!({"during": []}), &[]);
    write_package(&temp, "bad-timeout", json!({"postExecute": [{"script": "tools/a.mjs", "timeout": 60}]}), &[("tools/a.mjs", "export default () => null")]);
    write_package(&temp, "ok", json!({
        "preExecute": [{"matcher": "^Bash$", "script": "tools/a.mjs"}],
        "execute": [{"script": "tools/a.mjs"}],
        "postExecute": [{"script": "tools/a.mjs"}]
    }), &[("tools/a.mjs", "export default () => null")]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-stage")["error"].as_str().unwrap().contains("不认识 during"));
    assert!(find("user:bad-timeout")["error"].as_str().unwrap().contains("1~10 秒"));
    let ok = find("user:ok");
    assert_eq!(ok["error"], "");
    assert_eq!(ok["package"]["toolPipeline"], json!({"preExecute": 1, "execute": 1, "postExecute": 1}));
    assert_eq!(ok["package"]["hookEvents"], json!(["PreToolUse"]), "前两段落到 PreToolUse，Codex 配置里会写分发条目");
}

#[tokio::test]
async fn pre_execute_rewrites_or_denies_and_execute_replaces_the_tool() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-guard", json!({"preExecute": [{"matcher": "^Bash$", "script": "tools/pre.mjs"}]}), &[("tools/pre.mjs", r#"
export default (input, { name, callId }) => {
  if (input.command.includes("rm -rf")) return { deny: `不许删：${name}/${callId}` };
  if (input.command.startsWith("ls")) return { ...input, command: input.command + " -la" };
  return null;
};"#)]);
    write_package(&temp, "b-cache", json!({"execute": [{"matcher": "^Bash$", "script": "tools/exec.mjs"}]}), &[("tools/exec.mjs", r#"
export default (input) => input.command === "date" ? "2026-10-30（缓存的结果）" : undefined;"#)]);

    let payload = |command: &str| json!({"tool_name": "Bash", "tool_use_id": "call_1", "tool_input": {"command": command}});
    let denied = dispatch(&manager, "PreToolUse", payload("rm -rf /tmp/x")).await.unwrap();
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    assert!(denied["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap().contains("不许删：Bash/call_1"));

    let rewritten = dispatch(&manager, "PreToolUse", payload("ls")).await.unwrap();
    assert_eq!(rewritten["hookSpecificOutput"]["permissionDecision"], "allow");
    assert_eq!(rewritten["hookSpecificOutput"]["updatedInput"]["command"], "ls -la");

    let untouched = dispatch(&manager, "PreToolUse", payload("pwd")).await.unwrap();
    assert_eq!(untouched, json!({}), "都不干预时放行");

    let executed = dispatch(&manager, "PreToolUse", payload("date")).await.unwrap();
    assert_eq!(executed["hookSpecificOutput"]["permissionDecision"], "deny", "代为执行：原工具不跑");
    let reason = executed["hookSpecificOutput"]["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("2026-10-30（缓存的结果）"), "不经代理时模型也能从理由里读到结果：{reason}");
    let token = reason.split("[[codexpp-exec:").nth(1).unwrap().split("]]").next().unwrap();
    assert_eq!(load_tool_execution(&manager, token, "call_1").as_deref(), Some("2026-10-30（缓存的结果）"));
    assert!(load_tool_execution(&manager, token, "call_9").is_none(), "调用 id 对不上不换");
    assert!(load_tool_execution(&manager, "../etc", "call_1").is_none(), "令牌只认十六进制");

    // 网关请求侧：代执行的工具输出按令牌换成纯结果。
    let mut request = json!({"input": [
        {"type": "function_call", "name": "exec_command", "call_id": "call_1", "arguments": "{\"cmd\":\"date\"}"},
        {"type": "function_call_output", "call_id": "call_1", "output": format!("工具调用被拒绝：{reason}")},
    ]});
    apply_request_with(&manager, &mut request, "/v1/responses").await;
    assert_eq!(request["input"][1]["output"], "2026-10-30（缓存的结果）");

    // 别的工具输出里恰好带着旧令牌（比如 grep 到数据目录）：不换。
    let grep = format!("{}\nmore lines after it", tool_marker(token));
    let mut request = json!({"input": [
        {"type": "function_call_output", "call_id": "call_2", "output": format!("工具调用被拒绝：{reason}")},
        {"type": "function_call_output", "call_id": "call_1", "output": grep.clone()},
    ]});
    apply_request_with(&manager, &mut request, "/v1/responses").await;
    assert_ne!(request["input"][0]["output"], "2026-10-30（缓存的结果）", "调用 id 不同");
    assert_eq!(request["input"][1]["output"], grep, "令牌后面还有正文");
}

fn tool_marker(token: &str) -> String {
    codex_plus_core::extension_hooks::tool_exec_marker(token)
}

#[tokio::test]
async fn post_execute_rewrites_matching_tool_outputs_once() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let counter = temp.path().join("runs.txt");
    write_package(&temp, "redact", json!({"postExecute": [{"matcher": "^exec_command$", "script": "tools/post.mjs"}]}), &[("tools/post.mjs", &format!(r#"
import fs from "node:fs";
export default (output, {{ name, input, callId }}) => {{
  fs.appendFileSync({counter:?}, callId + "\n");
  return typeof output === "string" && output.includes("sk-") ? output.replace(/sk-\w+/g, "sk-***") + ` [${{name}}]` : undefined;
}};"#))]);
    let request = || json!({"input": [
        {"type": "function_call", "name": "exec_command", "call_id": "c1", "arguments": "{\"cmd\":\"env\"}"},
        {"type": "function_call_output", "call_id": "c1", "output": "KEY=sk-abc123"},
        {"type": "function_call", "name": "apply_patch", "call_id": "c2", "arguments": "{}"},
        {"type": "function_call_output", "call_id": "c2", "output": "sk-keep-this"},
        {"type": "function_call", "name": "exec_command", "call_id": "c3", "arguments": "{}"},
        {"type": "function_call_output", "call_id": "c3", "output": "nothing secret"},
    ]});
    let mut first = request();
    apply_request_with(&manager, &mut first, "/v1/responses").await;
    assert_eq!(first["input"][1]["output"], "KEY=sk-*** [exec_command]");
    assert_eq!(first["input"][3]["output"], "sk-keep-this", "matcher 不匹配的工具不动");
    assert_eq!(first["input"][5]["output"], "nothing secret", "返回 undefined 不改");
    // 第二次请求带着同样的历史：命中缓存，脚本不再跑，结果一致。
    let mut second = request();
    apply_request_with(&manager, &mut second, "/v1/responses").await;
    assert_eq!(second["input"][1]["output"], "KEY=sk-*** [exec_command]");
    let runs = fs::read_to_string(&counter).unwrap();
    assert_eq!(runs.lines().filter(|line| *line == "c1").count(), 1, "同一条输出只改写一次：{runs}");
}
