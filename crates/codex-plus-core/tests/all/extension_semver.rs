//! 拓展清单 engines.codexpp 的 semver 范围。

use codex_plus_core::extension_semver::satisfies;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use std::fs;

#[test]
fn ranges_follow_npm_semantics() {
    let cases = [
        ("2.1.4", "*", true),
        ("2.1.4", "2.x", true),
        ("2.1.4", "2", true),
        ("2.1.4", "3", false),
        ("2.1.4", "2.1", true),
        ("2.1.4", "2.2", false),
        ("2.1.4", "=2.1.4", true),
        ("2.1.4", "2.1.3", false),
        ("2.1.4", "^2.0.0", true),
        ("2.1.4", "^2.2", false),
        ("3.0.0", "^2.1", false),
        ("0.2.5", "^0.2.3", true),
        ("0.3.0", "^0.2.3", false),
        ("0.0.4", "^0.0.3", false),
        ("2.1.9", "~2.1.4", true),
        ("2.2.0", "~2.1.4", false),
        ("2.9.0", "~2", true),
        ("2.1.4", ">=2.1.0 <3", true),
        ("3.0.0", ">=2.1.0 <3", false),
        ("2.1.4", ">= 2.1.0 < 3", true),
        ("2.1.4", ">2.1", false),
        ("2.2.0", ">2.1", true),
        ("2.1.9", "<=2.1", true),
        ("2.2.0", "<=2.1", false),
        ("2.1.4", "1.0 - 2.1", true),
        ("2.2.0", "1.0 - 2.1", false),
        ("2.1.4", "^1 || ^2", true),
        ("4.0.0", "^1 || ^2", false),
        ("2.1.4-beta.1", "^2.1", true),
    ];
    for (version, range, expected) in cases {
        assert_eq!(satisfies(version, range).unwrap(), expected, "{version} 对 {range}");
    }
    assert!(satisfies("2.1.4", "").is_err());
    assert!(satisfies("2.1.4", ">=abc").is_err());
}

#[test]
fn manifest_engine_ranges_are_checked_against_codexpp_version() {
    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"));
    let write = |id: &str, engine: serde_json::Value| {
        let dir = temp.path().join("user").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "renderer": "main.js", "engines": {"codexpp": engine}}).to_string()).unwrap();
        fs::write(dir.join("main.js"), "").unwrap();
    };
    let current = codex_plus_core::extension_semver::codexpp_version();
    let major = current.split('.').next().unwrap().parse::<u64>().unwrap();
    write("ok-range", json!(format!("^{major}")));
    write("too-new", json!(format!(">={}", major + 1)));
    write("bad-range", json!(">=abc"));
    write("api-number", json!(1));
    let inventory = manager.inventory().unwrap();
    let error = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap()["error"].as_str().unwrap().to_string();
    assert_eq!(error("user:ok-range"), "");
    assert!(error("user:too-new").contains(&format!("当前是 {current}")), "{}", error("user:too-new"));
    assert!(error("user:bad-range").contains("版本范围写法不对"));
    assert_eq!(error("user:api-number"), "");
}
