//! 拓展在页面之外的时机（codexpp.json 的 `preload` / `launch`）：清单校验、启用列表、新文档脚本、启动参数合并、
//! 启动前后脚本。
use codex_plus_core::extension_launch::{LaunchContext, merged_launch_args, preload_document_script, run_scripts};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    let mut manifest = manifest;
    manifest["id"] = json!(id);
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
}

fn script(manager: &UserScriptManager, key: &str) -> Value {
    manager.inventory().unwrap()["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone()
}

#[test]
fn preload_and_launch_alone_make_a_valid_package() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "early", json!({ "preload": { "scripts": ["preload.js"], "reloadOnLaunch": true } }), &[("preload.js", "window.__early = codexPlusPreload.id;")]);
    write_package(&temp, "args", json!({ "launch": { "args": ["--lang=zh-CN"], "before": "launch/before.mjs", "timeout": 5 } }), &[("launch/before.mjs", "export default () => null;")]);
    let early = script(&manager, "user:early");
    assert_eq!(early["error"], "", "{early}");
    assert_eq!(early["package"]["preload"], json!({ "scripts": 1, "reloadOnLaunch": true }));
    let args = script(&manager, "user:args");
    assert_eq!(args["error"], "", "{args}");
    assert_eq!(args["package"]["launch"], json!({ "args": ["--lang=zh-CN"], "before": true, "after": false }));

    let (preloads, reload) = manager.enabled_preloads().unwrap();
    assert!(reload);
    assert_eq!(preloads.len(), 1);
    assert_eq!(preloads[0].0, "user:early");
    let document = preload_document_script(&preloads).unwrap();
    assert!(document.contains("window.__early = codexPlusPreload.id;"));
    assert!(document.contains("\"id\":\"early\""));
    assert!(document.contains("__codexPlusPreloadErrors"), "每个拓展各自 try/catch");

    let launches = manager.enabled_launches().unwrap();
    assert_eq!(launches.len(), 1);
    assert_eq!(launches[0].0, "args");

    // 停用、或用户脚本总开关关掉后都不再生效。
    manager.set_script_enabled("user:early", false).unwrap();
    assert!(manager.enabled_preloads().unwrap().0.is_empty());
    manager.set_global_enabled(false).unwrap();
    assert!(manager.enabled_launches().unwrap().is_empty());
}

#[test]
fn invalid_preload_and_launch_declarations_are_reported() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "outside", json!({ "preload": ["../escape.js"] }), &[]);
    write_package(&temp, "missing", json!({ "preload": "nope.js" }), &[]);
    write_package(&temp, "port", json!({ "launch": { "args": ["--remote-debugging-port=1"] } }), &[]);
    write_package(&temp, "shell", json!({ "launch": { "args": ["rm -rf /"] } }), &[]);
    write_package(&temp, "timeout", json!({ "launch": { "before": "b.mjs", "timeout": 999 } }), &[("b.mjs", "export default () => null;")]);
    for (key, needle) in [
        ("user:outside", "相对路径"),
        ("user:missing", "nope.js"),
        ("user:port", "--remote-debugging-port"),
        ("user:shell", "不合法"),
        ("user:timeout", "launch.timeout"),
    ] {
        let error = script(&manager, key)["error"].as_str().unwrap().to_string();
        assert!(error.contains(needle), "{key}: {error}");
    }
}

#[test]
fn extension_launch_args_follow_the_users_own_args() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a", json!({ "launch": { "args": ["--lang=en", "--enable-features=Foo"] } }), &[]);
    let launches = manager.enabled_launches().unwrap();
    let merged = merged_launch_args(&["--lang=zh-CN".to_string()], &launches);
    assert_eq!(merged, vec!["--lang=zh-CN".to_string(), "--enable-features=Foo".to_string()], "同名参数用户的优先");
}

#[tokio::test]
async fn launch_before_and_after_scripts_run_with_the_launch_context() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let out = temp.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let writer = |event: &str| format!(
        "import fs from 'node:fs'; export default (context) => {{ fs.writeFileSync({}, JSON.stringify(context)); }};",
        serde_json::to_string(&out.join(format!("{event}.json"))).unwrap()
    );
    write_package(&temp, "hooks", json!({ "launch": { "before": "before.mjs", "after": "after.mjs" }, "config": { "mode": { "type": "string", "default": "x" } } }),
        &[("before.mjs", &writer("before")), ("after.mjs", &writer("after")), ]);
    write_package(&temp, "broken", json!({ "launch": { "before": "before.mjs" } }), &[("before.mjs", "export default () => { throw new Error('boom'); };")]);
    let launches = manager.enabled_launches().unwrap();
    let context = LaunchContext { codex_home: temp.path().join("codex"), state_dir: temp.path().join("state"), app_dir: temp.path().join("app") };
    // 一个拓展的脚本抛错不影响别的拓展。
    run_scripts("before", &context, &launches).await;
    run_scripts("after", &context, &launches).await;
    let before: Value = serde_json::from_slice(&fs::read(out.join("before.json")).unwrap()).unwrap();
    assert_eq!(before["event"], "before");
    assert_eq!(before["extension"], "hooks");
    assert_eq!(before["config"]["mode"], "x");
    assert_eq!(before["codexHome"], json!(temp.path().join("codex")));
    let after: Value = serde_json::from_slice(&fs::read(out.join("after.json")).unwrap()).unwrap();
    assert_eq!(after["event"], "after");
}
