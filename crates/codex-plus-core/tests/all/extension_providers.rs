//! 拓展声明的模型供应商：清单校验、Key 取自拓展配置、并入网关供应商池（不落盘、已有同名优先）。

use codex_plus_core::gateway::store::GatewaySettings;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: Value) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    let mut manifest = manifest;
    manifest["id"] = json!(id);
    manifest["name"] = json!(id);
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
}

#[test]
fn providers_are_validated_resolved_and_merged_without_overriding() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "my-relay", json!({
        "config": { "apiKey": { "type": "string", "secret": true } },
        "providers": [{ "id": "myrelay", "name": "我的中转", "protocol": "chat", "baseUrl": "https://relay.example.com/v1",
                        "apiKey": { "config": "apiKey" }, "models": ["gpt-x", "gpt-y"] }]
    }));
    write_package(&temp, "local-model", json!({
        "providers": [{ "id": "local", "baseUrl": "http://127.0.0.1:8000/v1", "models": ["tiny"] }]
    }));
    write_package(&temp, "bad-protocol", json!({ "providers": [{ "id": "x", "protocol": "grpc", "baseUrl": "https://a", "models": ["m"] }] }));
    write_package(&temp, "bad-url", json!({ "providers": [{ "id": "x", "baseUrl": "ftp://a", "models": ["m"] }] }));
    write_package(&temp, "bad-key-ref", json!({ "providers": [{ "id": "x", "baseUrl": "https://a", "apiKey": { "config": "nope" }, "models": ["m"] }] }));
    write_package(&temp, "no-models", json!({ "providers": [{ "id": "x", "baseUrl": "https://a", "models": [] }] }));
    let inventory = manager.inventory().unwrap();
    let error = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap()["error"].as_str().unwrap().to_string();
    assert_eq!(error("user:my-relay"), "");
    assert!(error("user:bad-protocol").contains("不认识 grpc"));
    assert!(error("user:bad-url").contains("http(s)"));
    assert!(error("user:bad-key-ref").contains("没有声明"));
    assert!(error("user:no-models").contains("至少要有一个模型"));

    manager.set_extension_config("user:my-relay", json!({"apiKey": "sk-test"}).as_object().unwrap()).unwrap();
    let providers = manager.enabled_extension_providers().unwrap();
    let relay = providers.iter().find(|(_, provider)| provider.id == "myrelay").unwrap();
    assert_eq!(relay.0, "my-relay");
    assert_eq!(relay.1.api_key, "sk-test", "Key 取自拓展配置");

    // 并入网关：已有同 id 的供应商（用户自己配的）优先，拓展的被跳过。
    let mut settings: GatewaySettings = serde_json::from_value(json!({})).unwrap_or_default();
    let mut existing = serde_json::from_value::<codex_plus_core::gateway::store::GatewayProvider>(json!({
        "relay": serde_json::to_value(codex_plus_core::settings::RelayProfile { id: "local".into(), name: "我自己的".into(), ..Default::default() }).unwrap()
    })).unwrap();
    existing.source = "user".into();
    settings.providers.push(existing);
    let merged = settings.with_extension_providers_from(&manager);
    let ids = merged.providers.iter().map(|provider| (provider.relay.id.as_str(), provider.source.as_str())).collect::<Vec<_>>();
    assert!(ids.contains(&("local", "user")), "已有同 id 的保留用户自己的：{ids:?}");
    assert!(ids.contains(&("myrelay", "extension:my-relay")));
    let myrelay = merged.providers.iter().find(|provider| provider.relay.id == "myrelay").unwrap();
    assert_eq!(myrelay.relay.protocol, codex_plus_core::settings::RelayProtocol::ChatCompletions);
    assert_eq!(myrelay.relay.model_list, "gpt-x\ngpt-y");
    assert_eq!(myrelay.relay.api_key, "sk-test");
    assert!(!myrelay.relay.no_auth);

    // 停用拓展后不再并入。
    manager.set_script_enabled("user:my-relay", false).unwrap();
    let again = GatewaySettings::default().with_extension_providers_from(&manager);
    assert!(!again.providers.iter().any(|provider| provider.relay.id == "myrelay"));
}

/// 拓展写了 `override: true` 的供应商顶替已有同 id 的（位置不变），停用拓展后恢复成用户自己的。
#[test]
fn providers_with_override_replace_the_existing_one() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "swap", json!({
        "providers": [{ "id": "local", "name": "拓展的", "baseUrl": "http://127.0.0.1:9000/v1", "models": ["tiny"], "override": true }]
    }));
    let mut settings: GatewaySettings = serde_json::from_value(json!({})).unwrap_or_default();
    for id in ["first", "local", "last"] {
        let mut provider = serde_json::from_value::<codex_plus_core::gateway::store::GatewayProvider>(json!({
            "relay": serde_json::to_value(codex_plus_core::settings::RelayProfile { id: id.into(), name: id.into(), ..Default::default() }).unwrap()
        })).unwrap();
        provider.source = "user".into();
        settings.providers.push(provider);
    }
    let merged = settings.clone().with_extension_providers_from(&manager);
    let ids = merged.providers.iter().map(|provider| (provider.relay.id.as_str(), provider.source.as_str())).collect::<Vec<_>>();
    assert_eq!(ids, [("first", "user"), ("local", "extension:swap"), ("last", "user")]);
    assert_eq!(merged.providers[1].relay.base_url, "http://127.0.0.1:9000/v1");
    manager.set_script_enabled("user:swap", false).unwrap();
    let restored = settings.with_extension_providers_from(&manager);
    assert_eq!(restored.providers[1].source, "user");
}
