use codex_plus_core::user_scripts::{BOOTSTRAP_SCRIPT, SkipStartClaim, UserScriptManager};
use serde_json::json;
use std::{fs, process::Command};

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
fn skip_start_claim_survives_lost_ack_and_late_ack_cannot_consume_a_new_marker() {
    let temp = tempfile::tempdir().unwrap();
    let manager = || UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("config.json"));
    let first = manager();
    first.set_skip_next_start(true).unwrap();
    let claim = first.claim_skip_start("first-document").unwrap().unwrap();
    let restarted = manager();
    assert_eq!(restarted.claim_skip_start("first-document").unwrap(), Some(claim.clone()));
    assert!(restarted.claim_skip_start("second-document").is_err());
    assert!(restarted.load_config().skip_next_start);

    restarted.set_skip_next_start(true).unwrap();
    assert!(!first.consume_skip_start_claim(&claim).unwrap());
    assert!(restarted.load_config().skip_next_start);
    let new_claim = first.claim_skip_start("second-document").unwrap().unwrap();
    assert_ne!(claim.marker_id, new_claim.marker_id);
    let wrong_document = SkipStartClaim { document_id: "wrong-document".into(), ..new_claim.clone() };
    assert!(!first.consume_skip_start_claim(&wrong_document).unwrap());
    assert!(restarted.consume_skip_start_claim(&new_claim).unwrap());
    assert!(first.consume_skip_start_claim(&new_claim).unwrap());
    assert!(!restarted.load_config().skip_next_start);
    assert_eq!(manager().claim_skip_start("second-document").unwrap(), Some(new_claim));
    assert_eq!(manager().claim_skip_start("third-document").unwrap(), None);
}

#[test]
fn guarded_skip_start_migrates_legacy_marker_and_replays_without_reapplying() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("config.json");
    fs::write(&config, r#"{"skip_next_start":true}"#).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), &config);
    let source = json!({ "ownerEpoch": "owner", "generation": 1, "documentId": "page" });
    let (first, claim) = manager.build_guarded_initial_bundle("page", 1, &source).unwrap();
    let claim = claim.unwrap();
    assert!(manager.load_config().skip_start_marker.is_some());
    let (retry, retry_claim) = manager.build_guarded_initial_bundle("page", 2, &source).unwrap();
    assert_eq!(retry_claim, Some(claim.clone()));
    let output = Command::new("node").arg("-e").arg(r#"
const assert = require('node:assert/strict');
const vm = require('node:vm');
const window = { __codexPlusUserScripts: { documentId: 'page', scripts: {} } };
const context = vm.createContext({ window, console });
assert.equal(JSON.parse(vm.runInContext(process.argv[1], context)).accepted, true);
const source = window.__codexPlusBackendSource;
const retry = JSON.parse(vm.runInContext(process.argv[2], context));
assert.equal(retry.accepted, true);
assert.equal(retry.replayed, true);
assert.equal(window.__codexPlusBackendSource, source);
assert.equal(window.__codexPlusUserScripts.initialMode, 'skipped');
assert.equal(Object.keys(window.__codexPlusUserScripts.scripts).length, 0);
"#).arg(&first).arg(&retry).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(manager.consume_skip_start_claim(&claim).unwrap());
    let (next, skip) = manager.build_guarded_initial_bundle("different-page", 3, &source).unwrap();
    assert!(skip.is_none());
    assert!(next.contains("runtime.initialMode = \"scripts\""));
}

#[test]
fn user_script_config_writer_process() {
    let Some(root) = std::env::var_os("CODEX_PLUS_USER_SCRIPT_LOCK_TEST_DIR") else { return; };
    let writer = std::env::var("CODEX_PLUS_USER_SCRIPT_LOCK_TEST_WRITER").unwrap();
    let root = std::path::PathBuf::from(root);
    fs::write(root.join(format!("ready-{writer}")), b"ready").unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !root.join("start").is_file() {
        assert!(std::time::Instant::now() < deadline, "cross-process writer start timed out");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let manager = UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("config.json"));
    for index in 0..48 {
        manager.set_script_enabled(&format!("user:writer-{writer}-{index}.js"), false).unwrap();
    }
}

#[test]
fn independent_process_config_mutations_preserve_each_others_updates() {
    use std::process::Stdio;
    let temp = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for writer in ["0", "1"] {
        children.push(Command::new(&executable)
            .args(["--exact", "user_scripts_reload::user_script_config_writer_process", "--nocapture"])
            .env("CODEX_PLUS_USER_SCRIPT_LOCK_TEST_DIR", temp.path())
            .env("CODEX_PLUS_USER_SCRIPT_LOCK_TEST_WRITER", writer)
            .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(temp.path().join("ready-0").is_file() && temp.path().join("ready-1").is_file())
        && std::time::Instant::now() < deadline
    { std::thread::sleep(std::time::Duration::from_millis(5)); }
    // 无论就绪检查结果如何都放行并回收子进程。
    fs::write(temp.path().join("start"), b"start").unwrap();
    let results = children.into_iter().map(|child| child.wait_with_output().unwrap()).collect::<Vec<_>>();
    for output in results {
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("config.json"));
    assert_eq!(manager.load_config().scripts.len(), 96);
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
fn strict_reload_snapshot_rejects_invalid_config_and_script_content() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    let config = temp.path().join("config.json");
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &config);
    fs::write(user.join("demo.js"), "window.demo = true;").unwrap();

    let (bundle, revision) = manager.reload_snapshot().unwrap();
    assert_eq!(bundle, manager.build_enabled_bundle().unwrap());
    assert_eq!(revision, manager.reload_snapshot().unwrap().1);
    fs::write(&config, "{").unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(&config, r#"{"scripts":{"user:demo.js":"yes"}}"#).unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(&config, "{}").unwrap();
    fs::write(user.join("demo.js"), [0xff]).unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(&config, r#"{"scripts":{"user:demo.js":false}}"#).unwrap();
    assert!(manager.reload_snapshot().is_ok());
    let unreadable_config = temp.path().join("config-directory");
    fs::create_dir(&unreadable_config).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, unreadable_config);
    assert!(manager.reload_snapshot().is_err());
}

#[test]
fn strict_reload_snapshot_rejects_invalid_enabled_packages_and_partial_resources() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    let config = temp.path().join("config.json");
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, &config);
    write_package(&user, "demo", json!({
        "id": "demo", "renderer": "main.js",
        "backend": { "script": "backend.js", "provides": ["demo.service"] }
    }));
    let package = user.join("demo");
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    fs::write(package.join("backend.js"), "process.stdin.resume();").unwrap();
    let (bundle, _) = manager.reload_snapshot().unwrap();
    assert_eq!(bundle, manager.build_enabled_bundle().unwrap());
    assert!(bundle.contains("demo.service"));

    fs::write(package.join("main.js"), [0xff]).unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    fs::remove_file(package.join("backend.js")).unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(package.join("backend.js"), "process.stdin.resume();").unwrap();
    fs::write(package.join("assets"), "not a directory").unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::remove_file(package.join("assets")).unwrap();
    fs::write(package.join("locale"), "not a directory").unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::remove_file(package.join("locale")).unwrap();
    fs::write(package.join("codexpp.json"), "{").unwrap();
    assert!(manager.reload_snapshot().is_err());
    fs::write(&config, r#"{"scripts":{"user:demo":false}}"#).unwrap();
    assert!(manager.reload_snapshot().is_ok());
    fs::create_dir_all(user.join("bad").join("codexpp.json")).unwrap();
    assert!(manager.reload_snapshot().is_err());
}

#[test]
fn strict_reload_snapshot_tracks_backend_entry_and_matches_initial_revision() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    write_package(&user, "demo", json!({
        "id": "demo", "renderer": "main.js",
        "backend": { "script": "backend.js", "provides": ["demo.service"] }
    }));
    let package = user.join("demo");
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    fs::write(package.join("backend.js"), "module.exports = 1;").unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));
    let (bundle, first) = manager.reload_snapshot().unwrap();
    let source = json!({ "ownerEpoch": "owner", "generation": 1, "documentId": "document" });
    let (initial, skip) = manager.build_guarded_initial_bundle("document", 1, &source).unwrap();
    assert!(skip.is_none());
    assert!(initial.contains(&format!("runtime.appliedRevision = {};", json!(first))));
    fs::write(package.join("backend.js"), "module.exports = 2;").unwrap();
    let (changed_bundle, second) = manager.reload_snapshot().unwrap();
    assert_eq!(bundle, changed_bundle);
    assert_ne!(first, second);
    let (initial, _) = manager.build_guarded_initial_bundle("document", 2, &source).unwrap();
    assert!(initial.contains(&format!("runtime.appliedRevision = {};", json!(second))));
}

#[test]
fn guarded_initial_load_never_falls_back_after_binding_a_strict_backend_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    write_package(&user, "demo", json!({
        "id": "demo", "renderer": "main.js",
        "backend": { "script": "backend.js", "provides": ["demo.service"] }
    }));
    let package = user.join("demo");
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    fs::write(package.join("backend.js"), "module.exports = 1;").unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));
    let (_, revision) = manager.reload_snapshot().unwrap();
    let source = json!({ "ownerEpoch": "owner", "generation": 1, "documentId": "document", "revision": revision });
    assert!(manager.build_guarded_initial_bundle("document", 1, &source).is_ok());
    fs::write(package.join("backend.js"), "module.exports = 2;").unwrap();
    assert!(manager.build_guarded_initial_bundle("document", 2, &source).unwrap_err()
        .to_string().contains("已绑定后台快照不一致"));
    fs::write(package.join("codexpp.json"), "{").unwrap();
    assert!(manager.build_guarded_initial_bundle("document", 3, &source).unwrap_err()
        .to_string().contains("页面来源读取失败"));
    let unbound = json!({ "ownerEpoch": "owner", "generation": 2, "documentId": "document", "revision": null });
    let (diagnostic_bundle, _) = manager.build_guarded_initial_bundle("document", 4, &unbound).unwrap();
    assert!(diagnostic_bundle.contains("runtime.appliedRevision = null;"));
}

#[test]
fn strict_reload_snapshot_tracks_nested_dynamic_backend_dependencies_and_resources() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    write_package(&user, "demo", json!({
        "id": "demo", "renderer": "main.js",
        "backend": { "script": "backend.js", "provides": ["demo.service"] }
    }));
    let package = user.join("demo");
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    fs::write(package.join("backend.js"), "require('./lib/' + process.env.MODE); require('nested');").unwrap();
    fs::create_dir_all(package.join("lib")).unwrap();
    fs::create_dir_all(package.join("node_modules/nested")).unwrap();
    fs::write(package.join("lib/data.js"), "module.exports = 1").unwrap();
    fs::write(package.join("node_modules/nested/index.js"), "module.exports = 2").unwrap();
    fs::write(package.join("data.bin"), [0, 1, 2]).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));
    let (bundle, first) = manager.reload_snapshot().unwrap();
    fs::write(package.join("lib/data.js"), "module.exports = 3").unwrap();
    let (same_bundle, second) = manager.reload_snapshot().unwrap();
    assert_eq!(bundle, same_bundle);
    assert_ne!(first, second);
    fs::write(package.join("node_modules/nested/index.js"), "module.exports = 4").unwrap();
    let (_, third) = manager.reload_snapshot().unwrap();
    assert_ne!(second, third);
    fs::write(package.join("data.bin"), [0, 1, 3]).unwrap();
    let (_, fourth) = manager.reload_snapshot().unwrap();
    assert_ne!(third, fourth);
    fs::write(package.join("lib/new.js"), "module.exports = 5").unwrap();
    assert_ne!(fourth, manager.reload_snapshot().unwrap().1);
}

#[test]
fn strict_reload_snapshot_rejects_unbounded_package_files_instead_of_ignoring_them() {
    let temp = tempfile::tempdir().unwrap();
    let user = temp.path().join("user");
    fs::create_dir_all(&user).unwrap();
    write_package(&user, "demo", json!({ "id": "demo", "renderer": "main.js" }));
    let package = user.join("demo");
    fs::write(package.join("main.js"), "window.demo = true;").unwrap();
    let too_large = fs::File::create(package.join("dependency.bin")).unwrap();
    too_large.set_len(16 * 1024 * 1024 + 1).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), &user, temp.path().join("config.json"));
    assert!(manager.reload_snapshot().unwrap_err().to_string().contains("16 MiB"));
    #[cfg(unix)]
    {
        drop(too_large);
        fs::remove_file(package.join("dependency.bin")).unwrap();
        let outside = temp.path().join("outside.js");
        fs::write(&outside, "module.exports = 'outside';").unwrap();
        std::os::unix::fs::symlink(outside, package.join("outside.js")).unwrap();
        assert!(manager.reload_snapshot().unwrap_err().to_string().contains("超出包目录"));
    }
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
    // 包是目录，删除接口只删单文件，拒绝并保留目录。
    manager.set_script_enabled("user:hello", true).unwrap();
    let error = manager.delete_user_script("user:hello").unwrap_err();
    assert!(error.to_string().contains("拓展包"));
    assert!(user.join("hello/codexpp.json").exists());
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
