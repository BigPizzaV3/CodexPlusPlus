//! Claude 页面拓展：`codexpp.json` 的 `claudeDesktop.renderer` / `styles` / `methods`。
//!
//! 这三个字段的规则在 claude-local 里（ClaudeMom 仓库，单独的 ClaudeMom 也用它），Codex++ 解析自己目录里的
//! 拓展包时调的是同一份。这里对着两边共用的用例再跑一遍，并确认页面包受 Claude 的两级开关约束、
//! 页面调后台和 Code 会话里的按钮用的是同一个后台实例。

use codex_plus_core::extension_backend;
use codex_plus_core::extension_claude::{self, ClaudeDesktopExtension};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;
use std::time::Duration;

fn manager(temp: &tempfile::TempDir, enabled: bool) -> UserScriptManager {
    let settings = temp.path().join("settings.json");
    fs::write(&settings, json!({"claudeAppExtensions": enabled}).to_string()).unwrap();
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
        .with_settings_path(settings)
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: Value, backend: Option<&str>) {
    let root = temp.path().join("user").join(id);
    fs::create_dir_all(&root).unwrap();
    let mut full = json!({ "id": id, "name": format!("Extension {id}"), "version": "1.2.3" });
    full.as_object_mut().unwrap().extend(manifest.as_object().unwrap().clone());
    fs::write(root.join("codexpp.json"), full.to_string()).unwrap();
    fs::write(root.join("page.js"), "window.page = 1;").unwrap();
    fs::write(root.join("page.css"), "body{}").unwrap();
    if let Some(script) = backend {
        fs::write(root.join("main.mjs"), script).unwrap();
    }
}

/// 两个仓库共用的用例：claude-local 的单元测试对着同一份文件跑。这里走的是 Codex++ 自己的解析入口，
/// 页面部分得到的结果必须和 claude-local 一样。
#[test]
fn shared_manifest_cases_parse_the_same_as_in_claude_local() {
    let cases: Vec<Value> = serde_json::from_str(claude_local::extensions::CODEXPP_CASES).unwrap();
    assert!(cases.len() >= 12);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let parsed = ClaudeDesktopExtension::parse(&case["claudeDesktop"]);
        match &case["page"] {
            Value::Null => assert!(parsed.is_err(), "{name} 应当报错"),
            // 没有页面部分：这里要么整块不合法（比如空对象），要么只剩 Code 会话的声明。
            Value::Bool(false) => assert!(parsed.map(|declaration| declaration.page).unwrap_or(None).is_none(), "{name}"),
            expected => {
                let page = parsed.unwrap_or_else(|error| panic!("{name}: {error:#}")).page.unwrap_or_else(|| panic!("{name} 应当有页面部分"));
                assert_eq!(&serde_json::to_value(&page).unwrap(), expected, "{name}");
            }
        }
    }
    assert_eq!(claude_local::extensions::PAGE_API_VERSION, codex_plus_core::user_scripts::EXTENSION_API_VERSION);
}

#[test]
fn page_packages_follow_both_claude_switches_and_leave_mods_untouched() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "page-only", json!({ "claudeDesktop": { "renderer": "page.js", "styles": ["page.css"] } }), None);
    write_package(
        &temp,
        "full",
        json!({
            "description": "D",
            "permissions": { "routes": ["/backend/status"] },
            "backend": { "script": "main.mjs" },
            "config": { "token": { "type": "string", "secret": true, "default": "s3cret" } },
            "claudeDesktop": {
                "renderer": ["page.js"],
                "methods": ["status"],
                "buttons": [{ "id": "count", "label": "Count", "method": "add" }],
            },
        }),
        Some("export default () => ({})"),
    );
    write_package(
        &temp,
        "mods-only",
        json!({ "backend": { "script": "main.mjs" }, "claudeDesktop": { "buttons": [{ "id": "a", "label": "A", "method": "add" }] } }),
        Some("export default () => ({})"),
    );
    write_package(&temp, "codex-only", json!({ "renderer": "page.js" }), None);
    // 页面要调后台却没有声明后台：清单有误，哪边都不加载。
    write_package(&temp, "broken", json!({ "claudeDesktop": { "renderer": "page.js", "methods": ["status"] } }), None);

    let pages = manager.enabled_claude_page_packages();
    assert_eq!(pages.iter().map(|package| package.key.as_str()).collect::<Vec<_>>(), ["user:full", "user:page-only"]);
    let full = &pages[0];
    assert_eq!((full.id.as_str(), full.name.as_str(), full.version.as_str(), full.description.as_str()), ("full", "Extension full", "1.2.3", "D"));
    // 包目录和后台脚本是解析过符号链接的绝对路径（macOS 的临时目录在 /private 下）。
    let root = temp.path().join("user").join("full").canonicalize().unwrap();
    assert_eq!(full.root, root);
    assert_eq!((full.scripts.as_slice(), full.methods.as_slice()), (&["page.js".to_string()][..], &["status".to_string()][..]));
    assert_eq!(full.routes, ["/backend/status"]);
    assert_eq!(full.backend.as_deref(), Some(root.join("main.mjs").as_path()));
    assert_eq!(full.config, json!({ "token": "s3cret" }), "后台拿完整配置；交给页面前由 claude-local 去掉密钥");
    assert!(full.config_schema[0].secret);
    assert_eq!(full.data_dir, temp.path().join("claude-extension-data").join("full"), "和 Code 会话里的按钮用同一个数据目录");
    let page_only = &pages[1];
    assert!(page_only.backend.is_none() && page_only.methods.is_empty());
    assert_eq!(page_only.styles, ["page.css"]);

    // Mods 插件只收有按钮、事件、工具的包，下发的声明里没有页面部分。
    let mods = manager.enabled_claude_extensions().unwrap();
    assert_eq!(mods.iter().map(|extension| extension.key.as_str()).collect::<Vec<_>>(), ["user:full", "user:mods-only"]);
    assert!(serde_json::to_value(&mods).unwrap()[0].get("page").is_none());

    let broken = manager.inventory().unwrap()["scripts"].as_array().unwrap().iter().find(|script| script["key"] == "user:broken").cloned().unwrap();
    assert!(broken["error"].as_str().unwrap().contains("backend.script"), "{broken}");

    // 每个拓展自己的 Claude 开关。Codex 那边的开关不影响 Claude。
    manager.set_claude_script_enabled("user:full", false).unwrap();
    manager.set_script_enabled("user:page-only", false).unwrap();
    assert_eq!(manager.enabled_claude_page_packages().iter().map(|package| package.key.as_str()).collect::<Vec<_>>(), ["user:page-only"]);
    // Claude 总开关。
    manager.set_claude_extensions_enabled(false).unwrap();
    assert!(manager.enabled_claude_page_packages().is_empty());
}

/// 界面要知道一个包有没有 Claude 页面部分：清单摘要里带着。
#[test]
fn inventory_shows_the_page_part_of_a_package() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "page-only", json!({ "claudeDesktop": { "renderer": "page.js" } }), None);
    let inventory = manager.inventory().unwrap();
    let script = inventory["scripts"].as_array().unwrap().iter().find(|script| script["key"] == "user:page-only").unwrap();
    assert_eq!(script["package"]["claudeDesktop"]["page"], json!({ "scripts": ["page.js"], "styles": [], "methods": [] }));
    assert_eq!(script["claude_enabled"], true);
}

/// 页面里的拓展脚本和 Code 会话里的按钮调的是同一个后台实例；各自只能调自己那份声明里的方法。
#[tokio::test]
async fn page_calls_and_mods_calls_share_one_backend_instance() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(
        &temp,
        "shared",
        json!({
            "backend": { "script": "main.mjs" },
            "config": { "token": { "type": "string", "secret": true, "default": "s3cret" } },
            "claudeDesktop": {
                "renderer": "page.js",
                "methods": ["status"],
                "buttons": [{ "id": "count", "label": "Count", "method": "add" }],
            },
        }),
        Some(
            r#"
export default (ctx) => {
  let count = 0;
  return {
    add() { return ++count; },
    status() { return { count, host: ctx.host, token: ctx.config.token, keys: Object.keys(ctx).sort() }; },
  };
};
"#,
        ),
    );
    let button = json!({ "key": "user:shared", "method": "add", "sessionId": "claude-test" });
    let page = |method: &'static str| extension_backend::call_claude_page(&manager, "user:shared", method, Value::Null, Duration::from_secs(10), Some("page-1"));
    let check = async {
        assert_eq!(extension_claude::call(&manager, &button).await.unwrap(), 1);
        assert_eq!(extension_claude::call(&manager, &button).await.unwrap(), 2);
        let status = page("status").await.unwrap();
        assert_eq!(status["count"], 2, "页面看到的是按钮点过两次之后的同一个后台");
        assert_eq!((status["host"].as_str(), status["token"].as_str()), (Some("claudeDesktop"), Some("s3cret")));
        // 后台脚本拿到的 ctx 和单独的 ClaudeMom 里一样（用的是 claude-local 里那份运行器）。
        assert_eq!(
            status["keys"],
            json!(["config", "dataDir", "emit", "extension", "host", "llm", "log", "provide", "service", "sessionQuery", "sessions", "ui"])
        );
        // 页面不能调按钮的方法，按钮也不能调页面的方法。
        let refused = page("add").await.unwrap_err().to_string();
        assert!(refused.contains("无权调用"), "{refused}");
        let other = json!({ "key": "user:shared", "method": "status" });
        assert!(extension_claude::call(&manager, &other).await.is_err());
        // 关掉这个拓展的 Claude 开关，页面那条路也调不到了。
        manager.set_claude_script_enabled("user:shared", false).unwrap();
        assert!(page("status").await.is_err());
    };
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(check).catch_unwind().await;
    extension_backend::stop_all_claude(&manager).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
