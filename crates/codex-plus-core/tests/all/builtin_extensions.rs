//! 内置拓展：Codex++ 自带功能挂在拓展内核上，可在配置里单独停用，也可被拓展包的 `replaces` 顶替。

use std::fs;

use codex_plus_core::user_scripts::{BUILTIN_EXTENSIONS, UserScriptManager};
use serde_json::json;

fn manager(root: &std::path::Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("user_scripts.json"))
}

fn write_package(root: &std::path::Path, id: &str, manifest: serde_json::Value) {
    let dir = root.join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("renderer.js"), "codexPlus.extension(() => {});").unwrap();
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
}

#[test]
fn builtins_are_listed_enabled_by_default_and_can_be_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    let builtins = manager.builtin_extensions().unwrap();
    assert_eq!(builtins.as_array().unwrap().len(), BUILTIN_EXTENSIONS.len());
    assert!(builtins.as_array().unwrap().iter().all(|item| item["enabled"] == true));

    manager.set_script_enabled("builtin:agent-pet", false).unwrap();
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["disabled"], json!(["agent-pet"]));
    let pet = manager.builtin_extensions().unwrap().as_array().unwrap().iter().find(|item| item["id"] == "agent-pet").cloned().unwrap();
    assert_eq!(pet["enabled"], false);

    // 状态总随注入包下发，用户脚本总开关关着也一样（页面据此卸下内置功能）。
    manager.set_global_enabled(false).unwrap();
    let bundle = manager.build_enabled_bundle().unwrap();
    assert!(bundle.contains("window.__codexPlusBuiltinExtensionState"), "{bundle}");
    assert!(bundle.contains("\"agent-pet\""));
}

#[test]
fn enabled_package_with_replaces_takes_over_a_builtin() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    write_package(temp.path(), "my-pet", json!({ "id": "my-pet", "renderer": "renderer.js", "replaces": ["builtin:agent-pet", "usage-statusline"] }));
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["replacedBy"]["agent-pet"], "user:my-pet");
    assert_eq!(state["replacedBy"]["usage-statusline"], "user:my-pet");
    let inventory = manager.inventory().unwrap();
    let package = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:my-pet").cloned().unwrap();
    assert_eq!(package["package"]["replaces"], json!(["agent-pet", "usage-statusline"]));

    // 顶替者停用后内置的回来。
    manager.set_script_enabled("user:my-pet", false).unwrap();
    assert_eq!(manager.builtin_extension_state().unwrap()["replacedBy"], json!({}));
}

#[test]
fn replaces_must_name_a_known_builtin() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    write_package(temp.path(), "bad", json!({ "id": "bad", "renderer": "renderer.js", "replaces": ["no-such-thing"] }));
    let inventory = manager.inventory().unwrap();
    let package = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:bad").cloned().unwrap();
    assert!(package["error"].as_str().unwrap().contains("不是内置拓展"), "{package}");
    assert_eq!(manager.builtin_extension_state().unwrap()["replacedBy"], json!({}));
}

#[test]
fn builtin_extension_active_follows_disable_and_replaces() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    assert!(manager.builtin_extension_active("pet-real-mouse"));
    manager.set_script_enabled("builtin:pet-real-mouse", false).unwrap();
    assert!(!manager.builtin_extension_active("pet-real-mouse"));
    assert!(manager.builtin_extension_active("paste-fix"));
    write_package(temp.path(), "my-paste", json!({ "id": "my-paste", "renderer": "renderer.js", "replaces": ["paste-fix"] }));
    assert!(!manager.builtin_extension_active("paste-fix"));
}

/// 内置拓展的 id 在 Rust 清单、页面清单、管理器文案三处要一致。
#[test]
fn builtin_extension_ids_match_page_catalog_and_manager_text() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = fs::read_to_string(root.join("assets/inject/renderer-inject/96-builtin-extensions.js")).unwrap();
    let manager_text = fs::read_to_string(root.join("apps/codex-plus-manager/src/App.tsx")).unwrap();
    for (id, _, _) in BUILTIN_EXTENSIONS {
        assert!(catalog.contains(&format!("id: \"{id}\"")), "页面内置拓展清单缺 {id}");
        assert!(manager_text.contains(&format!("case \"{id}\": return {{ name: t(")), "管理器内置拓展文案缺 {id}");
    }
}
