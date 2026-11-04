//! Claude Mods 的显式清单、生成插件及后台宿主隔离。

use codex_plus_core::extension_backend;
use codex_plus_core::extension_claude::{self, ClaudeDesktopExtension};
use codex_plus_core::user_scripts::{ExtensionWrite, UserScriptManager};
use serde_json::{Value, json};
use std::fs;
use std::time::Duration;

fn manager(temp: &tempfile::TempDir, enabled: bool) -> UserScriptManager {
    let settings = temp.path().join("settings.json");
    fs::write(&settings, json!({"claudeAppExtensions": enabled}).to_string()).unwrap();
    UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("user_scripts.json"),
    ).with_settings_path(settings)
}

fn declaration() -> Value {
    json!({
        "buttons": [{"id": "count", "label": "Count", "method": "add"}],
        "events": {"session.start": "onClaudeLifecycle"},
        "tools": [{
            "name": "inspect",
            "description": "Inspect extension state",
            "method": "inspect",
            "inputSchema": {"type": "object", "properties": {}},
        }],
    })
}

fn write_package(temp: &tempfile::TempDir, id: &str, claude: Option<Value>, script: &str) {
    let root = temp.path().join("user").join(id);
    fs::create_dir_all(&root).unwrap();
    let mut manifest = json!({
        "id": id,
        "name": format!("Extension {id}"),
        "backend": {"script": "main.mjs"},
        "config": {"secret": {"type": "string", "secret": true, "default": "private-config-value"}},
    });
    if let Some(claude) = claude {
        manifest["claudeDesktop"] = claude;
    }
    fs::write(root.join("codexpp.json"), manifest.to_string()).unwrap();
    fs::write(root.join("main.mjs"), script).unwrap();
}

#[test]
fn claude_manifest_requires_explicit_valid_public_methods() {
    let parsed = ClaudeDesktopExtension::parse(&declaration()).unwrap();
    assert!(parsed.permits("add"));
    assert!(parsed.permits("inspect"));
    assert!(parsed.permits("onClaudeLifecycle"));
    assert!(!parsed.permits("undeclared"));
    assert!(!parsed.permits("__tick"));
    assert_eq!(parsed.tools[0].input_schema["additionalProperties"], false);

    for invalid in [
        json!({}),
        json!({"renderer": "claude.js"}),
        json!({"events": {"unknown.event": "run"}}),
        json!({"events": {"turn.start": "__service"}}),
        json!({"buttons": [{"id": "x", "label": "X", "method": "constructor"}]}),
        json!({"buttons": [{"id": "x", "label": "X", "method": "run"}, {"id": "x", "label": "Y", "method": "run"}]}),
        json!({"tools": [{"name": "x", "method": "run", "inputSchema": {"type": "object"}}]}),
        json!({"tools": [{"name": "x", "method": "run", "inputSchema": {"type": "object", "properties": {}, "additionalProperties": true}}]}),
        json!({"tools": [{"name": "x", "method": "run", "inputSchema": {"type": "object", "properties": {"tool": {"type": "string"}}}}]}),
        json!({"tools": [{"name": "x", "method": "run", "inputSchema": {"type": "object", "properties": {"tool_use_id": {"type": "string"}}}}]}),
    ] {
        assert!(ClaudeDesktopExtension::parse(&invalid).is_err(), "accepted {invalid}");
    }
}

#[test]
fn claude_inventory_is_opt_in_and_does_not_add_codex_capabilities() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, false);
    write_package(&temp, "codex-only", None, "export default () => ({})");
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
    codex_plus_core::settings::SettingsStore::new(temp.path().join("settings.json"))
        .update(json!({"claudeAppExtensions": true})).unwrap();

    let inventory = serde_json::to_value(manager.enabled_claude_extensions().unwrap()).unwrap();
    assert_eq!(inventory.as_array().unwrap().len(), 1);
    assert_eq!(inventory[0]["key"], "user:shared");
    assert_eq!(inventory[0]["events"]["session.start"], "onClaudeLifecycle");
    assert!(!inventory.to_string().contains("private-config-value"));
    assert!(!inventory.to_string().contains(&temp.path().display().to_string()));
    assert!(!manager.build_enabled_bundle().unwrap().contains("onClaudeLifecycle"));
    assert!(manager.enabled_backend_services().unwrap().is_empty());
    assert!(manager.enabled_extension_schedules().unwrap().is_empty());
    assert!(manager.enabled_extension_backend("user:codex-only").is_ok(), "旧 backend-only 包保持可用");

    manager.set_script_enabled("user:shared", false).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1, "Codex 单项开关不再影响 Claude");
    manager.set_claude_script_enabled("user:shared", false).unwrap();
    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
    manager.set_claude_script_enabled("user:shared", true).unwrap();
    manager.set_global_enabled(false).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1, "Codex 总开关不再影响 Claude");
    manager.set_claude_extensions_enabled(false).unwrap();
    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
}

#[test]
fn claude_switches_and_inventory_are_independent_of_codex() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    write_package(&temp, "codex-only", None, "export default () => ({})");

    manager.set_claude_script_enabled("user:shared", false).unwrap();
    let inventory = manager.inventory().unwrap();
    let shared = inventory["scripts"].as_array().unwrap().iter()
        .find(|script| script["key"] == "user:shared").unwrap();
    assert_eq!(inventory["enabled"], true);
    assert_eq!(inventory["claude_enabled"], true);
    assert_eq!(shared["enabled"], true);
    assert_eq!(shared["claude_enabled"], false);
    assert!(manager.enabled_extension_backend("user:shared").is_ok());
    assert!(manager.set_claude_script_enabled("user:codex-only", true).is_err());
    assert!(manager.set_claude_script_enabled("user:missing", true).is_err());

    manager.set_claude_script_enabled("user:shared", true).unwrap();
    manager.set_script_enabled("user:shared", false).unwrap();
    manager.set_global_enabled(false).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1);
    assert!(manager.enabled_extension_backend("user:shared").is_err());
    manager.set_claude_extensions_enabled(false).unwrap();
    manager.set_global_enabled(true).unwrap();
    manager.set_script_enabled("user:shared", true).unwrap();
    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
    assert!(manager.enabled_extension_backend("user:shared").is_ok());
}

#[test]
fn legacy_claude_item_switch_is_preserved_when_codex_changes() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    fs::write(temp.path().join("user_scripts.json"), json!({
        "enabled": true,
        "scripts": {"user:shared": false},
    }).to_string()).unwrap();

    manager.set_script_enabled("user:shared", true).unwrap();
    assert!(manager.enabled_extension_backend("user:shared").is_ok());
    assert!(manager.enabled_claude_extensions().unwrap().is_empty(), "迁移后保留 Claude 原来的关闭状态");
    manager.set_claude_script_enabled("user:shared", true).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1);
    manager.set_script_enabled("user:shared", false).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1);
}

#[test]
fn legacy_global_off_does_not_silently_enable_claude_extensions() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    fs::write(temp.path().join("user_scripts.json"), json!({
        "enabled": false,
        "scripts": {},
    }).to_string()).unwrap();

    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
    manager.set_global_enabled(true).unwrap();
    assert!(manager.enabled_claude_extensions().unwrap().is_empty());
    assert!(manager.enabled_extension_backend("user:shared").is_ok());
    manager.set_claude_script_enabled("user:shared", true).unwrap();
    manager.set_global_enabled(false).unwrap();
    assert_eq!(manager.enabled_claude_extensions().unwrap().len(), 1);
}

#[test]
fn extension_writer_preserves_claude_opt_in() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    let mut write = ExtensionWrite {
        id: "created".to_string(),
        name: "Created".to_string(),
        backend: Some(json!({"script": "main.mjs"})),
        ..Default::default()
    };
    write.files.insert("main.mjs".to_string(), "export default () => ({})".to_string());
    write.extra.insert("claudeDesktop".to_string(), declaration());
    let result = manager.write_extension_package(&write).unwrap();
    assert!(result["manifestError"].is_null(), "{result}");
    assert_eq!(manager.enabled_claude_extensions().unwrap()[0].key, "user:created");
}

#[test]
fn generated_claude_plugin_contains_only_public_declarations_and_persistent_mcp() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    let state = temp.path().join("state");
    let executable = temp.path().join("CodexPlusPlus");
    let root = extension_claude::prepare_managed_plugin_at(&manager, &state, &executable).unwrap();
    let manifest: Value = serde_json::from_slice(&fs::read(root.join(".claude-plugin/plugin.json")).unwrap()).unwrap();
    let mcp: Value = serde_json::from_slice(&fs::read(root.join(".mcp.json")).unwrap()).unwrap();
    let hooks: Value = serde_json::from_slice(&fs::read(root.join("hooks/hooks.json")).unwrap()).unwrap();
    let registration = fs::read_to_string(root.join("hooks/register.js")).unwrap();
    assert_eq!(manifest["name"], extension_claude::PLUGIN_NAME);
    assert_eq!(manifest["author"]["name"], "CodexPlusPlus");
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["command"], executable.display().to_string());
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["args"][0], extension_claude::MCP_FLAG);
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["args"][1], "--state-dir");
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["args"][2], state.display().to_string());
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["args"][3], "--settings-path");
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["args"][4], temp.path().join("settings.json").display().to_string());
    let config_env = if cfg!(windows) { "APPDATA" } else { "XDG_CONFIG_HOME" };
    assert_eq!(mcp["mcpServers"]["codexpp-extensions"]["env"][config_env],
        codex_plus_core::user_scripts::default_user_scripts_config_dir().parent().unwrap().display().to_string());
    assert_eq!(hooks, json!({"modules": ["./register.js"]}));
    assert!(registration.contains("plugin:codexpp-extension-host:codexpp-extensions"));
    assert!(registration.contains("onClaudeLifecycle"));
    assert!(!registration.contains("/*CODEXPP_CONFIG*/"));
    assert!(!registration.contains("private-config-value"));
    assert!(!registration.contains(&temp.path().display().to_string()));
    fs::write(root.join("keep.txt"), "keep").unwrap();
    extension_claude::prepare_managed_plugin_at(&manager, &state, &executable).unwrap();
    assert_eq!(fs::read_to_string(root.join("keep.txt")).unwrap(), "keep");
}

#[tokio::test]
async fn claude_mcp_rejects_unscoped_calls_without_starting_node() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "codex-only", None, "export default () => ({})");
    write_package(&temp, "shared", Some(declaration()), "export default () => ({})");
    let request = |arguments| json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": extension_claude::TOOL_NAME, "arguments": arguments},
    });
    for arguments in [
        json!({"key": "user:codex-only", "method": "add"}),
        json!({"key": "user:shared", "method": "__tick"}),
        json!({"key": "user:shared", "method": "undeclared"}),
        json!({"key": "user:shared", "method": "add", "sessionId": 12}),
        json!({"key": "user:shared", "method": "add", "route": "/settings/set"}),
    ] {
        let response = extension_claude::handle_request(&manager, request(arguments)).await.unwrap();
        assert_eq!(response["result"]["isError"], true, "{response}");
    }
    let initialize = extension_claude::handle_request(&manager, json!({
        "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2025-06-18"},
    })).await.unwrap();
    assert_eq!(initialize["result"]["protocolVersion"], "2025-06-18");
    assert!(extension_claude::handle_request(&manager, json!({
        "jsonrpc": "2.0", "method": "notifications/initialized",
    })).await.is_none());
}

#[tokio::test]
async fn claude_backends_keep_state_but_isolate_codex_host_and_data() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    write_package(&temp, "shared", Some(declaration()), r#"
export default (ctx) => {
  let count = 0;
  return {
    add() { return ++count; },
    async inspect() {
      let hostError = null;
      try { await ctx.sessionQuery.listSessions(); } catch (error) { hostError = error.message; }
      return { host: ctx.host, dataDir: ctx.dataDir, hostError };
    },
    onClaudeLifecycle() { return true; },
  };
};
"#);
    let claude = |method| json!({"key": "user:shared", "method": method, "sessionId": "claude-test"});
    let check = async {
        assert_eq!(extension_claude::call(&manager, &claude("add")).await.unwrap(), 1);
        assert_eq!(extension_claude::call(&manager, &claude("add")).await.unwrap(), 2);
        assert_eq!(extension_backend::call(&manager, "user:shared", "add", Value::Null, Duration::from_secs(10), None).await.unwrap(), 1);
        extension_backend::stop_all(&manager).await;
        assert_eq!(extension_claude::call(&manager, &claude("add")).await.unwrap(), 3);
        let place = extension_claude::call(&manager, &claude("inspect")).await.unwrap();
        assert_eq!(place["host"], "claudeDesktop");
        // 用 join 拼：Windows 上分隔符是 \\，手写 "/" 对不上。
        assert_eq!(place["dataDir"], temp.path().join("claude-extension-data").join("shared").display().to_string());
        assert!(place["hostError"].as_str().unwrap().contains("未开放 Codex 宿主调用"));
        manager.set_script_enabled("user:shared", false).unwrap();
        assert_eq!(extension_claude::call(&manager, &claude("add")).await.unwrap(), 4);
        manager.set_claude_script_enabled("user:shared", false).unwrap();
        assert!(extension_claude::call(&manager, &claude("add")).await.is_err());
        manager.set_claude_script_enabled("user:shared", true).unwrap();
        assert_eq!(extension_claude::call(&manager, &claude("add")).await.unwrap(), 1);
        codex_plus_core::settings::SettingsStore::new(temp.path().join("settings.json"))
            .update(json!({"claudeAppExtensions": false})).unwrap();
        assert!(extension_claude::call(&manager, &claude("add")).await.unwrap_err().to_string().contains("已关闭"));
    };
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(check).catch_unwind().await;
    extension_backend::stop_all_claude(&manager).await;
    extension_backend::stop_all(&manager).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[tokio::test]
async fn claude_session_end_timeout_stops_backend_and_allows_recovery() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp, true);
    let mut declared = declaration();
    declared["events"] = json!({"session.end": "onEnd"});
    write_package(&temp, "slow", Some(declared), r#"
export default () => {
  let count = 0;
  return {
    add() { return ++count; },
    onEnd() { return new Promise(() => {}); },
  };
};
"#);
    let call = |method, params| json!({"key": "user:slow", "method": method, "params": params});
    let check = async {
        assert_eq!(extension_claude::call(&manager, &call("add", Value::Null)).await.unwrap(), 1);
        let start = std::time::Instant::now();
        let error = extension_claude::call(&manager, &call(
            "onEnd", json!({"host": "claudeDesktop", "event": "session.end"}),
        )).await.unwrap_err();
        assert!(error.to_string().contains("超时"), "{error:#}");
        assert!(start.elapsed() < Duration::from_secs(2), "session.end 不能等待默认 20 秒后台预算");
        assert_eq!(extension_claude::call(&manager, &call("add", Value::Null)).await.unwrap(), 1);
    };
    use futures_util::FutureExt;
    let result = std::panic::AssertUnwindSafe(check).catch_unwind().await;
    extension_backend::stop_all_claude(&manager).await;
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
