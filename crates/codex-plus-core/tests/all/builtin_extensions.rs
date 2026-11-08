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
    // 粘贴监听随内置拓展装卸、开关每次粘贴时现查，改设置不必重启；强制中文仍要重启。
    assert_eq!(builtin(&manager, "paste-fix")["restartRequired"], false);
    assert_eq!(builtin(&manager, "force-chinese-locale")["restartRequired"], true);
    assert_eq!(builtin(&manager, "session-delete")["enabled"], true);
    assert_eq!(builtin(&manager, "session-delete")["params"], json!(["codexAppSessionDeleteSkipConfirm"]));

    manager.set_script_enabled("builtin:paste-fix", true).unwrap();
    assert_eq!(settings(temp.path())["codexAppPasteFix"], true);
    assert_eq!(builtin(&manager, "paste-fix")["enabled"], true);
    manager.set_script_enabled("builtin:session-delete", false).unwrap();
    assert_eq!(settings(temp.path())["codexAppSessionDelete"], false);
    assert_eq!(builtin(&manager, "session-delete")["enabled"], false);

    // 嵌套字段。
    manager.set_script_enabled("builtin:dictation", true).unwrap();
    assert_eq!(settings(temp.path())["dictation"]["enabled"], true);
    assert_eq!(builtin(&manager, "dictation")["enabled"], true);

    // 有设置字段的不进下发的停用集合，功能自己读设置；也不在用户脚本配置里另记一份。
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["disabled"], json!([]));
    assert!(manager.load_config().scripts.keys().all(|key| !key.starts_with("builtin:")));
}

#[test]
fn reported_usage_compaction_defaults_off_and_persists_without_enabling_proxy() {
    use codex_plus_core::settings::BackendSettings;
    let defaults = BackendSettings::default();
    assert!(!defaults.codex_app_reported_usage_compaction);
    let legacy: BackendSettings = serde_json::from_value(json!({})).unwrap();
    assert!(!legacy.codex_app_reported_usage_compaction);
    let imported: BackendSettings =
        serde_json::from_value(json!({"codexAppReportedUsageCompaction": true})).unwrap();
    assert!(imported.codex_app_reported_usage_compaction);
    assert_eq!(serde_json::to_value(imported).unwrap()["codexAppReportedUsageCompaction"], true);

    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    let before = settings(temp.path());
    assert_eq!(builtin(&manager, "reported-usage-compaction")["enabled"], false);
    assert_eq!(builtin(&manager, "reported-usage-compaction")["setting"], "codexAppReportedUsageCompaction");
    manager.set_script_enabled("builtin:reported-usage-compaction", true).unwrap();
    let enabled = settings(temp.path());
    assert_eq!(enabled["codexAppReportedUsageCompaction"], true);
    for key in ["relayProfiles", "relayProfilesEnabled", "codexAppAutoTakeover"] {
        assert_eq!(enabled[key], before[key], "{key} must not change when enabling this experiment");
    }
    manager.set_script_enabled("builtin:reported-usage-compaction", false).unwrap();
    assert_eq!(settings(temp.path())["codexAppReportedUsageCompaction"], false);
}

#[test]
fn reported_usage_compaction_is_master_gated_and_yields_to_builtin_replacement() {
    use codex_plus_core::builtin_takeover::mask_settings_with;
    use codex_plus_core::settings::BackendSettings;
    let def = codex_plus_core::settings_catalog::builtin_extension_def("reported-usage-compaction").unwrap();
    assert!(def.master_gated);
    assert!(def.replaceable);
    let mut settings = BackendSettings::default();
    settings.enhancements_enabled = true;
    settings.codex_app_reported_usage_compaction = true;
    let replaced = ["reported-usage-compaction".to_string()].into();
    assert!(!mask_settings_with(settings.clone(), &replaced).codex_app_reported_usage_compaction);
    assert!(mask_settings_with(settings, &Default::default()).codex_app_reported_usage_compaction);
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
    write_package(temp.path(), "my-pet", json!({ "id": "my-pet", "renderer": "renderer.js", "replaces": ["builtin:dictation", "usage-statusline"] }));
    let state = manager.builtin_extension_state().unwrap();
    assert_eq!(state["replacedBy"]["dictation"], "user:my-pet");
    assert_eq!(state["replacedBy"]["usage-statusline"], "user:my-pet");
    let inventory = manager.inventory().unwrap();
    let package = inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:my-pet").cloned().unwrap();
    assert_eq!(package["package"]["replaces"], json!(["dictation", "usage-statusline"]));

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

/// Rust 侧生效的内置拓展也能被顶替（Rust 侧经 builtin_takeover 让位）；只有拓展不加载时才生效的官方额度解锁不能。
#[test]
fn rust_side_builtins_can_be_replaced_except_standalone_quota_unlock() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(temp.path());
    for id in codex_plus_core::builtin_takeover::RUST_SIDE_BUILTINS {
        assert_eq!(builtin(&manager, id)["replaceable"], true, "{id}");
    }
    assert_eq!(builtin(&manager, "standalone-quota-unlock")["replaceable"], false);
    write_package(temp.path(), "fast", json!({ "id": "fast", "renderer": "renderer.js", "replaces": ["native-fast-unlock", "force-chinese-locale"] }));
    write_package(temp.path(), "quota", json!({ "id": "quota", "renderer": "renderer.js", "replaces": ["standalone-quota-unlock"] }));
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).cloned().unwrap();
    assert_eq!(find("user:fast")["error"], "", "{}", find("user:fast"));
    assert!(find("user:quota")["error"].as_str().unwrap().contains("不能被拓展顶替"), "{}", find("user:quota"));
    assert_eq!(manager.builtin_extension_state().unwrap()["replacedBy"]["native-fast-unlock"], "user:fast");
}

/// 被顶替的 Rust 侧内置拓展按「关」处理；增强总开关关着时不按顶替处理（拓展这时不加载）。
#[test]
fn replaced_rust_side_builtins_are_switched_off_in_effective_settings() {
    use codex_plus_core::builtin_takeover::{RUST_SIDE_BUILTINS, mask_settings_with};
    let mut settings = codex_plus_core::settings::BackendSettings::default();
    settings.enhancements_enabled = true;
    settings.codex_app_native_fast_unlock = true;
    settings.codex_app_force_chinese_locale = true;
    settings.codex_app_model_whitelist_unlock = true;
    settings.codex_app_native_predictions = true;
    let replaced: std::collections::BTreeSet<String> = ["native-fast-unlock", "native-predictions"].iter().map(|id| id.to_string()).collect();
    let masked = mask_settings_with(settings.clone(), &replaced);
    assert!(!masked.codex_app_native_fast_unlock);
    assert!(!masked.codex_app_native_predictions);
    assert!(masked.codex_app_force_chinese_locale, "没被顶替的保持原值");
    assert!(masked.codex_app_model_whitelist_unlock);
    settings.enhancements_enabled = false;
    assert!(mask_settings_with(settings, &replaced).codex_app_native_fast_unlock);
    // 每个 Rust 侧内置拓展都有对应的设置项，且被顶替后读成关。
    for id in RUST_SIDE_BUILTINS {
        let def = codex_plus_core::settings_catalog::builtin_extension_def(id).unwrap();
        let setting = def.setting.clone().expect("Rust 侧内置拓展都有设置项");
        let mut on = codex_plus_core::settings::BackendSettings::default();
        on.enhancements_enabled = true;
        let mut value = serde_json::to_value(&on).unwrap();
        value[&setting] = json!(true);
        let on: codex_plus_core::settings::BackendSettings = serde_json::from_value(value).unwrap();
        let only: std::collections::BTreeSet<String> = [id.to_string()].into();
        let masked = serde_json::to_value(mask_settings_with(on, &only)).unwrap();
        assert_eq!(masked[&setting], json!(false), "{id} 被顶替后 {setting} 应读成关");
    }
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
