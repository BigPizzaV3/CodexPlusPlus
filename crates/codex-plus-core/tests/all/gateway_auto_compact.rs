//! #3100：网关供应商池按模型设自动压缩阈值（窗口的百分比）。
//!
//! - 设了的模型：Codex 模型目录里 `auto_compact_token_limit = 窗口 × 百分比`；
//! - 没设的模型、一个都没设时：目录与原来一致（`auto_compact_token_limit` 为 null，交给 Codex 默认）；
//! - 多家都有的模型（自动组入口）取各家里最小的那个；
//! - 不合法的百分比在保存时拒绝，不写盘；旧版 gateway.json 没有这一项也能读。

use std::collections::BTreeMap;

use codex_plus_core::gateway::apply::{
    ApplyContext, ProviderInput, codex_gateway_model_catalog, codex_gateway_profile,
    create_provider, enable_binding, refresh_enabled, set_provider_auto_compact,
    set_provider_windows,
};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore, ProviderAutoCompact};
use codex_plus_core::settings::{BackendSettings, RelayProtocol};
use codex_plus_core::tools::ToolId;
use codex_plus_core::tools::claude::ClaudeContext;
use serde_json::Value;

struct Fixture {
    _temp: tempfile::TempDir,
    ctx: ApplyContext,
    id: String,
    /// gateway.json 的路径（直接读写原文用）。
    path: std::path::PathBuf,
}

fn fixture(port: u16) -> Fixture {
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
        port,
        agent_env: codex_plus_core::gateway::agents::AgentEnv::at(temp.path().join("home")),
    };
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    ctx.settings
        .save(&BackendSettings {
            relay_profiles_enabled: true,
            ..BackendSettings::default()
        })
        .unwrap();
    create_provider(&ctx.gateway, &deepseek_input(), "preset:deepseek", None).unwrap();
    let id = ctx.gateway.load().unwrap().providers[0].relay.id.clone();
    Fixture {
        _temp: temp,
        ctx,
        id,
        path: state.join("gateway.json"),
    }
}

fn deepseek_input() -> ProviderInput {
    ProviderInput {
        name: "DeepSeek".into(),
        base_url: "https://api.deepseek.test/v1".into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: vec!["sk-ds".into()],
        model: "deepseek-v4-pro".into(),
        model_list: "deepseek-v4-pro\ndeepseek-v4-flash".into(),
        price: None,
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
        subscription_policy: None,
        strip_responses_image_detail: None,
        strip_responses_output_encrypted_content: None,
        disable_tool_search: None,
        custom_headers: None,
    }
}

fn binding() -> AgentBinding {
    AgentBinding {
        model: "deepseek-v4-pro".into(),
        token: "gateway-token".into(),
        context_window: Some(1_000_000),
        ..AgentBinding::default()
    }
}

fn compact(models: &[(&str, f64)], fallback: Option<f64>) -> ProviderAutoCompact {
    ProviderAutoCompact {
        models: models
            .iter()
            .map(|(model, percent)| (model.to_string(), *percent))
            .collect(),
        fallback,
    }
}

/// 读 Codex 的模型目录：`config.toml` 里 `model_catalog_json` 指向的文件。
fn codex_catalog(ctx: &ApplyContext) -> Value {
    let config = std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let pointer = parsed
        .get("model_catalog_json")
        .and_then(toml::Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| panic!("网关供应商要写模型目录：{config}"));
    let path = if pointer.is_absolute() {
        pointer
    } else {
        ctx.codex_home.join(pointer)
    };
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn entry<'a>(catalog: &'a Value, slug: &str) -> &'a Value {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["slug"] == slug)
        .unwrap_or_else(|| panic!("目录里没有 {slug}：{catalog}"))
}

#[test]
fn percent_becomes_compact_limit_in_codex_catalog() {
    let _state = crate::common::IsolatedTestState::new();
    let Fixture { ctx, id, _temp, .. } = fixture(57401);
    set_provider_windows(
        &ctx.gateway,
        &id,
        [("deepseek-v4-pro".into(), 500_000)].into(),
        None,
    )
    .unwrap();
    set_provider_auto_compact(&ctx.gateway, &id, compact(&[("DeepSeek-V4-Pro", 85.0)], None))
        .unwrap();
    enable_binding(&ctx, &ToolId::Codex, binding()).unwrap();

    let config = std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    assert!(config.contains("model_catalog_json"), "{config}");
    // 按模型的阈值只进目录，不写对所有模型生效的顶层键。
    assert!(!config.contains("model_auto_compact_token_limit"), "{config}");
    let catalog = codex_catalog(&ctx);
    let pro = entry(&catalog, "deepseek-v4-pro");
    assert_eq!(pro["context_window"], 500_000);
    assert_eq!(pro["auto_compact_token_limit"], 425_000);
    // 没设的模型照旧交给 Codex 默认。
    assert!(entry(&catalog, "deepseek-v4-flash")["auto_compact_token_limit"].is_null());
}

#[test]
fn fallback_percent_covers_models_without_their_own() {
    let _state = crate::common::IsolatedTestState::new();
    let Fixture { ctx, id, _temp, .. } = fixture(57402);
    set_provider_auto_compact(
        &ctx.gateway,
        &id,
        compact(&[("deepseek-v4-pro", 70.0)], Some(80.0)),
    )
    .unwrap();
    enable_binding(&ctx, &ToolId::Codex, binding()).unwrap();

    let catalog = codex_catalog(&ctx);
    // 两个模型都用绑定的默认窗口 1M。
    assert_eq!(entry(&catalog, "deepseek-v4-pro")["auto_compact_token_limit"], 700_000);
    assert_eq!(entry(&catalog, "deepseek-v4-flash")["auto_compact_token_limit"], 800_000);
}

#[test]
fn unset_keeps_catalog_unchanged_and_clearing_restores_default() {
    let _state = crate::common::IsolatedTestState::new();
    let Fixture { ctx, id, path, _temp } = fixture(57403);
    let gateway = ctx.gateway.load().unwrap();
    let before = codex_gateway_model_catalog(&gateway, &binding());
    assert_eq!(before.model_auto_compact, "", "没设时不带任何压缩阈值");
    let profile = codex_gateway_profile(&gateway, &binding(), ctx.port, false).unwrap();
    assert_eq!(profile.model_auto_compact, "");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("\"autoCompact\":"), "没设时 gateway.json 不多写供应商级字段：{raw}");

    enable_binding(&ctx, &ToolId::Codex, binding()).unwrap();
    let catalog = codex_catalog(&ctx);
    for model in catalog["models"].as_array().unwrap() {
        assert!(model["auto_compact_token_limit"].is_null(), "{model}");
    }

    // 设上再清掉：回到 Codex 默认。
    set_provider_auto_compact(&ctx.gateway, &id, compact(&[("deepseek-v4-pro", 85.5)], None))
        .unwrap();
    refresh_enabled(&ctx).unwrap();
    assert_eq!(
        entry(&codex_catalog(&ctx), "deepseek-v4-pro")["auto_compact_token_limit"],
        855_000
    );
    set_provider_auto_compact(&ctx.gateway, &id, ProviderAutoCompact::default()).unwrap();
    refresh_enabled(&ctx).unwrap();
    assert!(entry(&codex_catalog(&ctx), "deepseek-v4-pro")["auto_compact_token_limit"].is_null());
    // 只检查供应商一级的字段：relay 里的 `autoCompactLimit`（直连通道的压缩上限）与这里无关。
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("\"autoCompact\":"), "{raw}");
}

#[test]
fn shared_model_auto_entry_uses_smallest_member_percent() {
    let _state = crate::common::IsolatedTestState::new();
    let Fixture { ctx, id, _temp, .. } = fixture(57404);
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "Mirror".into(),
            base_url: "https://api.mirror.test/v1".into(),
            ..deepseek_input()
        },
        "",
        None,
    )
    .unwrap();
    let mirror = ctx.gateway.load().unwrap().providers[1].relay.id.clone();
    set_provider_auto_compact(&ctx.gateway, &id, compact(&[("deepseek-v4-pro", 90.0)], None))
        .unwrap();
    set_provider_auto_compact(&ctx.gateway, &mirror, compact(&[("deepseek-v4-pro", 72.0)], None))
        .unwrap();

    let gateway = ctx.gateway.load().unwrap();
    let catalog = codex_gateway_model_catalog(&gateway, &binding());
    let map: BTreeMap<String, String> = serde_json::from_str(&catalog.model_auto_compact).unwrap();
    assert_eq!(map.get(&format!("{id}/deepseek-v4-pro")).map(String::as_str), Some("90%"));
    assert_eq!(map.get(&format!("{mirror}/deepseek-v4-pro")).map(String::as_str), Some("72%"));
    // 多家都有的裸名（自动切换入口）取最小值。
    assert_eq!(map.get("deepseek-v4-pro").map(String::as_str), Some("72%"), "{map:?}");
    assert!(!map.keys().any(|model| model.contains("flash")), "{map:?}");
}

#[test]
fn invalid_percent_is_rejected_without_writing() {
    let Fixture { ctx, id, _temp, .. } = fixture(57405);
    set_provider_auto_compact(&ctx.gateway, &id, compact(&[("deepseek-v4-pro", 80.0)], None))
        .unwrap();
    for bad in [
        compact(&[("deepseek-v4-pro", 0.0)], None),
        compact(&[("deepseek-v4-pro", 100.5)], None),
        compact(&[("deepseek-v4-pro", f64::NAN)], None),
        compact(&[], Some(-5.0)),
    ] {
        assert!(set_provider_auto_compact(&ctx.gateway, &id, bad).is_err());
    }
    let saved = ctx.gateway.load().unwrap();
    assert_eq!(saved.providers[0].auto_compact, compact(&[("deepseek-v4-pro", 80.0)], None));
    // 保留两位小数、去掉模型名两端空白。
    let saved = set_provider_auto_compact(
        &ctx.gateway,
        &id,
        compact(&[(" deepseek-v4-pro ", 84.329412)], Some(100.0)),
    )
    .unwrap();
    assert_eq!(
        saved.providers[0].auto_compact,
        compact(&[("deepseek-v4-pro", 84.33)], Some(100.0))
    );
    assert!(set_provider_auto_compact(&ctx.gateway, "missing", ProviderAutoCompact::default()).is_err());
}

#[test]
fn legacy_gateway_file_without_field_still_loads() {
    let Fixture { ctx, id, path, _temp } = fixture(57406);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("\"autoCompact\":"));
    let gateway = ctx.gateway.load().unwrap();
    let provider = gateway.provider(&id).unwrap();
    assert!(provider.auto_compact.is_empty());
    assert_eq!(provider.auto_compact_for("deepseek-v4-pro"), None);

    // 新字段的写法：按模型 + 默认值，读回来按模型优先、忽略大小写和窗口后缀。
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    value["providers"][0]["autoCompact"] =
        serde_json::json!({ "models": { "deepseek-v4-pro[1M]": 75 }, "fallback": 88.5 });
    std::fs::write(&path, value.to_string()).unwrap();
    let gateway = ctx.gateway.load().unwrap();
    let provider = gateway.provider(&id).unwrap();
    assert_eq!(provider.auto_compact_for("DEEPSEEK-V4-PRO"), Some(75.0));
    assert_eq!(provider.auto_compact_for("deepseek-v4-flash"), Some(88.5));
}
