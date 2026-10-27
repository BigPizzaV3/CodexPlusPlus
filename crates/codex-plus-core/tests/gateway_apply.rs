//! 网关模式开启 / 关闭对 Agent 配置文件的改写与恢复，以及网关自己的供应商池、路由组。

use codex_plus_core::gateway::apply::{
    ApplyContext, GroupInput, ProviderInput, codex_importable, copy_claude_provider,
    copy_codex_provider, create_provider, delete_group, delete_provider, disable_binding,
    enable_binding, note_claude_switched, note_codex_switched, reapply_if_port_changed,
    refresh_enabled, reorder_providers, save_group, set_provider_enabled, update_provider,
};
use codex_plus_core::gateway::store::{
    AgentBinding, ClaudeRoleModels, GatewayStore, GroupMember, GroupMode,
};
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
    };
    std::fs::create_dir_all(&ctx.claude.claude_home).unwrap();
    std::fs::create_dir_all(&ctx.grok_home).unwrap();
    // 网关池里放一个供应商；直连模式的 settings 与它无关。
    create_provider(&ctx.gateway, &deepseek_input(), "preset:deepseek").unwrap();
    Fixture { _temp: temp, ctx }
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
        proxy: None,
    }
}

/// 直连模式的设置：只用于关闭网关时恢复 Grok 原来的供应商。
fn settings() -> BackendSettings {
    BackendSettings::default()
}

fn binding() -> AgentBinding {
    AgentBinding {
        model: "deepseek-v4-pro".into(),
        claude_roles: ClaudeRoleModels {
            haiku: "deepseek-v4-flash".into(),
            ..ClaudeRoleModels::default()
        },
        context_window: Some(1_000_000),
        ..AgentBinding::default()
    }
}

fn read_settings_json(ctx: &ApplyContext) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(ctx.claude.claude_home.join("settings.json")).unwrap(),
    )
    .unwrap()
}

fn seed_claude_provider(ctx: &ApplyContext) {
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
}

#[test]
fn claude_enable_points_at_gateway_and_disable_restores_previous_provider() {
    let fixture = fixture(57399);
    let ctx = &fixture.ctx;
    // 用户自己的键：网关开关都不能动它。
    std::fs::write(
        ctx.claude.claude_home.join("settings.json"),
        r#"{ "permissions": { "allow": ["Bash(ls)"] }, "env": { "MY_FLAG": "1" } }"#,
    )
    .unwrap();
    seed_claude_provider(ctx);

    let gateway = enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    let saved = gateway.binding(&ToolId::Claude).unwrap();
    assert!(saved.enabled);
    assert!(saved.token.starts_with("cpx-gw-claude-"));
    assert_eq!(saved.restore.claude_previous_provider_id, "kimi");
    assert_eq!(saved.restore.applied_port, Some(57399));

    let live = read_settings_json(ctx);
    let env = &live["env"];
    assert_eq!(
        env["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:57399/gw/claude"
    );
    assert_eq!(env["ANTHROPIC_AUTH_TOKEN"], saved.token.as_str());
    assert_eq!(env["ANTHROPIC_MODEL"], "deepseek-v4-pro");
    assert_eq!(
        codex_plus_core::gateway::drift::check(ctx, &ToolId::Claude),
        None,
        "刚接入不算被改"
    );
    assert_eq!(env["ANTHROPIC_DEFAULT_SONNET_MODEL"], "deepseek-v4-pro");
    assert_eq!(env["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "deepseek-v4-flash");
    assert_eq!(env["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "1000000");
    assert_eq!(env["MY_FLAG"], "1");
    assert_eq!(live["permissions"]["allow"][0], "Bash(ls)");
    assert_eq!(
        claude::current_provider_id(&ctx.claude).unwrap(),
        GATEWAY_PROVIDER_ID
    );

    // 再次保存（改模型）不换 token、不丢恢复现场。
    let mut updated = binding();
    updated.model = "deepseek-v4-flash".into();
    let gateway = enable_binding(ctx, &ToolId::Claude, updated).unwrap();
    let again = gateway.binding(&ToolId::Claude).unwrap();
    assert_eq!(again.token, saved.token);
    assert_eq!(again.restore.claude_previous_provider_id, "kimi");
    assert_eq!(
        read_settings_json(ctx)["env"]["ANTHROPIC_MODEL"],
        "deepseek-v4-flash"
    );

    let gateway = disable_binding(ctx, &settings(), &ToolId::Claude).unwrap();
    assert!(!gateway.binding(&ToolId::Claude).unwrap().enabled);
    let live = read_settings_json(ctx);
    assert_eq!(
        live["env"]["ANTHROPIC_BASE_URL"],
        "https://api.kimi.test/anthropic"
    );
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-kimi");
    assert!(live["env"].get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none());
    assert!(live["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none());
    assert_eq!(live["env"]["MY_FLAG"], "1");
    assert_eq!(live["permissions"]["allow"][0], "Bash(ls)");
    let payload = claude::load(&ctx.claude).unwrap();
    assert_eq!(payload.current_id, "kimi");
    assert!(
        payload
            .providers
            .iter()
            .all(|provider| provider.id != GATEWAY_PROVIDER_ID)
    );
}

#[test]
fn claude_without_previous_provider_snapshots_live_config_and_restores_it() {
    let fixture = fixture(57398);
    let ctx = &fixture.ctx;
    std::fs::write(
        ctx.claude.claude_home.join("settings.json"),
        r#"{ "env": { "ANTHROPIC_BASE_URL": "https://hand.test", "ANTHROPIC_AUTH_TOKEN": "sk-hand" } }"#,
    )
    .unwrap();
    let gateway = enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    assert!(
        !gateway
            .binding(&ToolId::Claude)
            .unwrap()
            .restore
            .claude_previous_provider_id
            .is_empty()
    );
    disable_binding(ctx, &settings(), &ToolId::Claude).unwrap();
    let live = read_settings_json(ctx);
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://hand.test");
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-hand");
}

#[test]
fn claude_with_nothing_configured_is_cleared_on_disable() {
    let fixture = fixture(57397);
    let ctx = &fixture.ctx;
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    disable_binding(ctx, &settings(), &ToolId::Claude).unwrap();
    let live = read_settings_json(ctx);
    let env = live
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    assert!(
        env.keys().all(|key| !key.starts_with("ANTHROPIC_")),
        "{live}"
    );
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), "");
}

#[test]
fn switching_claude_provider_directly_turns_binding_off() {
    let fixture = fixture(57396);
    let ctx = &fixture.ctx;
    seed_claude_provider(ctx);
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    claude::switch_provider(&ctx.claude, "kimi").unwrap();
    note_claude_switched(&ctx.gateway, "kimi").unwrap();
    assert!(
        !ctx.gateway
            .load()
            .unwrap()
            .binding(&ToolId::Claude)
            .unwrap()
            .enabled
    );
}

#[test]
fn port_change_rewrites_enabled_agents() {
    let fixture = fixture(57395);
    enable_binding(&fixture.ctx, &ToolId::Claude, binding()).unwrap();
    let mut moved = fixture.ctx.clone();
    moved.port = 58001;
    assert_eq!(reapply_if_port_changed(&moved).unwrap(), 1);
    assert_eq!(
        read_settings_json(&moved)["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:58001/gw/claude"
    );
    assert_eq!(reapply_if_port_changed(&moved).unwrap(), 0);
}

#[test]
fn grok_enable_writes_gateway_endpoint_and_disable_restores_profile() {
    let fixture = fixture(57394);
    let ctx = &fixture.ctx;
    let mut settings = settings();
    let grok_profile = RelayProfile {
        id: "grok-direct".into(),
        name: "直连".into(),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::ChatCompletions,
        upstream_base_url: "https://api.x.test/v1".into(),
        api_key: "sk-x".into(),
        model_list: "grok-5".into(),
        ..RelayProfile::default()
    };
    let mut shard = settings.tool_config(&ToolId::Grok);
    shard.relay_profiles = vec![grok_profile];
    shard.active_relay_id = "grok-direct".into();
    settings.set_tool_config(&ToolId::Grok, shard);

    let gateway = enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    let token = gateway.binding(&ToolId::Grok).unwrap().token.clone();
    let config = std::fs::read_to_string(ctx.grok_home.join("config.toml")).unwrap();
    assert!(
        config.contains("http://127.0.0.1:57394/gw/grok/v1"),
        "{config}"
    );
    assert!(config.contains("api_backend = \"responses\""), "{config}");
    assert!(config.contains(&token), "{config}");
    assert!(config.contains("deepseek-v4-pro"), "{config}");
    assert_eq!(
        codex_plus_core::gateway::drift::check(ctx, &ToolId::Grok),
        None,
        "刚接入不算被改"
    );

    disable_binding(ctx, &settings, &ToolId::Grok).unwrap();
    let config = std::fs::read_to_string(ctx.grok_home.join("config.toml")).unwrap();
    assert!(config.contains("https://api.x.test/v1"), "{config}");
    assert!(!config.contains("/gw/grok"), "{config}");
}

/// Codex：走直连模式自己的切换流程，托管供应商进出列表，断开时切回原来的供应商。
#[test]
fn codex_enable_switches_to_managed_gateway_profile_and_disable_switches_back() {
    let fixture = fixture(57395);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "直连",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "gpt-direct",
        "gpt-direct",
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
    let config = || std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    assert!(config().contains("api.direct.test"), "{}", config());

    let gateway = enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let saved = gateway.binding(&ToolId::Codex).unwrap();
    assert_eq!(saved.restore.codex_previous_relay_id, "direct");
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "codex-plus-gateway");
    assert!(
        config().contains("http://127.0.0.1:57395/gw/codex/v1"),
        "{}",
        config()
    );
    assert!(
        settings
            .relay_profiles
            .iter()
            .any(|profile| profile.id == "direct"),
        "原来的供应商还在"
    );
    assert_eq!(
        codex_plus_core::gateway::drift::check(ctx, &ToolId::Codex),
        None,
        "刚接入不算被改"
    );

    // 池变了再写一次：仍在网关上，回填的是托管供应商自己。
    refresh_enabled(ctx).unwrap();
    assert_eq!(
        ctx.settings.load().unwrap().active_relay_id,
        "codex-plus-gateway"
    );

    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "direct");
    assert!(
        !settings
            .relay_profiles
            .iter()
            .any(|profile| profile.id == "codex-plus-gateway"),
        "托管供应商已删除"
    );
    assert!(config().contains("api.direct.test"), "{}", config());
    assert!(!config().contains("/gw/codex"), "{}", config());

    // 在 Codex 供应商页直接切走：绑定随之断开。
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    note_codex_switched(&ctx.gateway, "direct", "codex-plus-gateway").unwrap();
    assert!(
        !ctx.gateway
            .load()
            .unwrap()
            .binding(&ToolId::Codex)
            .unwrap()
            .enabled
    );
    // 托管供应商还在列表里：在 Codex 供应商页选回它，等于重新接入。
    note_codex_switched(&ctx.gateway, "codex-plus-gateway", "direct").unwrap();
    let saved = ctx.gateway.load().unwrap();
    let binding = saved.binding(&ToolId::Codex).unwrap();
    assert!(binding.enabled);
    assert_eq!(binding.restore.codex_previous_relay_id, "direct");
}

/// Codex 的实时配置被别的工具改过（直连模式里切换会报「实时配置冲突」）：
/// 照样能接入网关，断开时把接入前的文件原样写回。
#[test]
fn codex_with_externally_edited_config_connects_and_restores_it_byte_for_byte() {
    let fixture = fixture(57396);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "直连",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "gpt-direct",
        "gpt-direct",
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
    // 别的工具把 Codex 指到了自己那里。
    let path = ctx.codex_home.join("config.toml");
    let edited = std::fs::read_to_string(&path).unwrap().replace(
        "https://api.direct.test/v1",
        "http://127.0.0.1:3425/other-tool",
    );
    std::fs::write(&path, &edited).unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let config = std::fs::read_to_string(&path).unwrap();
    assert!(config.contains("/gw/codex/v1"), "{config}");
    assert_eq!(
        ctx.settings.load().unwrap().active_relay_id,
        "codex-plus-gateway"
    );
    assert!(
        ctx.backup_root.join("codex-before-gateway.json").exists(),
        "实时配置对不上时存了原样副本"
    );

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited, "原样还原");
    assert!(!ctx.backup_root.join("codex-before-gateway.json").exists());
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "direct");
    assert!(
        !settings
            .relay_profiles
            .iter()
            .any(|profile| profile.id == "codex-plus-gateway")
    );
}

/// Codex 用 ChatGPT 账号登录着：接入网关后保留官方登录身份（内置 OpenAI 供应商 + 登录 token），
/// 请求经本地代理（openai_base_url）转给网关；断开后回到原来的供应商。
#[test]
fn signed_in_codex_keeps_its_chatgpt_identity_through_the_gateway() {
    let fixture = fixture(57397);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "直连",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "gpt-direct",
        "gpt-direct",
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
    // Codex 自己登录着 ChatGPT。
    std::fs::write(
        ctx.codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x","access_token":"at-codex","refresh_token":"rt","account_id":"acc"}}"#,
    )
    .unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let config = std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    assert!(config.contains("model_provider = \"openai\""), "{config}");
    assert!(config.contains("openai_base_url"), "{config}");
    assert!(
        config.contains("http://127.0.0.1:57397/gw/codex/v1"),
        "{config}"
    );
    let auth: Value =
        serde_json::from_str(&std::fs::read_to_string(ctx.codex_home.join("auth.json")).unwrap())
            .unwrap();
    assert_eq!(auth["tokens"]["access_token"], "at-codex", "官方登录保留");
    assert_eq!(
        codex_plus_core::gateway::drift::check(ctx, &ToolId::Codex),
        None,
        "刚接入不算被改"
    );
    let settings = ctx.settings.load().unwrap();
    assert_eq!(
        settings.active_relay_session_provider(),
        codex_plus_core::settings::RelaySessionProvider::Openai
    );

    // 身份已一致：启动时不重写。退出 ChatGPT 登录后：重写成普通的自定义供应商。
    assert!(!codex_plus_core::gateway::apply::sync_codex_identity(ctx).unwrap());
    std::fs::write(
        ctx.codex_home.join("auth.json"),
        r#"{"OPENAI_API_KEY":"sk-x"}"#,
    )
    .unwrap();
    assert!(codex_plus_core::gateway::apply::sync_codex_identity(ctx).unwrap());
    assert_eq!(
        ctx.settings.load().unwrap().active_relay_session_provider(),
        codex_plus_core::settings::RelaySessionProvider::Custom
    );

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let config = std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    assert!(!config.contains("/gw/codex"), "{config}");
    assert_eq!(ctx.settings.load().unwrap().active_relay_id, "direct");
}

/// Codex 自己没登录、池里有网关登录的 ChatGPT 账号：接入时借给 Codex 登录（官方身份），
/// 借出期间以 Codex 那份 token 为准；断开时把最新一代收回网关，Codex 回到没登录的样子。
#[test]
fn pool_chatgpt_account_is_lent_to_codex_and_taken_back() {
    let fixture = fixture(57398);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut direct = codex_plus_core::provider_import::pool_relay_profile(
        "直连",
        "https://api.direct.test/v1",
        "sk-direct",
        RelayProtocol::Responses,
        "gpt-direct",
        "gpt-direct",
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
    let auth_path = ctx.codex_home.join("auth.json");
    let read_auth = || -> Value {
        serde_json::from_str(&std::fs::read_to_string(&auth_path).unwrap_or_default())
            .unwrap_or(Value::Null)
    };
    assert!(read_auth().get("tokens").is_none(), "Codex 本来没登录");

    // 网关里登录的 ChatGPT 账号。
    let chatgpt = RelayProfile {
        name: "ChatGPT Pro".into(),
        relay_mode: RelayMode::Official,
        protocol: RelayProtocol::Responses,
        auth_contents: json!({
            "auth_mode": "chatgpt",
            "codex_plus_gateway": true,
            "tokens": { "id_token": "idt", "access_token": "at-1", "refresh_token": "rt-1", "account_id": "acc-pool" }
        })
        .to_string(),
        model_list: "gpt-6.1-sol".into(),
        ..RelayProfile::default()
    };
    let (_, pool_id) =
        codex_plus_core::gateway::apply::add_provider(&ctx.gateway, chatgpt, "login:chatgpt")
            .unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let auth = read_auth();
    assert_eq!(auth["auth_mode"], "chatgpt");
    assert_eq!(auth["tokens"]["account_id"], "acc-pool");
    assert_eq!(auth["tokens"]["refresh_token"], "rt-1");
    assert_eq!(auth["tokens"]["id_token"], "idt");
    assert_eq!(
        ctx.settings.load().unwrap().active_relay_session_provider(),
        codex_plus_core::settings::RelaySessionProvider::Openai,
        "借到登录后以官方身份接入"
    );
    assert_eq!(
        codex_plus_core::gateway::drift::check(ctx, &ToolId::Codex),
        None,
        "刚接入不算被改"
    );
    let lent = ctx.gateway.load().unwrap();
    assert!(codex_plus_core::official_login::is_lent_to_codex(
        &lent.provider(&pool_id).unwrap().relay
    ));

    // 借出期间 Codex 自己刷新了 token。
    let mut refreshed = read_auth();
    refreshed["tokens"]["access_token"] = json!("at-2");
    refreshed["tokens"]["refresh_token"] = json!("rt-2");
    std::fs::write(&auth_path, refreshed.to_string()).unwrap();

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let back = ctx.gateway.load().unwrap();
    let relay = &back.provider(&pool_id).unwrap().relay;
    assert!(!codex_plus_core::official_login::is_lent_to_codex(relay));
    let pool_auth: Value = serde_json::from_str(&relay.auth_contents).unwrap();
    assert_eq!(pool_auth["tokens"]["refresh_token"], "rt-2", "收回最新一代");
    assert_eq!(pool_auth["codex_plus_gateway"], true);
    assert!(read_auth().get("tokens").is_none(), "Codex 回到没登录");
    assert_eq!(ctx.settings.load().unwrap().active_relay_id, "direct");
}

/// 网关供应商的独立代理存在 gateway.json，载入后只用它（不按 id 查直连模式的代理表）；
/// 编辑时不传表示不改，空串表示清掉，写错的地址拦下。
#[test]
fn gateway_provider_proxy_is_its_own_setting() {
    let fixture = fixture(57400);
    let ctx = &fixture.ctx;
    let mut input = deepseek_input();
    input.name = "代理测试".into();
    input.proxy = Some(" http://127.0.0.1:7890 ".into());
    let (_, id) = create_provider(&ctx.gateway, &input, "preset:deepseek").unwrap();
    let loaded = ctx.gateway.load().unwrap();
    let provider = loaded.provider(&id).unwrap();
    assert_eq!(provider.proxy, "http://127.0.0.1:7890");
    assert_eq!(
        provider.relay.gateway_proxy.as_deref(),
        Some("http://127.0.0.1:7890")
    );
    assert_eq!(
        codex_plus_core::relay_proxy::proxy_for_relay(&provider.relay).as_deref(),
        Some("http://127.0.0.1:7890")
    );

    input.proxy = None;
    update_provider(&ctx.gateway, &id, &input).unwrap();
    assert_eq!(
        ctx.gateway.load().unwrap().provider(&id).unwrap().proxy,
        "http://127.0.0.1:7890",
        "不传不改"
    );
    input.proxy = Some(String::new());
    update_provider(&ctx.gateway, &id, &input).unwrap();
    let cleared = ctx.gateway.load().unwrap();
    assert_eq!(cleared.provider(&id).unwrap().proxy, "");
    assert_eq!(
        cleared
            .provider(&id)
            .unwrap()
            .relay
            .gateway_proxy
            .as_deref(),
        Some("")
    );
    input.proxy = Some("127.0.0.1:7890".into());
    assert!(
        update_provider(&ctx.gateway, &id, &input).is_err(),
        "缺协议头的地址拦下"
    );

    // 详情里单独改价格与代理：地址、Key、模型不动。
    let before = ctx.gateway.load().unwrap().provider(&id).unwrap().relay.clone();
    let price = codex_plus_core::gateway::store::ProviderPrice {
        currency: "USD".into(),
        input: 2.0,
        cached_input: 0.1,
        output: 10.0,
    };
    let saved = codex_plus_core::gateway::apply::set_provider_extras(
        &ctx.gateway,
        &id,
        Some(price.clone()),
        "socks5://127.0.0.1:1080",
    )
    .unwrap();
    let provider = saved.provider(&id).unwrap();
    assert_eq!(provider.price, Some(price));
    assert_eq!(provider.proxy, "socks5://127.0.0.1:1080");
    assert_eq!(provider.relay.base_url, before.base_url);
    assert_eq!(provider.relay.api_key, before.api_key);
    assert_eq!(provider.relay.model_list, before.model_list);
    assert!(
        codex_plus_core::gateway::apply::set_provider_extras(&ctx.gateway, &id, None, "bad")
            .is_err()
    );
}

#[test]
fn enabling_requires_a_pool_and_a_routable_model() {
    let fixture = fixture(57393);
    let ctx = &fixture.ctx;
    let mut unknown = binding();
    unknown.model = "not-in-pool".into();
    assert!(enable_binding(ctx, &ToolId::Claude, unknown).is_err());
    // 「挑选」把唯一的供应商排除掉 → 没有可用供应商。
    let mut excluded = binding();
    excluded.pool_exclude = vec!["deepseek".into()];
    assert!(enable_binding(ctx, &ToolId::Claude, excluded).is_err());
    assert!(ctx.gateway.load().unwrap().bindings.is_empty());
    // `<供应商 id>/<模型>` 能路由，即使模型不在列表里。
    let mut qualified = binding();
    qualified.model = "deepseek/deepseek-v4-preview".into();
    assert!(enable_binding(ctx, &ToolId::Claude, qualified).is_ok());
}

#[test]
fn providers_live_only_in_the_gateway_store() {
    let fixture = fixture(57392);
    let store = &fixture.ctx.gateway;
    // 新建：多个 Key 写成命名 Key；重名换成新短名。
    let mut input = deepseek_input();
    input.keys = vec!["sk-a".into(), " sk-b ".into(), "sk-a".into()];
    let (gateway, id) = create_provider(store, &input, "preset:deepseek").unwrap();
    assert_eq!(id, "deepseek-2");
    let relay = &gateway.provider("deepseek-2").unwrap().relay;
    assert_eq!(relay.api_keys.len(), 2);
    assert_eq!(relay.api_key, "sk-a");
    assert_eq!(
        gateway.provider("deepseek-2").unwrap().source,
        "preset:deepseek"
    );

    // 读回来地址、模型、Key 都还在。
    let loaded = store.load().unwrap();
    let relay = &loaded.provider("deepseek").unwrap().relay;
    assert_eq!(relay.base_url, "https://api.deepseek.test/v1");
    assert_eq!(relay.api_key, "sk-ds");
    assert_eq!(
        codex_plus_core::gateway::route::relay_models(relay),
        vec!["deepseek-v4-pro", "deepseek-v4-flash"]
    );

    // 编辑：Key 留空沿用原来的，id 不变。
    let mut edit = deepseek_input();
    edit.name = "DeepSeek 主力".into();
    edit.keys.clear();
    edit.model_list = "deepseek-v4-pro".into();
    let gateway = update_provider(store, "deepseek-2", &edit).unwrap();
    let relay = &gateway.provider("deepseek-2").unwrap().relay;
    assert_eq!(relay.name, "DeepSeek 主力");
    assert_eq!(relay.api_keys.len(), 2, "Key 留空时保留原来的两个 Key");

    // 排序、开关、删除。
    let gateway = reorder_providers(store, &["deepseek-2".into()]).unwrap();
    assert_eq!(gateway.providers[0].relay.id, "deepseek-2");
    let gateway = set_provider_enabled(store, "deepseek", false).unwrap();
    assert!(!gateway.provider("deepseek").unwrap().enabled);
    let (_, group) = save_group(
        store,
        &GroupInput {
            id: String::new(),
            name: "Daily coding".into(),
            mode: GroupMode::InOrder,
            members: vec![
                GroupMember {
                    model: "deepseek-2/deepseek-v4-pro".into(),
                    weight: 1,
                },
                GroupMember {
                    model: "deepseek/deepseek-v4-flash".into(),
                    weight: 1,
                },
            ],
        },
    )
    .unwrap();
    assert_eq!(group, "daily-coding");
    let gateway = delete_provider(store, "deepseek-2").unwrap();
    assert!(gateway.provider("deepseek-2").is_none());
    assert_eq!(
        gateway.group("daily-coding").unwrap().members.len(),
        1,
        "删掉供应商时路由组里指向它的成员一起去掉"
    );
    assert!(delete_provider(store, "deepseek-2").is_err());
}

#[test]
fn groups_are_saved_validated_and_unlinked_on_delete() {
    let fixture = fixture(57391);
    let store = &fixture.ctx.gateway;
    let member = |model: &str| GroupMember {
        model: model.into(),
        weight: 0,
    };
    assert!(
        save_group(
            store,
            &GroupInput {
                name: "空".into(),
                ..GroupInput::default()
            }
        )
        .is_err(),
        "至少一个成员"
    );
    let (_, fast) = save_group(
        store,
        &GroupInput {
            name: "Fast".into(),
            mode: GroupMode::Rotate,
            members: vec![
                member("deepseek-v4-flash"),
                member("group/fast"),
                member("deepseek-v4-flash"),
            ],
            ..GroupInput::default()
        },
    )
    .unwrap();
    let (gateway, coder) = save_group(
        store,
        &GroupInput {
            name: "Coder".into(),
            members: vec![member("deepseek/deepseek-v4-pro"), member("group/fast")],
            ..GroupInput::default()
        },
    )
    .unwrap();
    let saved = gateway.group(&fast).unwrap();
    assert_eq!(saved.members.len(), 1, "去掉自引用和重复成员");
    assert_eq!(saved.members[0].weight, 1, "权重至少为 1");
    assert_eq!(saved.mode, GroupMode::Rotate);
    let gateway = delete_group(store, &fast).unwrap();
    assert_eq!(
        gateway.group(&coder).unwrap().members.len(),
        1,
        "删掉的组从别的组里移除"
    );
}

#[test]
fn imports_copy_data_and_leave_the_direct_mode_untouched() {
    let provider = ClaudeProvider {
        id: "kimi".into(),
        name: "Kimi".into(),
        website_url: String::new(),
        notes: String::new(),
        settings_config: json!({ "env": {
            "ANTHROPIC_BASE_URL": "https://api.kimi.test/anthropic",
            "ANTHROPIC_AUTH_TOKEN": "sk-kimi",
            "ANTHROPIC_MODEL": "kimi-k2.5",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": "kimi-k2.5-mini"
        }}),
        created_at: 0,
        ccs_source_id: String::new(),
    };
    let relay = copy_claude_provider(&provider).unwrap();
    assert_eq!(relay.protocol, RelayProtocol::AnthropicMessages);
    assert_eq!(relay.base_url, "https://api.kimi.test/anthropic");
    assert_eq!(relay.api_key, "sk-kimi");
    assert_eq!(relay.anthropic_auth_header, "authorization");
    assert_eq!(relay.model_list, "kimi-k2.5\nkimi-k2.5-mini");

    // Codex 供应商：普通 API 供应商原样复制；官方登录账号和聚合供应商不能复制。
    let mut settings = BackendSettings::default();
    settings.relay_profiles = vec![
        RelayProfile {
            id: "api".into(),
            name: "API".into(),
            relay_mode: RelayMode::PureApi,
            base_url: "https://api.test/v1".into(),
            api_key: "sk-api".into(),
            ..RelayProfile::default()
        },
        RelayProfile {
            id: "agg".into(),
            name: "聚合".into(),
            relay_mode: RelayMode::Aggregate,
            ..RelayProfile::default()
        },
    ];
    let before = settings.clone();
    let copied = copy_codex_provider(&settings, "api").unwrap();
    assert_eq!(copied.api_key, "sk-api");
    assert!(copy_codex_provider(&settings, "agg").is_err());
    assert_eq!(
        codex_importable(&settings)
            .iter()
            .map(|relay| relay.id.as_str())
            .collect::<Vec<_>>(),
        vec!["api"]
    );
    assert_eq!(settings, before, "复制不改直连模式的数据");
}

#[test]
fn pool_changes_follow_into_grok_model_list() {
    let fixture = fixture(57390);
    let ctx = &fixture.ctx;
    enable_binding(ctx, &ToolId::Grok, AgentBinding::default()).unwrap();
    let config = || std::fs::read_to_string(ctx.grok_home.join("config.toml")).unwrap();
    assert!(config().contains("deepseek-v4-pro"), "{}", config());
    assert!(config().contains("deepseek-v4-flash"), "{}", config());
    assert!(!config().contains("kimi-k3"), "{}", config());

    let mut kimi = deepseek_input();
    kimi.name = "Kimi".into();
    kimi.model = "kimi-k3".into();
    kimi.model_list = "kimi-k3".into();
    create_provider(&ctx.gateway, &kimi, "").unwrap();
    assert_eq!(refresh_enabled(ctx).unwrap(), 1);
    assert!(config().contains("kimi-k3"), "{}", config());
    save_group(
        &ctx.gateway,
        &GroupInput {
            name: "Fast".into(),
            members: vec![GroupMember {
                model: "kimi-k3".into(),
                weight: 1,
            }],
            ..GroupInput::default()
        },
    )
    .unwrap();
    refresh_enabled(ctx).unwrap();
    assert!(config().contains("group/fast"), "{}", config());
    set_provider_enabled(&ctx.gateway, "kimi", false).unwrap();
    refresh_enabled(ctx).unwrap();
    assert!(!config().contains("kimi-k3"), "{}", config());
}
