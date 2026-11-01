//! 拓展的 README（Codex「拓展」详情页的「细节」页签）：找文件、中文优先、截断、正文引用到的包内资源；以及清单里的图标。

use codex_plus_core::bundled_extensions::{AGENT_PET_KEY, materialize};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{json, Value};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, files: &[(&str, &[u8])]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "renderer": "main.js"}).to_string()).unwrap();
    fs::write(dir.join("main.js"), "").unwrap();
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
fn readme_prefers_chinese_and_returns_referenced_assets() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(
        &temp,
        "notes",
        &[
            ("readme.md", b"# Notes\n\nEnglish"),
            ("README.zh-CN.md", "# 便签\n\n![截图](./assets/shot.png)".as_bytes()),
            ("assets/shot.png", b"\x89PNG"),
            ("assets/unused.png", b"\x89PNG"),
        ],
    );
    let item = script(&manager, "user:notes");
    assert_eq!(item["package"]["readme"], true);
    assert_eq!(item["package"]["renderer"], 1, "「功能」页签要显示页面脚本数");
    let readme = manager.extension_readme("user:notes").unwrap();
    assert_eq!(readme["status"], "ok");
    assert_eq!(readme["file"], "README.zh-CN.md", "中文版优先");
    assert!(readme["markdown"].as_str().unwrap().starts_with("# 便签"));
    assert_eq!(readme["truncated"], false);
    let assets = readme["assets"].as_object().unwrap();
    assert!(assets["assets/shot.png"].as_str().unwrap().starts_with("data:image/png;base64,"));
    assert!(!assets.contains_key("assets/unused.png"), "只给正文引用到的资源");
}

#[test]
fn packages_without_readme_and_single_file_scripts_return_empty_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "plain", &[]);
    fs::write(temp.path().join("user/solo.js"), "console.log(1)").unwrap();
    assert_eq!(script(&manager, "user:plain")["package"]["readme"], false);
    for key in ["user:plain", "user:solo.js"] {
        let readme = manager.extension_readme(key).unwrap();
        assert_eq!(readme["markdown"], "", "{key}");
    }
    assert!(manager.extension_readme("user:missing").is_err());
}

#[test]
fn oversized_readme_is_truncated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let big = "a".repeat(600 * 1024);
    write_package(&temp, "big", &[("README.md", big.as_bytes())]);
    let readme = manager.extension_readme("user:big").unwrap();
    assert_eq!(readme["truncated"], true);
    assert_eq!(readme["markdown"].as_str().unwrap().len(), 512 * 1024);
}

#[test]
fn bundled_agent_pet_ships_a_readme() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
        .with_bundled_dir(temp.path().join("bundled"));
    assert_eq!(script(&manager, AGENT_PET_KEY)["package"]["readme"], true);
    let readme = manager.extension_readme(AGENT_PET_KEY).unwrap();
    assert!(readme["markdown"].as_str().unwrap().starts_with("# Agent 桌宠"));
}

#[test]
fn package_icon_resolves_to_an_asset_data_url_or_https_and_ignores_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let icon = |id: &str, value: Value, files: &[(&str, &[u8])]| {
        let dir = temp.path().join("user").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("codexpp.json"), json!({"id": id, "renderer": "main.js", "icon": value}).to_string()).unwrap();
        fs::write(dir.join("main.js"), "").unwrap();
        for (name, content) in files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        script(&manager, &format!("user:{id}"))["package"]["icon"].clone()
    };
    let svg: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
    assert!(icon("a", json!("assets/icon.svg"), &[("assets/icon.svg", svg)]).as_str().unwrap().starts_with("data:image/svg+xml;base64,"));
    assert!(icon("b", json!("./icon.svg"), &[("assets/icon.svg", svg)]).as_str().unwrap().starts_with("data:image/svg+xml;base64,"), "不写 assets/ 前缀也认");
    assert_eq!(icon("c", json!("https://cdn.test/i.png"), &[]), "https://cdn.test/i.png");
    assert_eq!(icon("d", json!("http://cdn.test/i.png"), &[]), Value::Null, "只认 https");
    assert_eq!(icon("e", json!("assets/missing.png"), &[]), Value::Null);
    assert_eq!(icon("f", json!("assets/click.mp3"), &[("assets/click.mp3", b"ID3")]), Value::Null, "不是图片不当图标");
    assert_eq!(icon("g", json!(""), &[]), Value::Null);
}

#[test]
fn bundled_agent_pet_has_an_icon() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
        .with_bundled_dir(temp.path().join("bundled"));
    assert!(script(&manager, AGENT_PET_KEY)["package"]["icon"].as_str().unwrap().starts_with("data:image/svg+xml;base64,"));
}
