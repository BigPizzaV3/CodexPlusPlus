//! #3161：网关里只发给官方账号的模型，没设窗口时用本机 Codex 官方缓存（models_cache.json）的窗口，
//! 不被内置兼容资料（给第三方中转用的 272000）压小；用户设过的窗口仍然优先，中转不受影响。

use codex_plus_core::gateway::apply::codex_gateway_model_catalog;
use codex_plus_core::gateway::store::GatewaySettings;
use serde_json::json;

struct CodexHome(Option<std::ffi::OsString>);
impl CodexHome {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("CODEX_HOME");
        unsafe { std::env::set_var("CODEX_HOME", path) };
        Self(previous)
    }
}
impl Drop for CodexHome {
    fn drop(&mut self) {
        match self.0.take() {
            Some(value) => unsafe { std::env::set_var("CODEX_HOME", value) },
            None => unsafe { std::env::remove_var("CODEX_HOME") },
        }
    }
}

fn gateway(providers: Vec<serde_json::Value>, windows: serde_json::Value) -> GatewaySettings {
    serde_json::from_value(json!({
        "version": 3,
        "bindings": { "codex": { "enabled": true, "model": "gpt-6.1-sol", "token": "local" } },
        "providers": providers.into_iter().map(|relay| json!({
            "relay": relay, "modelWindows": windows.clone()
        })).collect::<Vec<_>>(),
        "autoModels": false
    }))
    .unwrap()
}

fn official() -> serde_json::Value {
    json!({ "id": "chatgpt", "name": "ChatGPT", "relayMode": "official", "protocol": "responses",
        "model": "gpt-6.1-sol", "modelList": "gpt-6.1-sol\ngpt-6.1-terra" })
}

fn relay() -> serde_json::Value {
    json!({ "id": "relay", "name": "Relay", "relayMode": "pureApi", "protocol": "responses",
        "apiKey": "sk-test", "upstreamBaseUrl": "https://relay.example.test/v1",
        "model": "gpt-6.1-sol", "modelList": "gpt-6.1-sol" })
}

fn lines(gateway: &GatewaySettings) -> Vec<String> {
    let binding = gateway.bindings["codex"].clone();
    codex_gateway_model_catalog(gateway, &binding)
        .model_list
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn official_members_use_native_cache_windows_but_explicit_windows_win() {
    let _state = crate::common::IsolatedTestState::new();
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("models_cache.json"),
        json!({ "models": [
            { "slug": "gpt-6.1-sol", "context_window": 500_000u64, "max_context_window": 872_000u64,
              "effective_context_window_percent": 95 },
            { "slug": "gpt-6.1-terra", "context_window": 400_000u64 }
        ]})
        .to_string(),
    )
    .unwrap();
    let _home = CodexHome::set(home.path());

    // 只有官方账号：用原生窗口。
    let only_official = lines(&gateway(vec![official()], json!({})));
    assert!(only_official.contains(&"gpt-6.1-sol[500000]".to_string()), "{only_official:?}");
    assert!(only_official.contains(&"gpt-6.1-terra[400000]".to_string()), "{only_official:?}");

    // 用户在供应商池里显式设的窗口优先。
    let explicit = lines(&gateway(vec![official()], json!({ "gpt-6.1-sol": 300_000 })));
    assert!(explicit.contains(&"gpt-6.1-sol[300000]".to_string()), "{explicit:?}");

    // 只发给中转的模型不套官方账号的窗口，照旧交给内置资料。
    let relay_only = lines(&gateway(vec![relay()], json!({})));
    assert!(relay_only.contains(&"gpt-6.1-sol".to_string()), "{relay_only:?}");
}
