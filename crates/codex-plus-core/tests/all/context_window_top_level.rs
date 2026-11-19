//! #3168：模型单独设了 1M 窗口，Codex 里仍显示 128K。
//!
//! Codex 里顶层 `model_context_window` / `model_auto_compact_token_limit` 对所有模型生效，
//! 并压过模型目录里每个模型自己的值（真实 Codex CLI 实测）。顶层值来自通用配置或供应商级
//! 「上下文大小」时，有逐模型设置的那些模型必须以目录为准：顶层值折进目录作为其余模型的值，
//! 顶层键去掉。没有逐模型设置时顶层单值行为不变（见 relay_config 里 #2590 的用例）。

use codex_plus_core::gateway::apply::{
    ApplyContext, ProviderInput, create_provider, enable_binding, set_provider_auto_compact,
    set_provider_windows,
};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore, ProviderAutoCompact};
use codex_plus_core::relay_config::apply_relay_profile_files_to_home_with_context;
use codex_plus_core::settings::{BackendSettings, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use codex_plus_core::tools::claude::ClaudeContext;
use serde_json::Value;

fn catalog_of(home: &std::path::Path) -> (String, Value) {
    let config = std::fs::read_to_string(home.join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let pointer = parsed
        .get("model_catalog_json")
        .and_then(toml::Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("要写模型目录：{config}"));
    let path = if pointer.is_absolute() { pointer } else { home.join(pointer) };
    let catalog = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    (config, catalog)
}

fn model<'a>(catalog: &'a Value, slug: &str) -> &'a Value {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == slug)
        .unwrap_or_else(|| panic!("目录里没有 {slug}：{catalog}"))
}

#[test]
fn gateway_per_model_windows_win_over_common_config_top_level_limits() {
    let _state = crate::common::IsolatedTestState::new();
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let ctx = ApplyContext {
        gateway: GatewayStore::at(state.join("gateway.json")),
        claude: ClaudeContext {
            store_path: state.join("claude-providers.json"),
            claude_home: temp.path().join("claude"),
            backup_root: state.join("backups"),
        },
        grok_home: temp.path().join("grok"),
        codex_home: temp.path().join("codex"),
        settings: codex_plus_core::settings::SettingsStore::new(state.join("settings.json")),
        backup_root: state.join("backups"),
        port: 57412,
        agent_env: codex_plus_core::gateway::agents::AgentEnv::at(temp.path().join("home")),
    };
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    // 通用配置里留着旧的全局窗口与压缩阈值（例如从本机 config.toml 带过来的）。
    ctx.settings
        .save(&BackendSettings {
            relay_profiles_enabled: true,
            relay_common_config_contents:
                "model_context_window = 128000\nmodel_auto_compact_token_limit = 100000\n".into(),
            ..BackendSettings::default()
        })
        .unwrap();
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "系统默认".into(),
            base_url: "https://relay.example.test/v1".into(),
            protocol: RelayProtocol::Responses,
            keys: vec!["sk-x".into()],
            model: "gpt-6.1-sol".into(),
            model_list: "gpt-6.1-sol\ngpt-6-sol".into(),
            price: None,
            model_prices: None,
            proxy: None,
            strip_image_generation_tool: None,
            subscription_policy: None,
            strip_responses_image_detail: None,
            disable_tool_search: None,
            custom_headers: None,
        },
        "custom",
        None,
    )
    .unwrap();
    let id = ctx.gateway.load().unwrap().providers[0].relay.id.clone();
    // 只给 gpt-6.1-sol 单独设 1M 和 90%；gpt-6-sol 不设。
    set_provider_windows(&ctx.gateway, &id, [("gpt-6.1-sol".into(), 1_000_000)].into(), None)
        .unwrap();
    set_provider_auto_compact(
        &ctx.gateway,
        &id,
        ProviderAutoCompact {
            models: [("gpt-6.1-sol".to_string(), 90.0)].into(),
            fallback: None,
        },
    )
    .unwrap();
    enable_binding(
        &ctx,
        &ToolId::Codex,
        AgentBinding { model: "gpt-6.1-sol".into(), token: "t".into(), ..AgentBinding::default() },
    )
    .unwrap();

    let (config, catalog) = catalog_of(&ctx.codex_home);
    assert!(!config.contains("model_context_window"), "{config}");
    assert!(!config.contains("model_auto_compact_token_limit"), "{config}");
    let sol = model(&catalog, "gpt-6.1-sol");
    assert_eq!(sol["context_window"], 1_000_000);
    assert_eq!(sol["auto_compact_token_limit"], 900_000);
    // 没单独设的模型仍按原来的全局值。
    let other = model(&catalog, "gpt-6-sol");
    assert_eq!(other["context_window"], 128_000);
    assert_eq!(other["auto_compact_token_limit"], 100_000);
}

fn direct_profile(model_windows: &str, model_auto_compact: &str) -> RelayProfile {
    RelayProfile {
        id: "relay-a".into(),
        name: "Relay A".into(),
        protocol: RelayProtocol::Responses,
        config_contents: r#"model_provider = "custom"
model = "gpt-6.1-sol"

[model_providers.custom]
name = "custom"
base_url = "https://relay.example.test/v1"
wire_api = "responses"
experimental_bearer_token = "sk-new"
"#
        .into(),
        auth_contents: r#"{"OPENAI_API_KEY":"sk-new"}"#.into(),
        model_list: "gpt-6.1-sol\nother-model".into(),
        model_windows: model_windows.into(),
        model_auto_compact: model_auto_compact.into(),
        context_window: "128000".into(),
        auto_compact_limit: "100000".into(),
        ..RelayProfile::default()
    }
}

#[test]
fn direct_per_model_window_wins_over_provider_level_window() {
    let temp = tempfile::tempdir().unwrap();
    let profile = direct_profile(r#"{"gpt-6.1-sol":"1M"}"#, "");
    apply_relay_profile_files_to_home_with_context(temp.path(), &profile, "").unwrap();

    let (config, catalog) = catalog_of(temp.path());
    assert!(!config.contains("model_context_window"), "{config}");
    // 没有逐模型压缩阈值：顶层压缩值照旧（单值行为不变）。
    assert!(config.contains("model_auto_compact_token_limit = 100000"), "{config}");
    assert_eq!(model(&catalog, "gpt-6.1-sol")["context_window"], 1_000_000);
    assert_eq!(model(&catalog, "other-model")["context_window"], 128_000);
}

#[test]
fn direct_per_model_compact_percent_wins_over_provider_level_limit() {
    let temp = tempfile::tempdir().unwrap();
    let profile = direct_profile(r#"{"gpt-6.1-sol":"1M"}"#, r#"{"gpt-6.1-sol":"90%"}"#);
    apply_relay_profile_files_to_home_with_context(temp.path(), &profile, "").unwrap();

    let (config, catalog) = catalog_of(temp.path());
    assert!(!config.contains("model_context_window"), "{config}");
    assert!(!config.contains("model_auto_compact_token_limit"), "{config}");
    assert_eq!(model(&catalog, "gpt-6.1-sol")["auto_compact_token_limit"], 900_000);
    assert_eq!(model(&catalog, "other-model")["auto_compact_token_limit"], 100_000);
}
