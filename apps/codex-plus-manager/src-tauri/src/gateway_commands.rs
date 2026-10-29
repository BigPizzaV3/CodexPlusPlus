//! 网关模式命令：Agent 接入开关、网关自己的供应商池与路由组、Key 健康状态。
//!
//! 网关模式与直连模式完全独立：供应商池、路由组只存在 `gateway.json`；
//! 直连模式的 Codex / Claude / Grok 供应商只在「导入」时被读取并复制一份。

use serde::Serialize;

use codex_plus_core::gateway::activity::{self, ActivitySnapshot};
use codex_plus_core::gateway::apply::{self, ApplyContext, GroupInput, ProviderInput};
use codex_plus_core::gateway::key_pool::{self, PoolSnapshot};
use codex_plus_core::gateway::model_sync;
use codex_plus_core::gateway::store::{
    AgentBinding, GatewayGroup, GatewaySettings, GatewayStore, KeyRotationMode, new_client_token,
};
use codex_plus_core::settings::{BackendSettings, RelayProfile, RelayProtocol, SettingsStore};
use codex_plus_core::tools::ToolId;
use codex_plus_core::tools::claude::{self, ClaudeContext, GATEWAY_PROVIDER_ID};

use crate::commands::{CommandResult, failed, ok, sync_resident_helper};

/// 供应商在界面上的样子：不含 Key 原文。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPoolEntry {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub protocol: RelayProtocol,
    pub official: bool,
    pub models: Vec<String>,
    pub key_count: usize,
    pub enabled: bool,
    /// 预设 key 或导入来源，界面判断「已添加」用。
    pub source: String,
    pub price: Option<codex_plus_core::gateway::store::ProviderPrice>,
    /// 按模型的价格（上游模型名 → 价格）。
    pub model_prices:
        std::collections::BTreeMap<String, codex_plus_core::gateway::store::ProviderPrice>,
    /// 能画图的模型（不进聊天模型列表）：ChatGPT 账号是内置的一项，其余按名字挑。
    pub image_models: Vec<String>,
    /// 按模型的模型资料（思考等级等），格式同直连模式的「模型配置」JSON。
    pub model_metadata: String,
    /// 按模型的上下文窗口（上游模型名 → token）。
    pub model_windows: std::collections::BTreeMap<String, u64>,
    /// 没单独写窗口的模型用的窗口。
    pub context_window: Option<u64>,
    /// 登录得到的账号：`chatgpt` / `copilot` / `grok`；普通供应商为空。
    pub account: String,
    /// 订阅账号的套餐（如「ChatGPT Pro」「Copilot Pro+」）；普通供应商为空。
    pub plan: String,
    /// 这个 ChatGPT 账号正借给 Codex 登录（Codex 以官方身份接入网关）。
    pub lent_to_codex: bool,
    /// 网关供应商单独走的 HTTP 代理（空串表示不单独走代理）。
    pub proxy: String,
    /// 转发前移除托管生图工具（#228）。
    pub strip_image_generation_tool: bool,
    /// 上次检查时上游有、列表里没有的模型（界面提示新模型）。
    pub upstream_new: Vec<String>,
    /// 上次检查时列表里有、上游已经没有的模型。
    pub upstream_gone: Vec<String>,
    /// 订阅账号的额度（ChatGPT 5 小时 / 每周、Copilot 本月高级请求）；查不到或普通供应商为空。
    pub quota: Option<codex_plus_core::gateway::quota::ProviderQuota>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAgentView {
    pub agent: String,
    pub name: String,
    /// 本机装的 CLI 版本（后台探测，探测完成前为空）。
    pub version: Option<String>,
    /// 开启网关时改写的配置文件，界面上展示给用户。
    pub config_path: String,
    pub enabled: bool,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
    /// 开着网关、但配置已被别的工具改掉（请求不再经过 Codex++）。
    pub drift: Option<codex_plus_core::gateway::drift::Drift>,
}

/// 界面上的自动组：组本身 + 是否隐藏 + 有没有改过设置。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoGroupView {
    #[serde(flatten)]
    pub group: GatewayGroup,
    pub hidden: bool,
    pub customized: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySourceEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayPayload {
    /// 绑定与选项；供应商池单独放在 `pool`（不带 Key），这里的 `providers` 总是空的。
    pub gateway: GatewaySettings,
    pub port: u16,
    pub helper_running: bool,
    pub pool: Vec<GatewayPoolEntry>,
    pub groups: Vec<GatewayGroup>,
    /// 自动组：池里多家都有的模型各一个（含已隐藏的）。
    pub auto_groups: Vec<AutoGroupView>,
    pub agents: Vec<GatewayAgentView>,
    pub health: PoolSnapshot,
    /// 路由动态：各 Agent 最近一次请求怎么走的、计数、正在回答的上游。
    pub activity: ActivitySnapshot,
    /// 各（供应商, 模型）最近的平均首包耗时，「路由」页、供应商池「按模型」和「速度优先」用。
    pub speeds: Vec<codex_plus_core::gateway::speed::SpeedView>,
    /// 可复制进网关的直连模式供应商。
    pub codex_providers: Vec<GatewayPoolEntry>,
    pub claude_providers: Vec<GatewaySourceEntry>,
    pub grok_profiles: Vec<GatewaySourceEntry>,
    /// OpenCode 自己保存的 OpenCode Go / Zen Key（不含 Key 本身）。
    pub opencode_keys: Vec<GatewaySourceEntry>,
    /// 直连模式的聚合供应商：可以搬进网关（成员进池 + 同名模型组）。
    pub codex_aggregates: Vec<apply::AggregateSource>,
    /// 直连模式的官方登录账号：不能复制，要在网关里单独登录（名字已打码）。
    pub codex_official: Vec<GatewaySourceEntry>,
    /// 「生图模型」能选的：ChatGPT 订阅账号和各家的生图模型。
    pub image_options: Vec<codex_plus_core::gateway::images::ImageOption>,
    /// 生图选「自动」时先用的那一项。
    pub image_auto: Option<codex_plus_core::gateway::images::ImageOption>,
}

const GATEWAY_AGENTS: &[ToolId] = &[ToolId::Codex, ToolId::Claude, ToolId::Grok];

fn port() -> u16 {
    codex_plus_core::resident_helper::resident_helper_port()
        .unwrap_or_else(codex_plus_core::protocol_proxy::protocol_proxy_port)
}

fn context() -> ApplyContext {
    ApplyContext::default_paths(port())
}

fn parse_agent(agent: &str) -> anyhow::Result<ToolId> {
    let agent = ToolId::parse(agent);
    if !GATEWAY_AGENTS.contains(&agent) {
        anyhow::bail!("{} 不支持网关模式", agent.as_str());
    }
    Ok(agent)
}

fn pool_entry(
    relay: &RelayProfile,
    enabled: bool,
    source: &str,
    price: Option<codex_plus_core::gateway::store::ProviderPrice>,
    model_prices: std::collections::BTreeMap<
        String,
        codex_plus_core::gateway::store::ProviderPrice,
    >,
    upstream: (Vec<String>, Vec<String>),
) -> GatewayPoolEntry {
    GatewayPoolEntry {
        id: relay.id.clone(),
        name: relay.name.clone(),
        base_url: relay.base_url.trim().to_string(),
        protocol: relay.protocol,
        official: codex_plus_core::official_login::member_uses_official_login_transport(relay),
        models: codex_plus_core::gateway::route::relay_models(relay),
        key_count: key_pool::expand_keys(relay).len(),
        enabled,
        source: source.to_string(),
        price,
        model_prices,
        model_windows: Default::default(),
        context_window: None,
        model_metadata: relay.model_metadata.clone(),
        image_models: codex_plus_core::gateway::images::provider_image_models(relay),
        account: account_kind(relay),
        plan: codex_plus_core::gateway::accounts::account_plan(relay),
        lent_to_codex: codex_plus_core::official_login::is_lent_to_codex(relay),
        proxy: relay.gateway_proxy.clone().unwrap_or_default(),
        strip_image_generation_tool: relay.strip_image_generation_tool,
        upstream_new: upstream.0,
        upstream_gone: upstream.1,
        quota: codex_plus_core::gateway::quota::get(&relay.id),
    }
}

fn account_kind(relay: &RelayProfile) -> String {
    if codex_plus_core::official_login::member_uses_official_login_transport(relay) {
        return "chatgpt".to_string();
    }
    serde_json::from_str::<serde_json::Value>(&relay.auth_contents)
        .ok()
        .and_then(|value| {
            value
                .get(codex_plus_core::gateway::accounts::ACCOUNT_KIND_FIELD)
                .and_then(|kind| kind.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

fn build_payload(settings: &BackendSettings, gateway: GatewaySettings) -> GatewayPayload {
    let port = port();
    let pool = gateway
        .providers
        .iter()
        .map(|provider| GatewayPoolEntry {
            model_windows: provider.model_windows.clone(),
            context_window: provider.context_window,
            ..pool_entry(
                &provider.relay,
                provider.enabled,
                &provider.source,
                provider.price.clone(),
                provider.model_prices.clone(),
                (
                    provider.upstream_new.clone(),
                    provider.upstream_gone.clone(),
                ),
            )
        })
        .collect();
    let codex_aggregates = apply::codex_aggregates(settings, &gateway);
    let codex_official = apply::codex_official_accounts(settings)
        .into_iter()
        .map(|relay| GatewaySourceEntry {
            id: relay.id.clone(),
            name: codex_plus_core::gateway::accounts::mask_account_name(&relay.name),
        })
        .collect();
    let codex_providers = apply::codex_importable(settings)
        .into_iter()
        .map(|relay| {
            pool_entry(
                relay,
                true,
                "",
                None,
                Default::default(),
                Default::default(),
            )
        })
        .collect();
    let drift_ctx = ApplyContext::default_paths(port);
    let agents = GATEWAY_AGENTS
        .iter()
        .map(|agent| {
            let binding = gateway.binding(agent);
            GatewayAgentView {
                agent: agent.as_str().to_string(),
                name: codex_plus_core::tools::tool_display_name(agent).to_string(),
                version: agent_version(agent),
                config_path: agent_config_path(agent).to_string(),
                enabled: binding.is_some_and(|binding| binding.enabled),
                base_url: apply::gateway_base_url(port, agent),
                default_model: binding
                    .map(|binding| apply::resolved_default_model(&gateway, binding))
                    .unwrap_or_default(),
                models: codex_plus_core::gateway::server::exposed_models(&gateway, agent),
                drift: binding
                    .filter(|binding| binding.enabled)
                    .and_then(|_| codex_plus_core::gateway::drift::check(&drift_ctx, agent)),
            }
        })
        .collect();
    let claude_providers = claude::load(&ClaudeContext::default_paths())
        .map(|payload| {
            payload
                .providers
                .into_iter()
                .filter(|provider| provider.id != GATEWAY_PROVIDER_ID)
                .map(|provider| GatewaySourceEntry {
                    id: provider.id,
                    name: provider.name,
                })
                .collect()
        })
        .unwrap_or_default();
    let grok_profiles = settings
        .tools
        .get(&ToolId::Grok)
        .map(|config| {
            config
                .relay_profiles
                .iter()
                .map(|profile| GatewaySourceEntry {
                    id: profile.id.clone(),
                    name: profile.name.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let groups = gateway.groups.clone();
    let auto_groups = codex_plus_core::gateway::route::auto_groups(&gateway)
        .into_iter()
        .map(|group| {
            let key = group.name.to_ascii_lowercase();
            AutoGroupView {
                hidden: gateway
                    .auto_groups
                    .get(&key)
                    .is_some_and(|settings| settings.hidden),
                customized: gateway.auto_groups.contains_key(&key),
                group,
            }
        })
        .collect();
    // 生图选项要在清掉供应商之前算。
    let image_options = codex_plus_core::gateway::images::options(&gateway);
    let image_auto = codex_plus_core::gateway::images::auto_choice(&gateway);
    let mut gateway = gateway;
    gateway.providers.clear();
    GatewayPayload {
        image_options,
        image_auto,
        gateway,
        port,
        helper_running: codex_plus_core::resident_helper::resident_helper_port().is_some(),
        pool,
        groups,
        auto_groups,
        agents,
        health: key_pool::snapshot(std::time::Instant::now()),
        activity: activity::snapshot(),
        speeds: codex_plus_core::gateway::speed::snapshot(),
        codex_providers,
        claude_providers,
        grok_profiles,
        opencode_keys: codex_plus_core::gateway::opencode::load_stored_keys()
            .into_iter()
            .map(|(key, _)| GatewaySourceEntry {
                id: key.id,
                name: key.name,
            })
            .collect(),
        codex_aggregates,
        codex_official,
    }
}

fn current_payload() -> Option<GatewayPayload> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    let gateway = GatewayStore::default().load().ok()?;
    Some(build_payload(&settings, gateway))
}

static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn set_app_handle(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// 设置改动事件：主窗口收到后重新读取设置。
pub const SETTINGS_CHANGED_EVENT: &str = "codex-plus-settings-changed";

/// Codex 接入网关会改直连模式的设置（加入 / 删除托管供应商、切换当前供应商），
/// Codex 设置页也会经桥接写设置：通知主窗口重新读取，免得它拿旧设置整份写回。
pub(crate) fn notify_settings_changed() {
    if let Some(app) = APP.get() {
        let _ = tauri::Emitter::emit(app, SETTINGS_CHANGED_EVENT, ());
    }
}

fn respond<T>(
    success: &str,
    failure: &str,
    result: anyhow::Result<T>,
) -> CommandResult<Option<GatewayPayload>> {
    notify_settings_changed();
    match result {
        Ok(_) => ok(success, current_payload()),
        Err(error) => failed(&format!("{failure}：{error:#}"), current_payload()),
    }
}

/// 供应商池或路由组变了：开着的 Agent 重写配置，让它的模型列表跟上。重写失败只记日志。
fn refresh_agents<T>(result: anyhow::Result<T>) -> anyhow::Result<T> {
    notify_settings_changed();
    if result.is_ok()
        && let Err(error) = apply::refresh_enabled(&context())
    {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "gateway.refresh_agents_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
    result
}

/// 界面每隔几秒轮询一次：读设置、gateway.json、检查各 Agent 配置漂移都是文件 IO，
/// 放到阻塞线程池里做，不占主线程。返回格式不变。
#[tauri::command]
pub async fn load_gateway() -> CommandResult<Option<GatewayPayload>> {
    match tauri::async_runtime::spawn_blocking(load_gateway_blocking).await {
        Ok(result) => result,
        Err(error) => failed(&format!("读取网关状态失败：{error}"), None),
    }
}

fn load_gateway_blocking() -> CommandResult<Option<GatewayPayload>> {
    let settings = SettingsStore::default().load().unwrap_or_default();
    match GatewayStore::default().load() {
        Ok(gateway) => ok("网关状态已加载。", Some(build_payload(&settings, gateway))),
        Err(error) => failed(&format!("读取网关状态失败：{error:#}"), None),
    }
}

/// 开启或更新一个 Agent 的网关接入：改写它的配置指向网关，并确保常驻 helper 在跑。
#[tauri::command]
pub fn save_gateway_binding(
    agent: String,
    binding: AgentBinding,
) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let agent = parse_agent(&agent)?;
        let saved = apply::enable_binding(&context(), &agent, binding)?;
        if sync_resident_helper().is_none() {
            anyhow::bail!("网关绑定已写入，但本地网关没能启动，请查看诊断日志");
        }
        Ok(saved)
    })();
    let name = codex_plus_core::tools::tool_display_name(&ToolId::parse(&agent));
    respond(
        &format!("{name} 已接入 Codex++ 网关，新开的会话生效。"),
        "接入网关失败",
        result,
    )
}

/// 断开一个 Agent，恢复它原来的直连设置。
#[tauri::command]
pub fn disable_gateway_binding(agent: String) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let agent = parse_agent(&agent)?;
        let settings = SettingsStore::default().load()?;
        apply::disable_binding(&context(), &settings, &agent)
    })();
    let name = codex_plus_core::tools::tool_display_name(&ToolId::parse(&agent));
    respond(
        &format!("{name} 已断开并还原，新开的会话生效。"),
        "断开失败",
        result,
    )
}

/// 给 Agent 换一个新的本地 token（旧 token 立即失效）；开着的绑定同步重写配置。
#[tauri::command]
pub fn rotate_gateway_token(agent: String) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let agent = parse_agent(&agent)?;
        let store = GatewayStore::default();
        // 带锁读改写：只换这个 Agent 的 token，不拿旧状态覆盖同时刷新写回的登录 token。
        let (gateway, binding) = store.update(|gateway| {
            let Some(binding) = gateway.bindings.get_mut(agent.as_str()) else {
                anyhow::bail!("还没有配置网关绑定");
            };
            binding.token = new_client_token(&agent);
            Ok(binding.clone())
        })?;
        if binding.enabled {
            return apply::enable_binding(&context(), &agent, binding);
        }
        Ok(gateway)
    })();
    respond("已更换网关 token。", "更换网关 token 失败", result)
}

/// 网关全局选项：Key 轮换方式、退出时恢复直连。
#[tauri::command]
pub fn save_gateway_options(
    key_rotation: String,
    restore_on_exit: bool,
) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let (gateway, ()) = GatewayStore::default().update(|gateway| {
            gateway.key_policy.mode = match key_rotation.as_str() {
                "roundRobin" => KeyRotationMode::RoundRobin,
                _ => KeyRotationMode::Sticky,
            };
            gateway.restore_on_exit = restore_on_exit;
            Ok(())
        })?;
        Ok(gateway)
    })();
    respond("网关选项已保存。", "保存网关选项失败", result)
}

/// 「多家自动切换」开关：多家都有的模型额外列一份按顺序自动切换的入口。改完重写开着的 Agent 配置。
#[tauri::command]
pub fn save_gateway_auto_models(enabled: bool) -> CommandResult<Option<GatewayPayload>> {
    let result = GatewayStore::default().update(|gateway| {
        gateway.auto_models = enabled;
        Ok(())
    });
    respond(
        "网关选项已保存。",
        "保存网关选项失败",
        refresh_agents(result),
    )
}

/// 「模型名后带供应商」：多家都有的模型在 Codex 的模型列表和输入框里名字后面带不带「（供应商）」。
#[tauri::command]
pub fn save_gateway_model_suffix(enabled: bool) -> CommandResult<Option<GatewayPayload>> {
    let result = GatewayStore::default().update(|gateway| {
        gateway.model_suffix = enabled;
        Ok(())
    });
    respond(
        "网关选项已保存。",
        "保存网关选项失败",
        refresh_agents(result),
    )
}

/// 某个 Agent 的生图模型：空串是自动，`off` 是关闭，其余是 `<供应商 id>/<模型>`。
#[tauri::command]
pub fn save_gateway_image_model(
    agent: String,
    model: String,
) -> CommandResult<Option<GatewayPayload>> {
    let result = parse_agent(&agent).and_then(|agent| {
        GatewayStore::default().update(|gateway| {
            let model = model.trim();
            if model.is_empty() {
                gateway.image_models.remove(agent.as_str());
            } else {
                gateway
                    .image_models
                    .insert(agent.as_str().to_string(), model.to_string());
            }
            Ok(())
        })
    });
    respond("生图模型已保存。", "保存生图模型失败", result)
}

/// 某个 Agent 的生图结果只给链接时，要不要下载下来补成图片数据（默认要）。
#[tauri::command]
pub fn save_gateway_image_inline(
    agent: String,
    enabled: bool,
) -> CommandResult<Option<GatewayPayload>> {
    let result = parse_agent(&agent).and_then(|agent| {
        GatewayStore::default().update(|gateway| {
            gateway.image_keep_urls.retain(|id| id != agent.as_str());
            if !enabled {
                gateway.image_keep_urls.push(agent.as_str().to_string());
            }
            Ok(())
        })
    });
    respond("生图选项已保存。", "保存生图选项失败", result)
}

/// 供应商各模型内置的思考等级（编辑思考等级时当作没改过的显示值）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderReasoningPayload {
    pub models: Vec<apply::ModelReasoningView>,
}

#[tauri::command]
pub fn gateway_provider_reasoning(id: String) -> CommandResult<Option<ProviderReasoningPayload>> {
    let result = GatewayStore::default()
        .load()
        .and_then(|gateway| apply::provider_reasoning(&gateway, &id));
    match result {
        Ok(models) => ok(
            "已读取思考等级。",
            Some(ProviderReasoningPayload { models }),
        ),
        Err(error) => failed(&format!("读取思考等级失败：{error:#}"), None),
    }
}

/// 保存供应商按模型的思考等级（模型资料 JSON）；写进 Agent 的模型目录，存完重写 Agent 配置。
#[tauri::command]
pub fn save_gateway_provider_metadata(
    id: String,
    model_metadata: String,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "思考等级已保存。",
        "保存思考等级失败",
        refresh_agents(apply::set_provider_metadata(
            &GatewayStore::default(),
            &id,
            &model_metadata,
        )),
    )
}

/// 自动组卡片上的设置（模式、会话固定、隐藏成员、手动选中的成员、隐藏整个自动组）。和默认一样时记录会被删掉。
#[tauri::command]
pub fn save_gateway_auto_group(
    model: String,
    settings: codex_plus_core::gateway::store::AutoGroupSettings,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "自动组已保存。",
        "保存自动组失败",
        refresh_agents(apply::save_auto_group(
            &GatewayStore::default(),
            &model,
            settings,
        )),
    )
}

/// 保存模型收藏（管理器模型下拉里的星标）。写给 Codex 的模型目录会标出收藏，所以也要重写开着的 Agent 配置。
#[tauri::command]
pub fn save_gateway_favorites(models: Vec<String>) -> CommandResult<Option<GatewayPayload>> {
    let mut list: Vec<String> = Vec::new();
    for model in models
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
    {
        if !list.iter().any(|known| known == model) {
            list.push(model.to_string());
        }
    }
    let result = GatewayStore::default().update(|gateway| {
        gateway.favorite_models = list;
        Ok(())
    });
    respond("收藏已保存。", "保存收藏失败", refresh_agents(result))
}

/// 用内置预设或手填的地址、Key 新建网关供应商。
#[tauri::command]
pub fn create_gateway_provider(
    input: ProviderInput,
    source: String,
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    let name = input.name.trim().to_string();
    respond(
        &format!("已添加「{name}」，接入的 Agent 现在能用它的模型了。"),
        "添加供应商失败",
        refresh_added(apply::create_provider(
            &GatewayStore::default(),
            &input,
            &source,
            skip_agents.as_deref(),
        )),
    )
}

/// 编辑网关供应商；Key 留空表示不改。
#[tauri::command]
pub fn update_gateway_provider(
    id: String,
    input: ProviderInput,
) -> CommandResult<Option<GatewayPayload>> {
    let result = refresh_agents(apply::update_provider(
        &GatewayStore::default(),
        &id,
        &input,
    ));
    // 表单里可能改了模型列表：新模型按公开价格表补价格。
    if result.is_ok() {
        autofill_prices(Some(id));
    }
    respond("供应商已保存。", "保存供应商失败", result)
}

/// 只改供应商的价格与独立代理（在供应商池的详情里编辑）。
#[tauri::command]
pub fn save_gateway_provider_extras(
    id: String,
    price: Option<codex_plus_core::gateway::store::ProviderPrice>,
    model_prices: Option<
        std::collections::BTreeMap<String, codex_plus_core::gateway::store::ProviderPrice>,
    >,
    proxy: String,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "已保存。",
        "保存失败",
        apply::set_provider_extras(&GatewayStore::default(), &id, price, model_prices, &proxy),
    )
}

/// 改供应商的上下文窗口（按模型的和其余模型用的默认值）；窗口写在 Agent 的模型资料里，存完重写 Agent 配置。
#[tauri::command]
pub fn save_gateway_provider_windows(
    id: String,
    windows: std::collections::BTreeMap<String, u64>,
    fallback: Option<u64>,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "上下文窗口已保存。",
        "保存上下文窗口失败",
        refresh_agents(apply::set_provider_windows(
            &GatewayStore::default(),
            &id,
            windows,
            fallback,
        )),
    )
}

/// 用存下的凭据重新拉一个供应商的模型，和现有列表比较（不改列表，只记下差异供界面提示）。
#[tauri::command]
pub async fn check_gateway_provider_models(
    id: String,
) -> CommandResult<Option<model_sync::ModelDiff>> {
    match model_sync::check_provider(&GatewayStore::default(), &id).await {
        Ok(diff) => ok(
            &format!(
                "上游有 {} 个模型：新增 {} 个，已下架 {} 个。",
                diff.upstream.len(),
                diff.added.len(),
                diff.removed.len()
            ),
            Some(diff),
        ),
        Err(error) => failed(&format!("获取模型列表失败：{error:#}"), None),
    }
}

/// 供应商详情里的「从上游获取」：上游新增的模型直接加进列表，已有的不删。
#[tauri::command]
pub async fn fetch_gateway_provider_models(id: String) -> CommandResult<Option<GatewayPayload>> {
    let result = model_sync::fetch_provider_models(&GatewayStore::default(), &id).await;
    let message = match &result {
        Ok((_, diff)) if diff.added.is_empty() => format!(
            "上游有 {} 个模型，列表已是最新。",
            diff.upstream.len()
        ),
        Ok((_, diff)) => format!(
            "上游有 {} 个模型，已加入 {} 个：{}。",
            diff.upstream.len(),
            diff.added.len(),
            diff.added.join("、")
        ),
        Err(_) => String::new(),
    };
    let gone = result.as_ref().map_or(0, |(_, diff)| diff.removed.len());
    let message = if gone > 0 {
        format!("{message}另有 {gone} 个上游已没有，可在「编辑」里移除。")
    } else {
        message
    };
    let added = result.as_ref().is_ok_and(|(_, diff)| !diff.added.is_empty());
    let result = refresh_agents(result);
    if added {
        autofill_prices(Some(id));
    }
    respond(&message, "获取模型列表失败", result)
}

/// 保存手动编辑或刷新后的模型列表；新加的模型按公开价格表补价格。
#[tauri::command]
pub fn save_gateway_provider_models(
    id: String,
    models: Vec<String>,
    default_model: Option<String>,
) -> CommandResult<Option<GatewayPayload>> {
    let result = refresh_agents(model_sync::set_provider_models(
        &GatewayStore::default(),
        &id,
        &models,
        default_model.as_deref(),
    ));
    if result.is_ok() {
        autofill_prices(Some(id));
    }
    respond("模型列表已保存。", "保存模型列表失败", result)
}

/// 添加供应商之后：同 [`refresh_agents`]，并在后台按公开价格表给新模型填好价格。
fn refresh_added<T>(result: anyhow::Result<T>) -> anyhow::Result<T> {
    if result.is_ok() {
        autofill_prices(None);
        refresh_quotas();
    }
    refresh_agents(result)
}

/// 一键同步价格：按公开价格表更新这个供应商所有模型的价格并保存。
#[tauri::command]
pub async fn sync_gateway_provider_prices(id: String) -> CommandResult<Option<GatewayPayload>> {
    match model_sync::sync_public_prices(&GatewayStore::default(), &id).await {
        Ok((0, total)) => failed(
            &format!("公开价格表里没找到这 {total} 个模型，需要的话手动填写。"),
            current_payload(),
        ),
        Ok((found, total)) => ok(
            &format!("已按公开价格表更新 {found} / {total} 个模型的价格。"),
            current_payload(),
        ),
        Err(error) => failed(&format!("同步价格失败：{error:#}"), current_payload()),
    }
}

/// 后台按公开价格表给没填价的模型补价格（添加供应商、改模型列表后调用）。补完由界面的轮询读到。
fn autofill_prices(only: Option<String>) {
    tauri::async_runtime::spawn(async move {
        let _ = model_sync::fill_missing_prices(&GatewayStore::default(), only.as_deref()).await;
    });
}

/// 后台查一遍订阅账号的额度（刚登录的账号不用等下一轮）。
fn refresh_quotas() {
    tauri::async_runtime::spawn(async move {
        let Ok(gateway) = GatewayStore::default().load() else {
            return;
        };
        for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
            if codex_plus_core::gateway::quota::supports(&provider.relay) {
                let _ = codex_plus_core::gateway::quota::refresh(&provider.relay).await;
            }
        }
    });
}

/// 手动刷新一个账号的额度。
#[tauri::command]
pub async fn refresh_gateway_quota(id: String) -> CommandResult<Option<GatewayPayload>> {
    let relay = GatewayStore::default()
        .load()
        .ok()
        .and_then(|gateway| gateway.provider(&id).map(|provider| provider.relay.clone()));
    let Some(relay) = relay else {
        return failed(&format!("供应商不存在：{id}"), current_payload());
    };
    match codex_plus_core::gateway::quota::refresh(&relay).await {
        Ok(Some(_)) => ok("额度已刷新。", current_payload()),
        Ok(None) => failed("这个供应商没有可查的额度。", current_payload()),
        Err(error) => failed(&format!("查询额度失败：{error:#}"), current_payload()),
    }
}

/// 后台任务：启动时补一次价格，之后每小时看一眼，超过一天没检查的供应商重新拉上游模型；
/// 另外每 10 分钟查一遍订阅账号的额度。
pub fn start_model_watch() {
    tauri::async_runtime::spawn(model_sync::watch(GatewayStore::default()));
    tauri::async_runtime::spawn(codex_plus_core::gateway::quota::watch(
        GatewayStore::default(),
    ));
}

/// 删除网关供应商（直连模式的数据不受影响）。
#[tauri::command]
pub fn delete_gateway_provider(id: String) -> CommandResult<Option<GatewayPayload>> {
    // ChatGPT API 账号：删除时顺带让 OpenAI 吊销它的 refresh token（失败不影响删除）。
    if let Some(relay) = GatewayStore::default()
        .load()
        .ok()
        .and_then(|gateway| gateway.provider(&id).map(|provider| provider.relay.clone()))
        .filter(|relay| {
            codex_plus_core::gateway::accounts::relay_account_kind(relay).as_deref()
                == Some("chatgpt-api")
        })
    {
        tauri::async_runtime::spawn(async move {
            let _ = codex_plus_core::gateway::accounts::chatgpt_api_revoke(&relay).await;
        });
    }
    // 正借给 Codex 登录的账号：先从 Codex 收回，免得 Codex 留着一个网关已经不管的登录。
    let store = GatewayStore::default();
    if store.load().ok().and_then(|gateway| {
        gateway
            .provider(&id)
            .map(|provider| codex_plus_core::official_login::is_lent_to_codex(&provider.relay))
    }) == Some(true)
    {
        let _ = apply::return_lent_login(
            &store,
            &codex_plus_core::relay_config::default_codex_home_dir(),
        );
    }
    respond(
        "已删除。",
        "删除供应商失败",
        refresh_agents(apply::delete_provider(&GatewayStore::default(), &id)),
    )
}

/// 打开或关掉池里的某个供应商。
#[tauri::command]
pub fn set_gateway_provider_enabled(
    id: String,
    enabled: bool,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        if enabled {
            "已打开，接入的 Agent 能用它的模型了。"
        } else {
            "已关闭，它不再参与调度。"
        },
        "切换供应商失败",
        refresh_agents(apply::set_provider_enabled(
            &GatewayStore::default(),
            &id,
            enabled,
        )),
    )
}

/// 调整供应商池顺序。
#[tauri::command]
pub fn reorder_gateway_providers(ids: Vec<String>) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "已调整顺序。",
        "调整顺序失败",
        refresh_agents(apply::reorder_providers(&GatewayStore::default(), &ids)),
    )
}

/// 把 OpenCode 里保存的 OpenCode Go / Zen Key 加进网关池，模型取目录里网关能用的。
async fn import_opencode(id: &str, skip_agents: Option<&[String]>) -> anyhow::Result<String> {
    use codex_plus_core::gateway::opencode;
    let (entry, key) = opencode::load_stored_keys()
        .into_iter()
        .find(|(entry, _)| entry.id == id)
        .ok_or_else(|| anyhow::anyhow!("OpenCode 里没有 {id} 的 Key"))?;
    let base_url = opencode::base_url_of(&entry.id)
        .ok_or_else(|| anyhow::anyhow!("不认识的 OpenCode 网关：{id}"))?;
    let models = match opencode::plan_of(base_url) {
        Some(plan) => opencode::catalog_models(plan).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let input = ProviderInput {
        name: entry.name.clone(),
        base_url: base_url.to_string(),
        protocol: codex_plus_core::settings::RelayProtocol::ChatCompletions,
        keys: vec![key],
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        price: None,
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
    };
    apply::create_provider(
        &GatewayStore::default(),
        &input,
        &format!("opencode:{id}"),
        skip_agents,
    )?;
    Ok(if models.is_empty() {
        format!(
            "已添加「{}」。没读到模型目录，请在供应商里获取模型。",
            entry.name
        )
    } else {
        format!("已添加「{}」，{} 个模型。", entry.name, models.len())
    })
}

/// 批量导入里的一项：`source` 是 `codex` / `claude` / `grok` / `opencode` / `aggregate`。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImportItem {
    pub source: String,
    pub id: String,
}

/// 从直连模式（和 OpenCode）一次导入多个供应商。已在池里的跳过；聚合供应商搬成「成员进池 + 同名模型组」。
/// 每项单独成败，结果汇总成一句话，没法照搬的地方附在后面。
#[tauri::command]
pub async fn import_gateway_providers(
    items: Vec<ImportItem>,
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    let skip = skip_agents.as_deref();
    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut groups: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for item in &items {
        let already = GatewayStore::default().load().is_ok_and(|gateway| {
            gateway
                .providers
                .iter()
                .any(|provider| provider.source == format!("{}:{}", item.source, item.id))
        });
        if already {
            skipped += 1;
            continue;
        }
        let result: anyhow::Result<()> = match item.source.as_str() {
            "opencode" => import_opencode(&item.id, skip).await.map(|_| added += 1),
            "aggregate" => (|| {
                let settings = SettingsStore::default().load()?;
                let outcome = apply::import_codex_aggregate(
                    &GatewayStore::default(),
                    &settings,
                    &item.id,
                    skip,
                )?;
                // 部分成员失败时 outcome 照样带着已导入的数目，失败原因在 notes 里。
                added += outcome.added;
                notes.extend(outcome.notes);
                if let Some(id) = outcome.group {
                    let name = GatewayStore::default()
                        .load()
                        .ok()
                        .and_then(|gateway| gateway.group(&id).map(|group| group.name.clone()))
                        .unwrap_or(id);
                    groups.push(name);
                }
                Ok(())
            })(),
            source => import_from_direct(source, &item.id, skip).map(|_| added += 1),
        };
        if let Err(error) = result {
            failures.push(format!("{error:#}"));
        }
    }
    let _ = refresh_added(Ok(()));
    let mut message = format!("已导入 {added} 个供应商");
    if !groups.is_empty() {
        message.push_str(&format!("，建好模型组「{}」", groups.join("」「")));
    }
    if skipped > 0 {
        message.push_str(&format!("，{skipped} 个已在池里"));
    }
    message.push('。');
    for note in notes.iter().chain(failures.iter()) {
        message.push_str(&format!("\n· {note}"));
    }
    if added == 0 && groups.is_empty() && !failures.is_empty() {
        failed(&message, current_payload())
    } else {
        ok(&message, current_payload())
    }
}

/// 从直连模式复制一项进池：`codex` / `claude` / `grok`。
fn import_from_direct(source: &str, id: &str, skip: Option<&[String]>) -> anyhow::Result<String> {
    let relay = match source {
        "codex" => apply::copy_codex_provider(&SettingsStore::default().load()?, id)?,
        "claude" => {
            let provider = claude::load(&ClaudeContext::default_paths())?
                .providers
                .into_iter()
                .find(|provider| provider.id == id)
                .ok_or_else(|| anyhow::anyhow!("Claude 供应商不存在：{id}"))?;
            apply::copy_claude_provider(&provider)?
        }
        "grok" => apply::copy_grok_profile(&SettingsStore::default().load()?, id)?,
        other => anyhow::bail!("不认识的导入来源：{other}"),
    };
    let (_, id) = apply::add_provider(
        &GatewayStore::default(),
        relay,
        &format!("{source}:{id}"),
        skip,
    )?;
    Ok(id)
}

/// 新建或编辑网关路由组。
#[tauri::command]
pub fn save_gateway_group(group: GroupInput) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "模型组已保存。",
        "保存模型组失败",
        refresh_agents(apply::save_group(&GatewayStore::default(), &group)),
    )
}

/// 删除网关路由组。
#[tauri::command]
pub fn delete_gateway_group(id: String) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "模型组已删除。",
        "删除模型组失败",
        refresh_agents(apply::delete_group(&GatewayStore::default(), &id)),
    )
}

/// 在访达 / 资源管理器里显示网关改动的 Agent 配置文件。
#[tauri::command]
pub fn reveal_gateway_agent_config(agent: String) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let agent = parse_agent(&agent)?;
        let relative = agent_config_path(&agent).trim_start_matches("~/");
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("找不到用户目录"))?;
        let path = home.join(relative);
        if !path.exists() {
            anyhow::bail!("{} 还不存在", path.display());
        }
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status()?;
        #[cfg(target_os = "windows")]
        let status = std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .status()?;
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let status = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(&path))
            .status()?;
        let _ = status;
        GatewayStore::default().load()
    })();
    respond("已在文件管理器中显示。", "打开失败", result)
}

fn agent_config_path(agent: &ToolId) -> &'static str {
    match agent {
        ToolId::Claude => "~/.claude/settings.json",
        ToolId::Grok => "~/.grok/config.toml",
        _ => "",
    }
}

/// Agent CLI 版本：第一次读取时在后台线程探测并缓存，探测完成前返回 None。
fn agent_version(agent: &ToolId) -> Option<String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, Once, OnceLock};
    static VERSIONS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    static PROBE: Once = Once::new();
    let versions = VERSIONS.get_or_init(Mutex::default);
    PROBE.call_once(|| {
        std::thread::spawn(|| {
            for (agent, binary) in [("claude", "claude"), ("grok", "grok")] {
                if let Some(version) = probe_cli_version(binary) {
                    let mut guard = VERSIONS
                        .get_or_init(Mutex::default)
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.insert(agent.to_string(), version);
                }
            }
        });
    });
    versions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(agent.as_str())
        .cloned()
}

/// 在 PATH 和常见安装目录里找 CLI，跑 `--version`（最多等 4 秒），取第一个形如 1.2.3 的片段。
fn probe_cli_version(binary: &str) -> Option<String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let mut dirs_to_try: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for extra in [
        ".local/bin",
        ".claude/local",
        ".grok/bin",
        ".npm-global/bin",
        ".bun/bin",
        ".volta/bin",
    ] {
        dirs_to_try.push(home.join(extra));
    }
    dirs_to_try.push("/opt/homebrew/bin".into());
    dirs_to_try.push("/usr/local/bin".into());
    let path = dirs_to_try
        .into_iter()
        .map(|dir| dir.join(binary))
        .find(|candidate| candidate.is_file())?;
    let mut child = std::process::Command::new(path)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    parse_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_version(text: &str) -> Option<String> {
    text.split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ')' || ch == ',')
        .map(|token| token.trim_start_matches('v'))
        .find(|token| {
            token.split('.').count() >= 2
                && token.split('.').all(|part| {
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                })
                && token.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::parse_version;

    #[test]
    fn cli_version_is_the_first_dotted_number() {
        assert_eq!(
            parse_version("2.1.310 (Claude Code)\n").as_deref(),
            Some("2.1.310")
        );
        assert_eq!(
            parse_version("grok v0.4.2-beta").as_deref(),
            Some("0.4.2-beta")
        );
        assert_eq!(parse_version("no version here"), None);
    }
}

/// 用量汇总。`range`：`today` / `7d` / `30d` / `all`。
#[tauri::command]
pub fn load_gateway_usage(
    range: String,
) -> CommandResult<Option<codex_plus_core::gateway::usage::UsageSummary>> {
    use codex_plus_core::gateway::usage::{UsageStore, local_day};
    let days = match range.as_str() {
        "today" => Some(1),
        "7d" => Some(7),
        "all" => None,
        _ => Some(30),
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default();
    let gateway = GatewayStore::default().load().unwrap_or_default();
    match UsageStore::default().summary(&local_day(now), days, &gateway) {
        Ok(summary) => ok("用量已加载。", Some(summary)),
        Err(error) => failed(&format!("读取用量失败：{error:#}"), None),
    }
}

/// 清空用量统计（请求日志不受影响）。
#[tauri::command]
pub fn clear_gateway_usage() -> CommandResult<Option<GatewayPayload>> {
    respond(
        "用量统计已清空。",
        "清空用量失败",
        codex_plus_core::gateway::usage::UsageStore::default().clear(),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayBalances {
    pub balances: Vec<codex_plus_core::gateway::balance::ProviderBalance>,
}

/// 查池里有余额接口的供应商的余额（联网，每家最多 10 秒）。
#[tauri::command]
pub async fn load_gateway_balances() -> CommandResult<GatewayBalances> {
    let gateway = GatewayStore::default().load().unwrap_or_default();
    let balances = codex_plus_core::gateway::balance::fetch_balances(&gateway).await;
    ok("余额已刷新。", GatewayBalances { balances })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessions {
    pub sessions: Vec<codex_plus_core::gateway::sessions::AgentSession>,
}

/// Claude Code / Grok 的本地会话（Codex 的会话走 `list_local_sessions`）。
#[tauri::command]
pub async fn list_agent_sessions(agent: String) -> CommandResult<AgentSessions> {
    let sessions = tauri::async_runtime::spawn_blocking(move || {
        use codex_plus_core::gateway::sessions;
        match agent.as_str() {
            "claude" => {
                sessions::list_claude_sessions(&ClaudeContext::default_paths().claude_home, 300)
            }
            "grok" => sessions::list_grok_sessions(
                &codex_plus_core::grok_config::default_grok_home_dir(),
                300,
            ),
            _ => Vec::new(),
        }
    })
    .await
    .unwrap_or_default();
    ok("会话已加载。", AgentSessions { sessions })
}

/// 删掉的会话移到了哪里。CommandResult 会把 payload 摊平，只能是结构体，不能是字符串。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedSession {
    pub trash_path: Option<String>,
}

/// 删除 Claude Code / Grok 的一个本地会话：移进废纸篓，可以找回。Codex 的会话走 `delete_local_session`。
#[tauri::command]
pub fn delete_agent_session(agent: String, id: String) -> CommandResult<DeletedSession> {
    use codex_plus_core::gateway::sessions;
    let trash = sessions::default_trash_dir();
    let result = match agent.as_str() {
        "claude" => sessions::delete_claude_session(
            &ClaudeContext::default_paths().claude_home,
            &id,
            &trash,
        ),
        "grok" => sessions::delete_grok_session(
            &codex_plus_core::grok_config::default_grok_home_dir(),
            &id,
            &trash,
        ),
        other => Err(anyhow::anyhow!("{other} 的会话不在这里删除")),
    };
    match result {
        Ok(path) => ok(
            "会话已移到废纸篓，需要时可以从废纸篓找回。",
            DeletedSession {
                trash_path: Some(path.display().to_string()),
            },
        ),
        Err(error) => failed(&format!("删除失败：{error:#}"), DeletedSession::default()),
    }
}

/// 在终端里继续一个会话（执行 Agent 自己的恢复命令）。
#[tauri::command]
pub fn resume_agent_session(
    agent: String,
    id: String,
    cwd: String,
) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        use codex_plus_core::gateway::sessions;
        let command = sessions::resume_command(&agent, &id)?;
        let scratch = codex_plus_core::paths::default_app_state_dir().join("tmp");
        sessions::open_in_terminal(&cwd, &command, &scratch)
    })();
    match result {
        Ok(()) => ok("已在终端里打开。", None),
        Err(error) => failed(&format!("打开失败：{error:#}"), None),
    }
}

/// 用系统默认浏览器打开登录页。
/// Windows 不能用 `cmd /c start`：授权地址里的 `&` 会被 cmd 当成命令分隔符，参数全丢。
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    if let Err(error) = codex_plus_core::windows_open_url(url) {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "gateway.login_browser_failed",
            serde_json::json!({ "error": format!("{error:#}") }),
        );
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// 进行中的订阅登录（ChatGPT / ChatGPT API / SuperGrok 的浏览器回调、Copilot 的设备码轮询）。
/// 同一时间只留一个：登录跑在单独的任务里，取消或开始新的登录时 abort 掉它，
/// 回调端口随监听器一起释放、轮询随任务停止。存的是（序号，中止函数）。
type LoginAbort = (u64, Box<dyn Fn() + Send>);
static LOGIN_TASK: std::sync::Mutex<Option<LoginAbort>> = std::sync::Mutex::new(None);
static LOGIN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// 登录被用户取消时的提示；界面按 status `cancelled` 识别，不弹失败提示。
const LOGIN_CANCELLED: &str = "登录已取消。";

fn login_slot() -> std::sync::MutexGuard<'static, Option<LoginAbort>> {
    LOGIN_TASK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 登录的结局：完成（成功或失败）或被取消。
enum LoginOutcome<T> {
    Done(anyhow::Result<T>),
    Cancelled,
}

/// 把一次登录放进可取消的任务里跑完。开始前先中止上一次还没结束的登录。
async fn cancellable_login<T, F>(login: F) -> LoginOutcome<T>
where
    T: Send + 'static,
    F: std::future::Future<Output = anyhow::Result<T>> + Send + 'static,
{
    // 先中止上一次、等它放掉回调端口，再开始新的：不然新登录绑端口时旧的监听可能还没释放。
    let previous = login_slot().take();
    if let Some((_, abort_previous)) = previous {
        abort_previous();
        let _ = tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(200))
        })
        .await;
    }
    let handle = tauri::async_runtime::spawn(login);
    let id = LOGIN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    {
        let abort = handle.inner().abort_handle();
        let mut slot = login_slot();
        if let Some((_, previous)) = slot.take() {
            previous();
        }
        *slot = Some((id, Box::new(move || abort.abort())));
    }
    let joined = handle.await;
    {
        let mut slot = login_slot();
        if slot.as_ref().is_some_and(|(current, _)| *current == id) {
            *slot = None;
        }
    }
    match joined {
        Ok(result) => LoginOutcome::Done(result),
        Err(tauri::Error::JoinError(error)) if error.is_cancelled() => LoginOutcome::Cancelled,
        Err(error) => LoginOutcome::Done(Err(anyhow::anyhow!("登录任务异常退出：{error}"))),
    }
}

/// 登录命令统一的返回：成功刷新价格 / 额度 / Agent，取消时返回 `cancelled`。
fn login_response(outcome: LoginOutcome<String>) -> CommandResult<Option<GatewayPayload>> {
    match outcome {
        LoginOutcome::Cancelled => CommandResult {
            status: "cancelled".to_string(),
            message: LOGIN_CANCELLED.to_string(),
            payload: current_payload(),
        },
        LoginOutcome::Done(result) => match refresh_added(result) {
            Ok(message) => ok(&message, current_payload()),
            Err(error) => failed(&format!("登录失败：{error:#}"), current_payload()),
        },
    }
}

/// 取消进行中的订阅登录（关掉验证码弹窗、点「取消登录」时调用）。没有进行中的登录时什么也不做。
#[tauri::command]
pub fn cancel_gateway_login() -> CommandResult<()> {
    let cancelled = login_slot().take().map(|(_, abort)| abort()).is_some();
    ok(
        if cancelled {
            LOGIN_CANCELLED
        } else {
            "没有进行中的登录。"
        },
        (),
    )
}

#[cfg(test)]
mod login_tests {
    use super::{LoginOutcome, cancel_gateway_login, cancellable_login, login_slot};

    /// 等后台登录任务登记好中止函数。
    fn wait_registered() {
        for _ in 0..200 {
            if login_slot().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("登录任务没有登记");
    }

    #[test]
    fn login_can_be_cancelled_and_a_new_login_aborts_the_previous_one() {
        tauri::async_runtime::block_on(async {
            // 取消：一直等不到回调的登录被中止，返回 Cancelled，登记位清空。
            let waiting = tauri::async_runtime::spawn(cancellable_login(std::future::pending::<
                anyhow::Result<String>,
            >()));
            wait_registered();
            assert_eq!(cancel_gateway_login().status, "ok");
            assert!(matches!(waiting.await.unwrap(), LoginOutcome::Cancelled));
            assert!(login_slot().is_none());

            // 开始新的登录时中止上一次；新的照常完成。
            let first = tauri::async_runtime::spawn(cancellable_login(std::future::pending::<
                anyhow::Result<String>,
            >()));
            wait_registered();
            let second = cancellable_login(async { Ok("done".to_string()) }).await;
            assert!(matches!(second, LoginOutcome::Done(Ok(ref message)) if message == "done"));
            assert!(matches!(first.await.unwrap(), LoginOutcome::Cancelled));
            assert!(login_slot().is_none());
        });
    }
}

/// 在网关里登录 ChatGPT 订阅：浏览器登录后加进网关的供应商池。同一个账号再登录只更新 token。
/// 与 Codex 自己的登录是两次独立的登录，互不影响。
#[tauri::command]
pub async fn login_gateway_chatgpt(
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let outcome = cancellable_login(async move {
        let account = accounts::chatgpt_login(open_in_browser).await?;
        let store = GatewayStore::default();
        // 同一个人的同一个账号已经登录过：只换 token。工作区成员共用账号 id，还要比用户；
        // 带锁读改写，只动这一个供应商。
        let (_, updated) = store.update(|gateway| {
            Ok(gateway
                .providers
                .iter_mut()
                .find(|provider| {
                    accounts::same_chatgpt_login(
                        &provider.relay.auth_contents,
                        &account.auth_contents,
                    )
                })
                .map(|provider| {
                    provider.relay.auth_contents = account.auth_contents.clone();
                    provider.relay.name.clone()
                }))
        })?;
        if let Some(name) = updated {
            return Ok(format!("已更新「{name}」的登录。"));
        }
        let mut relay = accounts::chatgpt_relay(&account, &[]);
        let models = codex_plus_core::model_catalog::fetch_relay_profile_model_ids(&relay)
            .await
            .map(|(models, _)| models)
            .unwrap_or_default();
        relay = accounts::chatgpt_relay(&account, &models);
        let name = relay.name.clone();
        apply::add_provider(&store, relay, "login:chatgpt", skip_agents.as_deref())?;
        Ok(if models.is_empty() {
            format!("已添加「{name}」。没读到模型列表，请在供应商里编辑补上。")
        } else {
            format!("已添加「{name}」，{} 个模型。", models.len())
        })
    })
    .await;
    login_response(outcome)
}

/// 账号类供应商（Copilot / SuperGrok）加进网关池；同一个账号（按 `identity_field`）已存在时只换凭据。
fn add_account_provider(
    relay: codex_plus_core::settings::RelayProfile,
    kind: &str,
    identity_field: &str,
    skip_agents: Option<&[String]>,
) -> anyhow::Result<String> {
    use codex_plus_core::gateway::accounts::ACCOUNT_KIND_FIELD;
    let identity = |contents: &str| {
        serde_json::from_str::<serde_json::Value>(contents)
            .ok()
            .filter(|value| {
                value.get(ACCOUNT_KIND_FIELD).and_then(|kind| kind.as_str()) == Some(kind)
            })
            .and_then(|value| {
                value
                    .get(identity_field)
                    .and_then(|field| field.as_str())
                    .map(str::to_string)
            })
            .filter(|value| !value.is_empty())
    };
    let store = GatewayStore::default();
    // 同一个账号已存在：带锁读改写，只换这一个供应商的凭据（和模型列表）。
    if let Some(id) = identity(&relay.auth_contents) {
        let (_, updated) = store.update(|gateway| {
            Ok(gateway
                .providers
                .iter_mut()
                .find(|provider| {
                    identity(&provider.relay.auth_contents).as_deref() == Some(id.as_str())
                })
                .map(|provider| {
                    provider.relay.auth_contents = relay.auth_contents.clone();
                    if !relay.model_list.trim().is_empty() {
                        provider.relay.model_list = relay.model_list.clone();
                    }
                    provider.relay.name.clone()
                }))
        })?;
        if let Some(name) = updated {
            return Ok(format!("已更新「{name}」的登录。"));
        }
    }
    let name = relay.name.clone();
    let count = relay
        .model_list
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    apply::add_provider(&store, relay, &format!("login:{kind}"), skip_agents)?;
    Ok(format!("已添加「{name}」，{count} 个模型。"))
}

/// Copilot 登录第一步：向 GitHub 要设备验证码，并在浏览器打开输入验证码的页面。
#[tauri::command]
pub async fn start_gateway_copilot_login()
-> CommandResult<Option<codex_plus_core::gateway::accounts::DeviceCode>> {
    match codex_plus_core::gateway::accounts::copilot_device_code().await {
        Ok(code) => {
            open_in_browser(&code.verification_uri);
            ok("请在浏览器里输入验证码。", Some(code))
        }
        Err(error) => failed(&format!("登录失败：{error:#}"), None),
    }
}

/// Copilot 登录第二步：等用户在 GitHub 上授权，换 Copilot token，加进网关池。
#[tauri::command]
pub async fn finish_gateway_copilot_login(
    code: codex_plus_core::gateway::accounts::DeviceCode,
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    let outcome = cancellable_login(async move {
        let relay = codex_plus_core::gateway::accounts::copilot_finish(&code).await?;
        add_account_provider(relay, "copilot", "login", skip_agents.as_deref())
    })
    .await;
    login_response(outcome)
}

/// 在网关里登录 SuperGrok：浏览器登录后加进网关池。与 Grok CLI 自己的登录互不影响。
#[tauri::command]
pub async fn login_gateway_grok(
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let outcome = cancellable_login(async move {
        let tokens = accounts::grok_login(open_in_browser).await?;
        let models = accounts::grok_models(&tokens.access_token).await;
        add_account_provider(
            accounts::grok_relay(&tokens, &models),
            "grok",
            "email",
            skip_agents.as_deref(),
        )
    })
    .await;
    login_response(outcome)
}

/// 在网关里登录 ChatGPT API（OpenAI 的「用 ChatGPT 登录」）：套餐额度直接用在 api.openai.com 上，
/// Agent 的指令原样保留。与 ChatGPT（Codex 订阅）是两种独立的登录。
#[tauri::command]
pub async fn login_gateway_chatgpt_api(
    skip_agents: Option<Vec<String>>,
) -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let outcome = cancellable_login(async move {
        let state_dir = codex_plus_core::paths::default_app_state_dir();
        let account = accounts::chatgpt_api_login(&state_dir, open_in_browser).await?;
        let models = accounts::chatgpt_api_models(
            &account.access_token,
            &codex_plus_core::codex_home::default_codex_home_dir(),
        )
        .await;
        add_account_provider(
            accounts::chatgpt_api_relay(&account, &models),
            "chatgpt-api",
            "subject",
            skip_agents.as_deref(),
        )
    })
    .await;
    login_response(outcome)
}

/// 清掉某个供应商的 Key 冷却与熔断状态。
#[tauri::command]
pub fn reset_gateway_key_health(relay_id: String) -> CommandResult<Option<GatewayPayload>> {
    key_pool::reset(&relay_id);
    ok("已重置该供应商的 Key 状态。", current_payload())
}

/// 管理器启动时：有 Agent 开着网关就拉起常驻 helper，并在端口变化或退出 / 更新前恢复过直连的情况下重写配置。
pub fn start_from_saved_state() {
    let Ok(gateway) = GatewayStore::default().load() else {
        return;
    };
    if !gateway.any_enabled() {
        return;
    }
    let Some(port) = sync_resident_helper() else {
        return;
    };
    let ctx = ApplyContext::default_paths(port);
    let result = if gateway.restore_on_exit || gateway.restored_at_exit {
        apply::reapply_enabled(&ctx)
    } else {
        apply::reapply_if_port_changed(&ctx)
    }
    .and_then(|count| apply::sync_codex_identity(&ctx).map(|_| count));
    if let Err(error) = result {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "gateway.startup_reapply_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
}

/// 管理器退出时：用户要求的话（默认开）把走网关的 Agent 恢复成直连。
pub fn restore_for_exit() {
    let settings = SettingsStore::default().load().unwrap_or_default();
    if let Err(error) = apply::restore_all_for_exit(&context(), &settings, false) {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "gateway.exit_restore_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
}

/// 安装更新前：不管「退出时恢复直连」开没开，都把走网关的 Agent 恢复成直连。
/// 新版本没起来（安装失败、降级到不认识网关的旧版）时 Agent 也能直接连上游。返回是否有 Agent 被还原。
pub fn restore_for_update() -> bool {
    let has_enabled = GatewayStore::default()
        .load()
        .is_ok_and(|gateway| gateway.any_enabled());
    if !has_enabled {
        return false;
    }
    let settings = SettingsStore::default().load().unwrap_or_default();
    let result = apply::restore_all_for_exit(&context(), &settings, true);
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "gateway.update_restore",
        serde_json::json!({
            "ok": result.is_ok(),
            "message": result.as_ref().err().map(|error| format!("{error:#}")),
        }),
    );
    true
}

/// 卸载前（卸载器带 `--codex-plus-uninstall-cleanup` 调用）：同安装更新前，不管开关都把走网关的
/// Agent 恢复成直连。卸载后网关不在了，不还原的话 Codex / Claude Code / Grok 会一直连一个没人监听的
/// 本机地址。结果只写诊断日志，不报错，后面的卸载清理照常进行。
pub fn restore_for_uninstall() {
    let has_enabled = GatewayStore::default()
        .load()
        .is_ok_and(|gateway| gateway.any_enabled());
    if !has_enabled {
        return;
    }
    let settings = SettingsStore::default().load().unwrap_or_default();
    let result = apply::restore_all_for_exit(&context(), &settings, true);
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "uninstall.gateway_restore",
        serde_json::json!({
            "ok": result.is_ok(),
            "message": result.as_ref().err().map(|error| format!("{error:#}")),
        }),
    );
}

/// 更新没装成、Codex++ 继续运行：把更新前恢复成直连的 Agent 重新接回网关。
pub fn undo_restore_for_update() {
    start_from_saved_state();
}
