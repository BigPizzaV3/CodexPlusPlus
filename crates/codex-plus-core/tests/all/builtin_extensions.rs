//! 内置拓展：Codex++ 自带功能挂在拓展内核上，可单独停用，也可被拓展包的 `replaces` 顶替。
//! 有设置字段的内置拓展，开关就是那个设置项；没有的记在用户脚本配置里。

use std::fs;

use codex_plus_core::settings::SettingsStore;
use codex_plus_core::settings_catalog::builtin_extension_defs;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};

fn manager(root: &std::path::Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("user_scripts.json"))
        .with_settings_path(root.join("settings.json"))
}

fn builtin(manager: &UserScriptManager, id: &str) -> Value {
    manager.builtin_extensions().unwrap().as_array().unwrap().iter().find(|item| item["id"] == id).cloned().unwrap()
}

fn settings(root: &std::path::Path) -> Value {
    serde_json::to_value(SettingsStore::new(root.join("settings.json")).load().unwrap()).unwrap()
}

fn write_package(root: &std::path::Path, id: &str, manifest: serde_json::Value) {
    let dir = root.join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("renderer.js"), "codexPlus.extension(() => {});").unwrap();
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
}

#[test]
fn builtins_without_a_setting_are_enabled_by_default_and_can_be_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    let builtins = manager.builtin_extensions().unwrap();
    assert_eq!(builtins.as_array().unwrap().len(), builtin_extension_defs().len());
    assert_eq!(builtin(&manager, "session-title-hook")["enabled"], true);
    assert_eq!(builtin(&manager, "session-title-hook")["setting"], Value::Null);

    manager.set_script_enabled("builtin:session-title-hook", false).unwrap();
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["disabled"], json!(["session-title-hook"]));
    assert_eq!(builtin(&manager, "session-title-hook")["enabled"], false);

    // 状态总随注入包下发，用户脚本总开关关着也一样（页面据此卸下内置功能）。
    manager.set_global_enabled(false).unwrap();
    let bundle = manager.build_enabled_bundle().unwrap();
    assert!(bundle.contains("window.__codexPlusBuiltinExtensionState"), "{bundle}");
    assert!(bundle.contains("\"session-title-hook\""));
}

#[test]
fn builtins_with_a_setting_use_that_setting_as_their_switch() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    // 默认值跟着设置走：粘贴修复默认关、会话删除默认开。
    assert_eq!(builtin(&manager, "paste-fix")["enabled"], false);
    assert_eq!(builtin(&manager, "paste-fix")["setting"], "codexAppPasteFix");
    assert_eq!(builtin(&manager, "paste-fix")["restartRequired"], true);
    assert_eq!(builtin(&manager, "session-delete")["enabled"], true);
    assert_eq!(builtin(&manager, "session-delete")["params"], json!(["codexAppSessionDeleteSkipConfirm"]));

    manager.set_script_enabled("builtin:paste-fix", true).unwrap();
    assert_eq!(settings(temp.path())["codexAppPasteFix"], true);
    assert_eq!(builtin(&manager, "paste-fix")["enabled"], true);
    manager.set_script_enabled("builtin:session-delete", false).unwrap();
    assert_eq!(settings(temp.path())["codexAppSessionDelete"], false);
    assert_eq!(builtin(&manager, "session-delete")["enabled"], false);

    // 嵌套字段与下拉型。
    manager.set_script_enabled("builtin:agent-pet", true).unwrap();
    assert_eq!(settings(temp.path())["agentPet"]["enabled"], true);
    assert_eq!(builtin(&manager, "agent-pet")["enabled"], true);
    manager.set_script_enabled("builtin:typing-effects", true).unwrap();
    assert_eq!(settings(temp.path())["codexAppTypingEffect"], "rainbow");
    assert_eq!(builtin(&manager, "typing-effects")["enabled"], true);
    manager.set_script_enabled("builtin:typing-effects", false).unwrap();
    assert_eq!(settings(temp.path())["codexAppTypingEffect"], "off");

    // 有设置字段的不进下发的停用集合，功能自己读设置；也不在用户脚本配置里另记一份。
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["disabled"], json!([]));
    assert!(manager.load_config().scripts.keys().all(|key| !key.starts_with("builtin:")));
}

#[test]
fn stale_disable_flags_for_setting_backed_builtins_are_ignored_and_cleared() {
    let temp = tempfile::tempdir().unwrap();
    // 旧版本把会话删除记成停用。
    fs::write(temp.path().join("user_scripts.json"), json!({ "enabled": true, "scripts": { "builtin:session-delete": false } }).to_string()).unwrap();
    let manager = manager(temp.path());
    assert_eq!(manager.builtin_extension_state().unwrap()["disabled"], json!([]));
    assert_eq!(builtin(&manager, "session-delete")["enabled"], true);
    manager.set_script_enabled("builtin:session-delete", true).unwrap();
    assert!(!manager.load_config().scripts.contains_key("builtin:session-delete"));
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

/// 在后台生效的内置拓展（不查顶替状态）不能被顶替，免得声明了 replaces 却实际没让位。
#[test]
fn backend_builtins_cannot_be_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    assert_eq!(builtin(&manager, "native-fast-unlock")["replaceable"], false);
    assert_eq!(builtin(&manager, "paste-fix")["replaceable"], true);
    write_package(temp.path(), "fast", json!({ "id": "fast", "renderer": "renderer.js", "replaces": ["native-fast-unlock"] }));
    let inventory = manager.inventory().unwrap();
    let package = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:fast").cloned().unwrap();
    assert!(package["error"].as_str().unwrap().contains("不能被拓展顶替"), "{package}");
}

#[test]
fn builtin_extension_active_follows_disable_and_replaces() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    assert!(manager.builtin_extension_active("session-title-hook"));
    manager.set_script_enabled("builtin:session-title-hook", false).unwrap();
    assert!(!manager.builtin_extension_active("session-title-hook"));
    assert!(manager.builtin_extension_active("paste-fix"));
    write_package(temp.path(), "my-paste", json!({ "id": "my-paste", "renderer": "renderer.js", "replaces": ["paste-fix"] }));
    assert!(!manager.builtin_extension_active("paste-fix"));
}

/// 内置拓展的 id 在清单（enhancement-settings.json）与页面清单两处要一致，且不重复。
#[test]
fn builtin_extension_ids_match_page_catalog() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = fs::read_to_string(root.join("assets/inject/renderer-inject/96-builtin-extensions.js")).unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for def in builtin_extension_defs() {
        let id = &def.id;
        assert!(seen.insert(id.clone()), "内置拓展 id 重复：{id}");
        assert!(!def.name.is_empty() && !def.description.is_empty() && !def.group.is_empty(), "{id} 缺名称、说明或分组");
        assert!(catalog.contains(&format!("id: \"{id}\"")), "页面内置拓展清单缺 {id}");
    }
}
