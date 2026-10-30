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
    // 页面查看与截图只读；没连 Codex 时如实报错，不伪造结果。
    for tool in ["codexpp_extension_inspect", "codexpp_extension_screenshot"] {
        let (error, message) = call(&creator, tool, json!({})).await;
        assert!(error, "{tool}");
        assert!(message.as_str().unwrap().contains("没有找到已连接的 Codex 页面"), "{message}");
    }
    let (error, message) = call(&creator, "codexpp_extension_inspect", json!({"script": "alert(1)"})).await;
    assert!(error && message.as_str().unwrap().contains("参数无效"), "不接受代码参数：{message}");
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

    // 工具脚本从 context.config 读到同一份。
    if codex_plus_core::dsh_plugins::locate_node().is_some() {
        let response = handle_request(&creator, json!({"jsonrpc": "2.0", "id": 21, "method": "tools/call", "params": {"name": "pomo__minutes", "arguments": {}}})).await.unwrap();
        assert_eq!(response["result"]["content"][0]["text"], "50|green");
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
        "id": "map-files", "name": "映射写法", "files": {"renderer.js": "codexPlus.extension(() => {});"},
    })).await;
    assert!(!error, "{written}");
    assert!(temp.path().join("user/map-files/renderer.js").is_file());

    // 不合格的资源：类型不对、相对路径、content 与 sourcePath 同时给、超过 1 MiB。
    let text = temp.path().join("notes.txt");
    fs::write(&text, "x").unwrap();
    let big = temp.path().join("big.png");
    fs::write(&big, vec![0u8; 1024 * 1024 + 1]).unwrap();
    for (files, expected) in [
        (json!([{"path": "assets/notes.txt", "sourcePath": text.to_string_lossy()}]), "不是支持的资源类型"),
        (json!([{"path": "assets/pet.png", "sourcePath": "pet.png"}]), "必须是绝对路径"),
        (json!([{"path": "assets/pet.png", "content": "x", "sourcePath": source.to_string_lossy()}]), "其中之一"),
        (json!([{"path": "assets/big.png", "sourcePath": big.to_string_lossy()}]), "超过单个资源上限"),
    ] {
        let (error, message) = call(&creator, "codexpp_extension_write", json!({"id": "bad-asset", "name": "坏资源", "files": files})).await;
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
