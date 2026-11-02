//! 随 Codex++ 发布的官方拓展包（来源 `bundled`）：Agent 桌宠从内置功能改成了拓展包。
use codex_plus_core::bundled_extensions::{AGENT_PET_KEY, agent_pet_files, agent_pet_settings_html, materialize};
use codex_plus_core::settings::{AgentPetSettings, BackendSettings};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};

fn manager(root: &std::path::Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("user_scripts.json"))
        .with_bundled_dir(root.join("bundled"))
}

fn script(manager: &UserScriptManager, key: &str) -> Value {
    manager.inventory().unwrap()["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).cloned().unwrap()
}

#[test]
fn agent_pet_package_is_a_valid_extension_that_only_uses_public_interfaces() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    let pet = script(&manager, AGENT_PET_KEY);
    assert_eq!(pet["error"], "", "{pet}");
    assert_eq!(pet["source"], "bundled");
    assert_eq!(pet["kind"], "package");
    let package = &pet["package"];
    assert_eq!(package["id"], "agent-pet");
    // 内嵌的两组音效随包的 assets/ 下发。
    for sound in ["assets/sounds/Ya1.mp3", "assets/sounds/Ya2.mp3", "assets/sounds/D1.mp3", "assets/sounds/D2.mp3"] {
        assert!(package["assets"].as_array().unwrap().iter().any(|item| item == sound), "{sound}");
    }
    // 数据路由都在清单里声明，且都在拓展路由白名单里。
    let api = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/inject/renderer-inject/91-extension-api.js")).unwrap();
    for route in package["routes"].as_array().unwrap() {
        assert!(api.contains(&format!("\"{}\"", route.as_str().unwrap())), "{route} 不在拓展路由白名单里");
    }
    // 页面脚本只用公开接口：不再碰注入层的私有函数与全局变量。
    for (path, content) in agent_pet_files() {
        if !path.ends_with(".js") || path == "whale-engine.js" {
            continue;
        }
        let source = String::from_utf8(content).unwrap();
        for private in [
            "postJson(",
            "codexPlusBackendSettings",
            "currentSessionRef(",
            "__codexPlusRemoteSessionRecoveryDispatcher",
            "__codexPlusBuiltinInactive",
            "registerCodexPlusExtensionSelector",
            "sendCodexPlusDiagnostic",
            "runScanStep",
            "__CODEX_PLUS_",
        ] {
            assert!(!source.contains(private), "{path} 还在用 {private}");
        }
    }
    // 引擎包成函数声明，不用 new Function / eval。
    let engine = agent_pet_files().into_iter().find(|(path, _)| *path == "whale-engine.js").unwrap().1;
    let engine = String::from_utf8(engine).unwrap();
    assert!(engine.trim_start().starts_with("function codexPlusWhaleEngine(window,document,fetch,"));
    assert!(!engine.contains("new Function("));
}

#[test]
fn materialize_is_idempotent_and_restores_edited_files() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("bundled");
    materialize(&directory).unwrap();
    let main = directory.join("agent-pet").join("main.js");
    let original = std::fs::read(&main).unwrap();
    let modified = std::fs::metadata(&main).unwrap().modified().unwrap();
    materialize(&directory).unwrap();
    assert_eq!(std::fs::metadata(&main).unwrap().modified().unwrap(), modified, "内容没变不重写");
    std::fs::write(&main, "// 被改坏了").unwrap();
    materialize(&directory).unwrap();
    assert_eq!(std::fs::read(&main).unwrap(), original);
}

#[test]
fn old_agent_pet_settings_migrate_into_the_extension_config_once() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    let mut settings = BackendSettings::default();
    settings.agent_pet = AgentPetSettings { enabled: true, image: "fox".into(), size: 160, volume: 0.3, ..AgentPetSettings::whale_widget() };
    settings.codex_app_whale_balance_protocol = "custom".into();
    settings.codex_app_whale_balance_path = "/user/balance".into();
    assert!(manager.migrate_bundled_agent_pet(&settings).unwrap());
    let pet = script(&manager, AGENT_PET_KEY);
    assert_eq!(pet["enabled"], true);
    let config = &pet["package"]["config"];
    assert_eq!(config["kind"], "whaleWidget");
    assert_eq!(config["image"], "fox");
    assert_eq!(config["size"], json!(160));
    assert_eq!(config["volume"], json!(0.3));
    assert_eq!(config["codexAppWhaleBalanceProtocol"], "custom");
    assert_eq!(config["codexAppWhaleBalancePath"], "/user/balance");
    // 只迁移一次：之后在拓展里改的配置、启停不会被旧设置盖掉。
    manager.set_script_enabled(AGENT_PET_KEY, false).unwrap();
    assert!(!manager.migrate_bundled_agent_pet(&settings).unwrap());
    assert_eq!(script(&manager, AGENT_PET_KEY)["enabled"], false);
}

#[test]
fn agent_pet_is_off_for_users_who_never_turned_the_pet_on() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    assert!(manager.migrate_bundled_agent_pet(&BackendSettings::default()).unwrap());
    let pet = script(&manager, AGENT_PET_KEY);
    assert_eq!(pet["enabled"], false);
    assert_eq!(pet["package"]["config"]["kind"], "image");
    // 停用时它声明的数据路由不算「有拓展在用」，桌宠路由不干活。
    assert!(!manager.route_declared("/agent-pet/image"));
    manager.set_script_enabled(AGENT_PET_KEY, true).unwrap();
    assert!(manager.route_declared("/agent-pet/image"));
    assert!(manager.route_declared("/whale/full"));
    assert!(!manager.route_declared("/whale/balance"));
}

#[test]
fn bundled_packages_cannot_be_deleted() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    assert!(manager.delete_user_script(AGENT_PET_KEY).is_err());
    assert!(temp.path().join("bundled/agent-pet/codexpp.json").exists());
}

#[test]
fn agent_pet_settings_page_inlines_the_shared_settings_ui_and_opens_while_disabled() {
    let html = agent_pet_settings_html();
    assert!(!html.contains("/*__AGENT_PET_SETTINGS_SCRIPTS__*/"), "占位要换掉");
    assert!(html.contains("function createCodexPlusAgentPetSettings(container, host)"));
    assert!(html.contains("manager.call(route, payload)"));
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    let pet = script(&manager, AGENT_PET_KEY);
    assert_eq!(pet["package"]["settingsPage"], true, "{pet}");
    // 设置不依赖拓展正在运行：停用时也能打开设置页、取包里的音效。
    manager.set_script_enabled(AGENT_PET_KEY, false).unwrap();
    let (title, page) = manager.extension_settings_page(AGENT_PET_KEY).unwrap();
    assert_eq!(title, "Agent 桌宠");
    assert_eq!(page, html);
    assert!(manager.extension_asset_for(AGENT_PET_KEY, "assets/sounds/Ya1.mp3", false).unwrap().starts_with("data:audio/mpeg;base64,"));
    assert!(manager.extension_asset(AGENT_PET_KEY, "assets/sounds/Ya1.mp3").is_err(), "管理器面板仍要求启用");
}

#[tokio::test]
async fn settings_pages_only_call_declared_extension_page_routes() {
    let temp = tempfile::tempdir().unwrap();
    materialize(&temp.path().join("bundled")).unwrap();
    let manager = manager(temp.path());
    let catalog = manager.extension_page_route(AGENT_PET_KEY, "/agent-pet/catalog", &json!({})).await.unwrap();
    assert_eq!(catalog["status"], "ok");
    assert_eq!(catalog["images"][0]["id"], "whale");
    assert!(catalog["sounds"].as_array().is_some_and(|sounds| !sounds.is_empty()));
    assert!(catalog["whaleImage"].as_str().unwrap().starts_with("data:image/png;base64,"));
    // 不开放给拓展页面的路由、没声明的路由都不行。
    for route in ["/whale/full", "/settings/set", "/user-scripts/set-config"] {
        let error = manager.extension_page_route(AGENT_PET_KEY, route, &json!({})).await.unwrap_err().to_string();
        assert!(error.contains("不能调用路由"), "{route}: {error}");
    }
    std::fs::create_dir_all(temp.path().join("user/other")).unwrap();
    std::fs::write(temp.path().join("user/other/codexpp.json"), r#"{"id":"other","renderer":["main.js"],"engines":{"codexpp":1}}"#).unwrap();
    std::fs::write(temp.path().join("user/other/main.js"), "").unwrap();
    let error = manager.extension_page_route("user:other", "/agent-pet/catalog", &json!({})).await.unwrap_err().to_string();
    assert!(error.contains("没有在清单 permissions.routes 里声明"), "{error}");
}
