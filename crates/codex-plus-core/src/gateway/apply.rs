//! 网关模式的开启 / 关闭，以及网关自己的供应商池、路由组的增删改。
//!
//! - Claude Code：在 `claude-providers.json` 里放一个托管供应商（[`GATEWAY_PROVIDER_ID`]），
//!   用现有的「关键字段」切换逻辑写进 `~/.claude/settings.json`，用户的 hooks 等键不动；
//!   关闭时切回开启前的供应商。
//! - Grok：合成一个指向网关的 Responses 供应商写进 `~/.grok/config.toml`；
//!   关闭时重新应用 Grok 页里当前选中的供应商。
//! - Codex：在直连模式的供应商列表里放一个托管供应商（[`CODEX_GATEWAY_RELAY_ID`]），
//!   用直连模式自己的切换流程写进 `~/.codex`（回填、冲突检测、模型目录都照旧）；
//!   关闭时切回开启前的供应商并删掉托管供应商。直连模式里切到别的供应商等于断开网关。
//! - 供应商池与路由组只存在 `gateway.json`；从直连模式导入是复制，之后互不影响。

use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::gateway::route::{self, binding_default_model};
use crate::gateway::store::{
    AgentBinding, GROUP_MODEL_PREFIX, GatewayGroup, GatewayProvider, GatewaySettings, GatewayStore,
    GroupMember, GroupMode, ProviderPrice, new_client_token,
};
use crate::settings::{
    BackendSettings, RelayApiKey, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use crate::tools::ToolId;
use crate::tools::claude::{self, ClaudeContext, ClaudeProvider, GATEWAY_PROVIDER_ID};

/// 开启/关闭时用到的全部路径，命令层用 [`ApplyContext::default_paths`]，测试指向临时目录。
#[derive(Debug, Clone)]
pub struct ApplyContext {
    pub gateway: GatewayStore,
    pub claude: ClaudeContext,
    pub grok_home: PathBuf,
    /// Codex 的配置目录与直连模式的设置（Codex 接入网关走直连模式的切换流程）。
    pub codex_home: PathBuf,
    pub settings: SettingsStore,
    pub backup_root: PathBuf,
    pub port: u16,
}

/// Codex 直连模式供应商列表里托管的网关供应商 id。
pub const CODEX_GATEWAY_RELAY_ID: &str = "codex-plus-gateway";

/// Codex 当前在用网关托管的供应商：本地代理收到的 Codex 请求只是转给网关的一跳。
pub fn codex_hops_through_gateway() -> bool {
    SettingsStore::default()
        .load()
        .is_ok_and(|settings| settings.active_relay_id == CODEX_GATEWAY_RELAY_ID)
}

impl ApplyContext {
    pub fn default_paths(port: u16) -> Self {
        Self {
            gateway: GatewayStore::default(),
            claude: ClaudeContext::default_paths(),
            grok_home: crate::grok_config::default_grok_home_dir(),
            codex_home: crate::relay_config::default_codex_home_dir(),
            settings: SettingsStore::default(),
            backup_root: crate::paths::default_app_state_dir().join("backups"),
            port,
        }
    }
}

/// Agent 访问网关的根地址（不含 `/v1`）。
pub fn gateway_base_url(port: u16, agent: &ToolId) -> String {
    format!("http://127.0.0.1:{port}/gw/{}", agent.as_str())
}

/// 开启前检查：池里有这个 Agent 能用的供应商，默认模型能路由。
fn ensure_routable(gateway: &GatewaySettings, binding: &AgentBinding) -> anyhow::Result<()> {
    if route::binding_pool_relays(gateway, binding).is_empty() {
        bail!("供应商池是空的或都已关闭：先在「供应商」里添加供应商");
    }
    let model = binding_default_model(gateway, binding);
    if model.is_empty() {
        bail!("没有可用的默认模型：请给池里的供应商配置模型");
    }
    if !route::model_is_routable(gateway, binding, &model) {
        bail!("供应商池里没有供应商提供模型 {model}");
    }
    Ok(())
}

/// 写给 Claude Code 的托管供应商。
pub fn claude_gateway_provider(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
) -> ClaudeProvider {
    let default_model = binding_default_model(gateway, binding);
    let role = |value: &str| {
        let (slug, _) = crate::model_suffix::parse_model_suffix(value.trim());
        if slug.trim().is_empty() {
            default_model.clone()
        } else {
            slug.trim().to_string()
        }
    };
    let mut env = Map::new();
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        json!(gateway_base_url(port, &ToolId::Claude)),
    );
    env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(binding.token));
    env.insert("ANTHROPIC_MODEL".to_string(), json!(default_model));
    let opus = role(&binding.claude_roles.opus);
    env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), json!(opus));
    env.insert("ANTHROPIC_DEFAULT_FABLE_MODEL".to_string(), json!(opus));
    env.insert(
        "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
        json!(role(&binding.claude_roles.sonnet)),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
        json!(role(&binding.claude_roles.haiku)),
    );
    if !binding.claude_roles.subagent.trim().is_empty() {
        env.insert(
            "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
            json!(role(&binding.claude_roles.subagent)),
        );
    }
    let window = binding.context_window.or_else(|| {
        let (_, window) = crate::model_suffix::parse_model_suffix(binding.model.trim());
        window
    });
    if let Some(window) = window {
        env.insert(
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS".to_string(),
            json!(window.to_string()),
        );
    }
    if binding.claude_enable_effort {
        env.insert("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT".to_string(), json!("1"));
    }
    ClaudeProvider {
        id: GATEWAY_PROVIDER_ID.to_string(),
        name: "Codex++ 网关".to_string(),
        website_url: String::new(),
        notes: "由 Codex++ 网关模式管理，请在网关模式里修改或关闭。".to_string(),
        settings_config: json!({ "env": Value::Object(env) }),
        created_at: 0,
        ccs_source_id: String::new(),
    }
}

/// 写给 Grok 的合成供应商：Responses 协议，地址是网关，Key 是网关 token。
pub fn grok_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
) -> RelayProfile {
    agent_gateway_profile(gateway, binding, port, &ToolId::Grok)
}

/// Responses 类 Agent（Grok、Codex）的合成供应商：地址是该 Agent 的网关入口，模型列表是整个池。
fn agent_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
    agent: &ToolId,
) -> RelayProfile {
    let default_model = binding_default_model(gateway, binding);
    let window = binding.context_window;
    let mut models = vec![default_model.clone()];
    // 池里所有能用的模型和路由组都进 Grok 的模型列表。
    for model in route::pool_models(gateway, binding)
        .into_iter()
        .chain(binding.declared_models())
    {
        if !models.contains(&model) {
            models.push(model);
        }
    }
    let model_list = models
        .iter()
        .filter(|model| !model.is_empty())
        .map(|model| match window {
            Some(window) => format!("{model}[{window}]"),
            None => model.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n");
    let base = format!("{}/v1", gateway_base_url(port, agent));
    RelayProfile {
        id: "codex-plus-gateway".to_string(),
        name: "Codex++ 网关".to_string(),
        protocol: RelayProtocol::Responses,
        relay_mode: RelayMode::PureApi,
        base_url: base.clone(),
        upstream_base_url: base,
        api_key: binding.token.clone(),
        model_list,
        ..RelayProfile::default()
    }
}

/// 写 Agent 配置。Claude 首次切换时若把 live 配置存成了「切换前的配置」，返回它的 id。
fn write_agent_config(
    ctx: &ApplyContext,
    agent: &ToolId,
    binding: &AgentBinding,
) -> anyhow::Result<Option<String>> {
    let gateway = ctx.gateway.load()?;
    match agent {
        ToolId::Claude => {
            claude::save_provider(
                &ctx.claude,
                claude_gateway_provider(&gateway, binding, ctx.port),
            )?;
            let before = claude::load(&ctx.claude)?
                .providers
                .into_iter()
                .map(|provider| provider.id)
                .collect::<Vec<_>>();
            let payload = claude::switch_provider(&ctx.claude, GATEWAY_PROVIDER_ID)?;
            let snapshot = payload
                .providers
                .iter()
                .find(|provider| !before.contains(&provider.id))
                .map(|provider| provider.id.clone());
            return Ok(snapshot);
        }
        ToolId::Grok => {
            let profile = grok_gateway_profile(&gateway, binding, ctx.port);
            crate::tools::grok::apply_profile_to_grok_at(
                &ctx.grok_home,
                &profile,
                &ctx.backup_root,
            )?;
        }
        ToolId::Codex => write_codex(ctx, &gateway, binding)?,
        other => bail!("{} 不支持网关模式", other.as_str()),
    }
    Ok(None)
}

/// Codex 的托管网关供应商：与直连模式里手加的 Responses 供应商一样存储（配置快照里带地址和 token）。
///
/// `signed_in`：Codex 用 ChatGPT 账号登录着。这时供应商带 openai 会话身份（与直连模式的
/// 「官方会话身份」一样）：Codex 保留内置的 OpenAI 供应商与登录，会话、额度、远程控制照旧，
/// 请求经本地代理（`openai_base_url`）转给网关。没登录或用 API Key 时是普通的自定义供应商。
pub fn codex_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
    signed_in: bool,
) -> anyhow::Result<RelayProfile> {
    let synthetic = agent_gateway_profile(gateway, binding, port, &ToolId::Codex);
    let mut profile = crate::provider_import::pool_relay_profile(
        "Codex++ 网关",
        &synthetic.base_url,
        &synthetic.api_key,
        RelayProtocol::Responses,
        &binding_default_model(gateway, binding),
        &synthetic.model_list,
        &[],
    )?;
    profile.id = CODEX_GATEWAY_RELAY_ID.to_string();
    profile.relay_mode = RelayMode::PureApi;
    if signed_in {
        profile.config_contents = with_openai_session_identity(&profile.config_contents)?;
    }
    crate::relay_config::normalize_relay_profile_for_storage(&mut profile)?;
    Ok(profile)
}

/// 把供应商配置改成 openai 会话身份：`model_provider = "openai"`，传输放进 `[model_providers.custom]`。
fn with_openai_session_identity(contents: &str) -> anyhow::Result<String> {
    let mut doc = contents
        .parse::<toml_edit::DocumentMut>()
        .context("托管供应商配置不是合法 TOML")?;
    let provider = doc
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default()
        .to_string();
    if provider == "openai" {
        return Ok(doc.to_string());
    }
    let mut table = doc
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|providers| providers.remove(&provider))
        .and_then(|item| item.into_table().ok())
        .context("托管供应商配置里没有传输供应商")?;
    table["name"] = toml_edit::value("custom");
    doc["model_providers"]["custom"] = toml_edit::Item::Table(table);
    doc["model_provider"] = toml_edit::value("openai");
    Ok(doc.to_string())
}

/// 池里能借给 Codex 登录的账号：打开着的、网关自己登录的 ChatGPT 账号（按池里的顺序取第一个）。
fn lendable_login(gateway: &GatewaySettings) -> Option<&GatewayProvider> {
    gateway.providers.iter().find(|provider| {
        provider.enabled && crate::official_login::is_gateway_owned_login(&provider.relay)
    })
}

/// 把网关的 ChatGPT 账号借给 Codex 登录：token 写进 Codex 的 auth.json（其余字段不动），
/// 网关里标记「借出」——之后这个账号以 Codex 那份为准，网关刷新也写回那里，两边始终同一代。
fn lend_login_to_codex(ctx: &ApplyContext, provider_id: &str) -> anyhow::Result<()> {
    let mut gateway = ctx.gateway.load()?;
    let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == provider_id)
    else {
        return Ok(());
    };
    let path = ctx.codex_home.join("auth.json");
    let live = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let merged = crate::official_login::auth_contents_with_member_login(
        &live,
        &provider.relay.auth_contents,
        false,
    )
    .context("Codex 已经登录了别的 ChatGPT 账号，不能再借用网关的账号")?;
    crate::settings::atomic_write(&path, merged.as_bytes())?;
    let mut auth: Value = serde_json::from_str(&provider.relay.auth_contents)?;
    auth[crate::official_login::LENT_TO_CODEX_MARKER] = Value::Bool(true);
    provider.relay.auth_contents = serde_json::to_string_pretty(&auth)?;
    ctx.gateway.save(&gateway)?;
    Ok(())
}

/// 收回借给 Codex 的账号：Codex 那份是最新一代，token 写回网关、去掉「借出」标记，
/// 再从 Codex 的 auth.json 里拿掉这个登录（借之前 Codex 本来就没登录）。
pub fn return_lent_login(store: &GatewayStore, codex_home: &std::path::Path) -> anyhow::Result<()> {
    let mut gateway = store.load()?;
    let path = codex_home.join("auth.json");
    let live = std::fs::read_to_string(&path).unwrap_or_default();
    let live_login = crate::official_login::official_login_from_auth_contents(&live);
    let mut changed = false;
    let mut strip_live = false;
    for provider in gateway
        .providers
        .iter_mut()
        .filter(|provider| crate::official_login::is_lent_to_codex(&provider.relay))
    {
        let mut auth: Value = serde_json::from_str(&provider.relay.auth_contents)?;
        let account = auth
            .pointer("/tokens/account_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if let Some(login) = live_login
            .as_ref()
            .filter(|login| !account.is_empty() && login.account_id == account)
        {
            auth["tokens"]["access_token"] = json!(login.access_token);
            if !login.refresh_token.is_empty() {
                auth["tokens"]["refresh_token"] = json!(login.refresh_token);
            }
            strip_live = true;
        }
        if let Some(object) = auth.as_object_mut() {
            object.remove(crate::official_login::LENT_TO_CODEX_MARKER);
        }
        provider.relay.auth_contents = serde_json::to_string_pretty(&auth)?;
        changed = true;
    }
    if changed {
        store.save(&gateway)?;
    }
    if strip_live && let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(&live) {
        for key in ["tokens", "auth_mode", "last_refresh"] {
            object.remove(key);
        }
        crate::settings::atomic_write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&Value::Object(object))?
            )
            .as_bytes(),
        )?;
    }
    Ok(())
}

/// Codex 现在是否用 ChatGPT 账号登录（auth.json 里有登录 token）。
fn codex_signed_in(codex_home: &std::path::Path) -> bool {
    std::fs::read_to_string(codex_home.join("auth.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|auth| {
            auth.pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
        })
}

/// 接入网关前 Codex 实时配置的原样副本。只在实时配置与直连模式的记录对不上时才存：
/// 那时直连模式没法从它自己的记录还原，断开时要把这份原样写回。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexLiveStash {
    #[serde(default)]
    config_toml: Option<String>,
    #[serde(default)]
    auth_json: Option<String>,
}

const CODEX_LIVE_FILES: [&str; 2] = ["config.toml", "auth.json"];

fn codex_stash_path(ctx: &ApplyContext) -> PathBuf {
    ctx.backup_root.join("codex-before-gateway.json")
}

fn capture_codex_live(ctx: &ApplyContext) -> CodexLiveStash {
    let read = |name: &str| std::fs::read_to_string(ctx.codex_home.join(name)).ok();
    CodexLiveStash {
        config_toml: read(CODEX_LIVE_FILES[0]),
        auth_json: read(CODEX_LIVE_FILES[1]),
    }
}

fn save_codex_stash(ctx: &ApplyContext, stash: &CodexLiveStash) -> anyhow::Result<()> {
    let path = codex_stash_path(ctx);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // 里面有 auth.json（凭据），只给自己读写。
    crate::settings::atomic_write(&path, serde_json::to_string_pretty(stash)?.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn take_codex_stash(ctx: &ApplyContext) -> Option<CodexLiveStash> {
    let path = codex_stash_path(ctx);
    let stash = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let _ = std::fs::remove_file(&path);
    stash
}

/// 把托管供应商放进直连模式的列表并切过去（已经在用时只刷新它的配置）。
///
/// 实时配置被别的工具改过、与直连模式的记录对不上（直连模式里切换会弹「实时配置冲突」）时，
/// 不往上一个供应商回填，先把实时文件原样存下来再切；断开时原样写回。
fn write_codex(
    ctx: &ApplyContext,
    gateway: &GatewaySettings,
    binding: &AgentBinding,
) -> anyhow::Result<()> {
    let mut settings = ctx.settings.load()?;
    if !settings.relay_profiles_enabled {
        bail!("Codex 的供应商配置总开关已关闭，无法接入网关：先在 Codex 设置里打开");
    }
    // Codex 自己没登录、池里有网关登录的 ChatGPT 账号：借给 Codex，让它以官方身份接入。
    let signed_in = codex_signed_in(&ctx.codex_home);
    let lend = (!signed_in)
        .then(|| lendable_login(gateway).map(|provider| provider.relay.id.clone()))
        .flatten();
    let profile = codex_gateway_profile(gateway, binding, ctx.port, signed_in || lend.is_some())?;
    match settings
        .relay_profiles
        .iter_mut()
        .find(|item| item.id == CODEX_GATEWAY_RELAY_ID)
    {
        Some(existing) => *existing = profile,
        None => settings.relay_profiles.push(profile),
    }
    let previous = std::mem::replace(
        &mut settings.active_relay_id,
        CODEX_GATEWAY_RELAY_ID.to_string(),
    );
    let switching_in = previous != CODEX_GATEWAY_RELAY_ID;
    let live = switching_in.then(|| capture_codex_live(ctx));
    let switched = crate::relay_switch::switch_relay_profile_in_home(
        &ctx.settings,
        &ctx.codex_home,
        settings.clone(),
        &previous,
    );
    match switched {
        Ok(_) => {
            if switching_in {
                // 直连模式能从自己的记录还原，不需要原样副本；清掉以前留下的。
                let _ = std::fs::remove_file(codex_stash_path(ctx));
            }
        }
        Err(error)
            if switching_in
                && error
                    .downcast_ref::<crate::relay_switch::LiveEndpointConflict>()
                    .is_some() =>
        {
            if let Some(live) = &live {
                save_codex_stash(ctx, live)?;
            }
            crate::relay_switch::switch_relay_profile_in_home_resolving_conflict(
                &ctx.settings,
                &ctx.codex_home,
                settings,
                &previous,
                crate::relay_switch::LiveConflictResolution::DiscardLiveChanges,
            )
            .context("把 Codex 切到网关失败")?;
        }
        Err(error) => return Err(error.context("把 Codex 切到网关失败")),
    }
    // 切换时回填上一个供应商用的还是借之前的 auth.json；切过去之后再写入借来的登录。
    if let Some(provider_id) = lend {
        lend_login_to_codex(ctx, &provider_id)?;
    }
    Ok(())
}

/// 开启（或更新）一个 Agent 的网关绑定，并把它的配置指向网关。
///
/// 已经开着时只更新绑定与配置，开启前记下的「恢复现场」保持不变。
pub fn enable_binding(
    ctx: &ApplyContext,
    agent: &ToolId,
    mut binding: AgentBinding,
) -> anyhow::Result<GatewaySettings> {
    let mut gateway = ctx.gateway.load()?;
    ensure_routable(&gateway, &binding)?;
    let previous = gateway.binding(agent).cloned();
    binding.token = previous
        .as_ref()
        .map(|previous| previous.token.clone())
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| new_client_token(agent));
    binding.restore = match previous.filter(|previous| previous.enabled) {
        Some(previous) => previous.restore,
        None => {
            let mut restore = crate::gateway::store::GatewayRestoreState::default();
            if *agent == ToolId::Claude {
                let current = claude::current_provider_id(&ctx.claude)?;
                if current != GATEWAY_PROVIDER_ID {
                    restore.claude_previous_provider_id = current;
                }
            }
            if *agent == ToolId::Codex {
                restore.codex_previous_relay_id = codex_active_relay_id(ctx)?;
            }
            restore
        }
    };
    binding.enabled = true;
    binding.restore.applied_port = Some(ctx.port);
    if let Some(snapshot) = write_agent_config(ctx, agent, &binding)?
        && binding.restore.claude_previous_provider_id.is_empty()
    {
        binding.restore.claude_previous_provider_id = snapshot;
    }
    // 写配置时可能改过 gateway.json（借出账号等），以最新的为准再写绑定。
    let mut gateway = ctx.gateway.load()?;
    gateway.bindings.insert(agent.as_str().to_string(), binding);
    ctx.gateway.save(&gateway)?;
    Ok(gateway)
}

/// 关闭一个 Agent 的网关绑定，把它的配置恢复成直连。绑定本身（目标、模型）保留，方便再次开启。
pub fn disable_binding(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
) -> anyhow::Result<GatewaySettings> {
    let gateway = ctx.gateway.load()?;
    let Some(binding) = gateway.bindings.get(agent.as_str()).cloned() else {
        return Ok(gateway);
    };
    match agent {
        ToolId::Claude => restore_claude(ctx, &binding)?,
        ToolId::Grok => restore_grok(ctx, settings)?,
        ToolId::Codex => restore_codex(ctx, &binding)?,
        _ => {}
    }
    // 恢复时可能改过 gateway.json（收回借出的账号等），以最新的为准。
    let mut gateway = ctx.gateway.load()?;
    if let Some(binding) = gateway.bindings.get_mut(agent.as_str()) {
        binding.enabled = false;
        binding.restore = Default::default();
    }
    ctx.gateway.save(&gateway)?;
    Ok(gateway)
}

fn restore_claude(ctx: &ApplyContext, binding: &AgentBinding) -> anyhow::Result<()> {
    let payload = claude::load(&ctx.claude)?;
    if payload.current_id == GATEWAY_PROVIDER_ID {
        let previous = binding.restore.claude_previous_provider_id.trim();
        let can_switch_back = !previous.is_empty()
            && payload
                .providers
                .iter()
                .any(|provider| provider.id == previous);
        if can_switch_back {
            claude::switch_provider(&ctx.claude, previous)?;
        } else {
            claude::clear_current(&ctx.claude)?;
        }
    }
    if payload
        .providers
        .iter()
        .any(|provider| provider.id == GATEWAY_PROVIDER_ID)
    {
        claude::delete_provider(&ctx.claude, GATEWAY_PROVIDER_ID)?;
    }
    Ok(())
}

/// Codex 当前在用的直连供应商（不是托管的网关供应商时）。
fn codex_active_relay_id(ctx: &ApplyContext) -> anyhow::Result<String> {
    let active = ctx.settings.load()?.active_relay_id;
    Ok(if active == CODEX_GATEWAY_RELAY_ID {
        String::new()
    } else {
        active
    })
}

/// 切回开启前的供应商（不在了就用列表里第一个），再删掉托管供应商。
/// 用户已经在直连模式里切走了的，只删托管供应商。
fn restore_codex(ctx: &ApplyContext, binding: &AgentBinding) -> anyhow::Result<()> {
    // 先收回借给 Codex 的账号，切回去时上一个供应商写的是它自己的 auth.json。
    return_lent_login(&ctx.gateway, &ctx.codex_home)?;
    let mut settings = ctx.settings.load()?;
    let is_managed = |profile: &RelayProfile| profile.id == CODEX_GATEWAY_RELAY_ID;
    if !settings.relay_profiles.iter().any(is_managed) {
        return Ok(());
    }
    if settings.active_relay_id == CODEX_GATEWAY_RELAY_ID {
        let previous = binding.restore.codex_previous_relay_id.trim();
        let exists = |id: &str| {
            !id.is_empty()
                && id != CODEX_GATEWAY_RELAY_ID
                && (settings
                    .relay_profiles
                    .iter()
                    .any(|profile| profile.id == id)
                    || settings
                        .aggregate_relay_profiles
                        .iter()
                        .any(|profile| profile.id == id))
        };
        let target = if exists(previous) {
            previous.to_string()
        } else {
            settings
                .relay_profiles
                .iter()
                .find(|profile| !is_managed(profile))
                .map(|profile| profile.id.clone())
                .context("Codex 的直连模式里没有别的供应商可切回：先在 Codex 的供应商页添加一个")?
        };
        settings.active_relay_id = target;
        crate::relay_switch::switch_relay_profile_in_home(
            &ctx.settings,
            &ctx.codex_home,
            settings,
            CODEX_GATEWAY_RELAY_ID,
        )
        .context("把 Codex 切回直连失败")?;
        settings = ctx.settings.load()?;
        // 接入前的实时配置与直连模式的记录对不上：把当时的文件原样写回。
        if let Some(stash) = take_codex_stash(ctx) {
            for (name, contents) in CODEX_LIVE_FILES
                .iter()
                .zip([stash.config_toml, stash.auth_json])
            {
                let path = ctx.codex_home.join(name);
                match contents {
                    Some(contents) => crate::settings::atomic_write(&path, contents.as_bytes())?,
                    None if path.exists() => std::fs::remove_file(&path)?,
                    None => {}
                }
            }
        }
    }
    // 切换会把请求里缺的供应商从磁盘补回，所以托管供应商要在切走之后单独删。
    settings
        .relay_profiles
        .retain(|profile| !is_managed(profile));
    ctx.settings.save(&settings)?;
    Ok(())
}

fn restore_grok(ctx: &ApplyContext, settings: &BackendSettings) -> anyhow::Result<()> {
    let Some(config) = settings.tools.get(&ToolId::Grok) else {
        return Ok(());
    };
    let Some(profile) = crate::tools::grok::active_grok_profile(config) else {
        return Ok(());
    };
    if crate::tools::grok::parse_grok_model_list(&profile.model_list).is_empty() {
        return Ok(());
    }
    crate::tools::grok::apply_profile_to_grok_at(&ctx.grok_home, &profile, &ctx.backup_root)?;
    Ok(())
}

/// 用户在 Claude 供应商列表里直接切到别的供应商：网关绑定随之失效，不再尝试恢复。
pub fn note_claude_switched(store: &GatewayStore, target_id: &str) -> anyhow::Result<()> {
    if target_id == GATEWAY_PROVIDER_ID {
        return Ok(());
    }
    let mut gateway = store.load()?;
    if let Some(binding) = gateway.bindings.get_mut(ToolId::Claude.as_str())
        && binding.enabled
    {
        binding.enabled = false;
        binding.restore = Default::default();
        store.save(&gateway)?;
    }
    Ok(())
}

/// 用户在 Grok 页直接应用了别的供应商：网关绑定随之失效。
pub fn note_grok_switched(store: &GatewayStore) -> anyhow::Result<()> {
    let mut gateway = store.load()?;
    if let Some(binding) = gateway.bindings.get_mut(ToolId::Grok.as_str())
        && binding.enabled
    {
        binding.enabled = false;
        binding.restore = Default::default();
        store.save(&gateway)?;
    }
    Ok(())
}

/// 用户在 Codex 的供应商页直接切换：切到别的供应商等于断开网关；
/// 切回托管的「Codex++ 网关」等于重新接入（断开时它还留在列表里，没被删）。
pub fn note_codex_switched(
    store: &GatewayStore,
    target_id: &str,
    previous_id: &str,
) -> anyhow::Result<()> {
    let mut gateway = store.load()?;
    let Some(binding) = gateway.bindings.get_mut(ToolId::Codex.as_str()) else {
        return Ok(());
    };
    let connected = target_id == CODEX_GATEWAY_RELAY_ID;
    if binding.enabled == connected {
        return Ok(());
    }
    binding.enabled = connected;
    binding.restore = Default::default();
    if connected && previous_id != CODEX_GATEWAY_RELAY_ID {
        binding.restore.codex_previous_relay_id = previous_id.to_string();
    }
    store.save(&gateway)?;
    Ok(())
}

/// 直连模式里切换供应商前：Codex 正被网关接管（当前是托管供应商、绑定开着）且要切到别的供应商时拦下。
pub fn codex_switch_blocked_by_gateway(settings: &SettingsStore, target_id: &str) -> bool {
    if target_id == CODEX_GATEWAY_RELAY_ID {
        return false;
    }
    let on_gateway = settings
        .load()
        .is_ok_and(|saved| saved.active_relay_id == CODEX_GATEWAY_RELAY_ID);
    on_gateway
        && GatewayStore::default()
            .load()
            .ok()
            .and_then(|gateway| {
                gateway
                    .binding(&ToolId::Codex)
                    .map(|binding| binding.enabled)
            })
            .unwrap_or(false)
}

/// Codex 开着网关时，托管供应商的会话身份要跟 Codex 的登录状态一致（登录了 ChatGPT 就保留官方身份）：
/// 不一致（之后才登录 / 退出，或旧版写的）就重写一次。返回是否重写了。
pub fn sync_codex_identity(ctx: &ApplyContext) -> anyhow::Result<bool> {
    let gateway = ctx.gateway.load()?;
    let Some(binding) = gateway
        .binding(&ToolId::Codex)
        .filter(|binding| binding.enabled)
    else {
        return Ok(false);
    };
    let settings = ctx.settings.load()?;
    if settings.active_relay_id != CODEX_GATEWAY_RELAY_ID {
        return Ok(false);
    }
    let keeps_login =
        settings.active_relay_session_provider() == crate::settings::RelaySessionProvider::Openai;
    let can_sign_in = codex_signed_in(&ctx.codex_home) || lendable_login(&gateway).is_some();
    if keeps_login == can_sign_in {
        return Ok(false);
    }
    write_codex(ctx, &gateway, &binding.clone())?;
    Ok(true)
}

/// 网关端口变了（例如 Windows 端口被占后换了新端口）：重写所有开着的 Agent 配置。
pub fn reapply_if_port_changed(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let mut gateway = ctx.gateway.load()?;
    let mut changed = 0;
    let agents = gateway
        .bindings
        .iter()
        .filter(|(_, binding)| binding.enabled && binding.restore.applied_port != Some(ctx.port))
        .map(|(agent, _)| ToolId::parse(agent))
        .collect::<Vec<_>>();
    for agent in agents {
        let Some(binding) = gateway.bindings.get_mut(agent.as_str()) else {
            continue;
        };
        binding.restore.applied_port = Some(ctx.port);
        let binding = binding.clone();
        write_agent_config(ctx, &agent, &binding)?;
        changed += 1;
    }
    if changed > 0 {
        // 写配置时可能改过 gateway.json：只把端口记到最新的那份上。
        let mut latest = ctx.gateway.load()?;
        for (agent, binding) in &gateway.bindings {
            if let Some(target) = latest.bindings.get_mut(agent) {
                target.restore.applied_port = binding.restore.applied_port;
            }
        }
        ctx.gateway.save(&latest)?;
    }
    Ok(changed)
}

/// 退出 Codex++ 时恢复所有走网关的 Agent（用户打开了「退出时恢复直连」）。
/// 绑定保留为开启状态，下次启动时由 [`reapply_enabled`] 重新指回网关。
pub fn restore_all_for_exit(ctx: &ApplyContext, settings: &BackendSettings) -> anyhow::Result<()> {
    let gateway = ctx.gateway.load()?;
    if !gateway.restore_on_exit {
        return Ok(());
    }
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        match ToolId::parse(agent) {
            ToolId::Claude => restore_claude(ctx, binding)?,
            ToolId::Grok => restore_grok(ctx, settings)?,
            ToolId::Codex => restore_codex(ctx, binding)?,
            _ => {}
        }
    }
    Ok(())
}

/// 供应商池或路由组变化后重写开着的 Agent 配置：Grok 的模型列表、Claude 的默认模型随池更新。
/// 不碰恢复现场；某个 Agent 写失败时继续写其它的，最后返回第一个错误。
pub fn refresh_enabled(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let gateway = ctx.gateway.load()?;
    let mut count = 0;
    let mut first_error = None;
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        match write_agent_config(ctx, &ToolId::parse(agent), binding) {
            Ok(_) => count += 1,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(count),
    }
}

/// 启动时把开着的绑定重新写进 Agent 配置（退出时恢复过直连，或端口变了）。
pub fn reapply_enabled(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let gateway = ctx.gateway.load()?;
    let mut count = 0;
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        let agent = ToolId::parse(agent);
        let mut binding = binding.clone();
        binding.restore.applied_port = Some(ctx.port);
        // Claude 恢复过直连时，重新开启前的「上一个供应商」要重新记录。
        if agent == ToolId::Claude {
            let current = claude::current_provider_id(&ctx.claude)?;
            if current != GATEWAY_PROVIDER_ID && !current.is_empty() {
                binding.restore.claude_previous_provider_id = current;
            }
        }
        if agent == ToolId::Codex {
            let current = codex_active_relay_id(ctx)?;
            if !current.is_empty() {
                binding.restore.codex_previous_relay_id = current;
            }
        }
        if let Some(snapshot) = write_agent_config(ctx, &agent, &binding)?
            && binding.restore.claude_previous_provider_id.is_empty()
        {
            binding.restore.claude_previous_provider_id = snapshot;
        }
        let mut latest = ctx.gateway.load()?;
        latest.bindings.insert(agent.as_str().to_string(), binding);
        ctx.gateway.save(&latest)?;
        count += 1;
    }
    Ok(count)
}

/// 把一个 Claude 供应商复制成网关供应商（Anthropic 协议）。只读 Claude 的数据，不改它。
pub fn copy_claude_provider(provider: &ClaudeProvider) -> anyhow::Result<RelayProfile> {
    let env = provider
        .settings_config
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let text = |key: &str| {
        env.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let base_url = text("ANTHROPIC_BASE_URL");
    if base_url.is_empty() {
        bail!(
            "Claude 供应商「{}」没有 ANTHROPIC_BASE_URL，无法导入",
            provider.name
        );
    }
    if base_url.contains("/gw/claude") {
        bail!("「{}」是网关托管的供应商，不能导入", provider.name);
    }
    let (api_key, bearer) = match (text("ANTHROPIC_AUTH_TOKEN"), text("ANTHROPIC_API_KEY")) {
        (token, _) if !token.is_empty() => (token, true),
        (_, key) if !key.is_empty() => (key, false),
        _ => bail!("Claude 供应商「{}」没有 Key，无法导入", provider.name),
    };
    let mut models = Vec::new();
    for key in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
    ] {
        let model = text(key);
        if !model.is_empty() && !models.contains(&model) {
            models.push(model);
        }
    }
    let mut profile = build_relay(&ProviderInput {
        name: provider.name.clone(),
        base_url,
        protocol: RelayProtocol::AnthropicMessages,
        keys: vec![api_key],
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        price: None,
        proxy: None,
    })?;
    if bearer {
        profile.anthropic_auth_header =
            crate::anthropic_bridge::ANTHROPIC_AUTH_HEADER_BEARER.to_string();
    }
    Ok(profile)
}

/// 把 Grok 页的一个供应商复制成网关供应商。只读 Grok 的数据，不改它。
pub fn copy_grok_profile(
    settings: &BackendSettings,
    profile_id: &str,
) -> anyhow::Result<RelayProfile> {
    let source = settings
        .tools
        .get(&ToolId::Grok)
        .and_then(|config| {
            config
                .relay_profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        })
        .with_context(|| format!("Grok 供应商不存在：{profile_id}"))?;
    let base_url = crate::tools::grok::profile_base_url(&source);
    if base_url.contains("/gw/grok") {
        bail!("「{}」指向网关本身，不能导入", source.name);
    }
    let models = crate::tools::grok::parse_grok_model_list(&source.model_list)
        .into_iter()
        .map(|(slug, _)| slug)
        .collect::<Vec<_>>();
    build_relay(&ProviderInput {
        name: source.name.clone(),
        base_url,
        protocol: source.protocol,
        keys: vec![source.api_key.clone()],
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        price: None,
        proxy: None,
    })
}

/// 可以从 Codex 复制过来的供应商：去掉聚合供应商和官方登录账号。
///
/// 官方登录的 refresh token 每用一次就换新的，刷新结果只写回 Codex 自己那一条；
/// 复制一份会让两边互相把对方的登录刷失效，所以网关里的订阅账号要单独登录。
pub fn codex_importable(settings: &BackendSettings) -> Vec<&RelayProfile> {
    settings
        .relay_profiles
        .iter()
        .filter(|relay| {
            relay.relay_mode != RelayMode::Aggregate
                && !crate::official_login::member_uses_official_login_transport(relay)
        })
        .collect()
}

/// 复制一个 Codex 供应商（地址、协议、多 Key、请求头等原样带上）。
pub fn copy_codex_provider(
    settings: &BackendSettings,
    relay_id: &str,
) -> anyhow::Result<RelayProfile> {
    let relay = settings
        .relay_profiles
        .iter()
        .find(|relay| relay.id == relay_id)
        .with_context(|| format!("Codex 供应商不存在：{relay_id}"))?;
    if relay.relay_mode == RelayMode::Aggregate {
        bail!(
            "「{}」是聚合供应商，网关的路由组请在「路由」里新建",
            relay.name
        );
    }
    if crate::official_login::member_uses_official_login_transport(relay) {
        bail!(
            "「{}」是官方登录账号，不能复制：两边共用登录会互相把对方刷失效，请在网关里单独登录",
            relay.name
        );
    }
    if relay.base_url.contains("/gw/") {
        bail!("「{}」指向网关本身，不能导入", relay.name);
    }
    Ok(relay.clone())
}

/// 新建 / 编辑网关供应商的输入。`keys` 每个一行；编辑时为空表示沿用原来的 Key。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub name: String,
    pub base_url: String,
    pub protocol: RelayProtocol,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_list: String,
    /// 每百万 token 的价格；不填就不估算费用。
    #[serde(default)]
    pub price: Option<ProviderPrice>,
    /// 这个供应商单独走的 HTTP 代理；不传表示不改，空串表示不单独走代理。
    #[serde(default)]
    pub proxy: Option<String>,
}

/// 代理地址只接受 http / https / socks5，空串表示不单独走代理。
fn clean_proxy(proxy: &str) -> anyhow::Result<String> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Ok(String::new());
    }
    let ok = ["http://", "https://", "socks5://", "socks5h://"]
        .iter()
        .any(|scheme| proxy.to_ascii_lowercase().starts_with(scheme));
    if !ok || reqwest::Url::parse(proxy).is_err() {
        bail!("代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）");
    }
    Ok(proxy.to_string())
}

fn clean_keys(keys: &[String]) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();
    for key in keys {
        let key = key.trim();
        if !key.is_empty() && !cleaned.iter().any(|known| known == key) {
            cleaned.push(key.to_string());
        }
    }
    cleaned
}

/// 按输入组装上游配置：多个 Key 写成命名 Key，参与多 Key 轮换。
fn build_relay(input: &ProviderInput) -> anyhow::Result<RelayProfile> {
    if input.base_url.contains("/gw/") {
        bail!("不能把网关自己加进供应商池");
    }
    let keys = clean_keys(&input.keys);
    let Some(first) = keys.first() else {
        bail!("API Key 为空");
    };
    let mut relay = crate::provider_import::pool_relay_profile(
        &input.name,
        &input.base_url,
        first,
        input.protocol,
        &input.model,
        &input.model_list,
        &[],
    )?;
    relay.relay_mode = RelayMode::PureApi;
    if keys.len() > 1 {
        relay.api_keys = keys
            .iter()
            .enumerate()
            .map(|(index, key)| RelayApiKey {
                id: format!("key-{}", index + 1),
                name: format!("Key {}", index + 1),
                api_key: key.clone(),
            })
            .collect();
        relay.active_api_key_id = "key-1".to_string();
    }
    crate::relay_config::normalize_relay_profile_for_storage(&mut relay)?;
    Ok(relay)
}

/// 把一份上游配置加进网关池，id 换成网关里不重复的短名。返回新供应商的 id。
pub fn add_provider(
    store: &GatewayStore,
    mut relay: RelayProfile,
    source: &str,
) -> anyhow::Result<(GatewaySettings, String)> {
    let mut gateway = store.load()?;
    // 登录得到的账号名里带邮箱，短名用来源（如 `login:chatgpt` → `chatgpt`）更干净。
    let slug_base = source
        .strip_prefix("login:")
        .unwrap_or(&relay.name)
        .to_string();
    relay.id = gateway.unique_slug(&slug_base, false);
    let id = relay.id.clone();
    gateway.providers.push(GatewayProvider {
        enabled: true,
        source: source.to_string(),
        price: None,
        proxy: String::new(),
        relay,
    });
    store.save(&gateway)?;
    Ok((gateway, id))
}

/// 用预设或手填的地址、Key 新建网关供应商。
pub fn create_provider(
    store: &GatewayStore,
    input: &ProviderInput,
    source: &str,
) -> anyhow::Result<(GatewaySettings, String)> {
    let proxy = clean_proxy(input.proxy.as_deref().unwrap_or_default())?;
    let (mut gateway, id) = add_provider(store, build_relay(input)?, source)?;
    if let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == id)
    {
        provider.price = clean_price(input.price.clone());
        provider.proxy = proxy;
        store.save(&gateway)?;
    }
    Ok((gateway, id))
}

/// 全是 0 的价格等于没填。
fn clean_price(price: Option<ProviderPrice>) -> Option<ProviderPrice> {
    price.filter(|price| price.input > 0.0 || price.cached_input > 0.0 || price.output > 0.0)
}

/// 编辑网关供应商：id 不变（Agent、路由组里的引用都还有效）；`keys` 为空时沿用原来的 Key。
pub fn update_provider(
    store: &GatewayStore,
    id: &str,
    input: &ProviderInput,
) -> anyhow::Result<GatewaySettings> {
    let mut gateway = store.load()?;
    let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == id)
    else {
        bail!("供应商不存在：{id}");
    };
    let mut input = input.clone();
    let keep_keys = clean_keys(&input.keys).is_empty();
    if keep_keys {
        input.keys = if provider.relay.api_keys.len() > 1 {
            provider
                .relay
                .api_keys
                .iter()
                .map(|key| key.api_key.clone())
                .collect()
        } else {
            vec![provider.relay.api_key.clone()]
        };
    }
    let mut relay = build_relay(&input)?;
    relay.id = id.to_string();
    // 原来的请求头、User-Agent、Anthropic 鉴权方式这些高级设置保留。
    relay.custom_headers = provider.relay.custom_headers.clone();
    relay.user_agent = provider.relay.user_agent.clone();
    relay.anthropic_auth_header = provider.relay.anthropic_auth_header.clone();
    provider.relay = relay;
    provider.price = clean_price(input.price.clone());
    if let Some(proxy) = &input.proxy {
        provider.proxy = clean_proxy(proxy)?;
    }
    store.save(&gateway)?;
    Ok(gateway)
}

/// 只改供应商的价格与独立代理（订阅账号也能改；地址、Key、模型不动）。
pub fn set_provider_extras(
    store: &GatewayStore,
    id: &str,
    price: Option<ProviderPrice>,
    proxy: &str,
) -> anyhow::Result<GatewaySettings> {
    let proxy = clean_proxy(proxy)?;
    let mut gateway = store.load()?;
    let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == id)
    else {
        bail!("供应商不存在：{id}");
    };
    provider.price = clean_price(price);
    provider.proxy = proxy;
    store.save(&gateway)?;
    store.load()
}

/// 删除网关供应商：同时清掉路由组里指向它的成员和 Agent「挑选」里的记录。直连模式的数据不受影响。
pub fn delete_provider(store: &GatewayStore, id: &str) -> anyhow::Result<GatewaySettings> {
    let mut gateway = store.load()?;
    let before = gateway.providers.len();
    gateway.providers.retain(|provider| provider.relay.id != id);
    if gateway.providers.len() == before {
        bail!("供应商不存在：{id}");
    }
    let prefix = format!("{id}/");
    for group in &mut gateway.groups {
        group
            .members
            .retain(|member| !member.model.starts_with(&prefix));
    }
    for binding in gateway.bindings.values_mut() {
        binding.pool_exclude.retain(|excluded| excluded != id);
    }
    crate::gateway::key_pool::reset(id);
    store.save(&gateway)?;
    Ok(gateway)
}

/// 打开或关掉池里的某个供应商：关掉后不参与路由，也不出现在 Agent 的模型列表里。
pub fn set_provider_enabled(
    store: &GatewayStore,
    id: &str,
    enabled: bool,
) -> anyhow::Result<GatewaySettings> {
    let mut gateway = store.load()?;
    let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == id)
    else {
        bail!("供应商不存在：{id}");
    };
    provider.enabled = enabled;
    store.save(&gateway)?;
    Ok(gateway)
}

/// 调整池里供应商的顺序（同一个模型有多家时按这个顺序尝试）。没列出的保持原顺序排在后面。
pub fn reorder_providers(store: &GatewayStore, ids: &[String]) -> anyhow::Result<GatewaySettings> {
    let mut gateway = store.load()?;
    let mut ordered: Vec<GatewayProvider> = Vec::new();
    for id in ids {
        if let Some(position) = gateway
            .providers
            .iter()
            .position(|provider| &provider.relay.id == id)
        {
            ordered.push(gateway.providers.remove(position));
        }
    }
    ordered.append(&mut gateway.providers);
    gateway.providers = ordered;
    store.save(&gateway)?;
    Ok(gateway)
}

/// 新建 / 编辑路由组的输入。`id` 为空表示新建。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mode: GroupMode,
    #[serde(default)]
    pub members: Vec<GroupMember>,
}

/// 保存路由组。成员去空、去重；不能直接包含自己。返回组 id。
pub fn save_group(
    store: &GatewayStore,
    input: &GroupInput,
) -> anyhow::Result<(GatewaySettings, String)> {
    let mut gateway = store.load()?;
    let name = input.name.trim();
    if name.is_empty() {
        bail!("请填写路由组名称");
    }
    let id = if input.id.trim().is_empty() {
        gateway.unique_slug(name, true)
    } else {
        input.id.trim().to_string()
    };
    let own = format!("{GROUP_MODEL_PREFIX}{id}");
    let mut members: Vec<GroupMember> = Vec::new();
    for member in &input.members {
        let model = member.model.trim();
        if model.is_empty() || model.eq_ignore_ascii_case(&own) {
            continue;
        }
        if !members.iter().any(|known| known.model == model) {
            members.push(GroupMember {
                model: model.to_string(),
                weight: member.weight.max(1),
            });
        }
    }
    if members.is_empty() {
        bail!("路由组至少要有一个成员");
    }
    let group = GatewayGroup {
        id: id.clone(),
        name: name.to_string(),
        mode: input.mode,
        members,
    };
    match gateway.groups.iter_mut().find(|known| known.id == id) {
        Some(existing) => *existing = group,
        None => gateway.groups.push(group),
    }
    store.save(&gateway)?;
    Ok((gateway, id))
}

/// 删除路由组：其它组里引用它的成员一并去掉。
pub fn delete_group(store: &GatewayStore, id: &str) -> anyhow::Result<GatewaySettings> {
    let mut gateway = store.load()?;
    let before = gateway.groups.len();
    gateway.groups.retain(|group| group.id != id);
    if gateway.groups.len() == before {
        bail!("路由组不存在：{id}");
    }
    let reference = format!("{GROUP_MODEL_PREFIX}{id}");
    for group in &mut gateway.groups {
        group.members.retain(|member| member.model != reference);
    }
    store.save(&gateway)?;
    Ok(gateway)
}

/// 绑定解析后的默认模型，UI 展示用。
pub fn resolved_default_model(gateway: &GatewaySettings, binding: &AgentBinding) -> String {
    route::binding_default_model(gateway, binding)
}
