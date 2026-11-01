//! 打字特效、背景图从内置功能改成了官方拓展包（来源 `bundled`）：包本身、旧设置迁移、背景图的读图路由。
use codex_plus_core::bundled_extensions::{
    IMAGE_OVERLAY_KEY, TYPING_EFFECTS_KEY, image_overlay_files, image_overlay_image, materialize, typing_effects_files,
};
use codex_plus_core::settings::BackendSettings;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Map, Value, json};

fn manager(root: &std::path::Path) -> UserScriptManager {
    UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("user_scripts.json"))
        .with_bundled_dir(root.join("bundled"))
}

fn script(manager: &UserScriptManager, key: &str) -> Value {
    manager.inventory().unwrap()["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).cloned().unwrap()
}

fn ready(root: &std::path::Path) -> UserScriptManager {
    materialize(&root.join("bundled")).unwrap();
    manager(root)
}

#[test]
fn both_packages_are_valid_extensions_that_only_use_public_interfaces() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    for (key, id) in [(TYPING_EFFECTS_KEY, "typing-effects"), (IMAGE_OVERLAY_KEY, "image-overlay")] {
        let item = script(&manager, key);
        assert_eq!(item["error"], "", "{item}");
        assert_eq!(item["source"], "bundled");
        assert_eq!(item["kind"], "package");
        assert_eq!(item["package"]["id"], id);
    }
    // 背景图声明的路由都在拓展路由白名单里；打字特效不用任何后端路由。
    let api = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/inject/renderer-inject/91-extension-api.js")).unwrap();
    let overlay = script(&manager, IMAGE_OVERLAY_KEY);
    for route in overlay["package"]["routes"].as_array().unwrap() {
        assert!(api.contains(&format!("\"{}\"", route.as_str().unwrap())), "{route} 不在拓展路由白名单里");
    }
    assert!(script(&manager, TYPING_EFFECTS_KEY)["package"]["routes"].as_array().is_none_or(Vec::is_empty));
    // 页面脚本只用公开接口：不碰注入层的私有函数与全局变量。
    for (path, content) in typing_effects_files().into_iter().chain(image_overlay_files()) {
        if !path.ends_with(".js") {
            continue;
        }
        let source = String::from_utf8(content).unwrap();
        for private in [
            "postJson(",
            "codexPlusSettings(",
            "codexPlusBackendSettings",
            "__codexPlusBuiltinInactive",
            "registerCodexPlusExtensionSelector",
            "sendCodexPlusDiagnostic",
            "isExtensionUiNode",
            "visibleElement(",
            "runScanStep",
            "__CODEX_PLUS_",
        ] {
            assert!(!source.contains(private), "{path} 还在用 {private}");
        }
    }
}

#[test]
fn image_path_is_a_file_field_with_an_image_filter() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    let schema = script(&manager, IMAGE_OVERLAY_KEY)["package"]["configSchema"].clone();
    let field = schema.as_array().unwrap().iter().find(|field| field["key"] == "imagePath").cloned().unwrap();
    assert_eq!(field["format"], "file");
    assert_eq!(field["extensions"], json!(["png", "jpg", "jpeg", "webp", "gif", "bmp"]));
    // 没写 format 的字段不带这两个键。
    let opacity = schema.as_array().unwrap().iter().find(|field| field["key"] == "opacity").cloned().unwrap();
    assert!(opacity.get("format").is_none() && opacity.get("extensions").is_none());
}

#[test]
fn file_format_is_only_accepted_on_string_fields() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    let write = |id: &str, config: Value| {
        let dir = temp.path().join("user").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = json!({ "id": id, "renderer": ["main.js"], "engines": { "codexpp": 1 }, "config": config });
        std::fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
        std::fs::write(dir.join("main.js"), "").unwrap();
    };
    write("ok-file", json!({ "path": { "type": "string", "format": "file", "extensions": ["txt"] } }));
    write("bad-kind", json!({ "count": { "type": "number", "format": "file" } }));
    write("bad-format", json!({ "path": { "type": "string", "format": "folder" } }));
    write("bad-extension", json!({ "path": { "type": "string", "format": "file", "extensions": [".png"] } }));
    write("orphan-extensions", json!({ "path": { "type": "string", "extensions": ["png"] } }));
    assert_eq!(script(&manager, "user:ok-file")["error"], "");
    for id in ["bad-kind", "bad-format", "bad-extension", "orphan-extensions"] {
        let error = script(&manager, &format!("user:{id}"))["error"].as_str().unwrap().to_string();
        assert!(error.contains("format") || error.contains("extensions"), "{id}: {error}");
    }
}

#[test]
fn old_typing_effect_setting_migrates_once() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    let settings = BackendSettings { codex_app_typing_effect: "stars".into(), ..BackendSettings::default() };
    manager.migrate_bundled_builtins(&settings).unwrap();
    let item = script(&manager, TYPING_EFFECTS_KEY);
    assert_eq!(item["enabled"], true);
    assert_eq!(item["package"]["config"]["effect"], "stars");
    // 只迁移一次：之后在拓展里改的启停、配置不会被旧设置盖掉。
    manager.set_script_enabled(TYPING_EFFECTS_KEY, false).unwrap();
    manager.migrate_bundled_builtins(&settings).unwrap();
    assert_eq!(script(&manager, TYPING_EFFECTS_KEY)["enabled"], false);
}

#[test]
fn typing_effects_stay_off_for_users_who_never_picked_one() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    manager.migrate_bundled_builtins(&BackendSettings::default()).unwrap();
    let item = script(&manager, TYPING_EFFECTS_KEY);
    assert_eq!(item["enabled"], false);
    assert_eq!(item["package"]["config"]["effect"], "rainbow", "启用后默认彩虹");
}

#[test]
fn old_image_overlay_settings_migrate_and_need_an_image_to_be_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    let settings = BackendSettings {
        codex_app_image_overlay_enabled: true,
        codex_app_image_overlay_path: "  /Users/me/Pictures/wall.png ".into(),
        codex_app_image_overlay_opacity: 60,
        codex_app_image_overlay_fit_mode: "tile".into(),
        ..BackendSettings::default()
    };
    manager.migrate_bundled_builtins(&settings).unwrap();
    let item = script(&manager, IMAGE_OVERLAY_KEY);
    assert_eq!(item["enabled"], true);
    assert_eq!(item["package"]["config"], json!({ "imagePath": "/Users/me/Pictures/wall.png", "opacity": 60, "fitMode": "tile" }));

    // 开着但没选图片：迁移成停用。
    let other = tempfile::tempdir().unwrap();
    let manager = ready(other.path());
    let settings = BackendSettings { codex_app_image_overlay_enabled: true, ..BackendSettings::default() };
    manager.migrate_bundled_builtins(&settings).unwrap();
    assert_eq!(script(&manager, IMAGE_OVERLAY_KEY)["enabled"], false);
    assert_eq!(script(&manager, IMAGE_OVERLAY_KEY)["package"]["config"]["fitMode"], "fit");
}

#[test]
fn image_route_reads_only_the_path_saved_in_the_extension_config() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    let save = |path: &str| {
        let mut values = Map::new();
        values.insert("imagePath".into(), json!(path));
        manager.set_extension_config(IMAGE_OVERLAY_KEY, &values).unwrap();
    };
    assert_eq!(image_overlay_image(&manager)["status"], "empty");

    let png = temp.path().join("wall.png");
    std::fs::write(&png, [0x89, b'P', b'N', b'G']).unwrap();
    save(&png.to_string_lossy());
    let result = image_overlay_image(&manager);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["dataUrl"], "data:image/png;base64,iVBORw==");

    save(&temp.path().join("missing.png").to_string_lossy());
    assert_eq!(image_overlay_image(&manager)["status"], "failed");
    let text = temp.path().join("notes.txt");
    std::fs::write(&text, "secret").unwrap();
    save(&text.to_string_lossy());
    let result = image_overlay_image(&manager);
    assert_eq!(result["status"], "failed", "不是图片就不读");
    assert!(result.get("dataUrl").is_none());
}

#[test]
fn image_route_is_declared_only_while_the_extension_is_enabled() {
    let temp = tempfile::tempdir().unwrap();
    let manager = ready(temp.path());
    manager.migrate_bundled_builtins(&BackendSettings::default()).unwrap();
    assert!(!manager.route_declared("/image-overlay/image"));
    manager.set_script_enabled(IMAGE_OVERLAY_KEY, true).unwrap();
    assert!(manager.route_declared("/image-overlay/image"));
}
