//! 断开网关 / 退出 / 更新前把 Agent 还原成接入前的样子：只改回网关改过的键，
//! 接入期间别的改动（MCP、刷新的 token、新登录）保留；状态对不上时也能断开。
//! 全部用临时目录里的假配置，不碰本机真实的 ~/.codex、~/.claude、~/.grok。

use codex_plus_core::gateway::apply::{
    ApplyContext, ProviderInput, create_provider, disable_binding, enable_binding, reapply_enabled,
    refresh_enabled, restore_all_for_exit,
};
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
    };
    std::fs::create_dir_all(&ctx.claude.claude_home).unwrap();
    std::fs::create_dir_all(&ctx.grok_home).unwrap();
    std::fs::create_dir_all(&ctx.codex_home).unwrap();
    create_provider(
        &ctx.gateway,
        &ProviderInput {
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.test/v1".into(),
            protocol: RelayProtocol::ChatCompletions,
            keys: vec!["sk-ds".into()],
            model: "deepseek-v4-pro".into(),
            model_list: "deepseek-v4-pro\ndeepseek-v4-flash".into(),
            price: None,
            model_prices: None,
            proxy: None,
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

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// 用户手写的 Grok 配置：有自己的地址、Key、默认模型和界面设置。
const GROK_HAND_WRITTEN: &str = r#"# 自己写的 Grok 配置
[ui]
yolo = false

[models]
default = "grok-5"

[endpoints]
models_base_url = "https://api.x.test/v1"

[model.grok-5]
model = "grok-5"
base_url = "https://api.x.test/v1"
api_backend = "chat_completions"
api_key = "sk-fake-grok"
"#;

/// 直连模式里没有 Grok 供应商：断开时照样写回接入前的配置（以前什么都不做，Grok 一直指着网关）。
#[test]
fn grok_disconnect_restores_hand_written_config_without_a_grok_provider() {
    let fixture = fixture(57411);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();

    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    let on_gateway = read(&path);
    assert!(on_gateway.contains("/gw/grok/v1"), "{on_gateway}");
    assert!(!on_gateway.contains("sk-fake-grok"), "{on_gateway}");
    assert!(ctx.backup_root.join("grok-before-gateway.json").exists());

    // 池变了重写一次，仍然能还原到接入前。
    refresh_enabled(ctx).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Grok).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN, "原样写回");
    assert!(!ctx.backup_root.join("grok-before-gateway.json").exists());
    assert!(
        !ctx.gateway
            .load()
            .unwrap()
            .binding(&ToolId::Grok)
            .unwrap()
            .enabled
    );
}

/// 接入期间用户改了 Grok 的其它设置：断开只改回网关改过的键，用户的改动留着。
#[test]
fn grok_disconnect_keeps_edits_made_while_on_the_gateway() {
    let fixture = fixture(57412);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    let edited = read(&path).replace("yolo = false", "yolo = true\ntheme = \"dark\"");
    std::fs::write(&path, &edited).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Grok).unwrap();
    let config: toml::Value = toml::from_str(&read(&path)).unwrap();
    assert_eq!(config["ui"]["theme"].as_str(), Some("dark"));
    assert_eq!(config["ui"]["yolo"].as_bool(), Some(true));
    assert_eq!(config["models"]["default"].as_str(), Some("grok-5"));
    assert_eq!(
        config["endpoints"]["models_base_url"].as_str(),
        Some("https://api.x.test/v1")
    );
    assert_eq!(
        config["model"]["grok-5"]["api_key"].as_str(),
        Some("sk-fake-grok")
    );
    assert!(config["model"].get("deepseek-v4-pro").is_none(), "{config}");
    assert!(!read(&path).contains("/gw/grok"));
}

/// 没有存档（旧版接入的）也没有 Grok 供应商可应用：报错，不假装还原成功，绑定保持开启。
#[test]
fn grok_disconnect_without_stash_or_provider_reports_an_error() {
    let fixture = fixture(57413);
    let ctx = &fixture.ctx;
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    std::fs::remove_file(ctx.backup_root.join("grok-before-gateway.json")).unwrap();

    let error = disable_binding(ctx, &BackendSettings::default(), &ToolId::Grok).unwrap_err();
    assert!(format!("{error:#}").contains("Grok"), "{error:#}");
    assert!(read(&ctx.grok_home.join("config.toml")).contains("/gw/grok"));
    assert!(
        ctx.gateway
            .load()
            .unwrap()
            .binding(&ToolId::Grok)
            .unwrap()
            .enabled
    );
}

/// 退出（默认开着「退出时恢复直连」）把 Agent 还原，绑定保持开启并记下「已恢复」；
/// 下次启动重新接回网关，再断开仍能回到最初的配置。
#[test]
fn exit_restore_then_startup_reapply_round_trip() {
    let fixture = fixture(57414);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    assert!(ctx.gateway.load().unwrap().restore_on_exit, "默认开");

    restore_all_for_exit(ctx, &BackendSettings::default(), false).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);
    let saved = ctx.gateway.load().unwrap();
    assert!(saved.restored_at_exit);
    assert!(saved.binding(&ToolId::Grok).unwrap().enabled, "绑定保留");
    // 退出流程会再调一次（更新前已经还原过）：不能把已还原的配置再改坏。
    restore_all_for_exit(ctx, &BackendSettings::default(), false).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);

    assert_eq!(reapply_enabled(ctx).unwrap(), 1);
    assert!(read(&path).contains("/gw/grok"));
    assert!(!ctx.gateway.load().unwrap().restored_at_exit, "标记清掉");

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Grok).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);
}

/// 用户关掉了「退出时恢复直连」：普通退出不还原；安装更新前（force）照样还原。
#[test]
fn update_restore_ignores_the_exit_switch() {
    let fixture = fixture(57415);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    let mut gateway = ctx.gateway.load().unwrap();
    gateway.restore_on_exit = false;
    ctx.gateway.save(&gateway).unwrap();

    restore_all_for_exit(ctx, &BackendSettings::default(), false).unwrap();
    assert!(read(&path).contains("/gw/grok"), "开关关着：普通退出不动");
    restore_all_for_exit(ctx, &BackendSettings::default(), true).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN, "更新前照样还原");
    assert!(ctx.gateway.load().unwrap().restored_at_exit);
}

/// Claude Code 接入前的配置与某个已存的供应商一样、但没有当前供应商：
/// 以前断开时只能清空（地址、Key 全丢），现在按接入前的 settings.json 还原。
#[test]
fn claude_disconnect_restores_live_config_that_matched_a_saved_provider() {
    let fixture = fixture(57416);
    let ctx = &fixture.ctx;
    let env = json!({
        "ANTHROPIC_BASE_URL": "https://api.kimi.test/anthropic",
        "ANTHROPIC_AUTH_TOKEN": "sk-fake-kimi",
        "ANTHROPIC_MODEL": "kimi-k2.5"
    });
    claude::save_provider(
        &ctx.claude,
        ClaudeProvider {
            id: "kimi".into(),
            name: "Kimi".into(),
            website_url: String::new(),
            notes: String::new(),
            settings_config: json!({ "env": env }),
            created_at: 0,
            ccs_source_id: String::new(),
        },
    )
    .unwrap();
    let path = ctx.claude.claude_home.join("settings.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(
            &json!({ "env": env, "permissions": { "allow": ["Bash(ls)"] } }),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), "");

    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    assert!(read(&path).contains("/gw/claude"));
    // 接入期间 Claude Code 自己加了一条权限。
    let mut live: Value = serde_json::from_str(&read(&path)).unwrap();
    live["permissions"]["deny"] = json!(["Bash(rm)"]);
    std::fs::write(&path, serde_json::to_string_pretty(&live).unwrap()).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Claude).unwrap();
    let live: Value = serde_json::from_str(&read(&path)).unwrap();
    assert_eq!(live["env"], env, "{live}");
    assert_eq!(
        live["permissions"]["deny"][0], "Bash(rm)",
        "接入期间的改动保留"
    );
    assert!(!ctx.backup_root.join("claude-before-gateway.json").exists());
    let payload = claude::load(&ctx.claude).unwrap();
    assert!(
        payload
            .providers
            .iter()
            .all(|provider| provider.id != GATEWAY_PROVIDER_ID)
    );
}

/// 在直连模式里放一个 Responses 供应商并切过去。
fn seed_codex_direct(ctx: &ApplyContext) {
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
}

fn append(path: &std::path::Path, text: &str) {
    let mut current = read(path);
    current.push_str(text);
    std::fs::write(path, current).unwrap();
}

/// Codex 的实时配置被别的工具改过（接入时存了副本）：断开时只改回网关改过的键，
/// 接入期间加的 MCP 段留着，Codex 刷新过的 token 不被旧的覆盖。
#[test]
fn codex_disconnect_keeps_mcp_section_and_refreshed_token() {
    let fixture = fixture(57417);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    let config_path = ctx.codex_home.join("config.toml");
    let auth_path = ctx.codex_home.join("auth.json");
    let edited = read(&config_path).replace(
        "https://api.direct.test/v1",
        "http://127.0.0.1:3425/other-tool",
    );
    std::fs::write(&config_path, &edited).unwrap();
    std::fs::write(
        &auth_path,
        r#"{"auth_mode":"chatgpt","tokens":{"id_token":"x","access_token":"at-1","refresh_token":"rt-1","account_id":"acc"}}"#,
    )
    .unwrap();

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    assert!(ctx.backup_root.join("codex-before-gateway.json").exists());
    assert!(read(&config_path).contains("/gw/codex/v1"));
    // 接入期间：加了一个 MCP，Codex 刷新了 token。
    append(
        &config_path,
        "\n[mcp_servers.x]\ncommand = \"node\"\nargs = [\"server.js\"]\n",
    );
    let mut auth: Value = serde_json::from_str(&read(&auth_path)).unwrap();
    auth["tokens"]["access_token"] = json!("at-2");
    auth["tokens"]["refresh_token"] = json!("rt-2");
    std::fs::write(&auth_path, auth.to_string()).unwrap();

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let config: toml::Value = toml::from_str(&read(&config_path)).unwrap();
    assert_eq!(
        config["mcp_servers"]["x"]["command"].as_str(),
        Some("node"),
        "{config}"
    );
    let text = read(&config_path);
    assert!(text.contains("http://127.0.0.1:3425/other-tool"), "{text}");
    assert!(!text.contains("/gw/codex"), "{text}");
    let auth: Value = serde_json::from_str(&read(&auth_path)).unwrap();
    assert_eq!(auth["tokens"]["access_token"], "at-2", "保留刷新后的 token");
    assert_eq!(auth["tokens"]["refresh_token"], "rt-2");
    assert_eq!(ctx.settings.load().unwrap().active_relay_id, "direct");
    assert!(!ctx.backup_root.join("codex-before-gateway.json").exists());
}

/// 接入前 Codex 没有 auth.json，接入期间用户登录了：断开不能把新登录删掉。
#[test]
fn codex_disconnect_keeps_login_made_while_on_the_gateway() {
    let fixture = fixture(57418);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    let config_path = ctx.codex_home.join("config.toml");
    let auth_path = ctx.codex_home.join("auth.json");
    let edited = read(&config_path).replace(
        "https://api.direct.test/v1",
        "http://127.0.0.1:3425/other-tool",
    );
    std::fs::write(&config_path, &edited).unwrap();
    let _ = std::fs::remove_file(&auth_path);

    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    std::fs::write(
        &auth_path,
        r#"{"auth_mode":"chatgpt","OPENAI_API_KEY":null,"tokens":{"id_token":"x","access_token":"at-new","refresh_token":"rt-new","account_id":"acc-new"}}"#,
    )
    .unwrap();

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let auth: Value = serde_json::from_str(&read(&auth_path)).unwrap();
    assert_eq!(auth["tokens"]["access_token"], "at-new", "{auth}");
}

/// 正常接入（直连模式能从自己的记录还原，没存副本）：接入期间加的 MCP 段断开后仍在。
#[test]
fn codex_normal_disconnect_keeps_mcp_section() {
    let fixture = fixture(57419);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    assert!(!ctx.backup_root.join("codex-before-gateway.json").exists());
    let config_path = ctx.codex_home.join("config.toml");
    append(&config_path, "\n[mcp_servers.x]\ncommand = \"node\"\n");

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let config: toml::Value = toml::from_str(&read(&config_path)).unwrap();
    assert_eq!(
        config["mcp_servers"]["x"]["command"].as_str(),
        Some("node"),
        "{config}"
    );
    assert!(read(&config_path).contains("api.direct.test"));
}

/// Codex 还停在托管的网关供应商上，但 gateway.json 里没有它的绑定（降级、换过文件）：
/// 断开照样切回直连，不再「被网关接管」卡住。
#[test]
fn codex_stuck_on_managed_profile_without_binding_switches_back() {
    let fixture = fixture(57420);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let mut gateway = ctx.gateway.load().unwrap();
    gateway.bindings.clear();
    ctx.gateway.save(&gateway).unwrap();

    let settings = ctx.settings.load().unwrap();
    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "direct");
    assert!(
        settings
            .relay_profiles
            .iter()
            .all(|profile| profile.id != "codex-plus-gateway")
    );
    assert!(!read(&ctx.codex_home.join("config.toml")).contains("/gw/codex"));
}

/// gateway.json 读不出来、托管供应商也已经不在列表里，只剩 active 指着它：照样切回直连。
#[test]
fn codex_stuck_with_unreadable_gateway_store_and_missing_profile_switches_back() {
    let fixture = fixture(57421);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let mut settings = ctx.settings.load().unwrap();
    settings
        .relay_profiles
        .retain(|profile: &RelayProfile| profile.id != "codex-plus-gateway");
    ctx.settings.save(&settings).unwrap();
    std::fs::write(&ctx.gateway.path, "{ 坏掉的 json").unwrap();

    disable_binding(ctx, &settings, &ToolId::Codex).unwrap();
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "direct");
    assert!(read(&ctx.codex_home.join("config.toml")).contains("api.direct.test"));
    assert_eq!(
        read(&ctx.gateway.path),
        "{ 坏掉的 json",
        "不碰读不出来的 gateway.json"
    );
}
