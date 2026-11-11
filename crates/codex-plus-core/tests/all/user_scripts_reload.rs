use codex_plus_core::user_scripts::{BOOTSTRAP_SCRIPT, UserScriptManager};
use serde_json::json;
use std::{fs, process::Command};

#[test]
fn unconfirmed_reload_marks_runtime_status_unknown() {
    let mut inventory = json!({
        "scripts": [
            { "status": "not_loaded", "enabled": true },
            { "status": "disabled", "enabled": false },
            { "status": "failed", "error": "invalid manifest" }
        ]
    });
    codex_plus_core::user_scripts::mark_inventory_runtime_unknown(&mut inventory);
    assert!(inventory["scripts"].as_array().unwrap().iter().all(|script| script["status"] == "unknown"));
    assert_eq!(inventory["scripts"][1]["enabled"], false);
    assert_eq!(inventory["scripts"][2]["error"], "invalid manifest");
}

#[tokio::test]
async fn explicit_reload_waits_for_a_slow_cdp_response() {
    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("config.json"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let command: serde_json::Value = serde_json::from_str(
            socket.next().await.unwrap().unwrap().to_text().unwrap(),
        )
        .unwrap();
        assert_eq!(command["method"], "Runtime.evaluate");
        assert_eq!(command["params"]["awaitPromise"], false);
        tokio::time::sleep(Duration::from_millis(5_500)).await;
        socket.send(Message::Text(json!({
            "id": command["id"],
            "result": { "result": { "type": "string", "value": r#"{"mode":"scripts","scripts":{}}"# } }
        }).to_string().into())).await.unwrap();
    });

    let result = codex_plus_core::user_scripts::reload_scripts_at(
        &format!("ws://{address}"),
        &manager,
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    server.await.unwrap();
}

#[tokio::test]
async fn initial_load_keeps_the_original_timeout() {
    use codex_plus_core::user_scripts::{ReloadOutcomeUnknown, load_scripts_at};
    use futures_util::StreamExt;
    use tokio::net::TcpListener;

    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("config.json"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let command: serde_json::Value = serde_json::from_str(
            socket.next().await.unwrap().unwrap().to_text().unwrap(),
        ).unwrap();
        assert_eq!(command["params"]["awaitPromise"], false);
        let _ = socket.next().await;
    });

    let error = load_scripts_at(&format!("ws://{address}"), &manager).await.unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>(), "{error:#}");
    assert!(!error.is::<ReloadOutcomeUnknown>(), "{error:#}");
    assert!(error.to_string().contains("after 5s"), "{error:#}");
    server.await.unwrap();
}

#[tokio::test]
async fn reload_distinguishes_command_timeout_from_connection_and_script_errors() {
    use codex_plus_core::user_scripts::{ReloadOutcomeUnknown, reload_scripts_at};
    use futures_util::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("config.json"),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let command_server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let command: serde_json::Value = serde_json::from_str(
            socket.next().await.unwrap().unwrap().to_text().unwrap(),
        ).unwrap();
        assert_eq!(command["method"], "Runtime.evaluate");
        let _ = socket.next().await;
    });
    let error = reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap_err();
    assert!(error.is::<ReloadOutcomeUnknown>(), "{error:#}");
    assert!(error.is::<tokio::time::error::Elapsed>(), "{error:#}");
    command_server.await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connect_server = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(6)).await;
    });
    let error = reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap_err();
    assert!(error.is::<tokio::time::error::Elapsed>(), "{error:#}");
    assert!(!error.is::<ReloadOutcomeUnknown>(), "{error:#}");
    connect_server.await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let exception_server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let command: serde_json::Value = serde_json::from_str(
            socket.next().await.unwrap().unwrap().to_text().unwrap(),
        ).unwrap();
        socket.send(Message::Text(json!({
            "id": command["id"],
            "result": { "exceptionDetails": { "exception": { "description": "test exception" } } }
        }).to_string().into())).await.unwrap();
    });
    let error = reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap_err();
    assert!(!error.is::<ReloadOutcomeUnknown>(), "{error:#}");
    assert!(error.to_string().contains("test exception"), "{error:#}");
    exception_server.await.unwrap();
}

#[test]
fn global_switch_preserves_individual_script_choices_after_reload() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("enabled.js"), "window.enabledScript = true;").unwrap();
    fs::write(user.join("disabled.js"), "window.disabledScript = true;").unwrap();
    let config_path = temp.path().join("config.json");
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &config_path);
    manager.set_script_enabled("user:disabled.js", false).unwrap();
    manager.set_global_enabled(false).unwrap();

    let reloaded = UserScriptManager::new(temp.path().join("builtin"), &user, &config_path);
    assert!(!reloaded.load_config().enabled);
    let config = reloaded.set_global_enabled(true).unwrap();
    assert!(config.enabled);
    assert_eq!(config.scripts.get("user:disabled.js"), Some(&false));
    let bundle = reloaded.build_initial_bundle().unwrap();
    assert!(bundle.contains("window.enabledScript = true"));
    assert!(!bundle.contains("window.disabledScript = true"));
}

#[test]
fn skip_next_start_is_one_shot_and_preserves_script_choices() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("unsafe.js"), "window.unsafeScript = true;").unwrap();
    let config_path = temp.path().join("config.json");
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &config_path);
    manager
        .set_script_enabled("user:unsafe.js", true)
        .unwrap();
    manager.set_skip_next_start(true).unwrap();

    assert_eq!(
        manager.inventory().unwrap()["skip_next_start"].as_bool(),
        Some(true)
    );
    let skipped = manager.build_initial_bundle().unwrap();
    assert!(skipped.contains("mode: \"skipped\""));
    assert!(!skipped.contains("window.unsafeScript = true"));
    assert!(!manager.load_config().skip_next_start);
    assert_eq!(manager.load_config().scripts.get("user:unsafe.js"), Some(&true));

    let next = manager.build_initial_bundle().unwrap();
    assert!(next.contains("mode: \"scripts\""));
    assert!(next.contains("window.unsafeScript = true"));
}

#[test]
fn reload_cleans_resources_and_applies_current_files_and_switches() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        &user,
        temp.path().join("config.json"),
    );
    let path = user.join("demo.js");
    fs::write(
        &path,
        r#"
window.active = (window.active || 0) + 1;
window.version = 1;
window.__codexPlusUserScripts.registerCleanup(() => { window.active--; });
"#,
    )
    .unwrap();
    let first = manager.build_reload_bundle().unwrap();
    fs::write(
        &path,
        r#"
window.active = (window.active || 0) + 1;
window.version = 2;
window.__codexPlusUserScripts.registerCleanup(() => { window.active--; });
"#,
    )
    .unwrap();
    let updated = manager.build_reload_bundle().unwrap();
    manager.set_script_enabled("user:demo.js", false).unwrap();
    let disabled = manager.build_reload_bundle().unwrap();
    manager.set_script_enabled("user:demo.js", true).unwrap();
    manager.set_global_enabled(false).unwrap();
    let globally_disabled = manager.build_reload_bundle().unwrap();
    manager.set_global_enabled(true).unwrap();
    fs::write(&path, "window.legacy = (window.legacy || 0) + 1;").unwrap();
    let legacy = manager.build_reload_bundle().unwrap();
    let initial = manager.build_initial_bundle().unwrap();
    manager.delete_user_script("user:demo.js").unwrap();
    let deleted = manager.build_reload_bundle().unwrap();
    let empty_initial = manager.build_initial_bundle().unwrap();
    let input = json!({"first": first, "updated": updated, "disabled": disabled,
        "globallyDisabled": globally_disabled, "legacy": legacy, "deleted": deleted,
        "bootstrap": BOOTSTRAP_SCRIPT, "initial": initial, "emptyInitial": empty_initial});
    let fixture = temp.path().join("bundles.json");
    fs::write(&fixture, input.to_string()).unwrap();
    let output = Command::new("node").arg("-e").arg(r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
const bundles = require(process.argv[1]);
let reloads = 0;
const timers = [];
const window = { location: { href: 'app://-/index.html', reload() { reloads++; } },
  electronBridge: {}, setTimeout(fn) { timers.push(fn); } };
window.top = window.self = window;
const context = vm.createContext({ window, process, console });
const run = (name) => JSON.parse(vm.runInContext(bundles[name], context));
assert.equal(run('first').mode, 'scripts');
assert.equal(window.active, 1);
for (let i = 0; i < 20; i++) run('updated');
assert.equal(window.active, 1);
assert.equal(window.version, 2);
assert.equal(run('disabled').mode, 'scripts');
assert.equal(window.active, 0);
assert.deepEqual(Object.keys(window.__codexPlusUserScripts.scripts), []);
run('first'); run('globallyDisabled');
assert.equal(window.active, 0);
run('first'); run('deleted');
assert.equal(window.active, 0);
run('legacy');
assert.throws(() => run('first'), /Codex\+\+/);
assert.equal(window.active, 0); // 不能不清理就追加新实例
window.__codexPlusUserScriptsBootstrap = true;
assert.equal(run('first').mode, 'page');
assert.equal(run('first').mode, 'page');
assert.equal(timers.length, 1);
timers.shift()();
assert.equal(reloads, 1);

// 清理抛错时不加载新实例。
window.__codexPlusUserScripts = { scripts: { broken: { cleanups: [() => { throw Error('cleanup'); }] } } };
assert.equal(run('updated').mode, 'page');
assert.equal(window.active, 0);

// 新页面启动只请求一次当前脚本，且不在子框架执行。
let requests = 0;
const fresh = { location: window.location, electronBridge: {},
  __codexSessionDeleteBridge(path) { assert.equal(path, '/user-scripts/load'); requests++; return Promise.resolve({}); } };
fresh.self = fresh.top = fresh;
const startup = vm.createContext({window: fresh, document: {readyState: 'complete'}, console});
vm.runInContext(bundles.bootstrap, startup);
vm.runInContext(bundles.bootstrap, startup);
assert.equal(requests, 1);
fresh.__codexPlusUserScriptsBootstrap = false;
fresh.top = {};
vm.runInContext(bundles.bootstrap, startup);
assert.equal(requests, 1);

// 重复启动请求、桥接恢复及刷新后的新页面均不得再次刷新或叠加旧脚本。
for (let page = 0; page < 3; page++) {
  let pageReloads = 0;
  const scheduled = [];
  const pageWindow = { electronBridge: {}, __codexPlusUserScriptsBootstrap: true,
    location: { href: 'app://-/index.html', reload() { pageReloads++; } },
    setTimeout(fn) { scheduled.push(fn); } };
  pageWindow.top = pageWindow.self = pageWindow;
  const pageContext = vm.createContext({window: pageWindow, console});
  for (let repeat = 0; repeat < 20; repeat++) vm.runInContext(bundles.initial, pageContext);
  assert.equal(pageWindow.legacy, 1);
  assert.equal(scheduled.length, 0);
  assert.equal(JSON.parse(vm.runInContext(bundles.legacy, pageContext)).mode, 'page');
  vm.runInContext(bundles.initial, pageContext); // 刷新在途时仍不追加实例。
  assert.equal(pageWindow.legacy, 1);
  assert.equal(scheduled.length, 1);
  scheduled.shift()();
  assert.equal(pageReloads, 1); // 只有显式重载产生一次刷新。
}

// 即便第一次无脚本，自动初始化也不能随重复请求变成隐式热重载。
const emptyWindow = {};
const emptyContext = vm.createContext({window: emptyWindow, console});
vm.runInContext(bundles.emptyInitial, emptyContext);
vm.runInContext(bundles.initial, emptyContext);
assert.equal(emptyWindow.legacy, undefined);
"#).arg(&fixture).output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_package(root: &std::path::Path, dir: &str, manifest: serde_json::Value) {
    let package = root.join(dir);
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("codexpp.json"), manifest.to_string()).unwrap();
}

#[test]
fn extension_packages_are_scanned_bundled_and_exposed_with_manifest() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("plain.js"), "window.plain = true;").unwrap();
    write_package(
        &user,
        "hello",
        json!({
            "id": "hello", "name": "Hello 拓展", "version": "0.2.0", "description": "示例",
            "engines": {"codexpp": 1},
            "renderer": ["a.js", "lib/b.js"],
            "permissions": {"routes": ["/diagnostics/log"]},
            "future": {"unknown": true}
        }),
    );
    fs::write(user.join("hello/a.js"), "window.order = ['a'];").unwrap();
    fs::create_dir_all(user.join("hello/lib")).unwrap();
    fs::write(user.join("hello/lib/b.js"), "window.order.push('b');").unwrap();
    // 没有清单的目录不是拓展包。
    fs::create_dir_all(user.join("notes")).unwrap();
    fs::write(user.join("notes/readme.js"), "window.notes = true;").unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        &user,
        temp.path().join("config.json"),
    );

    let inventory = manager.inventory().unwrap();
    let scripts = inventory["scripts"].as_array().unwrap();
    assert_eq!(scripts.len(), 2);
    let package = scripts.iter().find(|item| item["key"] == "user:hello").unwrap();
    assert_eq!(package["kind"], "package");
    assert_eq!(package["name"], "Hello 拓展");
    assert_eq!(package["status"], "not_loaded");
    assert_eq!(package["package"]["id"], "hello");
    assert_eq!(package["package"]["version"], "0.2.0");
    assert_eq!(package["package"]["routes"], json!(["/diagnostics/log"]));
    // 市场版本号只给市场脚本用，包的版本走 package.version，避免误判「可更新」。
    assert_eq!(package["version"], "");
    let plain = scripts.iter().find(|item| item["key"] == "user:plain.js").unwrap();
    assert_eq!(plain["kind"], "script");
    assert!(plain["package"].is_null());

    let bundle = manager.build_initial_bundle().unwrap();
    assert!(!bundle.contains("window.notes"));
    let output = Command::new("node")
        .arg("-e")
        .arg(
            r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
const window = { location: { href: 'app://-/index.html' }, electronBridge: {} };
window.top = window.self = window;
const status = JSON.parse(vm.runInContext(process.argv[1], vm.createContext({ window, process, console })));
assert.equal(JSON.stringify(window.order), '["a","b"]'); // vm 里的数组跨 realm，按文本比较
assert.equal(window.plain, true);
const record = status.scripts['user:hello'];
assert.equal(record.status, 'loaded');
assert.equal(record.manifest.id, 'hello');
assert.deepEqual(record.manifest.permissions.routes, ['/diagnostics/log']);
assert.equal(record.manifest.engines.codexpp, 1);
assert.equal(record.manifest.future, undefined, '清单只下发白名单字段');
assert.equal(status.scripts['user:plain.js'].manifest, null);
"#,
        )
        .arg(&bundle)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    manager.set_script_enabled("user:hello", false).unwrap();
    assert!(!manager.build_enabled_bundle().unwrap().contains("window.order"));
    // 包是目录：卸载时整个移进废纸篓（#2919），不做递归删除。用临时废纸篓，别把测试目录挪进真实的 ~/.Trash。
    manager.set_script_enabled("user:hello", true).unwrap();
    let trash = temp.path().join("trash");
    let config = manager.delete_user_script_with_trash("user:hello", &trash).unwrap();
    assert!(!config.scripts.contains_key("user:hello"));
    assert!(!user.join("hello").exists());
    assert!(trash.join("hello/codexpp.json").exists());
}

#[test]
fn invalid_or_incompatible_extension_packages_report_failure_and_are_not_injected() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    let outside = temp.path().join("outside.js");
    fs::create_dir_all(&user).unwrap();
    fs::write(&outside, "window.escaped = true;").unwrap();
    let cases = [
        ("future", json!({"id": "future", "engines": {"codexpp": 99}, "renderer": "a.js"}), "需要拓展 API v99"),
        ("escape", json!({"id": "escape", "renderer": "../../outside.js"}), "包内的相对路径"),
        ("absolute", json!({"id": "absolute", "renderer": outside.to_string_lossy()}), "包内的相对路径"),
        ("missing", json!({"id": "missing", "renderer": "nope.js"}), "不存在"),
        ("noentry", json!({"id": "noentry"}), "renderer 入口"),
        ("badid", json!({"id": "bad id", "renderer": "a.js"}), "id 必填"),
        ("badroutes", json!({"id": "badroutes", "renderer": "a.js", "permissions": {"routes": "/x"}}), "permissions.routes"),
    ];
    for (dir, manifest, _) in &cases {
        write_package(&user, dir, manifest.clone());
        fs::write(user.join(dir).join("a.js"), format!("window.loaded_{dir} = true;")).unwrap();
    }
    fs::create_dir_all(user.join("broken")).unwrap();
    fs::write(user.join("broken/codexpp.json"), "{ not json").unwrap();
    #[cfg(unix)]
    {
        write_package(&user, "symlink", json!({"id": "symlink", "renderer": "link.js"}));
        std::os::unix::fs::symlink(&outside, user.join("symlink/link.js")).unwrap();
    }
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        &user,
        temp.path().join("config.json"),
    );

    let inventory = manager.inventory().unwrap();
    let find = |key: &str| {
        inventory["scripts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .cloned()
            .unwrap_or_else(|| panic!("缺少 {key}"))
    };
    for (dir, _, message) in &cases {
        let item = find(&format!("user:{dir}"));
        assert_eq!(item["status"], "failed", "{dir}");
        assert!(item["error"].as_str().unwrap().contains(message), "{dir}: {}", item["error"]);
    }
    assert!(find("user:broken")["error"].as_str().unwrap().contains("不是合法的 JSON"));
    #[cfg(unix)]
    assert!(find("user:symlink")["error"].as_str().unwrap().contains("包内的文件"));
    // 停用后状态回到 disabled，不再显示清单错误。
    manager.set_script_enabled("user:future", false).unwrap();
    let disabled = manager.inventory().unwrap();
    let future = disabled["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:future").unwrap().clone();
    assert_eq!(future["status"], "disabled");
    assert_eq!(future["error"], "");

    let bundle = manager.build_enabled_bundle().unwrap();
    assert!(!bundle.contains("window.loaded_"));
    assert!(!bundle.contains("window.escaped"));
}
