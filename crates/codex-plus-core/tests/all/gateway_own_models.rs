//! Agent 自带的模型（直连，不经 Codex++）：列出哪些、直连的是谁，选中后断开网关并写进 Agent 自己的配置。

use codex_plus_core::gateway::apply::{ApplyContext, ProviderInput, create_provider, enable_binding};
use codex_plus_core::gateway::own_models::{own_models, set_shown, use_own_model};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore};
use codex_plus_core::settings::{BackendSettings, RelayMode, RelayProfile, RelayProtocol};
use codex_plus_core::tools::ToolId;
use codex_plus_core::tools::claude::{self, ClaudeContext, ClaudeProvider, GATEWAY_PROVIDER_ID};
use serde_json::{Value, json};

struct Fixture {
    _temp: tempfile::TempDir,
    ctx: ApplyContext,
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
    std::fs::create_dir_all(&ctx.claude.claude_home).unwrap();
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.test/v1".into(),
            protocol: RelayProtocol::ChatCompletions,
            keys: vec!["sk-ds".into()],
            model: "deepseek-v4-pro".into(),
            model_list: "deepseek-v4-pro".into(),
            price: None,
            model_prices: None,
            proxy: None,
            strip_image_generation_tool: None,
        },
        "preset:deepseek",
        None,
    )
    .unwrap();
    Fixture { _temp: temp, ctx }
}

fn binding() -> AgentBinding {
    AgentBinding {
        model: "deepseek-v4-pro".into(),
        ..AgentBinding::default()
    }
}

fn read_json(path: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// 直连模式里一个第三方供应商，两个模型，当前用它。
fn seed_codex_direct(ctx: &ApplyContext) {
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "我的中转",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "gpt-direct",
        "gpt-direct\ngpt-other",
        &[],
    )
    .unwrap();
    direct.id = "direct".into();
    direct.relay_mode = RelayMode::PureApi;
    codex_plus_core::relay_config::normalize_relay_profile_for_storage(&mut direct).unwrap();
    let mut seeded = BackendSettings::default();
    seeded.relay_profiles_enabled = true;
    seeded.relay_profiles = vec![direct];
    seeded.active_relay_id = "direct".into();
    codex_plus_core::relay_switch::switch_relay_profile_in_home(
        &ctx.settings,
        &ctx.codex_home,
        seeded,
        "",
    )
    .unwrap();
}

#[test]
fn codex_own_models_are_the_direct_provider_and_picking_one_disconnects_the_gateway() {
    let fixture = fixture(57411);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    let config = || std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();

    let own = own_models(ctx, &ToolId::Codex, None).expect("直连供应商有模型");
    assert_eq!(own.vendor, "我的中转", "直连的是直连模式的那个供应商");
    let ids: Vec<&str> = own.models.iter().map(|model| model.id.as_str()).collect();
    assert_eq!(ids, ["gpt-direct", "gpt-other"]);
    assert_eq!(own.current, "gpt-direct", "没接入时打勾的是 Codex 现在用的模型");

    let gateway = enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    assert!(config().contains("/gw/codex"), "{}", config());
    let own = own_models(ctx, &ToolId::Codex, gateway.binding(&ToolId::Codex)).unwrap();
    assert_eq!(own.vendor, "我的中转", "接入着时列的是断开后会回到的供应商");
    assert_eq!(own.current, "", "接入着时没有自带模型打勾");

    let settings = ctx.settings.load().unwrap();
    let gateway = use_own_model(ctx, &settings, &ToolId::Codex, "gpt-other").unwrap();
    assert!(!gateway.binding(&ToolId::Codex).unwrap().enabled, "选自带模型 = 断开网关");
    assert_eq!(ctx.settings.load().unwrap().active_relay_id, "direct");
    let live = config();
    assert!(!live.contains("/gw/codex"), "{live}");
    assert!(live.contains("api.direct.test"), "{live}");
    let doc: toml::Value = toml::from_str(&live).unwrap();
    assert_eq!(doc.get("model").and_then(|model| model.as_str()), Some("gpt-other"), "{live}");
    assert_eq!(own_models(ctx, &ToolId::Codex, None).unwrap().current, "gpt-other");
}

#[test]
fn codex_on_official_login_lists_the_cached_chatgpt_models_as_openai() {
    let fixture = fixture(57412);
    let ctx = &fixture.ctx;
    let official = RelayProfile {
        id: "official".into(),
        name: "ChatGPT".into(),
        relay_mode: RelayMode::Official,
        ..RelayProfile::default()
    };
    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![official];
    settings.active_relay_id = "official".into();
    ctx.settings.save(&settings).unwrap();
    std::fs::write(
        ctx.codex_home.join("models_cache.json"),
        json!({ "models": [
            { "slug": "gpt-6-sol", "display_name": "GPT-6-Sol", "visibility": "list" },
            { "slug": "codex-auto-review", "display_name": "Auto review", "visibility": "hide" },
            { "slug": "gpt-6-luna" }
        ]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(ctx.codex_home.join("config.toml"), "model = \"gpt-6-luna\"\n").unwrap();

    let own = own_models(ctx, &ToolId::Codex, None).unwrap();
    assert_eq!(own.vendor, "OpenAI");
    let listed: Vec<(&str, &str)> = own
        .models
        .iter()
        .map(|model| (model.id.as_str(), model.label.as_str()))
        .collect();
    assert_eq!(listed, [("gpt-6-sol", "GPT-6-Sol"), ("gpt-6-luna", "gpt-6-luna")], "隐藏的不列");
    assert_eq!(own.current, "gpt-6-luna");
}

#[test]
fn claude_own_models_follow_its_own_base_url_and_picking_one_restores_and_sets_the_model() {
    let fixture = fixture(57413);
    let ctx = &fixture.ctx;
    claude::save_provider(
        &ctx.claude,
        ClaudeProvider {
            id: "kimi".into(),
            name: "Kimi".into(),
            website_url: String::new(),
            notes: String::new(),
            settings_config: json!({ "env": {
                "ANTHROPIC_BASE_URL": "https://api.kimi.test/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "sk-kimi",
                "ANTHROPIC_MODEL": "kimi-k2.5"
            }}),
            created_at: 0,
            ccs_source_id: String::new(),
        },
    )
    .unwrap();
    claude::switch_provider(&ctx.claude, "kimi").unwrap();

    let own = own_models(ctx, &ToolId::Claude, None).unwrap();
    assert_eq!(own.vendor, "api.kimi.test", "自己配了地址：直连的是那个主机");
    assert!(own.models.iter().any(|model| model.id == "sonnet"));

    let gateway = enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), GATEWAY_PROVIDER_ID);
    let own = own_models(ctx, &ToolId::Claude, gateway.binding(&ToolId::Claude)).unwrap();
    assert_eq!(own.vendor, "api.kimi.test", "接入着时看断开后会回到的地址");
    assert_eq!(own.current, "");

    let gateway = use_own_model(ctx, &BackendSettings::default(), &ToolId::Claude, "sonnet").unwrap();
    assert!(!gateway.binding(&ToolId::Claude).unwrap().enabled);
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), "kimi");
    let live = read_json(&ctx.claude.claude_home.join("settings.json"));
    assert_eq!(live["model"], "sonnet");
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.test/anthropic");
    assert_eq!(live["env"]["ANTHROPIC_MODEL"], "sonnet", "供应商指定的模型优先，一起改掉才生效");
}

#[test]
fn claude_without_its_own_address_goes_to_anthropic_and_keeps_other_settings() {
    let fixture = fixture(57414);
    let ctx = &fixture.ctx;
    let path = ctx.claude.claude_home.join("settings.json");
    std::fs::write(&path, json!({ "permissions": { "allow": ["Bash"] }, "model": "claude-opus-4-5-20251101" }).to_string()).unwrap();

    let own = own_models(ctx, &ToolId::Claude, None).unwrap();
    assert_eq!(own.vendor, "Anthropic");
    assert_eq!(own.models[0].id, "claude-opus-4-5-20251101", "现在用的具体模型排在最前面");
    assert_eq!(own.current, "claude-opus-4-5-20251101");

    use_own_model(ctx, &BackendSettings::default(), &ToolId::Claude, "opus[1m]").unwrap();
    let live = read_json(&path);
    assert_eq!(live["model"], "opus[1m]");
    assert_eq!(live["permissions"]["allow"][0], "Bash", "别的键不动");
    assert!(live.get("env").is_none(), "没有的 env 不凭空加");
}

#[test]
fn agents_without_own_models_are_refused() {
    let fixture = fixture(57415);
    let ctx = &fixture.ctx;
    assert_eq!(own_models(ctx, &ToolId::Grok, None), None);
    assert!(use_own_model(ctx, &BackendSettings::default(), &ToolId::Grok, "grok-4").is_err());
    assert!(use_own_model(ctx, &BackendSettings::default(), &ToolId::Claude, "  ").is_err());
    // Codex 直连模式里没有供应商：断开后无处可回，不列。
    assert_eq!(own_models(ctx, &ToolId::Codex, None), None);
}

#[test]
fn own_model_switch_is_per_agent_and_saved_in_the_gateway_store() {
    let fixture = fixture(57416);
    let ctx = &fixture.ctx;
    assert!(ctx.gateway.load().unwrap().shows_own_models_for(&ToolId::Codex), "默认列出");

    set_shown(&ctx.gateway, &ToolId::Codex, false).unwrap();
    set_shown(&ctx.gateway, &ToolId::Codex, false).unwrap();
    let gateway = ctx.gateway.load().unwrap();
    assert!(!gateway.shows_own_models_for(&ToolId::Codex));
    assert!(gateway.shows_own_models_for(&ToolId::Claude), "只关 Codex 的");
    assert_eq!(gateway.hide_own_models, ["codex"], "重复关不重复记");
    let raw: Value = read_json(&fixture._temp.path().join("state").join("gateway.json"));
    assert_eq!(raw["hideOwnModels"], json!(["codex"]));

    set_shown(&ctx.gateway, &ToolId::Codex, true).unwrap();
    let raw: Value = read_json(&fixture._temp.path().join("state").join("gateway.json"));
    assert!(raw.get("hideOwnModels").is_none(), "都列时不落盘");
}
