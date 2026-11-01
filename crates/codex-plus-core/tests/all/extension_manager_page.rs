//! 拓展的管理器面板（codexpp.json 的 `manager.page`）：清单校验、读页面、读资源。

use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{json, Value};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: Value, files: &[(&str, &[u8])]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
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
fn manager_page_alone_is_a_valid_package_and_is_served_with_its_assets() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(
        &temp,
        "panel",
        json!({"id": "panel", "name": "面板拓展", "manager": {"page": "ui/panel.html", "title": "我的面板"}}),
        &[("ui/panel.html", b"<html><head></head><body>hi</body></html>"), ("assets/logo.png", b"\x89PNG")],
    );
    let item = script(&manager, "user:panel");
    assert_eq!(item["error"], "", "只声明 manager 也是合法的拓展包");
    assert_eq!(item["package"]["managerPage"]["title"], "我的面板");
    assert_eq!(item["package"]["managerPage"]["nav"], Value::Null, "没写 nav 时不进侧栏");
    let (title, html) = manager.extension_manager_page("user:panel").unwrap();
    assert_eq!(title, "我的面板");
    assert!(html.contains("<body>hi</body>"));
    // 写不写 assets/ 前缀都认；包外、不存在的资源都拿不到。
    let logo = manager.extension_asset("user:panel", "assets/logo.png").unwrap();
    assert!(logo.starts_with("data:image/png;base64,"), "{logo}");
    assert_eq!(manager.extension_asset("user:panel", "logo.png").unwrap(), logo);
    assert!(manager.extension_asset("user:panel", "../codexpp.json").is_err());
    assert!(manager.extension_asset("user:panel", "missing.png").is_err());
}

#[test]
fn manager_page_title_defaults_to_the_package_name() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "named", json!({"id": "named", "name": "带名字", "renderer": "main.js", "manager": {"page": "panel.html"}}), &[("main.js", b""), ("panel.html", b"<p>x</p>")]);
    assert_eq!(script(&manager, "user:named")["package"]["managerPage"]["title"], "带名字");
}

#[test]
fn invalid_manager_pages_are_reported_on_the_package() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    fs::create_dir_all(temp.path().join("user")).unwrap();
    fs::write(temp.path().join("user").join("outside.html"), "<p>outside</p>").unwrap();
    let big = vec![b'a'; 1024 * 1024 + 1];
    let cases: Vec<(&str, Value, Vec<(&str, &[u8])>, &str)> = vec![
        ("missing", json!({"page": "nope.html"}), vec![], "不存在"),
        ("outside", json!({"page": "../outside.html"}), vec![], "必须在拓展包目录内"),
        ("script", json!({"page": "panel.js"}), vec![("panel.js", b"")], "必须是 .html 文件"),
        ("big", json!({"page": "panel.html"}), vec![("panel.html", big.as_slice())], "超过 1 MiB"),
        ("nopage", json!({"title": "x"}), vec![], "manager.page 必填"),
        ("notobject", json!("panel.html"), vec![], "manager 必须是对象"),
    ];
    for (id, manager_field, files, expected) in cases {
        write_package(&temp, id, json!({"id": id, "renderer": "main.js", "manager": manager_field}), &[&[("main.js", b"" as &[u8])][..], &files[..]].concat());
        let error = script(&manager, &format!("user:{id}"))["error"].as_str().unwrap().to_string();
        assert!(error.contains(expected), "{id}: {error}");
        assert!(manager.extension_manager_page(&format!("user:{id}")).is_err());
    }
}

#[test]
fn disabled_extensions_do_not_serve_their_manager_page() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "off", json!({"id": "off", "manager": {"page": "panel.html"}}), &[("panel.html", b"<p>x</p>"), ("assets/a.png", b"\x89PNG")]);
    manager.set_script_enabled("user:off", false).unwrap();
    assert!(manager.extension_manager_page("user:off").unwrap_err().to_string().contains("已停用"));
    assert!(manager.extension_asset("user:off", "a.png").is_err());
}

/// 拓展页面可以进侧栏（nav）、顶替管理器的内置页面（replaces）、声明能读的只读数据（data）；名字都要认得。
#[test]
fn manager_pages_declare_nav_replacements_and_data() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(
        &temp,
        "usage",
        json!({ "id": "usage", "name": "用量看板", "manager": { "page": "page.html", "nav": "management", "icon": "bar-chart", "replaces": ["gatewayUsage", "requestLog"], "data": ["requestLogs", "gatewayUsage"] } }),
        &[("page.html", b"<p>hi</p>")],
    );
    write_package(&temp, "nav-true", json!({ "id": "nav-true", "manager": { "page": "page.html", "nav": true } }), &[("page.html", b"<p>hi</p>")]);
    let page = &script(&manager, "user:usage")["package"]["managerPage"];
    assert_eq!(page["nav"], "management");
    assert_eq!(page["icon"], "bar-chart");
    assert_eq!(page["replaces"], json!(["gatewayUsage", "requestLog"]));
    assert_eq!(page["data"], json!(["requestLogs", "gatewayUsage"]));
    assert_eq!(script(&manager, "user:nav-true")["package"]["managerPage"]["nav"], "management");

    // 「万物皆拓展」「设置」这类是停用拓展的入口，不能顶替；不认识的数据名、分组也报错。
    for (id, manager_field, needle) in [
        ("no-self", json!({ "page": "page.html", "replaces": ["userScripts"] }), "userScripts"),
        ("no-settings", json!({ "page": "page.html", "replaces": ["settings"] }), "settings"),
        ("bad-data", json!({ "page": "page.html", "data": ["relayKeys"] }), "relayKeys"),
        ("bad-nav", json!({ "page": "page.html", "nav": "sidebar" }), "manager.nav"),
    ] {
        write_package(&temp, id, json!({ "id": id, "manager": manager_field }), &[("page.html", b"<p>hi</p>")]);
        let error = script(&manager, &format!("user:{id}"))["error"].as_str().unwrap().to_string();
        assert!(error.contains(needle), "{id}: {error}");
    }
}
