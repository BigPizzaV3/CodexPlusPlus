//! 拓展的常驻后台：清单校验、用真实 Node 跑通的方法调用、事件长轮询与停止。

use super::common::with_extension_backend_cleanup;
use codex_plus_core::extension_backend::{call, events, stop, stop_all};
use codex_plus_core::user_scripts::UserScriptManager;
use futures_util::FutureExt;
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
    // key 带随机后缀：日志、排查时好认（后台表按拓展目录隔离，不同目录的同名拓展本就不会互相影响）。
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

    with_extension_backend_cleanup(&[&manager], async {
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
        let first = events(&manager, &key, 0, Duration::from_millis(100)).await;
        let list = first["events"].as_array().unwrap();
        assert_eq!(list.len(), 3, "{first}");
        assert_eq!(list[2]["payload"]["count"], 6);
        let next = first["next"].as_u64().unwrap();
        let empty = events(&manager, &key, next, Duration::from_millis(100)).await;
        assert_eq!(empty["events"].as_array().unwrap().len(), 0);
        assert_eq!(empty["running"], true);

        // 长轮询等到新事件立即返回。
        let waiter = tokio::spawn({
            let key = key.clone();
            let manager = manager.clone();
            async move { events(&manager, &key, next, Duration::from_secs(10)).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap();
        let woke = tokio::time::timeout(Duration::from_secs(5), waiter).await.unwrap().unwrap();
        assert_eq!(woke["events"][0]["payload"]["count"], 8);

        // 停止后状态清零，取事件不会把后台拉起来；再调用会重新启动。
        stop_all(&manager).await;
        assert_eq!(events(&manager, &key, 0, Duration::from_millis(50)).await["running"], false);
        assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
        stop_all(&manager).await;

        // 实例级停止：最近调用方不是发 stop 的旧实例时不停（热重载时新实例已经拉起了后台）。
        assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), Some("old")).await.unwrap(), 2);
        assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), Some("new")).await.unwrap(), 4);
        assert!(!stop(&manager, &key, Some("old")).await, "旧实例的 stop 不能停掉新实例在用的后台");
        assert!(stop(&manager, &key, Some("new")).await);
        assert!(!stop(&manager, &key, None).await, "已经停了");

        // 改了配置（或脚本）后下一次调用按新内容重起。
        assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
        manager.set_extension_config(&key, json!({"step": 10}).as_object().unwrap()).unwrap();
        assert_eq!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 10);

        // 停用后调用报错，并把还在跑的后台停掉。
        manager.set_script_enabled(&key, false).unwrap();
        assert!(call(&manager, &key, "add", json!(null), Duration::from_secs(10), None).await.unwrap_err().to_string().contains("已停用"));
        assert_eq!(events(&manager, &key, 0, Duration::from_millis(50)).await["running"], false);
    }).await;
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
    with_extension_backend_cleanup(&[&manager], async {
        let error = call(&manager, &format!("user:{id}"), "x", json!(null), Duration::from_secs(10), None).await.unwrap_err();
        assert!(error.to_string().contains("启动就坏"), "{error}");
    }).await;
}

/// 后台按拓展目录隔离：两个目录里同名的拓展各有各的后台，停一个目录的全部后台不波及另一个目录
/// （合并测试程序里各用例并发跑，一个用例的 stop_all 曾把别的用例正在用的后台杀掉）。
#[tokio::test]
async fn backends_are_scoped_to_their_extension_directory() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let script = [("backend/main.mjs", "export default () => { let count = 0; return { add() { count += 1; return count; } }; };")];
    let (first_dir, second_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (first, second) = (manager(&first_dir), manager(&second_dir));
    for temp in [&first_dir, &second_dir] {
        write_package(temp, "same-name", json!({"backend": {"script": "backend/main.mjs"}}), &script);
    }
    let key = "user:same-name";
    async fn add(manager: &UserScriptManager) -> anyhow::Result<serde_json::Value> {
        call(manager, "user:same-name", "add", json!(null), Duration::from_secs(10), None).await
    }
    with_extension_backend_cleanup(&[&first, &second], async {
        assert_eq!(add(&first).await.unwrap(), 1);
        assert_eq!(add(&second).await.unwrap(), 1, "另一个目录的同名拓展不共用后台");
        assert_eq!(add(&first).await.unwrap(), 2, "也不因指纹不同把对方换掉");

        stop_all(&second).await;
        assert_eq!(events(&second, key, 0, Duration::from_millis(50)).await["running"], false);
        assert_eq!(events(&first, key, 0, Duration::from_millis(50)).await["running"], true);
        assert_eq!(add(&first).await.unwrap(), 3, "停别的目录不影响这个目录在跑的后台");
        assert!(!stop(&second, key, None).await);
        assert!(stop(&first, key, None).await);
    }).await;
}

#[tokio::test]
async fn backend_cleanup_after_panic_preserves_other_directories() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let (first_dir, second_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (first, second) = (manager(&first_dir), manager(&second_dir));
    for temp in [&first_dir, &second_dir] {
        write_package(temp, "cleanup-probe", json!({"backend": {"script": "backend/main.mjs"}}), &[(
            "backend/main.mjs",
            "export default () => { let count = 0; return { add() { return ++count; } }; };",
        )]);
    }
    let key = "user:cleanup-probe";
    with_extension_backend_cleanup(&[&first, &second], async {
        assert_eq!(call(&second, key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 1);
        let failure = std::panic::AssertUnwindSafe(with_extension_backend_cleanup(&[&first], async {
            assert_eq!(call(&first, key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 1);
            panic!("intentional backend cleanup failure");
        })).catch_unwind().await.expect_err("用例的原始 panic 必须继续向外抛出");
        assert_eq!(failure.downcast_ref::<&str>(), Some(&"intentional backend cleanup failure"));
        assert_eq!(events(&first, key, 0, Duration::ZERO).await["running"], false, "断言失败后撤下后台槽位");
        assert_eq!(events(&second, key, 0, Duration::ZERO).await["running"], true, "清理不影响另一个目录");
        assert_eq!(call(&second, key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 2);
        assert_eq!(call(&first, key, "add", json!(null), Duration::from_secs(10), None).await.unwrap(), 1, "原目录重新调用时启动全新后台");
    }).await;
    assert_eq!(events(&first, key, 0, Duration::ZERO).await["running"], false, "正常返回也清理后台");
    assert_eq!(events(&second, key, 0, Duration::ZERO).await["running"], false);
}

#[test]
fn backend_cleanup_after_panic_allows_runtime_to_drop() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    enum RuntimeEvent {
        Dropping,
        Completed(std::thread::Result<()>),
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(|| {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let temp = tempfile::tempdir().unwrap();
            let manager = manager(&temp);
            write_package(&temp, "runtime-cleanup", json!({"backend": {"script": "backend/main.mjs"}}), &[(
                "backend/main.mjs",
                "export default () => ({ ready() { return true; } });",
            )]);
            runtime.block_on(async {
                let failure = std::panic::AssertUnwindSafe(with_extension_backend_cleanup(&[&manager], async {
                    assert_eq!(call(&manager, "user:runtime-cleanup", "ready", json!(null), Duration::from_secs(10), None).await.unwrap(), true);
                    panic!("intentional runtime cleanup failure");
                })).catch_unwind().await.expect_err("测试必须走 panic 清理路径");
                assert_eq!(failure.downcast_ref::<&str>(), Some(&"intentional runtime cleanup failure"));
            });
            // 只给运行时析构计时，不把 Node 冷启动算进 10 秒预算。
            sender.send(RuntimeEvent::Dropping).unwrap();
            drop(runtime);
        });
        let _ = sender.send(RuntimeEvent::Completed(outcome));
    });
    let first = receiver.recv_timeout(Duration::from_secs(30)).expect("后台清理用例未能结束");
    let outcome = match first {
        RuntimeEvent::Dropping => match receiver.recv_timeout(Duration::from_secs(10)).expect("清理后运行时仍卡在子进程管道") {
            RuntimeEvent::Completed(outcome) => outcome,
            RuntimeEvent::Dropping => panic!("运行时析构信号重复"),
        },
        RuntimeEvent::Completed(outcome) => outcome,
    };
    worker.join().unwrap();
    if let Err(failure) = outcome {
        std::panic::resume_unwind(failure);
    }
}

#[tokio::test]
async fn backend_can_query_sessions_through_the_host() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let id = format!("backend-host-{}", std::process::id());
    write_package(&temp, &id, json!({"backend": {"script": "backend/main.mjs"}}), &[(
        "backend/main.mjs",
        r#"export default ({ sessionQuery }) => ({
  async title(id) { return await sessionQuery.readTitle(id).then((value) => "OK:" + value, (error) => "ERR:" + error.message); },
  async bad() { return await sessionQuery.deleteSession?.("x") ?? "没有写方法"; },
});"#,
    )]);
    let key = format!("user:{id}");
    with_extension_backend_cleanup(&[&manager], async {
        // 宿主调用的错误原样回到后台（不存在的会话，或本机没有 Codex 数据库）。
        let reply = call(&manager, &key, "title", json!("no-such-session-0000"), Duration::from_secs(10), None).await.unwrap();
        let reply = reply.as_str().unwrap();
        assert!(reply.starts_with("ERR:") && (reply.contains("没有找到会话") || reply.contains("会话数据库")), "{reply}");
        assert_eq!(call(&manager, &key, "bad", json!(null), Duration::from_secs(10), None).await.unwrap(), "没有写方法", "只读：没有写方法");
        stop(&manager, &key, None).await;
    }).await;
}

#[tokio::test]
async fn backends_and_tools_call_services_provided_by_other_backends() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let suffix = std::process::id();
    let provider = format!("svc-provider-{suffix}");
    let consumer = format!("svc-consumer-{suffix}");
    write_package(&temp, &provider, json!({"backend": {"script": "backend/main.mjs", "provides": ["counter"]}}), &[(
        "backend/main.mjs",
        r#"export default ({ provide }) => {
  let total = 0;
  provide("counter", { add(n) { total += n; return total; }, boom() { throw new Error("计数器坏了"); } });
  return { total: () => total };
};"#,
    )]);
    write_package(&temp, &consumer, json!({"backend": {"script": "backend/main.mjs"}}), &[(
        "backend/main.mjs",
        r#"export default ({ service }) => ({
  async bump(n) { return await service("counter").add(n); },
  async broken() { return await service("counter").boom().catch((error) => "ERR:" + error.message); },
  async missing() { return await service("nobody").x().catch((error) => "ERR:" + error.message); },
});"#,
    )]);
    let inventory = manager.inventory().unwrap();
    let provider_summary = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == format!("user:{provider}")).unwrap().clone();
    assert_eq!(provider_summary["package"]["services"], json!(["counter"]));
    assert_eq!(manager.enabled_backend_services().unwrap(), vec![("counter".to_string(), format!("user:{provider}"))]);
    with_extension_backend_cleanup(&[&manager], async {
        // 使用方后台经宿主调用提供方后台：提供方按需拉起，状态在提供方里累加。
        let consumer_key = format!("user:{consumer}");
        assert_eq!(call(&manager, &consumer_key, "bump", json!(2), Duration::from_secs(10), None).await.unwrap(), 2);
        assert_eq!(call(&manager, &consumer_key, "bump", json!(3), Duration::from_secs(10), None).await.unwrap(), 5);
        assert_eq!(call(&manager, &format!("user:{provider}"), "total", json!(null), Duration::from_secs(10), None).await.unwrap(), 5, "同一个提供方实例");
        assert!(call(&manager, &consumer_key, "broken", json!(null), Duration::from_secs(10), None).await.unwrap().as_str().unwrap().contains("计数器坏了"));
        assert!(call(&manager, &consumer_key, "missing", json!(null), Duration::from_secs(10), None).await.unwrap().as_str().unwrap().contains("没有拓展提供服务 nobody"));
        // 页面经桥接路由调用同一个服务。
        let direct = codex_plus_core::extension_backend::call_service(&manager, &json!({"name": "counter", "method": "add", "args": [10]})).await.unwrap();
        assert_eq!(direct, 15);
        stop(&manager, &consumer_key, None).await;
        stop(&manager, &format!("user:{provider}"), None).await;
    }).await;
}

#[tokio::test]
async fn a_backend_can_call_another_backends_service_while_starting() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let suffix = std::process::id();
    let provider = format!("init-provider-{suffix}");
    let consumer = format!("init-consumer-{suffix}");
    write_package(&temp, &provider, json!({"backend": {"script": "backend/main.mjs", "provides": [format!("greeter{suffix}")]}}), &[(
        "backend/main.mjs",
        &format!(r#"export default ({{ provide }}) => {{ provide("greeter{suffix}", {{ hello: (name) => "你好 " + name }}); return {{}}; }};"#),
    )]);
    // 使用方在默认导出里就等着调用提供方的服务：启动期间不能和全局锁互相等待。
    write_package(&temp, &consumer, json!({"backend": {"script": "backend/main.mjs"}}), &[(
        "backend/main.mjs",
        &format!(r#"export default async ({{ service }}) => {{ const greeting = await service("greeter{suffix}").hello("拓展"); return {{ greeting: () => greeting }}; }};"#),
    )]);
    with_extension_backend_cleanup(&[&manager], async {
        let started = std::time::Instant::now();
        let greeting = call(&manager, &format!("user:{consumer}"), "greeting", json!(null), Duration::from_secs(20), None).await.unwrap();
        assert_eq!(greeting, "你好 拓展");
        assert!(started.elapsed() < Duration::from_secs(10), "不应等到启动超时：{:?}", started.elapsed());
        stop(&manager, &format!("user:{consumer}"), None).await;
        stop(&manager, &format!("user:{provider}"), None).await;
    }).await;
}

#[tokio::test]
async fn http_routes_and_schedules_reach_the_backend() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    use codex_plus_core::extension_backend::route_http_with;
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let id = format!("http-probe-{}", std::process::id());
    let key = format!("user:{id}");
    write_package(&temp, &id, json!({
        "backend": {"script": "backend/main.mjs", "http": true},
        "schedules": [{"method": "tick", "everyMinutes": 1}, {"method": "missing", "at": "00:00"}]
    }), &[("backend/main.mjs", r#"
let ticks = 0;
export default () => ({
  http(request) {
    if (request.path === "/text") return `hi ${request.query}`;
    if (request.path === "/json") return { status: 201, json: { got: request.json, method: request.method } };
    return { status: 404, body: "nope" };
  },
  tick() { ticks += 1; },
  ticks() { return ticks; },
});"#)]);
    with_extension_backend_cleanup(&[&manager], async {
        let headers = "POST /ext/x HTTP/1.1\r\nHost: 127.0.0.1:57321\r\nContent-Type: application/json\r\n";
        let path = format!("/ext/{id}/text?a=1");
        for (headers, reason) in [
            ("GET /ext/x HTTP/1.1\r\n", "缺少 Host 的请求"),
            ("GET /ext/x HTTP/1.1\r\nHost: evil.example:57321\r\n", "DNS 重绑定的非本机 Host"),
            ("GET /ext/x HTTP/1.1\r\nHost: 127.0.0.1:57321\r\nOrigin: https://evil.example\r\n", "带 Origin 的网页"),
            ("GET /ext/x HTTP/1.1\r\nHost: 127.0.0.1:57321\r\nSec-Fetch-Site: cross-site\r\n", "不带 Origin 的跨站资源请求"),
        ] {
            assert_eq!(route_http_with(&manager, "GET", &path, headers, b"").await.0, 403, "{reason} 不得调用拓展");
        }
        let same_origin = "GET /ext/x HTTP/1.1\r\nHost: localhost:57321\r\nSec-Fetch-Site: same-origin\r\n";
        assert_eq!(route_http_with(&manager, "GET", &path, same_origin, b"").await.0, 200, "同源页面的 GET 仍可用");
        let navigation = "GET /ext/x HTTP/1.1\r\nHost: [::1]:57321\r\nSec-Fetch-Site: none\r\n";
        assert_eq!(route_http_with(&manager, "GET", &path, navigation, b"").await.0, 200, "直接打开本机页面仍可用");
        let (status, kind, body) = route_http_with(&manager, "GET", &format!("/ext/{id}/text?a=1"), headers, b"").await;
        assert_eq!((status, kind.as_str(), body.as_slice()), (200, "text/plain; charset=utf-8", b"hi a=1".as_slice()));
        let (status, _, body) = route_http_with(&manager, "POST", &format!("/ext/{id}/json"), headers, br#"{"x":1}"#).await;
        assert_eq!(status, 201);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), json!({"got": {"x": 1}, "method": "POST"}));
        assert_eq!(route_http_with(&manager, "GET", &format!("/ext/{id}/other"), headers, b"").await.0, 404);
        assert_eq!(route_http_with(&manager, "GET", "/ext/not-there/x", headers, b"").await.0, 404, "没声明 http 的拓展不开放");

        // 定时：第一次 tick 只记起点；把上次运行时间拨回 2 分钟前，再 tick 就会调用。
        let schedules = json!({"schedules": [{"method": "tick", "everyMinutes": 1}]});
        call(&manager, &key, "__tick", schedules.clone(), Duration::from_secs(20), None).await.unwrap();
        assert_eq!(call(&manager, &key, "ticks", json!(null), Duration::from_secs(20), None).await.unwrap(), 0);
        let state_path = temp.path().join("extension-data").join(&id).join(".schedules.json");
        let state: serde_json::Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
        let (state_key, last) = state.as_object().unwrap().iter().next().map(|(key, value)| (key.clone(), value.as_u64().unwrap())).unwrap();
        fs::write(&state_path, json!({ state_key: last - 120_000 }).to_string()).unwrap();
        let ran = call(&manager, &key, "__tick", schedules, Duration::from_secs(20), None).await.unwrap();
        assert_eq!(ran["ran"], json!([{"method": "tick"}]));
        assert_eq!(call(&manager, &key, "ticks", json!(null), Duration::from_secs(20), None).await.unwrap(), 1);
        assert_eq!(manager.enabled_extension_schedules().unwrap().len(), 2);
        stop(&manager, &key, None).await;
    }).await;
}

#[test]
fn schedules_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "no-backend", json!({"renderer": "main.js", "schedules": [{"method": "a", "everyMinutes": 5}]}), &[("main.js", "")]);
    write_package(&temp, "both", json!({"backend": {"script": "backend/main.mjs"}, "schedules": [{"method": "a", "everyMinutes": 5, "at": "09:00"}]}), &[("backend/main.mjs", "")]);
    write_package(&temp, "bad-time", json!({"backend": {"script": "backend/main.mjs"}, "schedules": [{"method": "a", "at": "25:00"}]}), &[("backend/main.mjs", "")]);
    let inventory = manager.inventory().unwrap();
    let error = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap()["error"].as_str().unwrap().to_string();
    assert!(error("user:no-backend").contains("需要同时声明 backend"));
    assert!(error("user:both").contains("二选一"));
    assert!(error("user:bad-time").contains("HH:MM"));
}
