//! 拓展的常驻后台：清单校验、用真实 Node 跑通的方法调用、事件长轮询与停止。

use codex_plus_core::extension_backend::{call, events, stop, stop_all};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use std::fs;
use std::time::Duration;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: serde_json::Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(dir.join("backend")).unwrap();
    let mut manifest = manifest;
    manifest["id"] = json!(id);
    manifest["name"] = json!(id);
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
    for (path, content) in files {
        fs::write(dir.join(path), content).unwrap();
    }
}

#[test]
fn backend_declarations_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-shape", json!({"backend": "backend/main.mjs"}), &[]);
    write_package(&temp, "missing-script", json!({"backend": {}}), &[]);
    write_package(&temp, "ok", json!({"backend": {"script": "backend/main.mjs"}}), &[("backend/main.mjs", "export default () => ({})")]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-shape")["error"].as_str().unwrap().contains("backend 必须是对象"));
    assert!(find("user:missing-script")["error"].as_str().unwrap().contains("backend 缺少 script"));
    assert_eq!(find("user:ok")["error"], "");
    assert_eq!(find("user:ok")["package"]["backend"], true);
    assert!(manager.enabled_extension_backend("user:ok").is_ok());
    manager.set_script_enabled("user:ok", false).unwrap();
    assert!(manager.enabled_extension_backend("user:ok").unwrap_err().to_string().contains("已停用"));
}

#[tokio::test]
async fn backend_keeps_state_emits_events_and_stops() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    // key 带随机后缀：后台表是进程级的，别和其他用例撞。
    let id = format!("backend-probe-{}", std::process::id());
    let key = format!("user:{id}");
    write_package(&temp, &id, json!({"backend": {"script": "backend/main.mjs"}, "config": {"step": {"type": "number", "default": 2}}}), &[(
        "backend/main.mjs",
        r#"import fs from "node:fs";
export default ({ config, dataDir, emit }) => {
  let count = 0;
  return {
    add() { count += config.step; emit("changed", { count }); return count; },
    where() { fs.writeFileSync("probe.txt", "ok"); return { dataDir, cwd: process.cwd() }; },
    fail() { throw new Error("后台方法出错"); },
    slow() { return new Promise((resolve) => setTimeout(resolve, 5000)); },
  };
};"#,
    )]);

    // 状态跨调用保留，配置默认值传到后台。
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 4);

    // 工作目录是拓展的数据目录。
    let place = call(&manager, &key, "where", json!(null), Duration::from_secs(10), None).await.unwrap();
    let data_dir = temp.path().join("extension-data").join(&id);
    assert!(data_dir.join("probe.txt").exists(), "后台写的相对路径落在数据目录：{place}");

    // 方法出错、没有的方法、超时都报错，且不拖垮后台。
    assert!(call(&manager, &key, "fail", json!(null), Duration::from_secs(10), None).await.unwrap_err().to_string().contains("后台方法出错"));
    assert!(call(&manager, &key, "nope", json!(null), Duration::from_secs(10), None).await.unwrap_err().to_string().contains("没有方法"));
    assert!(call(&manager, &key, "slow", json!(null), Duration::from_millis(200), None).await.unwrap_err().to_string().contains("超时"));
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 6);

    // 事件按序号增量取走。
    let first = events(&key, 0, Duration::from_millis(100)).await;
    let list = first["events"].as_array().unwrap();
    assert_eq!(list.len(), 3, "{first}");
    assert_eq!(list[2]["payload"]["count"], 6);
    let next = first["next"].as_u64().unwrap();
    let empty = events(&key, next, Duration::from_millis(100)).await;
    assert_eq!(empty["events"].as_array().unwrap().len(), 0);
    assert_eq!(empty["running"], true);

    // 长轮询等到新事件立即返回。
    let waiter = tokio::spawn({
        let key = key.clone();
        async move { events(&key, next, Duration::from_secs(10)).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap();
    let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap();
    assert_eq!(woke["events"][0]["payload"]["count"], 8);

    // 停止后状态清零，取事件不会把后台拉起来；再调用会重新启动。
    stop_all().await;
    assert_eq!(events(&key, 0, Duration::from_millis(50)).await["running"], false);
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
    stop_all().await;

    // 实例级停止：最近调用方不是发 stop 的旧实例时不停（热重载时新实例已经拉起了后台）。
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), Some("old")).await.unwrap(), 2);
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), Some("new")).await.unwrap(), 4);
    assert!(!stop(&key, Some("old")).await, "旧实例的 stop 不能停掉新实例在用的后台");
    assert!(stop(&key, Some("new")).await);
    assert!(!stop(&key, None).await, "已经停了");

    // 改了配置（或脚本）后下一次调用按新内容重起。
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
    manager.set_extension_config(&key, json!({"step": 10}).as_object().unwrap()).unwrap();
    assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 10);

    // 停用后调用报错，并把还在跑的后台停掉。
    manager.set_script_enabled(&key, false).unwrap();
    assert!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap_err().to_string().contains("已停用"));
    assert_eq!(events(&key, 0, Duration::from_millis(50)).await["running"], false);
}

#[tokio::test]
async fn backend_startup_failures_are_reported() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let id = format!("backend-broken-{}", std::process::id());
    write_package(&temp, &id, json!({"backend": {"script": "backend/main.mjs"}}), &[("backend/main.mjs", "throw new Error('启动就坏');")]);
    let error = call(&manager, &format!("user:{id}"), "x", json!(null), Duration::from_secs(10), None).await.unwrap_err();
    assert!(error.to_string().contains("启动就坏"), "{error}");
}
