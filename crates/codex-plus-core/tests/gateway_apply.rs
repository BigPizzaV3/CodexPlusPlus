//! 网关模式开启 / 关闭对 Agent 配置文件的改写与恢复，以及网关自己的供应商池、路由组。

use codex_plus_core::gateway::apply::{
    ApplyContext, GroupInput, ProviderInput, codex_importable, copy_claude_provider,
    copy_codex_provider, create_provider, delete_group, delete_provider, disable_binding,
    enable_binding, note_claude_switched, note_codex_switched, provider_reasoning,
    reapply_if_port_changed, refresh_enabled, reorder_providers, save_group, set_provider_enabled,
    set_provider_metadata, set_provider_windows, update_provider,
};
use codex_plus_core::gateway::store::{
    AgentBinding, ClaudeRoleModels, GatewayStore, GroupMember, GroupMode, GroupSticky,
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
    create_provider(&ctx.gateway, &deepseek_input(), "preset:deepseek", None).unwrap();
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
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
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
    // 接入前没有 settings.json：还原成没有（之后有别的改动时才留下文件）。
    let path = ctx.claude.claude_home.join("settings.json");
    let live: Value = std::fs::read_to_string(&path)
        .ok()
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap_or_else(|| json!({}));
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
    // Grok 页应用过这个供应商：接入前 config.toml 指向它。
    codex_plus_core::tools::grok::apply_profile_to_grok_at(
        &ctx.grok_home,
        &grok_profile,
        &ctx.backup_root,
    )
    .unwrap();
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

/// 网关模式给 Codex 的模型目录与直连模式的按模型聚合一样：显示名、推理档位、按供应商分组，
/// 模型组单独一组。没有内置元数据的模型（第三方的 claude-*）也要有推理档位。
#[test]
fn codex_gateway_catalog_carries_display_names_reasoning_and_provider_groups() {
    let fixture = fixture(57394);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "ChatGPT Pro · 2649340134@qq.com".into(),
            base_url: "https://relay.test/v1".into(),
            protocol: RelayProtocol::Responses,
            keys: vec!["sk-relay".into()],
            model: "claude-opus-5-5".into(),
            model_list: "claude-opus-5-5\ndeepseek-v4-pro".into(),
            price: None,
            model_prices: None,
            proxy: None,
            strip_image_generation_tool: None,
        },
        "",
        None,
    )
    .unwrap();
    save_group(
        &ctx.gateway,
        &GroupInput {
            name: "Daily".into(),
            mode: GroupMode::InOrder,
            members: vec![GroupMember {
                model: "claude-opus-5-5".into(),
                weight: 1,
            }],
            ..GroupInput::default()
        },
    )
    .unwrap();
    let mut seeded = BackendSettings::default();
    seeded.relay_profiles_enabled = true;
    ctx.settings.save(&seeded).unwrap();
    // 打开「多家自动切换」；收藏里有一个旧版的裸名收藏。
    ctx.gateway
        .update(|gateway| {
            gateway.auto_models = true;
            gateway.favorite_models = vec!["deepseek-v4-pro".into()];
            Ok(())
        })
        .unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let catalog = codex_catalog(ctx);
    let model = |slug: &str| {
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == slug)
            .cloned()
            .unwrap_or_else(|| panic!("目录里没有 {slug}：{catalog}"))
    };

    let opus = model("claude-opus-5-5");
    assert_eq!(opus["display_name"], "Claude-Opus-5-5");
    assert!(
        opus["supported_reasoning_levels"]
            .as_array()
            .is_some_and(|levels| !levels.is_empty()),
        "{opus}"
    );
    // 分组名和描述显示在 Codex 的模型选择器里，账号要打码。
    assert_eq!(opus["codex_plus_group"], "ChatGPT Pro · 26****34@qq.com");
    assert!(
        !opus["description"]
            .as_str()
            .unwrap_or_default()
            .contains("2649340134"),
        "悬停说明里也不能有原始账号：{}",
        opus["description"]
    );
    // 两家都提供的模型：每家名下各一份固定的（带供应商前缀，显示名带后缀分得开），
    // 再加一份裸名的「自动切换」入口单独成组。
    let deepseek_id = profile_id_of(ctx, "DeepSeek");
    let pinned = model(&format!("{deepseek_id}/deepseek-v4-pro"));
    assert_eq!(pinned["codex_plus_group"], "DeepSeek");
    assert_eq!(pinned["display_name"], "DeepSeek-V4-Pro（DeepSeek）");
    let second_id = profile_id_of(ctx, "ChatGPT Pro · 2649340134@qq.com");
    let second = model(&format!("{second_id}/deepseek-v4-pro"));
    // 打码后的账号名超过注入层认的后缀长度：用「·」前那段（池里没有别家同名开头）；分组名仍是完整账号名。
    assert_eq!(second["display_name"], "DeepSeek-V4-Pro（ChatGPT Pro）");
    assert_eq!(second["codex_plus_group"], "ChatGPT Pro · 26****34@qq.com");
    let auto = model("deepseek-v4-pro");
    assert_eq!(auto["display_name"], "DeepSeek-V4-Pro（自动）");
    assert_eq!(auto["codex_plus_group"], "自动组");
    // 旧版的裸名收藏：自动那份和各家那几份都标收藏（Codex 选择器里画星标）；别的模型不标。
    assert_eq!(auto["codex_plus_favorite"], true);
    assert_eq!(pinned["codex_plus_favorite"], true);
    assert_eq!(second["codex_plus_favorite"], true);
    assert!(
        model("claude-opus-5-5")
            .get("codex_plus_favorite")
            .is_none()
    );
    // 只有一家的模型照旧用裸名、不带后缀。
    assert_eq!(model("claude-opus-5-5")["display_name"], "Claude-Opus-5-5");
    let group = model("group/daily");
    assert_eq!(group["display_name"], "Daily");
    assert_eq!(group["codex_plus_group"], "模型组");

    // 注入层读的模型元数据（托管供应商的 model_metadata）同样给出推理档位与分组。
    let settings = ctx.settings.load().unwrap();
    let profile = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "codex-plus-gateway")
        .unwrap();
    let metadata: Value = serde_json::from_str(&profile.model_metadata).unwrap();
    let ui = codex_plus_core::model_suffix::model_ui_metadata_with_override(
        "claude-opus-5-5",
        metadata.get("claude-opus-5-5"),
    )
    .unwrap();
    assert_eq!(ui["displayName"], "Claude-Opus-5-5");
    assert!(
        ui["supportedReasoningEfforts"]
            .as_array()
            .is_some_and(|levels| !levels.is_empty())
    );
    assert_eq!(
        ui["codex_plus_group_id"],
        profile_id_of(ctx, "ChatGPT Pro · 2649340134@qq.com")
    );

    // 关掉「模型名后带供应商」：同名几份的显示名只差看不见的零宽字符，去掉它们都是模型名本身，但互不相同。
    ctx.gateway
        .update(|gateway| {
            gateway.model_suffix = false;
            Ok(())
        })
        .unwrap();
    refresh_enabled(ctx).unwrap();
    let settings = ctx.settings.load().unwrap();
    let profile = settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == "codex-plus-gateway")
        .unwrap();
    let metadata: Value = serde_json::from_str(&profile.model_metadata).unwrap();
    let names: Vec<String> = [
        format!("{deepseek_id}/deepseek-v4-pro"),
        format!("{second_id}/deepseek-v4-pro"),
        "deepseek-v4-pro".to_string(),
    ]
    .iter()
    .map(|slug| metadata[slug]["display_name"].as_str().unwrap().to_string())
    .collect();
    for name in &names {
        assert_eq!(
            name.replace(['\u{200B}', '\u{200C}'], ""),
            "DeepSeek-V4-Pro",
            "{name:?}"
        );
        assert!(!name.contains('（'), "看不到供应商后缀：{name:?}");
    }
    assert_ne!(names[0], names[1]);
    assert_ne!(names[1], names[2]);
    assert_ne!(names[0], names[2]);
}

/// 模型组在 Codex 里能不能开 Fast（#2740）：每个成员都支持才开，有一个不支持就不开。
#[test]
fn codex_group_offers_fast_only_when_every_member_supports_it() {
    let fixture = fixture(57397);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    for name in ["Relay A", "Relay B"] {
        create_provider(
            &ctx.gateway,
            &ProviderInput {
                name: name.into(),
                base_url: format!("https://{}.test/v1", name.replace(' ', "-").to_lowercase()),
                protocol: RelayProtocol::Responses,
                keys: vec!["sk-relay".into()],
                model: "gpt-x".into(),
                model_list: "gpt-x".into(),
                price: None,
                model_prices: None,
                proxy: None,
                strip_image_generation_tool: None,
            },
            "",
            None,
        )
        .unwrap();
        set_provider_metadata(
            &ctx.gateway,
            &profile_id_of(ctx, name),
            r#"{"gpt-x":{"codex_plus_fast_support":"supported"}}"#,
        )
        .unwrap();
    }
    let member = |model: &str| GroupMember {
        model: model.into(),
        weight: 1,
    };
    for (name, members) in [
        ("Fasty", vec![member("gpt-x")]),
        ("Mixed", vec![member("gpt-x"), member("deepseek-v4-pro")]),
    ] {
        save_group(
            &ctx.gateway,
            &GroupInput {
                name: name.into(),
                mode: GroupMode::InOrder,
                members,
                ..GroupInput::default()
            },
        )
        .unwrap();
    }
    let mut seeded = BackendSettings::default();
    seeded.relay_profiles_enabled = true;
    ctx.settings.save(&seeded).unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let catalog = codex_catalog(ctx);
    let entry = |slug: &str| {
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["slug"] == slug)
            .cloned()
            .unwrap_or_else(|| panic!("目录里没有 {slug}：{catalog}"))
    };
    let fast = |model: &Value| {
        model["service_tiers"]
            .as_array()
            .is_some_and(|tiers| tiers.iter().any(|tier| tier["id"] == "priority"))
    };
    assert!(
        fast(&entry("gpt-x")),
        "单个模型声明了 Fast：{}",
        entry("gpt-x")
    );
    let fasty = entry("group/fasty");
    assert!(fast(&fasty), "成员都支持 Fast 的组也能开：{fasty}");
    assert_eq!(fasty["display_name"], "Fasty");
    let mixed = entry("group/mixed");
    assert!(!fast(&mixed), "有成员不支持就不开：{mixed}");
}

fn profile_id_of(ctx: &ApplyContext, name: &str) -> String {
    ctx.gateway
        .load()
        .unwrap()
        .providers
        .iter()
        .find(|provider| provider.relay.name == name)
        .map(|provider| provider.relay.id.clone())
        .unwrap()
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
        codex_plus_core::gateway::apply::add_provider(&ctx.gateway, chatgpt, "login:chatgpt", None)
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
    let (_, id) = create_provider(&ctx.gateway, &input, "preset:deepseek", None).unwrap();
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
    let before = ctx
        .gateway
        .load()
        .unwrap()
        .provider(&id)
        .unwrap()
        .relay
        .clone();
    let price = codex_plus_core::gateway::store::ProviderPrice {
        currency: "USD".into(),
        input: 2.0,
        cached_input: 0.1,
        output: 10.0,
    };
    let pro = codex_plus_core::gateway::store::ProviderPrice {
        currency: "USD".into(),
        input: 4.0,
        cached_input: 0.2,
        output: 16.0,
    };
    let saved = codex_plus_core::gateway::apply::set_provider_extras(
        &ctx.gateway,
        &id,
        Some(price.clone()),
        Some(
            [
                ("deepseek-v4-pro".to_string(), pro.clone()),
                ("deepseek-v4-flash".to_string(), Default::default()),
            ]
            .into(),
        ),
        "socks5://127.0.0.1:1080",
    )
    .unwrap();
    let provider = saved.provider(&id).unwrap();
    assert_eq!(provider.price, Some(price.clone()));
    // 按模型的价格优先，没填（全 0）的项丢掉、回落到默认价；窗口后缀和大小写不影响匹配。
    assert_eq!(provider.model_prices.len(), 1);
    assert_eq!(provider.price_for("DeepSeek-V4-Pro[1M]"), Some(&pro));
    assert_eq!(provider.price_for("deepseek-v4-flash"), Some(&price));
    // 不传按模型的价格时保留原来的。
    let kept =
        codex_plus_core::gateway::apply::set_provider_extras(&ctx.gateway, &id, None, None, "")
            .unwrap();
    assert_eq!(kept.provider(&id).unwrap().model_prices.len(), 1);
    assert_eq!(
        kept.provider(&id).unwrap().price_for("deepseek-v4-flash"),
        None
    );
    assert_eq!(provider.proxy, "socks5://127.0.0.1:1080");
    assert_eq!(provider.relay.base_url, before.base_url);
    assert_eq!(provider.relay.api_key, before.api_key);
    assert_eq!(provider.relay.model_list, before.model_list);
    assert!(
        codex_plus_core::gateway::apply::set_provider_extras(&ctx.gateway, &id, None, None, "bad")
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
fn new_providers_follow_agent_picks_and_auto_join() {
    let fixture = fixture(57401);
    let store = &fixture.ctx.gateway;
    // Grok 关掉「添加供应商时自动加入」，Claude 保持默认。
    let mut gateway = store.load().unwrap();
    gateway.bindings.insert("claude".into(), binding());
    let mut grok = binding();
    grok.auto_join_providers = false;
    gateway.bindings.insert("grok".into(), grok);
    store.save(&gateway).unwrap();

    // 没传勾选：按各 Agent 的设置。
    let (gateway, id) = create_provider(store, &deepseek_input(), "preset:deepseek", None).unwrap();
    assert!(!gateway.bindings["claude"].pool_exclude.contains(&id));
    assert!(gateway.bindings["grok"].pool_exclude.contains(&id));

    // 传了勾选：取消勾选的 Agent 不用它，哪怕之前没有绑定也记下来。
    let skip = vec!["claude".to_string(), "codex".to_string()];
    let (gateway, id) =
        create_provider(store, &deepseek_input(), "preset:deepseek", Some(&skip)).unwrap();
    assert!(gateway.bindings["claude"].pool_exclude.contains(&id));
    assert!(gateway.bindings["codex"].pool_exclude.contains(&id));
    assert!(!gateway.bindings["codex"].enabled);
    assert!(
        !gateway.bindings["grok"].pool_exclude.contains(&id),
        "勾选了就给它用"
    );

    // 旧数据没有这个字段时默认自动加入。
    let parsed: AgentBinding = serde_json::from_str("{}").unwrap();
    assert!(parsed.auto_join_providers);
}

#[test]
fn providers_live_only_in_the_gateway_store() {
    let fixture = fixture(57392);
    let store = &fixture.ctx.gateway;
    // 新建：多个 Key 写成命名 Key；重名换成新短名。
    let mut input = deepseek_input();
    input.keys = vec!["sk-a".into(), " sk-b ".into(), "sk-a".into()];
    let (gateway, id) = create_provider(store, &input, "preset:deepseek", None).unwrap();
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
            sticky: GroupSticky::Session,
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
            ..GroupInput::default()
        },
    )
    .unwrap();
    assert_eq!(group, "daily-coding");
    assert_eq!(
        store.load().unwrap().group("daily-coding").unwrap().sticky,
        GroupSticky::Session,
        "会话固定方式随组保存"
    );
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
    assert_eq!(saved.sticky, GroupSticky::Cache, "没填时默认缓存期内固定");
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
    create_provider(&ctx.gateway, &kimi, "", None).unwrap();
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

fn direct_relay(id: &str, model: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: format!("中转 {id}"),
        relay_mode: RelayMode::PureApi,
        protocol: RelayProtocol::Responses,
        base_url: format!("https://{id}.test/v1"),
        api_key: format!("sk-{id}"),
        model: model.to_string(),
        model_list: model.to_string(),
        ..RelayProfile::default()
    }
}

#[test]
fn direct_aggregates_move_into_the_gateway_as_pool_providers_and_a_group() {
    use codex_plus_core::gateway::apply::{add_provider, codex_aggregates, import_codex_aggregate};
    use codex_plus_core::settings::{
        AggregateRelayMember, AggregateRelayProfile, AggregateRelayRoute, AggregateRelayStrategy,
    };
    let fixture = fixture(57395);
    let store = &fixture.ctx.gateway;
    let member = |relay_id: &str, weight: u32| AggregateRelayMember {
        relay_id: relay_id.to_string(),
        weight,
    };
    let aggregate = |id: &str, by_model: bool| AggregateRelayProfile {
        id: id.to_string(),
        name: format!("聚合 {id}"),
        session_provider: Default::default(),
        model_aggregation_enabled: by_model,
        strategy: AggregateRelayStrategy::WeightedRoundRobin,
        members: vec![
            member("relay-a", 3),
            member("relay-b", 1),
            member("gone", 1),
        ],
        routes: vec![AggregateRelayRoute {
            pattern: "gpt-*".into(),
            relay_id: "relay-b".into(),
            priority: 0,
        }],
        default_relay_id: String::new(),
        sync_default_member_login: false,
        auto_review_relay_id: String::new(),
    };
    let settings = BackendSettings {
        relay_profiles: vec![
            direct_relay("relay-a", "model-a"),
            direct_relay("relay-b", "model-b"),
        ],
        aggregate_relay_profiles: vec![
            aggregate("agg", false),
            aggregate("by-model", true),
            AggregateRelayProfile {
                members: Vec::new(),
                ..aggregate("empty", false)
            },
        ],
        ..BackendSettings::default()
    };
    // relay-a 之前单独导入过：复用，不再复制一份。
    let (_, imported_a) = add_provider(
        store,
        copy_codex_provider(&settings, "relay-a").unwrap(),
        "codex:relay-a",
        None,
    )
    .unwrap();

    let result = import_codex_aggregate(store, &settings, "agg", None).unwrap();
    assert_eq!(result.added, 1, "只新导入 relay-b");
    let gateway = store.load().unwrap();
    let group = gateway.group(result.group.as_deref().unwrap()).unwrap();
    assert_eq!(group.name, "聚合 agg");
    assert_eq!(group.mode, GroupMode::Weighted);
    let imported_b = gateway
        .providers
        .iter()
        .find(|provider| provider.source == "codex:relay-b")
        .unwrap()
        .relay
        .id
        .clone();
    assert_eq!(
        group
            .members
            .iter()
            .map(|member| (member.model.clone(), member.weight))
            .collect::<Vec<_>>(),
        vec![
            (format!("{imported_a}/model-a"), 3),
            (format!("{imported_b}/model-b"), 1)
        ]
    );
    assert!(
        result.notes.iter().any(|note| note.contains("gone")),
        "{:?}",
        result.notes
    );
    assert!(
        result.notes.iter().any(|note| note.contains("路由规则")),
        "{:?}",
        result.notes
    );
    assert!(
        codex_aggregates(&settings, &gateway)
            .iter()
            .any(|item| item.id == "agg" && item.imported && item.mode == GroupMode::Weighted)
    );

    // 再搬一次：不多导供应商、不多出模型组。
    let again = import_codex_aggregate(store, &settings, "agg", None).unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.group, result.group);
    assert_eq!(store.load().unwrap().groups.len(), 1);

    // 按模型聚合：成员进池就够了，不建组。
    let by_model = import_codex_aggregate(store, &settings, "by-model", None).unwrap();
    assert_eq!((by_model.added, by_model.group), (0, None));
    assert!(
        by_model
            .notes
            .iter()
            .any(|note| note.contains("按模型聚合"))
    );
    // 导入列表只列会建模型组的：按模型聚合（成员就是普通供应商）和没有成员的都不列。
    let listed: Vec<String> = codex_aggregates(&settings, &store.load().unwrap())
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(listed, vec!["agg".to_string()]);
}

#[test]
fn auto_group_settings_save_and_reset_to_default() {
    use codex_plus_core::gateway::apply::save_auto_group;
    use codex_plus_core::gateway::store::AutoGroupSettings;
    let fixture = fixture(57396);
    let store = &fixture.ctx.gateway;
    // 新文件：自动建组默认开。
    assert!(store.load().unwrap().auto_models);
    let changed = AutoGroupSettings {
        mode: GroupMode::Manual,
        pick: " kimi/deepseek-v4-pro ".into(),
        hide_members: true,
        ..AutoGroupSettings::default()
    };
    let gateway = save_auto_group(store, "DeepSeek-V4-Pro", changed).unwrap();
    let saved = gateway.auto_groups.get("deepseek-v4-pro").unwrap();
    assert_eq!(saved.mode, GroupMode::Manual);
    assert_eq!(saved.pick, "kimi/deepseek-v4-pro", "去掉首尾空格");
    assert!(saved.hide_members);
    // 改回默认：记录删掉，自动组回到「没改过」、跟着池子变。
    let gateway = save_auto_group(store, "deepseek-v4-pro", AutoGroupSettings::default()).unwrap();
    assert!(gateway.auto_groups.is_empty());
}

/// 读 Codex 的模型目录：`config.toml` 里 `model_catalog_json` 指向的文件。按 TOML 解析取值：
/// Windows 上路径里的反斜杠在 TOML 字符串里是转义过的，按文本截引号会得到错的路径。
fn codex_catalog(ctx: &ApplyContext) -> Value {
    let config = std::fs::read_to_string(ctx.codex_home.join("config.toml")).unwrap();
    let parsed: toml::Value = toml::from_str(&config).unwrap();
    let pointer = parsed
        .get("model_catalog_json")
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| panic!("网关供应商要写模型目录：{config}"));
    let pointer = std::path::PathBuf::from(pointer);
    let path = if pointer.is_absolute() {
        pointer
    } else {
        ctx.codex_home.join(pointer)
    };
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn context_windows_follow_each_model_in_the_pool() {
    let fixture = fixture(57396);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut seeded = BackendSettings::default();
    seeded.relay_profiles_enabled = true;
    ctx.settings.save(&seeded).unwrap();
    let id = ctx.gateway.load().unwrap().providers[0].relay.id.clone();
    set_provider_windows(
        &ctx.gateway,
        &id,
        [("DeepSeek-V4-Pro".to_string(), 1_000_000)].into(),
        Some(128_000),
    )
    .unwrap();

    // Agent 的默认窗口只给没写窗口的模型兜底；池里按模型写的优先。
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let catalog = codex_catalog(ctx);
    let window = |slug: &str| {
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["slug"] == slug)
            .unwrap_or_else(|| panic!("目录里没有 {slug}：{catalog}"))["context_window"]
            .clone()
    };
    assert_eq!(window("deepseek-v4-pro"), 1_000_000);
    assert_eq!(
        window("deepseek-v4-flash"),
        128_000,
        "没单独写的用这家的默认窗口"
    );

    // Claude Code 只有一个窗口：按默认模型的。
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    assert_eq!(
        read_settings_json(ctx)["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"],
        "1000000"
    );

    // 清掉供应商的窗口后回到 Agent 的默认窗口。
    set_provider_windows(&ctx.gateway, &id, Default::default(), None).unwrap();
    refresh_enabled(ctx).unwrap();
    let catalog = codex_catalog(ctx);
    let flash = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["slug"] == "deepseek-v4-flash")
        .unwrap()["context_window"]
        .clone();
    assert_eq!(flash, 1_000_000);
}

#[test]
fn imported_windows_move_into_the_gateway_provider() {
    let fixture = fixture(57397);
    let ctx = &fixture.ctx;
    let relay = RelayProfile {
        id: "relay".into(),
        name: "Relay".into(),
        relay_mode: RelayMode::PureApi,
        base_url: "https://relay.test/v1".into(),
        api_key: "sk-relay".into(),
        model: "kimi-k3".into(),
        model_list: "kimi-k3[256K]\nglm-5".into(),
        context_window: "200K".into(),
        ..RelayProfile::default()
    };
    codex_plus_core::gateway::apply::add_provider(&ctx.gateway, relay, "codex:relay", None)
        .unwrap();
    let gateway = ctx.gateway.load().unwrap();
    let provider = gateway.provider("relay").unwrap();
    assert_eq!(provider.window_for("kimi-k3"), Some(256_000));
    assert_eq!(provider.window_for("glm-5"), Some(200_000));
    assert!(
        provider
            .relay
            .model_windows
            .trim_matches(['{', '}'])
            .is_empty()
    );
    assert!(provider.relay.context_window.is_empty());
}

#[test]
fn reasoning_levels_are_edited_per_model_and_survive_provider_edits() {
    let fixture = fixture(57398);
    let ctx = &fixture.ctx;
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    let mut seeded = BackendSettings::default();
    seeded.relay_profiles_enabled = true;
    ctx.settings.save(&seeded).unwrap();
    let id = ctx.gateway.load().unwrap().providers[0].relay.id.clone();

    let builtin = provider_reasoning(&ctx.gateway.load().unwrap(), &id).unwrap();
    assert_eq!(builtin[0].model, "deepseek-v4-pro");
    assert!(!builtin[0].builtin_levels.is_empty(), "内置资料里有档位");

    set_provider_metadata(
        &ctx.gateway,
        &id,
        r#"{"deepseek-v4-pro":{"supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"default_reasoning_level":"high"}}"#,
    )
    .unwrap();
    // 编辑供应商（地址、Key、模型）不丢思考等级。
    update_provider(&ctx.gateway, &id, &deepseek_input()).unwrap();
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let catalog = codex_catalog(ctx);
    let pro = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["slug"] == "deepseek-v4-pro")
        .unwrap()
        .clone();
    let efforts: Vec<&str> = pro["supported_reasoning_levels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|level| level["effort"].as_str().unwrap())
        .collect();
    assert_eq!(efforts, ["low", "high"]);
    assert_eq!(pro["default_reasoning_level"], "high");
    // 内置档位读的是没改过的。
    let again = provider_reasoning(&ctx.gateway.load().unwrap(), &id).unwrap();
    assert_eq!(again[0].builtin_levels, builtin[0].builtin_levels);

    assert!(
        set_provider_metadata(&ctx.gateway, &id, "[1, 2]").is_err(),
        "不是对象要拒绝"
    );
}

/// #228：网关供应商的「移除托管生图工具」随新建写入，编辑时不传沿用原值，传了才改。
#[test]
fn gateway_provider_keeps_strip_image_generation_tool_across_edits() {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let input = ProviderInput {
        protocol: RelayProtocol::Responses,
        strip_image_generation_tool: Some(true),
        ..deepseek_input()
    };
    let (_, id) = create_provider(&store, &input, "", None).unwrap();
    let relay = |store: &GatewayStore| {
        store
            .load()
            .unwrap()
            .providers
            .into_iter()
            .find(|provider| provider.relay.id == id)
            .unwrap()
            .relay
    };
    assert!(relay(&store).strip_image_generation_tool);
    assert!(relay(&store).strips_image_generation_tool());

    let keep = ProviderInput { strip_image_generation_tool: None, ..input.clone() };
    update_provider(&store, &id, &keep).unwrap();
    assert!(relay(&store).strip_image_generation_tool);

    let off = ProviderInput { strip_image_generation_tool: Some(false), ..input };
    update_provider(&store, &id, &off).unwrap();
    assert!(!relay(&store).strip_image_generation_tool);
}
