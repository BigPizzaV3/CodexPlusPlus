//! 拓展钩子：清单校验与用真实 Node 跑通的分发。

use codex_plus_core::extension_hooks::dispatch;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, hooks: serde_json::Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(dir.join("hooks")).unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "hooks": hooks}).to_string()).unwrap();
    for (path, content) in files {
        fs::write(dir.join(path), content).unwrap();
    }
}

#[test]
fn hook_declarations_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-event", json!({"Interrupt": [{"script": "hooks/a.mjs"}]}), &[("hooks/a.mjs", "export default () => null")]);
    write_package(&temp, "bad-timeout", json!({"PreToolUse": [{"script": "hooks/a.mjs", "timeout": 999}]}), &[("hooks/a.mjs", "export default () => null")]);
    write_package(&temp, "escape", json!({"PreToolUse": [{"script": "../x.mjs"}]}), &[]);
    write_package(&temp, "ok", json!({"PreToolUse": [{"matcher": "^Bash$", "script": "hooks/a.mjs"}]}), &[("hooks/a.mjs", "export default () => null")]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-event")["error"].as_str().unwrap().contains("不支持事件 Interrupt"));
    assert!(find("user:bad-timeout")["error"].as_str().unwrap().contains("timeout"));
    assert!(find("user:escape")["error"].as_str().unwrap().contains("包内"));
    let ok = find("user:ok");
    assert_eq!(ok["error"], "", "只声明 hooks、没有页面入口也合法");
    assert_eq!(ok["package"]["hookEvents"], json!(["PreToolUse"]));
    let hooks = manager.enabled_extension_hooks().unwrap();
    assert_eq!(hooks.len(), 1);
    manager.set_script_enabled("user:ok", false).unwrap();
    assert!(manager.enabled_extension_hooks().unwrap().is_empty(), "停用的拓展不参与分发");
}

#[tokio::test]
async fn dispatch_runs_matching_hooks_with_node_and_merges() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(
        &temp,
        "guard",
        json!({"PreToolUse": [{"matcher": "^Bash$", "script": "hooks/guard.mjs", "timeout": 10}]}),
        &[("hooks/guard.mjs", r#"export default (payload) => payload.tool_input?.command?.includes("rm -rf")
  ? { hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: "禁止 rm -rf" } }
  : null;"#)],
    );
    write_package(
        &temp,
        "context",
        json!({"PreToolUse": [{"script": "hooks/ctx.mjs"}], "UserPromptSubmit": [{"script": "hooks/ctx.mjs"}]}),
        &[("hooks/ctx.mjs", r#"export default (payload, { event }) => ({ hookSpecificOutput: { hookEventName: event, additionalContext: `来自 ${event}` } });"#)],
    );
    write_package(
        &temp,
        "broken",
        json!({"PreToolUse": [{"script": "hooks/boom.mjs"}]}),
        &[("hooks/boom.mjs", "export default () => { throw new Error('坏钩子'); }")],
    );

    let denied = dispatch(&manager, "PreToolUse", json!({"tool_name": "Bash", "tool_input": {"command": "rm -rf /tmp/x"}})).await.unwrap();
    assert_eq!(denied["hookSpecificOutput"]["permissionDecision"], "deny");
    assert_eq!(denied["hookSpecificOutput"]["permissionDecisionReason"], "[guard] 禁止 rm -rf");
    assert_eq!(denied["hookSpecificOutput"]["additionalContext"], "来自 PreToolUse", "坏钩子只记日志，不影响其它拓展");

    // matcher 不命中：guard 不运行。
    let other = dispatch(&manager, "PreToolUse", json!({"tool_name": "apply_patch", "tool_input": {"command": "rm -rf"}})).await.unwrap();
    assert!(other["hookSpecificOutput"].get("permissionDecision").is_none());

    let prompt = dispatch(&manager, "UserPromptSubmit", json!({"prompt": "hi"})).await.unwrap();
    assert_eq!(prompt["hookSpecificOutput"]["additionalContext"], "来自 UserPromptSubmit");
    assert_eq!(dispatch(&manager, "Stop", json!({})).await.unwrap(), json!({}), "没有拓展声明的事件直接放行");
    assert!(dispatch(&manager, "Interrupt", json!({})).await.is_err());
}

#[test]
fn sync_reports_only_newly_added_events() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let home = temp.path().join("codex");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "model = \"x\"\n").unwrap();
    let exe = std::path::Path::new("/opt/CodexPlusPlus");
    write_package(&temp, "a", json!({"Stop": [{"script": "hooks/a.mjs"}]}), &[("hooks/a.mjs", "export default () => null")]);
    let first = codex_plus_core::extension_hooks::sync_codex_config(&home, exe, &manager).unwrap();
    assert_eq!(first["added"], json!(["Stop"]));
    let again = codex_plus_core::extension_hooks::sync_codex_config(&home, exe, &manager).unwrap();
    assert_eq!((again["changed"].clone(), again["added"].clone()), (json!(false), json!([])));
    manager.set_script_enabled("user:a", false).unwrap();
    let removed = codex_plus_core::extension_hooks::sync_codex_config(&home, exe, &manager).unwrap();
    assert_eq!((removed["changed"].clone(), removed["added"].clone()), (json!(true), json!([])));
    assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), "model = \"x\"\n", "移除后恢复原样");
}

#[tokio::test]
async fn model_request_hooks_observe_each_step_and_return_nothing() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let log = temp.path().join("steps.log");
    let script = format!(
        r#"import fs from "node:fs";
export default (payload, {{ event }}) => {{
  fs.appendFileSync({log:?}, `${{event}} ${{payload.model}} ${{payload.token_usage?.total_tokens ?? "-"}}\n`);
  return {{ decision: "block", reason: "观察类钩子的输出会被丢掉" }};
}};"#
    );
    write_package(
        &temp,
        "steps",
        json!({"PreModelRequest": [{"matcher": "^gpt-", "script": "hooks/step.mjs"}], "PostModelRequest": [{"script": "hooks/step.mjs"}]}),
        &[("hooks/step.mjs", &script)],
    );
    let pre = dispatch(&manager, "PreModelRequest", json!({"hook_event_name": "PreModelRequest", "session_id": "s1", "turn_id": "t1", "model": "gpt-x"})).await.unwrap();
    assert_eq!(pre, json!({}), "Codex 不采用这两个事件的输出，Codex++ 也不回任何字段");
    let skipped = dispatch(&manager, "PreModelRequest", json!({"model": "other"})).await.unwrap();
    assert_eq!(skipped, json!({}));
    dispatch(&manager, "PostModelRequest", json!({"model": "gpt-x", "status": "completed", "token_usage": {"total_tokens": 11}})).await.unwrap();
    assert_eq!(fs::read_to_string(&log).unwrap(), "PreModelRequest gpt-x -\nPostModelRequest gpt-x 11\n", "matcher 按模型名匹配");
}
