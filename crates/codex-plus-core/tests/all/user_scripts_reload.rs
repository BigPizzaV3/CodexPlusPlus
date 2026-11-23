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

/// 假的 CDP 页面：对下一条 Runtime.evaluate 回一段运行时状态 JSON。
async fn answer_next_evaluate(listener: tokio::net::TcpListener, status: &'static str) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    let command: serde_json::Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
    assert_eq!(command["method"], "Runtime.evaluate");
    socket.send(Message::Text(json!({
        "id": command["id"],
        "result": { "result": { "type": "string", "value": status } }
    }).to_string().into())).await.unwrap();
}

/// 跳过拓展启动的那一场会话：页面初始化把跳过标记消费掉之后，仍然记着「这一场没有拓展」；热重载把拓展加载回来才清。
#[tokio::test]
async fn a_skipped_start_is_remembered_until_extensions_load_again() {
    use codex_plus_core::user_scripts::{load_scripts_at, reload_scripts_at};
    use tokio::net::TcpListener;

    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("config.json"));
    manager.set_skip_next_start(true).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(answer_next_evaluate(listener, r#"{"mode":"skipped","ran":true,"scripts":{}}"#));
    load_scripts_at(&format!("ws://{address}"), &manager).await.unwrap();
    server.await.unwrap();
    let config = manager.load_config();
    assert!(!config.skip_next_start, "跳过标记只管一次");
    assert!(config.skipped_session, "这一场会话没有拓展");

    // 桥接恢复时页面会再请求一次加载：页面早就初始化过，什么也不会执行，状态不变。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(answer_next_evaluate(listener, r#"{"mode":"scripts","ran":false,"scripts":{}}"#));
    load_scripts_at(&format!("ws://{address}"), &manager).await.unwrap();
    server.await.unwrap();
    assert!(manager.load_config().skipped_session);

    // 热重载退回整页刷新：拓展还没加载，交给新页面的初始化去记。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(answer_next_evaluate(listener, r#"{"mode":"page"}"#));
    reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap();
    server.await.unwrap();
    assert!(manager.load_config().skipped_session);

    // 页内热重载成功：拓展在跑了。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(answer_next_evaluate(listener, r#"{"mode":"scripts","scripts":{}}"#));
    reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap();
    server.await.unwrap();
    assert!(!manager.load_config().skipped_session);
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

/// 所有脚本拼在同一段里求值：一个脚本有语法错误，整段都解析不了。此时逐个编译找出出错的脚本，
/// 换成报错占位后重新执行，其它拓展照常加载，出错的那个显示具体原因。
#[tokio::test]
async fn a_script_with_a_syntax_error_does_not_stop_the_other_scripts_from_loading() {
    use codex_plus_core::user_scripts::load_scripts_at;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("broken.js"), "window.broken = (;").unwrap();
    fs::write(user.join("good.js"), "window.good = true;").unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));

    // 假的 CDP 页面：每条命令一个连接。带着 broken.js 源码的脚本一律报语法错误（整包求值、单独检查都一样）；
    // 单独检查是把脚本包进不调用的函数里求值，没错时得到 undefined。
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let commands = Arc::new(Mutex::new(Vec::<String>::new()));
    let server = tokio::spawn({
        let commands = commands.clone();
        async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                let command: serde_json::Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(command["method"], "Runtime.evaluate", "只用 Runtime.evaluate：compileScript 要先 Runtime.enable");
                let expression = command["params"]["expression"].as_str().unwrap().to_string();
                let result = if expression.contains("window.broken = (;") {
                    json!({ "exceptionDetails": { "text": "Uncaught", "exception": { "className": "SyntaxError", "description": "SyntaxError: Unexpected token ';'\n    at <anonymous>" } } })
                } else if expression.starts_with("void (function(){") {
                    json!({ "result": { "type": "undefined" } })
                } else {
                    json!({ "result": { "type": "string", "value": r#"{"mode":"scripts","ran":true,"scripts":{}}"# } })
                };
                commands.lock().unwrap().push(expression);
                socket.send(Message::Text(json!({ "id": command["id"], "result": result }).to_string().into())).await.unwrap();
            }
        }
    });

    let inventory = load_scripts_at(&format!("ws://{address}"), &manager).await.unwrap();
    server.abort();
    assert_eq!(inventory["reload_mode"], "scripts");

    let commands = commands.lock().unwrap().clone();
    // 整包求值失败 → 两个脚本各检查一次 → 重拼后再求值一次。
    assert_eq!(commands.len(), 4, "{commands:#?}");
    let (original, check_broken, check_good, rebuilt) = (&commands[0], &commands[1], &commands[2], &commands[3]);
    assert!(check_broken.starts_with("void (function(){") && check_broken.contains("window.broken = (;"));
    assert!(check_good.starts_with("void (function(){") && check_good.contains("window.good = true;"));
    assert!(original.contains("window.broken = (;") && original.contains("window.good = true;"));
    assert!(!rebuilt.contains("window.broken = (;"), "出错脚本的源码不再进注入包");
    assert!(rebuilt.contains("window.good = true;"));

    // 用真的 JS 引擎确认：原来的包整段解析不了，重拼的包能跑，且只有出错的脚本显示失败。
    let fixture = temp.path().join("bundles.json");
    fs::write(&fixture, json!({ "original": original, "checkBroken": check_broken, "checkGood": check_good, "rebuilt": rebuilt }).to_string()).unwrap();
    let output = Command::new("node").arg("-e").arg(r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
const bundles = require(process.argv[1]);
assert.throws(() => new vm.Script(bundles.original), SyntaxError);
const window = { location: { href: 'app://-/index.html' }, electronBridge: {} };
window.top = window.self = window;
const context = vm.createContext({ window, process, console });
// 单独检查：有语法错的报 SyntaxError；没错的只是得到 undefined，脚本一行也不执行。
assert.throws(() => vm.runInContext(bundles.checkBroken, context), { name: 'SyntaxError' }); // vm 里抛的错跨 realm，按名字比较
assert.equal(vm.runInContext(bundles.checkGood, context), undefined);
assert.equal(window.good, undefined);
assert.equal(window.__codexPlusUserScripts, undefined);
const status = JSON.parse(vm.runInContext(bundles.rebuilt, context));
assert.equal(status.ran, true);
assert.equal(window.good, true);
assert.equal(status.scripts['user:good.js'].status, 'loaded');
assert.equal(status.scripts['user:broken.js'].status, 'failed');
// 只有这一句：页面内部的调用栈、占位自己的调用栈都不带。
assert.equal(status.scripts['user:broken.js'].error, "脚本有语法错误，没有加载：SyntaxError: Unexpected token ';'");
"#).arg(&fixture).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// 页面抛的不是语法错误（逐个编译都没问题）时不重试：那时脚本可能已经执行了一部分。
#[tokio::test]
async fn a_runtime_exception_is_reported_without_retrying() {
    use codex_plus_core::user_scripts::reload_scripts_at;
    use futures_util::{SinkExt, StreamExt};
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("good.js"), "window.good = true;").unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let methods = Arc::new(Mutex::new(Vec::<String>::new()));
    let server = tokio::spawn({
        let methods = methods.clone();
        async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                let command: serde_json::Value = serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
                let expression = command["params"]["expression"].as_str().unwrap();
                // 单独检查每个脚本都没有语法问题；整包执行时抛的是运行时异常。
                let check = expression.starts_with("void (function(){");
                let result = if check {
                    json!({ "result": { "type": "undefined" } })
                } else {
                    json!({ "exceptionDetails": { "exception": { "description": "Error: 请先重启 Codex++" } } })
                };
                methods.lock().unwrap().push(if check { "check" } else { "bundle" }.to_string());
                socket.send(Message::Text(json!({ "id": command["id"], "result": result }).to_string().into())).await.unwrap();
            }
        }
    });
    let error = reload_scripts_at(&format!("ws://{address}"), &manager).await.unwrap_err();
    server.abort();
    assert!(error.to_string().contains("请先重启 Codex++"), "{error:#}");
    assert_eq!(*methods.lock().unwrap(), ["bundle", "check"], "整包只执行一次，不重试");
}

/// 读改写不能在读不出配置时按默认值写回去：那样停用标记、拓展配置（含密钥）、市场安装记录就全没了。
#[test]
fn an_unreadable_config_is_never_overwritten_with_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    fs::write(user.join("demo.js"), "window.demo = true;").unwrap();

    // 读不出来（这里用目录占住配置路径；实际是文件被占用、权限问题）：报错，什么都不写。
    let blocked = temp.path().join("blocked.json");
    fs::create_dir_all(&blocked).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &blocked);
    let error = manager.set_script_enabled("user:demo.js", false).unwrap_err();
    assert!(format!("{error:#}").contains("这次改动没有保存"), "{error:#}");
    assert!(manager.set_skip_next_start(true).is_err());
    assert!(blocked.is_dir(), "原来的东西还在");

    // 内容坏了：先另存一份再从默认值开始，后续操作不被卡死，原内容找得回来。
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, "{ \"enabled\": false, \"scripts\": { \"user:demo.js\": fal").unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &config_path);
    manager.set_script_enabled("user:demo.js", false).unwrap();
    assert_eq!(manager.load_config().scripts.get("user:demo.js"), Some(&false));
    let backups = fs::read_dir(temp.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("config.json.corrupt-"))
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert!(fs::read_to_string(backups[0].path()).unwrap().contains("\"user:demo.js\": fal"));

    // 文件不存在（第一次用）照常从默认值开始，不产生备份。
    let fresh = temp.path().join("fresh.json");
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &fresh);
    manager.set_script_enabled("user:demo.js", false).unwrap();
    assert!(manager.load_config().enabled);
    assert!(!fs::read_dir(temp.path()).unwrap().flatten().any(|entry| entry.file_name().to_string_lossy().starts_with("fresh.json.corrupt-")));
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
