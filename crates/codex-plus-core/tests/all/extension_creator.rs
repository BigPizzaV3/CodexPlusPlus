//! 「对话造拓展」MCP：拓展包写入的校验，以及 JSON-RPC 往返。

use codex_plus_core::extension_creator::{ExtensionCreator, handle_request};
use codex_plus_core::user_scripts::{ExtensionWrite, UserScriptManager};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;

fn creator(temp: &tempfile::TempDir) -> ExtensionCreator {
    ExtensionCreator {
        manager: UserScriptManager::new(
            temp.path().join("builtin"),
            temp.path().join("user"),
            temp.path().join("user_scripts.json"),
        ),
        // 不连 Codex：只验证写入与汇报，热重载应如实报告未执行。
        debug_ports: Vec::new(),
    }
}

async fn call(creator: &ExtensionCreator, name: &str, arguments: Value) -> (bool, Value) {
    call_in_thread(creator, "test-thread", name, arguments).await
}

async fn call_in_thread(creator: &ExtensionCreator, thread: &str, name: &str, arguments: Value) -> (bool, Value) {
    call_with_meta(creator, Some(json!({"threadId": thread})), name, arguments).await
}

async fn call_with_meta(creator: &ExtensionCreator, meta: Option<Value>, name: &str, arguments: Value) -> (bool, Value) {
    let mut params = json!({"name": name, "arguments": arguments});
    if let Some(meta) = meta {
        params["_meta"] = meta;
    }
    let response = handle_request(
        creator,
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params}),
    )
    .await
    .unwrap();
    assert_eq!(response["id"], 7);
    let result = &response["result"];
    let text = result["content"][0]["text"].as_str().unwrap();
    let is_error = result["isError"].as_bool().unwrap();
    let value = if is_error { Value::String(text.to_string()) } else { serde_json::from_str(text).unwrap() };
    (is_error, value)
}

fn write(id: &str, files: &[(&str, &str)]) -> ExtensionWrite {
    ExtensionWrite {
        id: id.to_string(),
        name: "测试拓展".to_string(),
        files: files
            .iter()
            .map(|(path, content)| (path.to_string(), content.to_string()))
            .collect::<BTreeMap<_, _>>(),
        ..ExtensionWrite::default()
    }
}

#[tokio::test]
async fn mcp_handshake_lists_tools_and_ignores_notifications() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let init = handle_request(
        &creator,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
    )
    .await
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["instructions"].as_str().unwrap().contains("codexpp_extension_write"));
    assert!(
        handle_request(&creator, json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await
            .is_none()
    );
    let tools = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))
        .await
        .unwrap();
    let names = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "codexpp_extension_guide",
            "codexpp_extension_list",
            "codexpp_extension_read",
            "codexpp_extension_inspect",
            "codexpp_extension_screenshot",
            "codexpp_extension_write",
            "codexpp_extension_set_enabled",
            "codexpp_extension_configure",
            "codexpp_extension_reload"
        ]
    );
    // 没有删除工具：删除留给用户自己。
    assert!(!names.iter().any(|name| name.contains("delete") || name.contains("remove")));
    // write 工具的 gateway 说明列出所有可关的内置环节，agent 造拓展时才看得到。
    let write = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "codexpp_extension_write")
        .unwrap();
    let gateway = write["inputSchema"]["properties"]["gateway"]["description"].as_str().unwrap();
    for segment in codex_plus_core::user_scripts::BUILTIN_GATEWAY_SEGMENTS {
        assert!(gateway.contains(segment), "{segment}: {gateway}");
    }
    // 页面查看与截图只读；没连 Codex 时如实报错，不伪造结果。
    for tool in ["codexpp_extension_inspect", "codexpp_extension_screenshot"] {
        let (error, message) = call(&creator, tool, json!({})).await;
        assert!(error, "{tool}");
        assert!(message.as_str().unwrap().contains("没有找到已连接的 Codex 页面"), "{message}");
    }
    let (error, message) = call(&creator, "codexpp_extension_inspect", json!({"script": "alert(1)"})).await;
    assert!(error && message.as_str().unwrap().contains("参数无效"), "不接受代码参数：{message}");
    let unknown = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 3, "method": "sampling/createMessage"}))
        .await
        .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);

    let (error, guide) = call(&creator, "codexpp_extension_guide", json!({})).await;
    assert!(!error);
    assert!(guide["guide"].as_str().unwrap().contains("codexPlus.extension"));
    assert!(guide["availableRoutes"].as_array().unwrap().contains(&json!("/diagnostics/log")));
    assert_eq!(guide["apiVersion"], 1);
}

#[tokio::test]
async fn write_read_and_disable_round_trip_without_codex() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let (error, written) = call(
        &creator,
        "codexpp_extension_write",
        json!({
            "id": "pomodoro", "name": "番茄钟", "description": "图标栏番茄钟",
            "routes": ["/diagnostics/log"],
            "files": [
                {"path": "renderer.js", "content": "codexPlus.extension((ctx) => {});"},
                {"path": "lib/util.js", "content": "// util"}
            ]
        }),
    )
    .await;
    assert!(!error, "{written}");
    assert_eq!(written["reload"]["reloaded"], false);
    assert!(written["reload"]["reason"].as_str().unwrap().contains("没有找到已连接的 Codex 页面"));
    assert_eq!(written["extension"]["key"], "user:pomodoro");
    assert_eq!(written["extension"]["kind"], "package");
    assert_eq!(written["extension"]["status"], "not_loaded");
    assert!(written["written"]["manifestError"].is_null());
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("user/pomodoro/codexpp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["id"], "pomodoro");
    assert_eq!(manifest["version"], "0.1.0");
    assert_eq!(manifest["renderer"], json!(["renderer.js"]));
    assert_eq!(manifest["permissions"]["routes"], json!(["/diagnostics/log"]));

    let (_, read) = call(&creator, "codexpp_extension_read", json!({"id": "pomodoro"})).await;
    let paths = read["files"].as_array().unwrap().iter().map(|file| file["path"].as_str().unwrap()).collect::<Vec<_>>();
    assert_eq!(paths, ["lib/util.js", "renderer.js"], "清单不在文件列表里");
    assert_eq!(read["manifest"]["name"], "番茄钟");

    // 重复写入即覆盖。
    let (error, _) = call(
        &creator,
        "codexpp_extension_write",
        json!({"id": "pomodoro", "name": "番茄钟", "version": "0.2.0", "files": [{"path": "renderer.js", "content": "// v2"}]}),
    )
    .await;
    assert!(!error);
    assert_eq!(fs::read_to_string(temp.path().join("user/pomodoro/renderer.js")).unwrap(), "// v2");

    let (error, disabled) = call(&creator, "codexpp_extension_set_enabled", json!({"id": "pomodoro", "enabled": false})).await;
    assert!(!error);
    assert_eq!(disabled["extension"]["status"], "disabled");
    let (error, _) = call(&creator, "codexpp_extension_set_enabled", json!({"id": "missing", "enabled": true})).await;
    assert!(error);

    let (_, listed) = call(&creator, "codexpp_extension_list", json!({})).await;
    assert_eq!(listed["codexConnected"], false);
    assert_eq!(listed["items"][0]["key"], "user:pomodoro");
}

#[tokio::test]
async fn conversation_keeps_its_extension_across_interleaved_calls_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    for (thread, id, name) in [("thread-a", "alpha", "甲"), ("thread-b", "beta", "乙")] {
        let (error, written) = call_in_thread(
            &service,
            thread,
            "codexpp_extension_write",
            json!({"id": id, "name": name, "files": {"renderer.js": "// initial"}}),
        ).await;
        assert!(!error, "{written}");
        assert_eq!(written["conversation"], json!({"threadId": thread, "extensionId": id}));
    }

    for (thread, id) in [("thread-b", "beta"), ("thread-a", "alpha")] {
        let content = format!("// {id} updated");
        let (error, written) = call_in_thread(
            &service, thread, "codexpp_extension_write",
            json!({"files": {"renderer.js": content}}),
        ).await;
        assert!(!error, "{written}");
        assert_eq!(written["extension"]["key"], format!("user:{id}"));
        assert_eq!(fs::read_to_string(temp.path().join("user").join(id).join("renderer.js")).unwrap(), content);
        for tool in ["codexpp_extension_guide", "codexpp_extension_list", "codexpp_extension_read"] {
            let (error, result) = call_in_thread(&service, thread, tool, json!({})).await;
            assert!(!error, "{tool}: {result}");
            assert_eq!(result["conversation"], json!({"threadId": thread, "extensionId": id}));
            if tool == "codexpp_extension_read" {
                assert_eq!(result["manifest"]["id"], id);
            }
        }
    }

    // 用新的服务对象读取落盘绑定，不能依赖当前 MCP 进程里的最近一次调用。
    drop(service);
    let restarted = creator(&temp);
    for (thread, id) in [("thread-a", "alpha"), ("thread-b", "beta")] {
        let (error, read) = call_in_thread(&restarted, thread, "codexpp_extension_read", json!({})).await;
        assert!(!error, "{read}");
        assert_eq!(read["manifest"]["id"], id);
        assert_eq!(read["conversation"], json!({"threadId": thread, "extensionId": id}));
    }
    let (error, _) = call_in_thread(&restarted, "thread-c", "codexpp_extension_read", json!({})).await;
    assert!(error, "新对话不能继承其他对话的当前拓展");
}

#[tokio::test]
async fn conversation_requires_explicit_new_extension_or_existing_selection() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    service.manager.write_extension_package(&write("reference", &[("renderer.js", "// reference")])).unwrap();
    let (error, created) = call(&service, "codexpp_extension_write", json!({
        "id": "original", "name": "原拓展", "files": {"renderer.js": "// original"},
    })).await;
    assert!(!error, "{created}");

    let (error, reference) = call(&service, "codexpp_extension_read", json!({"id": "reference"})).await;
    assert!(!error, "{reference}");
    assert_eq!(reference["manifest"]["id"], "reference");
    assert_eq!(reference["conversation"]["extensionId"], "original", "参考阅读不能切换编辑目标");
    for id in ["reference", "accidental-new"] {
        let (error, message) = call(&service, "codexpp_extension_write", json!({
            "id": id, "name": "误换目标", "files": {"renderer.js": "// should not write"},
        })).await;
        assert!(error, "{message}");
    }
    assert!(!temp.path().join("user/accidental-new").exists());
    assert_eq!(fs::read_to_string(temp.path().join("user/reference/renderer.js")).unwrap(), "// reference");

    let (error, created) = call(&service, "codexpp_extension_write", json!({
        "id": "separate", "name": "另一个拓展", "newExtension": true, "files": {"renderer.js": "// separate"},
    })).await;
    assert!(!error, "{created}");
    assert_eq!(created["conversation"]["extensionId"], "separate");
    for id in ["reference", "separate"] {
        let (error, message) = call(&service, "codexpp_extension_write", json!({
            "id": id, "name": "重名", "newExtension": true, "files": {"renderer.js": "// should not write"},
        })).await;
        assert!(error, "新建不能覆盖已经占用的 id：{message}");
    }
    let (error, _) = call(&service, "codexpp_extension_read", json!({"id": "missing", "select": true})).await;
    assert!(error);
    let (error, current) = call(&service, "codexpp_extension_read", json!({})).await;
    assert!(!error, "{current}");
    assert_eq!(current["manifest"]["id"], "separate", "新建或选择失败保留当前目标");
    assert_eq!(fs::read_to_string(temp.path().join("user/separate/renderer.js")).unwrap(), "// separate");

    let (error, selected) = call(&service, "codexpp_extension_read", json!({"id": "original", "select": true})).await;
    assert!(!error, "{selected}");
    assert_eq!(selected["conversation"]["extensionId"], "original");
    let (error, updated) = call(&service, "codexpp_extension_write", json!({"files": {"renderer.js": "// selected"}})).await;
    assert!(!error, "{updated}");
    assert_eq!(fs::read_to_string(temp.path().join("user/original/renderer.js")).unwrap(), "// selected");

    let (error, reference) = call_in_thread(&service, "fresh-thread", "codexpp_extension_read", json!({"id": "original"})).await;
    assert!(!error, "{reference}");
    assert!(reference["conversation"]["extensionId"].is_null());
    let (error, message) = call_in_thread(&service, "fresh-thread", "codexpp_extension_write", json!({
        "id": "original", "name": "不能直接接管", "files": {"renderer.js": "// should not write"},
    })).await;
    assert!(error, "{message}");
    let (error, selected) = call_in_thread(&service, "fresh-thread", "codexpp_extension_read", json!({"id": "original", "select": true})).await;
    assert!(!error, "{selected}");
    assert_eq!(selected["conversation"], json!({"threadId": "fresh-thread", "extensionId": "original"}));
}

#[tokio::test]
async fn conversation_write_and_selection_require_trusted_call_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    service.manager.write_extension_package(&write("existing", &[("renderer.js", "// existing")])).unwrap();
    for meta in [None, Some(json!({})), Some(json!({"sessionId": "not-a-thread"}))] {
        let (error, message) = call_with_meta(&service, meta.clone(), "codexpp_extension_write", json!({
            "id": "missing-context", "name": "无上下文", "files": {"renderer.js": "// should not write"},
        })).await;
        assert!(error, "{message}");
        let (error, message) = call_with_meta(
            &service, meta.clone(), "codexpp_extension_read", json!({"id": "existing", "select": true}),
        ).await;
        assert!(error, "{message}");
        for tool in ["codexpp_extension_guide", "codexpp_extension_list", "codexpp_extension_read"] {
            let arguments = if tool == "codexpp_extension_read" { json!({"id": "existing"}) } else { json!({}) };
            let (error, result) = call_with_meta(&service, meta.clone(), tool, arguments).await;
            assert!(!error, "{tool}: {result}");
            assert!(result["conversation"]["threadId"].is_null());
            assert!(result["conversation"]["extensionId"].is_null());
        }
    }
    assert!(!temp.path().join("user/missing-context").exists());
    assert!(!temp.path().join("extension-creator-contexts.json").exists(), "缺少身份时不能创建全局当前目标");
    assert_eq!(fs::read_to_string(temp.path().join("user/existing/renderer.js")).unwrap(), "// existing");
}

#[tokio::test]
async fn conversation_write_errors_keep_target_but_invalid_manifests_remain_repairable() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    let (error, message) = call(&service, "codexpp_extension_write", json!({
        "id": "invalid", "name": "无效", "files": {"../outside.js": "// should not write"},
    })).await;
    assert!(error, "{message}");
    let (error, listed) = call(&service, "codexpp_extension_list", json!({})).await;
    assert!(!error, "{listed}");
    assert!(listed["conversation"]["extensionId"].is_null());
    assert!(!temp.path().join("user/invalid").exists());

    let (error, written) = call(&service, "codexpp_extension_write", json!({
        "id": "valid", "name": "有效", "files": {"renderer.js": "// valid"},
    })).await;
    assert!(!error, "{written}");
    let (error, result) = call(&service, "codexpp_extension_write", json!({
        "id": "invalid", "name": "无效", "newExtension": true, "files": {"../outside.js": ""},
    })).await;
    assert!(error, "{result}");
    let (error, current) = call(&service, "codexpp_extension_read", json!({})).await;
    assert!(!error, "{current}");
    assert_eq!(current["manifest"]["id"], "valid", "真正写入失败不能切换绑定");

    let (error, written) = call(&service, "codexpp_extension_write", json!({
        "id": "invalid-entry", "name": "缺少入口", "newExtension": true,
        "renderer": "missing.js", "files": {"other.js": ""},
    })).await;
    assert!(!error, "{written}");
    assert!(written["written"]["manifestError"].is_string(), "{written}");
    assert_eq!(written["conversation"]["extensionId"], "invalid-entry", "包已落盘时绑定它供下一轮修复");
    let (error, repaired) = call(&service, "codexpp_extension_write", json!({
        "files": {"missing.js": "codexPlus.extension(() => {});"},
    })).await;
    assert!(!error, "{repaired}");
    assert!(repaired["written"]["manifestError"].is_null(), "{repaired}");
    assert_eq!(repaired["conversation"]["extensionId"], "invalid-entry");
    assert_eq!(repaired["extension"]["key"], "user:invalid-entry");
    assert_eq!(
        fs::read_to_string(temp.path().join("user/invalid-entry/missing.js")).unwrap(),
        "codexPlus.extension(() => {});",
    );
    assert_eq!(fs::read_to_string(temp.path().join("user/valid/renderer.js")).unwrap(), "// valid");
}

#[tokio::test]
async fn conversation_does_not_recreate_a_missing_bound_package() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    let (error, written) = call(&service, "codexpp_extension_write", json!({
        "id": "moved", "name": "已移走", "files": {"renderer.js": "// original"},
    })).await;
    assert!(!error, "{written}");
    fs::rename(temp.path().join("user/moved"), temp.path().join("moved-package")).unwrap();
    for arguments in [
        json!({"files": {"renderer.js": "// should not write"}}),
        json!({"id": "moved", "name": "已移走", "files": {"renderer.js": "// should not write"}}),
    ] {
        let (error, message) = call(&service, "codexpp_extension_write", arguments).await;
        assert!(error, "{message}");
    }
    assert!(!temp.path().join("user/moved").exists(), "绑定失效时不能偷偷重建空壳拓展");
    assert_eq!(fs::read_to_string(temp.path().join("moved-package/renderer.js")).unwrap(), "// original");
}

#[tokio::test]
async fn conversation_corrupt_state_blocks_writes_without_replacing_the_state() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    let state_path = temp.path().join("extension-creator-contexts.json");
    for contents in [
        "{not valid json".to_string(),
        json!({"version": 1, "threads": {"test-thread": "../outside"}}).to_string(),
    ] {
        fs::write(&state_path, &contents).unwrap();
        let (error, message) = call(&service, "codexpp_extension_write", json!({
            "id": "accidental", "name": "不能覆盖损坏状态", "files": {"renderer.js": "// should not write"},
        })).await;
        assert!(error, "{message}");
        assert_eq!(fs::read_to_string(&state_path).unwrap(), contents);
        assert!(!temp.path().join("user/accidental").exists());
    }
}

#[tokio::test]
async fn conversation_updates_preserve_omitted_manifest_fields() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    let (error, written) = call(&service, "codexpp_extension_write", json!({
        "id": "combined", "name": "组合拓展", "description": "已有功能", "version": "0.4.0",
        "renderer": ["main.js"], "routes": ["/diagnostics/log"],
        "config": {"minutes": {"type": "number", "default": 25}},
        "hooks": {"Stop": [{"script": "hook.mjs"}]},
        "gateway": {"request": [{"script": "request.mjs"}]},
        "backend": {"script": "backend.mjs"},
        "tools": [{"name": "count", "description": "计数", "script": "count.mjs"}],
        "manager": {"page": "manager.html", "title": "管理"},
        "instances": [{"id": "work", "config": {"minutes": 50}}],
        "replaces": ["usage-statusline"],
        "files": {
            "main.js": "codexPlus.extension(() => {});",
            "hook.mjs": "export default () => null;",
            "request.mjs": "export default () => null;",
            "backend.mjs": "export default () => ({});",
            "count.mjs": "export default () => 1;",
            "manager.html": "<p>管理</p>"
        },
    })).await;
    assert!(!error, "{written}");
    assert!(written["written"]["manifestError"].is_null(), "{written}");
    let manifest_path = temp.path().join("user/combined/codexpp.json");
    let before: Value = serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();

    let (error, updated) = call(&service, "codexpp_extension_write", json!({
        "files": {"main.js": "codexPlus.extension(() => { /* added feature */ });"},
    })).await;
    assert!(!error, "{updated}");
    assert!(updated["written"]["manifestError"].is_null(), "{updated}");
    let after: Value = serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(after, before, "只改代码时不能清掉已有入口、配置、钩子和其他能力声明");
    assert_eq!(fs::read_to_string(temp.path().join("user/combined/count.mjs")).unwrap(), "export default () => 1;");

    let (error, updated) = call(&service, "codexpp_extension_write", json!({
        "description": "", "version": "0.5.0", "routes": [], "hooks": {}, "replaces": [],
        "files": {"main.js": "codexPlus.extension(() => {});"},
    })).await;
    assert!(!error, "{updated}");
    let explicit: Value = serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(explicit["description"], "");
    assert_eq!(explicit["version"], "0.5.0");
    assert_eq!(explicit["permissions"]["routes"], json!([]));
    assert_eq!(explicit["hooks"], json!({}));
    assert_eq!(explicit["replaces"], json!([]));
    assert_eq!(explicit["config"], before["config"]);
    assert_eq!(explicit["tools"], before["tools"]);
}

#[tokio::test]
async fn conversation_configuration_uses_current_target_and_preserves_other_threads() {
    let temp = tempfile::tempdir().unwrap();
    let service = creator(&temp);
    for (thread, id) in [("thread-a", "alpha"), ("thread-b", "beta")] {
        let (error, written) = call_in_thread(&service, thread, "codexpp_extension_write", json!({
            "id": id, "name": id, "config": {"minutes": {"type": "number", "default": 25}},
            "files": {"renderer.js": "codexPlus.extension(() => {});"},
        })).await;
        assert!(!error, "{written}");
    }
    let (error, configured) = call_in_thread(
        &service, "thread-a", "codexpp_extension_configure", json!({"values": {"minutes": 50}}),
    ).await;
    assert!(!error, "{configured}");
    assert_eq!(configured["config"]["minutes"], 50);
    assert_eq!(configured["conversation"], json!({"threadId": "thread-a", "extensionId": "alpha"}));
    let (error, message) = call_in_thread(
        &service, "thread-a", "codexpp_extension_configure", json!({"id": "beta", "values": {"minutes": 90}}),
    ).await;
    assert!(error, "{message}");
    let (error, configured) = call_in_thread(
        &service, "thread-b", "codexpp_extension_configure", json!({"values": {"minutes": 30}}),
    ).await;
    assert!(!error, "{configured}");
    assert_eq!(configured["config"]["minutes"], 30);
    assert_eq!(configured["conversation"]["extensionId"], "beta");
    let stored: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user_scripts.json")).unwrap()).unwrap();
    assert_eq!(stored["configs"]["user:alpha"]["minutes"], 50);
    assert_eq!(stored["configs"]["user:beta"]["minutes"], 30);

    let (error, configured) = call_in_thread(
        &service, "fresh-thread", "codexpp_extension_configure", json!({"id": "alpha", "values": {"minutes": 45}}),
    ).await;
    assert!(!error, "{configured}");
    assert_eq!(configured["conversation"], json!({"threadId": "fresh-thread", "extensionId": "alpha"}));
}

#[tokio::test]
async fn write_rejects_unsafe_input_and_reports_manifest_errors() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let manager = &creator.manager;
    for (case, error) in [
        (write("bad id", &[("renderer.js", "")]), "拓展 id"),
        (write("x.js", &[("renderer.js", "")]), "拓展 id"),
        (write("ok", &[]), "files 需要"),
        (write("ok", &[("../escape.js", "")]), "包内的相对路径"),
        (write("ok", &[("/abs.js", "")]), "包内的相对路径"),
        (write("ok", &[("codexpp.json", "{}")]), "由清单参数生成"),
    ] {
        let message = format!("{:#}", manager.write_extension_package(&case).unwrap_err());
        assert!(message.contains(error), "{message}");
    }
    let big = "x".repeat(300 * 1024);
    assert!(manager.write_extension_package(&write("ok", &[("renderer.js", &big)])).is_err());
    assert!(!temp.path().join("user/ok").exists(), "校验失败不留下目录");

    // 不接管别的目录：已有同名目录但不是同 id 的拓展包。
    fs::create_dir_all(temp.path().join("user/notes")).unwrap();
    fs::write(temp.path().join("user/notes/readme.txt"), "keep").unwrap();
    let message = format!("{:#}", manager.write_extension_package(&write("notes", &[("renderer.js", "")])).unwrap_err());
    assert!(message.contains("不是 id 为 notes 的拓展包"), "{message}");
    assert!(!temp.path().join("user/notes/renderer.js").exists());

    #[cfg(unix)]
    {
        // 包内子目录是指向包外的软链时拒绝写入。
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        manager.write_extension_package(&write("linked", &[("renderer.js", "")])).unwrap();
        std::os::unix::fs::symlink(&outside, temp.path().join("user/linked/lib")).unwrap();
        let message = format!(
            "{:#}",
            manager.write_extension_package(&write("linked", &[("lib/x.js", "")])).unwrap_err()
        );
        assert!(message.contains("拓展包以外"), "{message}");
        assert!(!outside.join("x.js").exists());
    }

    // 入口不存在：包写好了，但清单校验失败，如实返回原因。
    let mut missing = write("noentry", &[("other.js", "")]);
    missing.renderer = vec!["renderer.js".to_string()];
    let written = manager.write_extension_package(&missing).unwrap();
    assert!(written["manifestError"].as_str().unwrap().contains("不存在"));
}

#[test]
fn inspect_script_is_a_single_function_expression() {
    // 页面查看脚本以 `(脚本)(参数)` 求值：必须恰好是一个函数表达式，语法错误要在这里暴露，而不是到用户页面上才报。
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = root.join("assets/extension-creator/inspect.js");
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg(r#"
const source = require("node:fs").readFileSync(process.argv[1], "utf8").trim();
const fn = new Function(`return (${source});`)();
if (typeof fn !== "function") throw new Error("inspect.js 不是函数表达式");
if (/\.(innerHTML|outerHTML|textContent)\s*=[^=]|\.remove\(|\.append|setAttribute\(|\.click\(/.test(source)) throw new Error("inspect.js 必须只读");
"#)
        .arg(&script)
        .output()
        .expect("node is required");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[tokio::test]
async fn extension_declared_tools_are_listed_and_callable() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let (error, written) = call(
        &creator,
        "codexpp_extension_write",
        json!({
            "id": "calc", "name": "计算器",
            "tools": [
                {"name": "add", "description": "两数相加", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]}, "script": "tools/add.mjs"},
                {"name": "rich", "description": "返回内容块", "script": "tools/rich.mjs"},
                {"name": "boom", "description": "会出错", "script": "tools/boom.mjs", "timeout": 5},
                {"name": "history", "description": "查会话", "script": "tools/history.mjs"}
            ],
            "files": [
                {"path": "tools/add.mjs", "content": "export default ({ a, b }, { extension, tool }) => `${extension}/${tool}: ${a + b}`;"},
                {"path": "tools/rich.mjs", "content": "export default async () => ({ content: [{ type: 'text', text: '第一段' }, { type: 'text', text: '第二段' }] });"},
                {"path": "tools/boom.mjs", "content": "export default () => { throw new Error('坏工具'); };"},
                {"path": "tools/history.mjs", "content": "export default async (args, { sessionQuery }) => sessionQuery.readTitle('no-such-session-0000').then((title) => 'OK:' + title, (error) => 'ERR:' + error.message);"}
            ]
        }),
    )
    .await;
    assert!(!error, "{written}");
    assert_eq!(written["extension"]["status"], "not_loaded");
    assert_eq!(written["extension"]["package"]["tools"], json!(["add", "rich", "boom", "history"]));
    let manifest: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user/calc/codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["renderer"], json!([]), "只声明工具的拓展没有页面入口");

    let list = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"})).await.unwrap();
    let names = list["result"]["tools"].as_array().unwrap().iter().map(|tool| tool["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(names.contains(&"calc__add".to_string()) && names.contains(&"codexpp_extension_write".to_string()));
    let add = list["result"]["tools"].as_array().unwrap().iter().find(|tool| tool["name"] == "calc__add").unwrap();
    assert_eq!(add["inputSchema"]["required"], json!(["a", "b"]));
    assert!(add["description"].as_str().unwrap().contains("[Codex++ 拓展 calc]"));

    let response = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "calc__add", "arguments": {"a": 2, "b": 3}}})).await.unwrap();
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(response["result"]["content"][0]["text"], "calc/add: 5");
    let rich = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": {"name": "calc__rich", "arguments": {}}})).await.unwrap();
    assert_eq!(rich["result"]["content"][1]["text"], "第二段");
    let (error, message) = call(&creator, "calc__boom", json!({})).await;
    assert!(error && message.as_str().unwrap().contains("坏工具"), "{message}");
    // 工具脚本经宿主调用只读查询会话：错误原样回到脚本（不存在的会话，或本机没有 Codex 数据库）。
    let history = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 13, "method": "tools/call", "params": {"name": "calc__history", "arguments": {}}})).await.unwrap();
    let text = history["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("ERR:") && (text.contains("没有找到会话") || text.contains("会话数据库")), "{text}");

    creator.manager.set_script_enabled("user:calc", false).unwrap();
    let list = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 12, "method": "tools/list"})).await.unwrap();
    assert!(!list["result"]["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == "calc__add"), "停用后工具消失");
    let (error, _) = call(&creator, "calc__add", json!({"a": 1, "b": 1})).await;
    assert!(error);
}

#[tokio::test]
async fn extension_config_defaults_validation_and_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let (error, written) = call(
        &creator,
        "codexpp_extension_write",
        json!({
            "id": "pomo", "name": "番茄钟",
            "config": {
                "minutes": {"type": "number", "title": "时长", "default": 25},
                "theme": {"type": "select", "title": "配色", "options": [{"value": "red", "label": "红"}, "green"]},
                "token": {"type": "string", "title": "令牌", "secret": true}
            },
            "tools": [{"name": "minutes", "description": "当前时长", "script": "tools/minutes.mjs"}],
            "files": [
                {"path": "renderer.js", "content": "codexPlus.extension(() => {});"},
                {"path": "tools/minutes.mjs", "content": "export default (_, { config }) => `${config.minutes}|${config.theme}`;"}
            ]
        }),
    )
    .await;
    assert!(!error, "{written}");
    let package = &written["extension"]["package"];
    assert_eq!(package["config"], json!({"minutes": 25, "theme": "red", "token": ""}), "默认值与类型兜底");
    assert_eq!(package["configSchema"][1]["options"][1], json!({"value": "green", "label": "green"}));

    // 类型不符、未声明的键都拒绝，存储保持不变。
    for bad in [json!({"minutes": "50"}), json!({"theme": "blue"}), json!({"unknown": 1})] {
        let (error, message) = call(&creator, "codexpp_extension_configure", json!({"id": "pomo", "values": bad})).await;
        assert!(error, "{message}");
    }
    let (error, configured) = call(&creator, "codexpp_extension_configure", json!({"id": "pomo", "values": {"minutes": 50, "theme": "green"}})).await;
    assert!(!error, "{configured}");
    assert_eq!(configured["config"], json!({"minutes": 50, "theme": "green", "token": ""}));
    let stored: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user_scripts.json")).unwrap()).unwrap();
    assert_eq!(stored["configs"]["user:pomo"], json!({"minutes": 50, "theme": "green"}));

    // 页面清单带生效配置。
    let bundle = creator.manager.build_enabled_bundle().unwrap();
    assert!(bundle.contains(r#""config":{"minutes":50,"theme":"green","token":""}"#), "页面侧 ctx.config 的来源");

    // secret 项：管理器里填的 Key 不交给模型；对话只改别的键时 Key 保留，传回打码值也保留。
    creator.manager.set_extension_config("user:pomo", &json!({"minutes": 50, "theme": "green", "token": "sk-secret"}).as_object().unwrap().clone()).unwrap();
    let (_, listed) = call(&creator, "codexpp_extension_list", json!({})).await;
    assert!(!listed.to_string().contains("sk-secret"), "清单不能带出 secret 值：{listed}");
    let item = listed["items"].as_array().unwrap().iter().find(|item| item["key"] == "user:pomo").unwrap();
    assert_eq!(item["package"]["config"]["token"], "******");
    let (error, configured) = call(&creator, "codexpp_extension_configure", json!({"id": "pomo", "values": {"minutes": 40, "token": "******"}})).await;
    assert!(!error, "{configured}");
    assert!(!configured.to_string().contains("sk-secret"), "配置结果不能带出 secret 值：{configured}");
    let stored: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user_scripts.json")).unwrap()).unwrap();
    assert_eq!(stored["configs"]["user:pomo"], json!({"minutes": 40, "theme": "green", "token": "sk-secret"}));

    // 工具脚本从 context.config 读到同一份。
    if codex_plus_core::dsh_plugins::locate_node().is_some() {
        let response = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 21, "method": "tools/call", "params": {"name": "pomo__minutes", "arguments": {}}})).await.unwrap();
        assert_eq!(response["result"]["content"][0]["text"], "40|green");
    }
}

#[test]
fn package_locales_are_loaded_and_validated() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let package = |id: &str, files: &[(&str, &str)]| {
        let dir = temp.path().join("user").join(id);
        fs::create_dir_all(dir.join("locale")).unwrap();
        fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "description": "原始描述", "renderer": "main.js"}).to_string()).unwrap();
        fs::write(dir.join("main.js"), "").unwrap();
        for (path, content) in files {
            fs::write(dir.join(path), content).unwrap();
        }
    };
    package("hello", &[
        ("locale/en.json", r#"{"meta": {"title": "Hello", "description": "Says hello"}, "hi": "Hi {name}"}"#),
        ("locale/zh-TW.json", r#"{"meta": {"title": "哈囉"}}"#),
        ("locale/README.txt", "不是语言文件，忽略"),
    ]);
    package("bad-name", &[("locale/english.json", "{}")]);
    package("bad-json", &[("locale/en.json", "[1]")]);
    let inventory = creator.manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    let hello = find("user:hello");
    assert_eq!(hello["error"], "");
    assert_eq!(hello["package"]["locales"]["en"], json!({"title": "Hello", "description": "Says hello"}), "摘要只带名称与描述");
    assert_eq!(hello["package"]["locales"]["zh-TW"]["title"], "哈囉");
    assert!(hello["package"]["locales"]["zh-TW"]["description"].is_null());
    assert!(find("user:bad-name")["error"].as_str().unwrap().contains("不是合法的语言 id"));
    assert!(find("user:bad-json")["error"].as_str().unwrap().contains("顶层必须是对象"));
    // 页面拿到的清单带整份词条。
    let bundle = creator.manager.build_initial_bundle().unwrap();
    assert!(bundle.contains("Hi {name}"), "renderer 清单带上包内词条");
}

#[tokio::test]
async fn write_accepts_lenient_shapes_and_copies_binary_assets() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    // 一张真实的 1x1 PNG，按字节复制进包。
    let png: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63,
        0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    let source = temp.path().join("pet.png");
    fs::write(&source, png).unwrap();

    // renderer 写成字符串、files 里混用 content 与 sourcePath。
    let (error, written) = call(&creator, "codexpp_extension_write", json!({
        "id": "pet", "name": "桌宠", "renderer": "main.js",
        "files": [
            {"path": "main.js", "content": "codexPlus.extension((ctx) => { window.__petSrc = ctx.asset('assets/pet.png'); });"},
            {"path": "assets/pet.png", "sourcePath": source.to_string_lossy()},
        ],
    })).await;
    assert!(!error, "{written}");
    assert!(written.get("items").is_none(), "写入结果只带这一个拓展，不带全部列表");
    assert_eq!(written["extension"]["package"]["assets"], json!(["assets/pet.png"]));
    assert_eq!(fs::read(temp.path().join("user/pet/assets/pet.png")).unwrap(), png, "按字节复制");
    let manifest: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user/pet/codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["renderer"], json!(["main.js"]));
    let bundle = creator.manager.build_initial_bundle().unwrap();
    assert!(bundle.contains("data:image/png;base64,iVBORw0KGgo"), "页面清单带上资源的 data URL");

    // files 写成 { 路径: 内容 } 也认。
    let (error, written) = call(&creator, "codexpp_extension_write", json!({
        "id": "map-files", "name": "映射写法", "newExtension": true,
        "files": {"renderer.js": "codexPlus.extension(() => {});"},
    })).await;
    assert!(!error, "{written}");
    assert!(temp.path().join("user/map-files/renderer.js").is_file());

    // 不合格的复制：不是资源类型、也不是 UTF-8 文本、相对路径、content 与 sourcePath 同时给、资源超过 1 MiB。
    let binary = temp.path().join("notes.bin");
    fs::write(&binary, [0xff_u8, 0xfe, 0x00]).unwrap();
    let big = temp.path().join("big.png");
    fs::write(&big, vec![0u8; 1024 * 1024 + 1]).unwrap();
    for (files, expected) in [
        (json!([{"path": "assets/notes.bin", "sourcePath": binary.to_string_lossy()}]), "不是 UTF-8"),
        (json!([{"path": "assets/pet.png", "sourcePath": "pet.png"}]), "必须是绝对路径"),
        (json!([{"path": "assets/pet.png", "content": "x", "sourcePath": source.to_string_lossy()}]), "其中之一"),
        (json!([{"path": "assets/big.png", "sourcePath": big.to_string_lossy()}]), "超过单个资源上限"),
    ] {
        let (error, message) = call(&creator, "codexpp_extension_write", json!({"id": "bad-asset", "name": "坏资源", "newExtension": true, "files": files})).await;
        assert!(error && message.as_str().unwrap().contains(expected), "{expected}: {message}");
    }
    assert!(!temp.path().join("user/bad-asset").exists(), "校验失败时什么都不写");
}

#[test]
fn headless_packages_do_not_block_hot_reload() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let dir = temp.path().join("user/hooks-only");
    fs::create_dir_all(dir.join("hooks")).unwrap();
    fs::write(dir.join("hooks/guard.mjs"), "export default () => null;").unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": "hooks-only", "name": "只有钩子", "hooks": {"PreToolUse": [{"script": "hooks/guard.mjs"}]}}).to_string()).unwrap();
    let page = temp.path().join("user/with-page");
    fs::create_dir_all(&page).unwrap();
    fs::write(page.join("main.js"), "codexPlus.extension(() => {});").unwrap();
    fs::write(page.join("codexpp.json"), json!({"id": "with-page", "name": "有页面", "renderer": "main.js"}).to_string()).unwrap();

    let bundle = creator.manager.build_initial_bundle().unwrap();
    let section = |key: &str| {
        let start = bundle.find(&format!("const key = \"{key}\"")).unwrap();
        bundle[start..].split("const key = ").nth(1).map(str::to_string).unwrap_or_default()
    };
    assert!(section("user:hooks-only").contains(".headless = true") && section("user:hooks-only").contains(".cleanups = [() => {}]"),
        "没有页面入口的包登记空清理函数，热重载不必整页刷新");
    assert!(!section("user:with-page").contains(".headless = true"), "有页面入口的包靠 codexPlus.extension 自己登记清理");
}

#[test]
fn config_schema_supports_text_ranges_arrays_objects_and_i18n() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let package = |id: &str, config: Value| {
        let dir = temp.path().join("user").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "renderer": "main.js", "config": config}).to_string()).unwrap();
        fs::write(dir.join("main.js"), "").unwrap();
    };
    package("rich", json!({
        "prompt": {"type": "text", "title": {"zh": "提示词", "en": "Prompt"}, "default": "第一行\n第二行"},
        "minutes": {"type": "number", "min": 5, "max": 120},
        "words": {"type": "array", "items": {"type": "string"}, "default": ["机密"]},
        "levels": {"type": "array", "items": {"type": "select", "options": ["low", "high"]}},
        "proxy": {"type": "object", "properties": {
            "host": {"type": "string", "default": "127.0.0.1"},
            "port": {"type": "number", "default": 7890},
            "auth": {"type": "object", "properties": {"user": {"type": "string"}}}
        }, "default": {"port": 1080}}
    }));
    package("too-deep", json!({"a": {"type": "object", "properties": {"b": {"type": "object", "properties": {"c": {"type": "object", "properties": {"d": {"type": "string"}}}}}}}}));
    package("nested-array", json!({"a": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}}}));
    package("bad-title", json!({"a": {"type": "string", "title": 3}}));
    let inventory = creator.manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    let rich = find("user:rich");
    assert_eq!(rich["error"], "");
    assert_eq!(rich["package"]["config"], json!({
        "prompt": "第一行\n第二行",
        "minutes": 5.0,
        "words": ["机密"],
        "levels": [],
        "proxy": {"host": "127.0.0.1", "port": 1080, "auth": {"user": ""}},
    }), "对象默认值由子字段默认值组成，清单 default 覆盖其中的键；数字默认取 min");
    let schema = rich["package"]["configSchema"].as_array().unwrap();
    assert_eq!(schema.iter().find(|field| field["key"] == "prompt").unwrap()["title"], json!({"zh": "提示词", "en": "Prompt"}));
    assert!(find("user:too-deep")["error"].as_str().unwrap().contains("嵌套过深"));
    assert!(find("user:nested-array")["error"].as_str().unwrap().contains("只能是简单类型"));
    assert!(find("user:bad-title")["error"].as_str().unwrap().contains("title 必须是字符串"));

    // 保存：对象只给部分键，其余取默认；超出范围、未声明的键、数组元素类型不符都拒绝。
    let values = |value: Value| value.as_object().unwrap().clone();
    let saved = creator.manager.set_extension_config("user:rich", &values(json!({
        "minutes": 30, "levels": ["high", "low"], "proxy": {"host": "10.0.0.1", "auth": {"user": "me"}},
    }))).unwrap();
    assert_eq!(saved["proxy"], json!({"host": "10.0.0.1", "port": 7890, "auth": {"user": "me"}}));
    assert_eq!(saved["levels"], json!(["high", "low"]));
    for bad in [json!({"minutes": 500}), json!({"proxy": {"hostname": "x"}}), json!({"levels": ["mid"]}), json!({"words": [1]})] {
        assert!(creator.manager.set_extension_config("user:rich", &values(bad.clone())).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn extension_prompts_and_resources_are_served_over_mcp() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let dir = temp.path().join("user/kb");
    fs::create_dir_all(dir.join("docs")).unwrap();
    fs::write(dir.join("docs/style.md"), "# 规范\n用短句。").unwrap();
    fs::write(dir.join("review.md"), "请审查 {{file}}，重点看 {{focus}}。").unwrap();
    fs::write(dir.join("codexpp.json"), json!({
        "id": "kb", "name": "知识库",
        "prompts": [{ "name": "review", "description": "审查", "arguments": [{ "name": "file", "required": true }, { "name": "focus" }], "template": "review.md" }],
        "resources": [{ "name": "写作规范", "file": "docs/style.md" }]
    }).to_string()).unwrap();
    let rpc = |id: u64, method: &str, params: Value| json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    let init = handle_request(&creator, rpc(1, "initialize", json!({}))).await.unwrap();
    assert!(init["result"]["capabilities"]["prompts"].is_object() && init["result"]["capabilities"]["resources"].is_object());
    let prompts = handle_request(&creator, rpc(2, "prompts/list", json!({}))).await.unwrap();
    assert_eq!(prompts["result"]["prompts"][0]["name"], "kb__review");
    let got = handle_request(&creator, rpc(3, "prompts/get", json!({"name": "kb__review", "arguments": {"file": "a.rs"}}))).await.unwrap();
    assert_eq!(got["result"]["messages"][0]["content"]["text"], "请审查 a.rs，重点看 。");
    let missing = handle_request(&creator, rpc(4, "prompts/get", json!({"name": "kb__review", "arguments": {}}))).await.unwrap();
    assert!(missing["error"]["message"].as_str().unwrap().contains("缺少参数 file"));
    let resources = handle_request(&creator, rpc(5, "resources/list", json!({}))).await.unwrap();
    let uri = resources["result"]["resources"][0]["uri"].as_str().unwrap().to_string();
    assert_eq!(uri, "codexpp://kb/docs/style.md");
    assert_eq!(resources["result"]["resources"][0]["mimeType"], "text/markdown");
    let read = handle_request(&creator, rpc(6, "resources/read", json!({"uri": uri}))).await.unwrap();
    assert_eq!(read["result"]["contents"][0]["text"], "# 规范\n用短句。");
    creator.manager.set_script_enabled("user:kb", false).unwrap();
    let after = handle_request(&creator, rpc(7, "resources/list", json!({}))).await.unwrap();
    assert!(after["result"]["resources"].as_array().unwrap().is_empty(), "停用后不再提供");
}

#[test]
fn package_instances_are_validated_and_reach_the_page() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let package = |id: &str, instances: serde_json::Value| {
        let dir = temp.path().join("user").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("codexpp.json"), json!({
            "id": id, "name": id, "renderer": "main.js",
            "config": { "label": { "type": "string", "default": "默认" } },
            "instances": instances,
        }).to_string()).unwrap();
        fs::write(dir.join("main.js"), "").unwrap();
    };
    package("multi", json!([{ "id": "work", "name": "工作", "config": { "label": "工作区" } }, { "id": "home" }]));
    package("dup", json!([{ "id": "a" }, { "id": "a" }]));
    package("undeclared", json!([{ "id": "a", "config": { "color": "red" } }]));
    package("bad-id", json!([{ "id": "有空 格" }]));
    let inventory = creator.manager.inventory().unwrap();
    let error = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap()["error"].as_str().unwrap().to_string();
    assert_eq!(error("user:multi"), "");
    assert!(error("user:dup").contains("重复"));
    assert!(error("user:undeclared").contains("没有在清单 config 里声明"));
    assert!(error("user:bad-id").contains("只能是"));
    let bundle = creator.manager.build_initial_bundle().unwrap();
    // 按 JSON 结构比较：serde_json 开不开 preserve_order（Linux 上随工作区依赖打开）键顺序不同。
    let start = bundle.find(r#""instances":["#).expect("页面清单带上实例") + r#""instances":"#.len();
    let instances: serde_json::Value = serde_json::Deserializer::from_str(&bundle[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(
        instances,
        json!([
            { "config": { "label": "工作区" }, "id": "work", "name": "工作" },
            { "config": {}, "id": "home", "name": "home" },
        ]),
        "页面清单带上实例"
    );
}

#[tokio::test]
async fn write_passes_manager_panel_and_replaces_into_the_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let (error, written) = call(
        &creator,
        "codexpp_extension_write",
        json!({
            "id": "my-usage", "name": "我的用量",
            "renderer": "renderer.js",
            "replaces": ["usage-statusline"],
            "manager": {"page": "manager.html", "title": "用量面板"},
            "files": [
                {"path": "renderer.js", "content": "codexPlus.extension((ctx) => {});"},
                {"path": "manager.html", "content": "<p>面板</p>"}
            ]
        }),
    )
    .await;
    assert!(!error, "{written}");
    assert!(written["written"]["manifestError"].is_null(), "{written}");
    let manifest: Value = serde_json::from_str(&fs::read_to_string(temp.path().join("user/my-usage/codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["replaces"], json!(["usage-statusline"]), "顶替内置拓展的声明写进清单");
    assert_eq!(manifest["manager"], json!({"page": "manager.html", "title": "用量面板"}), "管理器面板的声明写进清单");
    let (title, html) = creator.manager.extension_manager_page("user:my-usage").unwrap();
    assert_eq!((title.as_str(), html.as_str()), ("用量面板", "<p>面板</p>"));
}

#[tokio::test]
async fn guide_covers_the_public_extension_surface() {
    let temp = tempfile::tempdir().unwrap();
    let (_, guide) = call(&creator(&temp), "codexpp_extension_guide", json!({})).await;
    let guide = guide["guide"].as_str().unwrap();
    // 写入工具的参数说明里引用到的指南小节，指南里都要有。
    for section in ["## 管理器面板", "## 顶替 Codex++ 内置功能", "## 关掉 Codex++ 网关的内置环节", "## 在拓展包里带 dsh 插件", "## 用了第三方库的界面（打包产物）", "## 赶在 Codex 之前（preload）与启动前后（launch）"] {
        assert!(guide.contains(section), "指南缺少「{section}」");
    }
    for api in ["ctx.onScan", "settings/changed", "placement: \"inline\"", "window.codexPlusManager", "builtin:key-cooldown"] {
        assert!(guide.contains(api), "指南没写 {api}");
    }
    assert!(!guide.contains('\u{fffd}'), "指南里有乱码");
}

/// 本机文件经 sourcePath 复制：图片按资源复制，其余按文本复制（打包产物，单个上限 4 MiB，不受逐字写的 256 KiB 限制）；
/// 逐字写的 content 仍守 256 KiB。instances 与 dsh 写进清单。
#[tokio::test]
async fn write_copies_large_text_files_and_passes_instances_and_dsh() {
    let temp = tempfile::tempdir().unwrap();
    let creator = creator(&temp);
    let source = temp.path().join("dist");
    fs::create_dir_all(&source).unwrap();
    // 约 600 KiB 的「打包产物」：超过逐字写的上限，但在复制上限以内。
    let bundle = format!("codexPlus.extension((ctx) => {{ ctx.config; }});\n/* {} */\n", "x".repeat(600 * 1024));
    fs::write(source.join("renderer.js"), &bundle).unwrap();
    fs::write(source.join("logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    fs::write(source.join("blob.bin"), [0xff_u8, 0xfe, 0x00, 0x80]).unwrap();
    fs::write(source.join("huge.js"), vec![b'a'; 4 * 1024 * 1024 + 1]).unwrap();
    let dsh_package = r#"{"name":"inner-dsh","version":"0.1.0","type":"module","main":"index.js"}"#;
    let (error, written) = call(
        &creator,
        "codexpp_extension_write",
        json!({
            "id": "bundled", "name": "打包产物",
            "config": {"label": {"type": "string", "default": "默认"}},
            "instances": [{"id": "work", "name": "工作", "config": {"label": "工作区"}}, {"id": "home"}],
            "dsh": "dsh-plugin",
            "files": [
                {"path": "renderer.js", "sourcePath": source.join("renderer.js").to_string_lossy()},
                {"path": "assets/logo.png", "sourcePath": source.join("logo.png").to_string_lossy()},
                {"path": "dsh-plugin/package.json", "content": dsh_package},
                {"path": "dsh-plugin/index.js", "content": "export const name = 'inner'\nexport function apply() {}\n"}
            ]
        }),
    )
    .await;
    assert!(!error, "{written}");
    assert!(written["written"]["manifestError"].is_null(), "{written}");
    let root = temp.path().join("user/bundled");
    assert_eq!(fs::read_to_string(root.join("renderer.js")).unwrap(), bundle, "打包产物原样复制");
    assert_eq!(fs::read(root.join("assets/logo.png")).unwrap(), b"\x89PNG\r\n\x1a\n");
    let manifest: Value = serde_json::from_str(&fs::read_to_string(root.join("codexpp.json")).unwrap()).unwrap();
    assert_eq!(manifest["instances"][0]["id"], "work");
    assert_eq!(manifest["dsh"], "dsh-plugin");

    // 逐字写的 content 仍守 256 KiB，报错里提示改用 sourcePath。
    let (error, message) = call(&creator, "codexpp_extension_write", json!({"id": "inline", "name": "逐字", "newExtension": true, "files": [{"path": "renderer.js", "content": "x".repeat(300 * 1024)}]})).await;
    assert!(error && message.as_str().unwrap().contains("sourcePath"), "{message}");
    // 复制来的文本超过 4 MiB、不是 UTF-8、不是绝对路径都拒绝。
    for (path, source, expected) in [
        ("renderer.js", source.join("huge.js").to_string_lossy().into_owned(), "超过复制文件上限"),
        ("data.bin", source.join("blob.bin").to_string_lossy().into_owned(), "不是 UTF-8"),
        ("renderer.js", "dist/renderer.js".to_string(), "必须是绝对路径"),
    ] {
        let (error, message) = call(&creator, "codexpp_extension_write", json!({"id": "bad", "name": "坏", "newExtension": true, "files": [{"path": path, "sourcePath": source}]})).await;
        assert!(error && message.as_str().unwrap().contains(expected), "{path}: {message}");
    }
}

/// 取消接入：本工具条目置为 `enabled = false`，用户字段与别的 MCP 不动，重新接入后恢复启用。
#[test]
fn disconnect_disables_only_owned_entry_and_reconnect_restores_it() {
    use codex_plus_core::extension_creator::{LEGACY_MCP_FLAG, LEGACY_SERVER_ID, SERVER_ID, SPEC};
    use codex_plus_core::managed_mcp;

    let home = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let state = home.path().join("state");
    let config_path = home.path().join("config.toml");
    fs::write(&config_path, "[mcp_servers.other]\ncommand = 'x'\n").unwrap();

    // 没接入过：取消接入什么都不写。
    let receipt = managed_mcp::disable(&SPEC, home.path(), &executable, &state).unwrap();
    assert!(!receipt.changed);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), "[mcp_servers.other]\ncommand = 'x'\n");

    managed_mcp::install(&SPEC, home.path(), &executable, &state).unwrap();
    // 用户在条目上自加的字段要跨取消/重新接入保留。接入时已经写好启动超时，
    // 这里改它的值来模拟用户改动（再插一条同名键会让整份配置解析失败）。
    let with_user_field = fs::read_to_string(&config_path)
        .unwrap()
        .replace("startup_timeout_sec = 120.0", "startup_timeout_sec = 30");
    fs::write(&config_path, with_user_field).unwrap();

    let receipt = managed_mcp::disable(&SPEC, home.path(), &executable, &state).unwrap();
    assert!(receipt.changed);
    assert!(receipt.status.disabled);
    assert!(!receipt.status.registered);
    let doc: toml_edit::DocumentMut = fs::read_to_string(&config_path).unwrap().parse().unwrap();
    assert_eq!(doc["mcp_servers"][SERVER_ID]["enabled"].as_bool(), Some(false));
    assert_eq!(doc["mcp_servers"][SERVER_ID]["startup_timeout_sec"].as_integer(), Some(30));
    assert_eq!(doc["mcp_servers"]["other"]["command"].as_str(), Some("x"));
    // 重复取消不再写文件。
    assert!(!managed_mcp::disable(&SPEC, home.path(), &executable, &state).unwrap().changed);

    let receipt = managed_mcp::install(&SPEC, home.path(), &executable, &state).unwrap();
    assert!(receipt.status.registered);
    assert!(!receipt.status.disabled);
    let doc: toml_edit::DocumentMut = fs::read_to_string(&config_path).unwrap().parse().unwrap();
    assert_eq!(doc["mcp_servers"][SERVER_ID]["enabled"].as_bool(), Some(true));
    assert_eq!(doc["mcp_servers"][SERVER_ID]["startup_timeout_sec"].as_integer(), Some(30));

    // 纯转换：旧名条目归本工具所有也一并停用；别人的同名条目不碰。
    let legacy = format!(
        "[mcp_servers.{LEGACY_SERVER_ID}]\ncommand = 'x'\nargs = ['{LEGACY_MCP_FLAG}']\n\n[mcp_servers.{SERVER_ID}]\ncommand = 'other'\nargs = ['--x']\n"
    );
    let disabled: toml_edit::DocumentMut =
        managed_mcp::disable_managed_context(&SPEC, &legacy).unwrap().parse().unwrap();
    assert_eq!(disabled["mcp_servers"][LEGACY_SERVER_ID]["enabled"].as_bool(), Some(false));
    assert!(disabled["mcp_servers"][SERVER_ID].get("enabled").is_none());
    assert_eq!(managed_mcp::disable_managed_context(&SPEC, "model = 'x'\n").unwrap(), "model = 'x'\n");
}

/// 托管上下文里已停用的条目经切换供应商的 apply 流程合并后，不会在 live 配置里复活
/// （apply 流程把停用条目整条移出 live），托管上下文仍能认出「已取消接入」。
#[test]
fn disabled_managed_context_survives_the_relay_apply_path() {
    use codex_plus_core::extension_creator::{SERVER_ID, SPEC};
    use codex_plus_core::managed_mcp;

    let home = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let state = home.path().join("state");
    let managed = managed_mcp::upsert_managed_context(&SPEC, "", &executable, &state).unwrap();
    let managed = managed_mcp::disable_managed_context(&SPEC, &managed).unwrap();
    codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
        home.path(),
        "model = \"second\"\n",
        "{}",
        &managed,
        "",
        "",
    )
    .unwrap();
    let status = managed_mcp::status(&SPEC, home.path(), &executable, &state).unwrap();
    assert!(!status.registered);
    let live = fs::read_to_string(home.path().join("config.toml")).unwrap();
    assert!(!live.contains(SERVER_ID));
    assert!(managed_mcp::disabled_in_context(&SPEC, &managed));
    let enabled = managed_mcp::upsert_managed_context(&SPEC, &managed, &executable, &state).unwrap();
    assert!(!managed_mcp::disabled_in_context(&SPEC, &enabled));
}
