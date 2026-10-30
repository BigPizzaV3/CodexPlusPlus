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
    let response = handle_request(
        creator,
        json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": arguments}}),
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
            "codexpp_extension_write",
            "codexpp_extension_set_enabled",
            "codexpp_extension_reload"
        ]
    );
    // 没有删除工具：删除留给用户自己。
    assert!(!names.iter().any(|name| name.contains("delete") || name.contains("remove")));
    let unknown = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}))
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
