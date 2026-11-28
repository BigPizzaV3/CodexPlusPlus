//! Claude Code 直连共享供应商里的一家（Claude 页的「直连」）：断开网关、写进它自己的 settings.json。

use codex_plus_core::gateway::apply::{ApplyContext, ProviderInput, create_provider, enable_binding};
use codex_plus_core::gateway::claude_direct::{current_pool_provider, use_pool_provider};
use codex_plus_core::gateway::store::{AgentBinding, GatewayStore};
use codex_plus_core::settings::{BackendSettings, RelayProtocol};
use codex_plus_core::tools::ToolId;
use codex_plus_core::tools::claude::{self, ClaudeContext, GATEWAY_PROVIDER_ID};
use serde_json::Value;

struct Fixture {
    _temp: tempfile::TempDir,
    ctx: ApplyContext,
    kimi: String,
    deepseek: String,
}

fn input(name: &str, base_url: &str, protocol: RelayProtocol, key: &str, models: &str) -> ProviderInput {
    ProviderInput {
        name: name.into(),
        base_url: base_url.into(),
        protocol,
        keys: vec![key.into()],
        model: models.lines().next().unwrap_or_default().into(),
        model_list: models.into(),
        price: None,
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
        subscription_policy: None,
        strip_responses_image_detail: None,
        strip_responses_output_encrypted_content: None,
        image_compress: None,
        disable_tool_search: None,
        custom_headers: None,
    }
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
    let (_, kimi) = create_provider(
        &ctx.gateway,
        &input("Kimi", "https://api.kimi.test/anthropic/v1", RelayProtocol::AnthropicMessages, "sk-kimi", "kimi-k2.5\nkimi-k2-turbo"),
        "preset:kimi",
        None,
    )
    .unwrap();
    let (_, deepseek) = create_provider(
        &ctx.gateway,
        &input("DeepSeek", "https://api.deepseek.test/v1", RelayProtocol::ChatCompletions, "sk-ds", "deepseek-v4-pro"),
        "preset:deepseek",
        None,
    )
    .unwrap();
    Fixture { _temp: temp, ctx, kimi, deepseek }
}

fn live(ctx: &ApplyContext) -> Value {
    serde_json::from_str(&std::fs::read_to_string(ctx.claude.claude_home.join("settings.json")).unwrap()).unwrap()
}

#[test]
fn claude_code_goes_direct_to_an_anthropic_pool_provider_and_leaves_the_gateway() {
    let fixture = fixture(57461);
    let ctx = &fixture.ctx;
    enable_binding(ctx, &ToolId::Claude, AgentBinding { model: "kimi-k2.5".into(), ..AgentBinding::default() }).unwrap();
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), GATEWAY_PROVIDER_ID);

    let gateway = use_pool_provider(ctx, &BackendSettings::default(), &fixture.kimi, Some("kimi-k2-turbo")).unwrap();
    assert!(!gateway.binding(&ToolId::Claude).unwrap().enabled, "直连就是断开本地服务");
    assert_eq!(current_pool_provider(ctx).as_deref(), Some(fixture.kimi.as_str()));
    let live = live(ctx);
    assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://api.kimi.test/anthropic", "Claude Code 自己补 /v1/messages");
    assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-kimi");
    assert_eq!(live["env"]["ANTHROPIC_MODEL"], "kimi-k2-turbo");
    assert_eq!(live["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "kimi-k2-turbo", "第三方不认 Claude 的小模型名");

    // 再选一次同一家（换模型）：就地更新，不多出一项。
    use_pool_provider(ctx, &BackendSettings::default(), &fixture.kimi, None).unwrap();
    assert_eq!(live_model(ctx), "kimi-k2.5", "没选模型用这家的默认模型");
    let pool_items = claude::load(&ctx.claude).unwrap().providers.into_iter().filter(|provider| provider.id.starts_with("pool-")).count();
    assert_eq!(pool_items, 1);
}

fn live_model(ctx: &ApplyContext) -> String {
    live(ctx)["env"]["ANTHROPIC_MODEL"].as_str().unwrap_or_default().to_string()
}

#[test]
fn openai_protocol_providers_cannot_go_direct_and_the_gateway_stays_on() {
    let fixture = fixture(57462);
    let ctx = &fixture.ctx;
    enable_binding(ctx, &ToolId::Claude, AgentBinding { model: "kimi-k2.5".into(), ..AgentBinding::default() }).unwrap();

    let error = use_pool_provider(ctx, &BackendSettings::default(), &fixture.deepseek, None).unwrap_err();
    assert!(error.to_string().contains("Anthropic 协议"), "{error}");
    assert!(ctx.gateway.load().unwrap().binding(&ToolId::Claude).unwrap().enabled, "不能直连时不动现在的接入");
    assert_eq!(claude::current_provider_id(&ctx.claude).unwrap(), GATEWAY_PROVIDER_ID);
    assert!(use_pool_provider(ctx, &BackendSettings::default(), &fixture.kimi, Some("not-a-model")).is_err());
    assert_eq!(current_pool_provider(ctx), None);
}
