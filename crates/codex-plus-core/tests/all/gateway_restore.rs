//! 断开网关 / 退出 / 更新前把 Agent 还原成接入前的样子：只改回网关改过的键，
//! 接入期间别的改动（MCP、刷新的 token、新登录）保留；状态对不上时也能断开。
//! 全部用临时目录里的假配置，不碰本机真实的 ~/.codex、~/.claude、~/.grok。

use codex_plus_core::gateway::apply::{
    ApplyContext, ProviderInput, create_provider, disable_binding, enable_binding, reapply_enabled,
    reapply_if_port_changed, refresh_enabled, restore_all_for_exit, sync_codex_identity,
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

/// 退出（打开了「退出时恢复直连」）把 Agent 还原，绑定保持开启并记下「已恢复」；
/// 下次启动重新接回网关，再断开仍能回到最初的配置。
#[test]
fn exit_restore_then_startup_reapply_round_trip() {
    let fixture = fixture(57414);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    assert!(!ctx.gateway.load().unwrap().restore_on_exit, "默认关");
    let mut gateway = ctx.gateway.load().unwrap();
    gateway.restore_on_exit = true;
    ctx.gateway.save(&gateway).unwrap();

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

/// 「退出时恢复直连」关着（默认）：普通退出不还原；安装更新前（force）照样还原。
#[test]
fn update_restore_ignores_the_exit_switch() {
    let fixture = fixture(57415);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();

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

fn grok_stash_path(ctx: &ApplyContext) -> std::path::PathBuf {
    ctx.backup_root.join("grok-before-gateway.json")
}

/// 别的工具把 Grok 指向了自己的地址。
const GROK_OTHER_TOOL: &str = r#"[models]
default = "grok-other"

[model.grok-other]
model = "grok-other"
base_url = "http://127.0.0.1:15721/v1"
api_backend = "responses"
"#;

fn binding_enabled(ctx: &ApplyContext, agent: &ToolId) -> bool {
    ctx.gateway
        .load()
        .unwrap()
        .binding(agent)
        .is_some_and(|binding| binding.enabled)
}

/// 把存档里记下的改动清空，模拟「存下原文后、记下改动前进程就退出了」。
fn forget_recorded_changes(path: &std::path::Path) {
    let mut saved: Value = serde_json::from_str(&read(path)).unwrap();
    saved["changed"] = json!([]);
    std::fs::write(path, saved.to_string()).unwrap();
}

/// 存档没记下改了哪些键（接入写完配置就崩了），配置还指向网关：断开时按网关会写的键兜底，
/// 不能把指向网关的配置当成「接入前」写回去。
#[test]
fn grok_disconnect_with_empty_change_list_falls_back_to_gateway_keys() {
    let fixture = fixture(57422);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    forget_recorded_changes(&grok_stash_path(ctx));

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Grok).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);
    assert!(!grok_stash_path(ctx).exists());
}

/// 同上，Claude Code：接入前没有当前供应商、手写的地址和 Key 只在 settings.json 里，
/// 「上一个供应商」也没来得及记下。断开时按关键字段兜底，地址和 Key 不丢。
#[test]
fn claude_disconnect_with_empty_change_list_falls_back_to_provider_keys() {
    let fixture = fixture(57423);
    let ctx = &fixture.ctx;
    let env = json!({
        "ANTHROPIC_BASE_URL": "https://api.kimi.test/anthropic",
        "ANTHROPIC_AUTH_TOKEN": "sk-fake-kimi",
    });
    let path = ctx.claude.claude_home.join("settings.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(
            &json!({ "env": env, "permissions": { "allow": ["Bash(ls)"] } }),
        )
        .unwrap(),
    )
    .unwrap();
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    assert!(read(&path).contains("/gw/claude"));
    forget_recorded_changes(&ctx.backup_root.join("claude-before-gateway.json"));
    let mut gateway = ctx.gateway.load().unwrap();
    gateway
        .bindings
        .get_mut("claude")
        .unwrap()
        .restore
        .claude_previous_provider_id
        .clear();
    ctx.gateway.save(&gateway).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Claude).unwrap();
    let live: Value = serde_json::from_str(&read(&path)).unwrap();
    assert_eq!(live["env"], env, "{live}");
    assert_eq!(live["permissions"]["allow"][0], "Bash(ls)");
}

/// 接着网关时 Grok 被别的工具改走：退出 / 更新前还原不能写回存档盖掉用户的新选择；
/// 存档丢掉、绑定关掉，下次启动也不再接回。
#[test]
fn grok_taken_over_by_another_tool_is_not_clobbered_on_restore() {
    let fixture = fixture(57424);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    std::fs::write(&path, GROK_OTHER_TOOL).unwrap();

    // 池变化时的重写也不碰它（界面会提示漂移）。
    assert_eq!(refresh_enabled(ctx).unwrap(), 0);
    assert_eq!(read(&path), GROK_OTHER_TOOL);
    assert!(binding_enabled(ctx, &ToolId::Grok));

    restore_all_for_exit(ctx, &BackendSettings::default(), true).unwrap();
    assert_eq!(read(&path), GROK_OTHER_TOOL, "用户的新选择不动");
    assert!(!grok_stash_path(ctx).exists());
    assert!(!binding_enabled(ctx, &ToolId::Grok), "等于用户自己断开了");

    assert_eq!(reapply_enabled(ctx).unwrap(), 0);
    assert_eq!(read(&path), GROK_OTHER_TOOL);
}

/// 接着网关时 Claude Code 被别的工具改走：断开时 settings.json 不动，
/// 只删托管供应商、清掉 Codex++ 记的当前供应商和存档。
#[test]
fn claude_taken_over_by_another_tool_is_not_clobbered_on_disconnect() {
    let fixture = fixture(57425);
    let ctx = &fixture.ctx;
    let path = ctx.claude.claude_home.join("settings.json");
    std::fs::write(
        &path,
        r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.kimi.test/anthropic"}}"#,
    )
    .unwrap();
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    let other = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:15721","ANTHROPIC_AUTH_TOKEN":"sk-other"}}"#;
    std::fs::write(&path, other).unwrap();

    disable_binding(ctx, &BackendSettings::default(), &ToolId::Claude).unwrap();
    assert_eq!(read(&path), other);
    assert!(!binding_enabled(ctx, &ToolId::Claude));
    assert!(!ctx.backup_root.join("claude-before-gateway.json").exists());
    let payload = claude::load(&ctx.claude).unwrap();
    assert_ne!(payload.current_id, GATEWAY_PROVIDER_ID);
    assert!(
        payload
            .providers
            .iter()
            .all(|provider| provider.id != GATEWAY_PROVIDER_ID)
    );
}

/// 接着网关时 Codex 的 config.toml 被别的工具改走：更新前还原不写 ~/.codex，
/// 只把直连模式的记录改回开启前的供应商、删掉托管供应商，绑定关掉。
#[test]
fn codex_taken_over_by_another_tool_is_not_clobbered_on_restore() {
    let fixture = fixture(57426);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
    let config_path = ctx.codex_home.join("config.toml");
    let other = "model_provider = \"other\"\nmodel = \"gpt-other\"\n\n[model_providers.other]\nname = \"other\"\nbase_url = \"http://127.0.0.1:15721/v1\"\n";
    std::fs::write(&config_path, other).unwrap();

    let settings = ctx.settings.load().unwrap();
    restore_all_for_exit(ctx, &settings, true).unwrap();
    assert_eq!(read(&config_path), other);
    let settings = ctx.settings.load().unwrap();
    assert_eq!(settings.active_relay_id, "direct");
    assert!(
        settings
            .relay_profiles
            .iter()
            .all(|profile| profile.id != "codex-plus-gateway")
    );
    assert!(!binding_enabled(ctx, &ToolId::Codex));
    assert_eq!(reapply_enabled(ctx).unwrap(), 0);
    assert_eq!(read(&config_path), other);
}

/// 更新前还原成直连后、新版本启动前，用户用别的工具把 Grok 指向了别处：启动时不再接回网关。
#[test]
fn startup_reapply_skips_agent_changed_after_update_restore() {
    let fixture = fixture(57427);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    restore_all_for_exit(ctx, &BackendSettings::default(), true).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);
    std::fs::write(&path, GROK_OTHER_TOOL).unwrap();

    assert_eq!(reapply_enabled(ctx).unwrap(), 0);
    assert_eq!(read(&path), GROK_OTHER_TOOL);
    assert!(!binding_enabled(ctx, &ToolId::Grok));
    assert!(!ctx.gateway.load().unwrap().restored_at_exit);
}

/// 启动时重新接入：一个 Agent 失败不影响其它的；有失败时「已恢复」标记留着，下次启动再试。
#[test]
fn startup_reapply_continues_after_one_agent_fails() {
    let fixture = fixture(57428);
    let ctx = &fixture.ctx;
    let path = ctx.grok_home.join("config.toml");
    std::fs::write(&path, GROK_HAND_WRITTEN).unwrap();
    enable_binding(ctx, &ToolId::Claude, binding()).unwrap();
    enable_binding(ctx, &ToolId::Grok, binding()).unwrap();
    restore_all_for_exit(ctx, &BackendSettings::default(), true).unwrap();
    assert_eq!(read(&path), GROK_HAND_WRITTEN);
    // Claude 的供应商列表坏了：它排在 Grok 前面，以前第一个错误就中断，Grok 永远接不回。
    let store = read(&ctx.claude.store_path);
    std::fs::write(&ctx.claude.store_path, "{ 坏掉的 json").unwrap();

    let error = reapply_enabled(ctx).unwrap_err();
    assert!(format!("{error:#}").contains("claude"), "{error:#}");
    assert!(read(&path).contains("/gw/grok"), "Grok 照样接回");
    assert!(
        ctx.gateway.load().unwrap().restored_at_exit,
        "有失败：标记留着"
    );

    std::fs::write(&ctx.claude.store_path, store).unwrap();
    assert_eq!(reapply_enabled(ctx).unwrap(), 2);
    assert!(!ctx.gateway.load().unwrap().restored_at_exit);
    assert!(read(&ctx.claude.claude_home.join("settings.json")).contains("/gw/claude"));
}

fn port_change_with_failed_agent(failed: ToolId) {
    let mut fixture = fixture(57429);
    let ctx = &fixture.ctx;
    seed_codex_direct(ctx);
    for agent in [ToolId::Claude, ToolId::Codex, ToolId::Grok] {
        enable_binding(ctx, &agent, binding()).unwrap();
    }
    let claude_store = read(&ctx.claude.store_path);
    if failed == ToolId::Claude {
        std::fs::write(&ctx.claude.store_path, "{ invalid json").unwrap();
    } else {
        let mut settings = ctx.settings.load().unwrap();
        settings.relay_profiles_enabled = false;
        ctx.settings.save(&settings).unwrap();
    }
    fixture.ctx.port = 57430;
    let ctx = &fixture.ctx;
    let error = reapply_if_port_changed(ctx).unwrap_err();
    assert!(format!("{error:#}").contains(failed.as_str()), "{error:#}");
    let gateway = ctx.gateway.load().unwrap();
    for (agent, path) in [
        (ToolId::Claude, ctx.claude.claude_home.join("settings.json")),
        (ToolId::Codex, ctx.codex_home.join("config.toml")),
        (ToolId::Grok, ctx.grok_home.join("config.toml")),
    ] {
        let expected = if agent == failed { 57429 } else { 57430 };
        assert_eq!(
            gateway.binding(&agent).unwrap().restore.applied_port,
            Some(expected),
            "{agent:?}"
        );
        assert!(read(&path).contains(&expected.to_string()), "{agent:?}");
    }
    if failed == ToolId::Claude {
        std::fs::write(&ctx.claude.store_path, claude_store).unwrap();
    } else {
        let mut settings = ctx.settings.load().unwrap();
        settings.relay_profiles_enabled = true;
        ctx.settings.save(&settings).unwrap();
    }
    assert_eq!(reapply_if_port_changed(ctx).unwrap(), 1, "只重试失败项");
    assert_eq!(reapply_if_port_changed(ctx).unwrap(), 0);
}

#[test]
fn port_change_continues_after_first_agent_fails() {
    port_change_with_failed_agent(ToolId::Claude);
}

#[test]
fn port_change_records_successes_before_and_after_failed_codex() {
    port_change_with_failed_agent(ToolId::Codex);
}

#[test]
fn startup_identity_sync_preserves_external_takeover_in_both_reapply_paths() {
    for restore_on_exit in [false, true] {
        let fixture = fixture(57431);
        let ctx = &fixture.ctx;
        seed_codex_direct(ctx);
        enable_binding(ctx, &ToolId::Codex, binding()).unwrap();
        ctx.gateway
            .update(|gateway| {
                gateway.restore_on_exit = restore_on_exit;
                Ok(())
            })
            .unwrap();
        let config_path = ctx.codex_home.join("config.toml");
        let auth_path = ctx.codex_home.join("auth.json");
        let other = "model_provider = \"other\"\n[model_providers.other]\nbase_url = \"https://other.test/v1\"\n";
        let login = r#"{"auth_mode":"chatgpt","tokens":{"access_token":"fake-new-login","refresh_token":"fake-refresh","account_id":"fake-account"}}"#;
        std::fs::write(&config_path, other).unwrap();
        std::fs::write(&auth_path, login).unwrap();
        let settings = ctx.settings.revision().unwrap();
        let gateway = read(&ctx.gateway.path);
        if restore_on_exit {
            assert_eq!(reapply_enabled(ctx).unwrap(), 0);
        } else {
            let mut moved = ctx.clone();
            moved.port += 1;
            assert_eq!(reapply_if_port_changed(&moved).unwrap(), 0);
        }
        assert!(!sync_codex_identity(ctx).unwrap());
        assert_eq!(read(&config_path), other);
        assert_eq!(read(&auth_path), login);
        assert_eq!(ctx.settings.revision().unwrap(), settings);
        assert_eq!(read(&ctx.gateway.path), gateway);
        assert!(binding_enabled(ctx, &ToolId::Codex));
    }
}
