//! 网关模式命令：Agent 接入开关、网关自己的供应商池与路由组、Key 健康状态。
//!
//! 网关模式与直连模式完全独立：供应商池、路由组只存在 `gateway.json`；
//! 直连模式的 Codex / Claude / Grok 供应商只在「导入」时被读取并复制一份。

use serde::Serialize;

use codex_plus_core::gateway::activity::{self, ActivitySnapshot};
use codex_plus_core::gateway::apply::{self, ApplyContext, GroupInput, ProviderInput};
use codex_plus_core::gateway::key_pool::{self, PoolSnapshot};
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
    /// 登录得到的账号：`chatgpt` / `copilot` / `grok`；普通供应商为空。
    pub account: String,
    /// 这个 ChatGPT 账号正借给 Codex 登录（Codex 以官方身份接入网关）。
    pub lent_to_codex: bool,
    /// 网关供应商单独走的 HTTP 代理（空串表示不单独走代理）。
    pub proxy: String,
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
    pub agents: Vec<GatewayAgentView>,
    pub health: PoolSnapshot,
    /// 路由动态：各 Agent 最近一次请求怎么走的、计数、正在回答的上游。
    pub activity: ActivitySnapshot,
    /// 可复制进网关的直连模式供应商。
    pub codex_providers: Vec<GatewayPoolEntry>,
    pub claude_providers: Vec<GatewaySourceEntry>,
    pub grok_profiles: Vec<GatewaySourceEntry>,
    /// OpenCode 自己保存的 OpenCode Go / Zen Key（不含 Key 本身）。
    pub opencode_keys: Vec<GatewaySourceEntry>,
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
        account: account_kind(relay),
        lent_to_codex: codex_plus_core::official_login::is_lent_to_codex(relay),
        proxy: relay.gateway_proxy.clone().unwrap_or_default(),
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
        .map(|provider| {
            pool_entry(
                &provider.relay,
                provider.enabled,
                &provider.source,
                provider.price.clone(),
            )
        })
        .collect();
    let codex_providers = apply::codex_importable(settings)
        .into_iter()
        .map(|relay| pool_entry(relay, true, "", None))
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
    let mut gateway = gateway;
    gateway.providers.clear();
    GatewayPayload {
        gateway,
        port,
        helper_running: codex_plus_core::resident_helper::resident_helper_port().is_some(),
        pool,
        groups,
        agents,
        health: key_pool::snapshot(std::time::Instant::now()),
        activity: activity::snapshot(),
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

#[tauri::command]
pub fn load_gateway() -> CommandResult<Option<GatewayPayload>> {
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
        let mut gateway = store.load()?;
        let Some(binding) = gateway.bindings.get_mut(agent.as_str()) else {
            anyhow::bail!("还没有配置网关绑定");
        };
        binding.token = new_client_token(&agent);
        let binding = binding.clone();
        store.save(&gateway)?;
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
        let store = GatewayStore::default();
        let mut gateway = store.load()?;
        gateway.key_policy.mode = match key_rotation.as_str() {
            "roundRobin" => KeyRotationMode::RoundRobin,
            _ => KeyRotationMode::Sticky,
        };
        gateway.restore_on_exit = restore_on_exit;
        store.save(&gateway)?;
        Ok(gateway)
    })();
    respond("网关选项已保存。", "保存网关选项失败", result)
}

/// 用内置预设或手填的地址、Key 新建网关供应商。
#[tauri::command]
pub fn create_gateway_provider(
    input: ProviderInput,
    source: String,
) -> CommandResult<Option<GatewayPayload>> {
    let name = input.name.trim().to_string();
    respond(
        &format!("已添加「{name}」，接入的 Agent 现在能用它的模型了。"),
        "添加供应商失败",
        refresh_agents(apply::create_provider(
            &GatewayStore::default(),
            &input,
            &source,
        )),
    )
}

/// 编辑网关供应商；Key 留空表示不改。
#[tauri::command]
pub fn update_gateway_provider(
    id: String,
    input: ProviderInput,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "供应商已保存。",
        "保存供应商失败",
        refresh_agents(apply::update_provider(
            &GatewayStore::default(),
            &id,
            &input,
        )),
    )
}

/// 只改供应商的价格与独立代理（在供应商池的详情里编辑）。
#[tauri::command]
pub fn save_gateway_provider_extras(
    id: String,
    price: Option<codex_plus_core::gateway::store::ProviderPrice>,
    proxy: String,
) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "已保存。",
        "保存失败",
        apply::set_provider_extras(&GatewayStore::default(), &id, price, &proxy),
    )
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
            "已关闭，它不再参与路由。"
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

/// 从直连模式复制一个供应商进网关：`codex` / `claude` / `grok`。复制后两边互不影响。
#[tauri::command]
pub fn import_gateway_provider(
    source: String,
    id: String,
) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let relay = match source.as_str() {
            "codex" => apply::copy_codex_provider(&SettingsStore::default().load()?, &id)?,
            "claude" => {
                let provider = claude::load(&ClaudeContext::default_paths())?
                    .providers
                    .into_iter()
                    .find(|provider| provider.id == id)
                    .ok_or_else(|| anyhow::anyhow!("Claude 供应商不存在：{id}"))?;
                apply::copy_claude_provider(&provider)?
            }
            "grok" => apply::copy_grok_profile(&SettingsStore::default().load()?, &id)?,
            other => anyhow::bail!("不认识的导入来源：{other}"),
        };
        apply::add_provider(&GatewayStore::default(), relay, &format!("{source}:{id}"))
    })();
    respond(
        "已复制进网关的供应商池，原来的供应商不受影响。",
        "导入失败",
        refresh_agents(result),
    )
}

/// 把 OpenCode 里保存的 OpenCode Go / Zen Key 加进网关池，模型取目录里网关能用的。
#[tauri::command]
pub async fn import_gateway_opencode_key(id: String) -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::opencode;
    let result = async {
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
            proxy: None,
        };
        apply::create_provider(&GatewayStore::default(), &input, &format!("opencode:{id}"))?;
        Ok(if models.is_empty() {
            format!(
                "已添加「{}」。没读到模型目录，请在供应商里获取模型。",
                entry.name
            )
        } else {
            format!("已添加「{}」，{} 个模型。", entry.name, models.len())
        })
    }
    .await;
    match refresh_agents(result) {
        Ok(message) => ok(&message, current_payload()),
        Err(error) => failed(&format!("导入失败：{error:#}"), current_payload()),
    }
}

/// 新建或编辑网关路由组。
#[tauri::command]
pub fn save_gateway_group(group: GroupInput) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "路由组已保存。",
        "保存路由组失败",
        refresh_agents(apply::save_group(&GatewayStore::default(), &group)),
    )
}

/// 删除网关路由组。
#[tauri::command]
pub fn delete_gateway_group(id: String) -> CommandResult<Option<GatewayPayload>> {
    respond(
        "路由组已删除。",
        "删除路由组失败",
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

/// 删除 Claude Code / Grok 的一个本地会话：移进废纸篓，可以找回。Codex 的会话走 `delete_local_session`。
#[tauri::command]
pub fn delete_agent_session(agent: String, id: String) -> CommandResult<Option<String>> {
    use codex_plus_core::gateway::sessions;
    let trash = sessions::default_trash_dir();
    let result = match agent.as_str() {
        "claude" => sessions::delete_claude_session(&ClaudeContext::default_paths().claude_home, &id, &trash),
        "grok" => sessions::delete_grok_session(&codex_plus_core::grok_config::default_grok_home_dir(), &id, &trash),
        other => Err(anyhow::anyhow!("{other} 的会话不在这里删除")),
    };
    match result {
        Ok(path) => ok("会话已移到废纸篓，需要时可以从废纸篓找回。", Some(path.display().to_string())),
        Err(error) => failed(&format!("删除失败：{error:#}"), None),
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

/// 托盘快捷面板开关。
#[tauri::command]
pub fn save_gateway_tray_panel(enabled: bool) -> CommandResult<Option<GatewayPayload>> {
    let result = (|| {
        let store = GatewayStore::default();
        let mut gateway = store.load()?;
        gateway.tray_panel = enabled;
        store.save(&gateway)?;
        Ok(gateway)
    })();
    respond(
        if enabled {
            "点击托盘图标会弹出快捷面板。"
        } else {
            "托盘图标恢复原来的点击行为。"
        },
        "保存失败",
        result,
    )
}

/// 用系统默认浏览器打开登录页。
fn open_in_browser(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/c", "start", "", url])
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

/// 在网关里登录 ChatGPT 订阅：浏览器登录后加进网关的供应商池。同一个账号再登录只更新 token。
/// 与 Codex 自己的登录是两次独立的登录，互不影响。
#[tauri::command]
pub async fn login_gateway_chatgpt() -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let result: anyhow::Result<String> = async {
        let account = accounts::chatgpt_login(open_in_browser).await?;
        let store = GatewayStore::default();
        let mut gateway = store.load()?;
        // 同一个账号已经登录过：只换 token。
        if !account.account_id.is_empty()
            && let Some(provider) = gateway.providers.iter_mut().find(|provider| {
                codex_plus_core::official_login::chatgpt_account_id_from_auth_contents(
                    &provider.relay.auth_contents,
                ) == account.account_id
            })
        {
            provider.relay.auth_contents = account.auth_contents.clone();
            let name = provider.relay.name.clone();
            store.save(&gateway)?;
            return Ok(format!("已更新「{name}」的登录。"));
        }
        let mut relay = accounts::chatgpt_relay(&account, &[]);
        let models = codex_plus_core::model_catalog::fetch_relay_profile_model_ids(&relay)
            .await
            .map(|(models, _)| models)
            .unwrap_or_default();
        relay = accounts::chatgpt_relay(&account, &models);
        let name = relay.name.clone();
        apply::add_provider(&store, relay, "login:chatgpt")?;
        Ok(if models.is_empty() {
            format!("已添加「{name}」。没读到模型列表，请在供应商里编辑补上。")
        } else {
            format!("已添加「{name}」，{} 个模型。", models.len())
        })
    }
    .await;
    let result = refresh_agents(result);
    match result {
        Ok(message) => ok(&message, current_payload()),
        Err(error) => failed(&format!("登录失败：{error:#}"), current_payload()),
    }
}

/// 账号类供应商（Copilot / SuperGrok）加进网关池；同一个账号（按 `identity_field`）已存在时只换凭据。
fn add_account_provider(
    relay: codex_plus_core::settings::RelayProfile,
    kind: &str,
    identity_field: &str,
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
    let mut gateway = store.load()?;
    if let Some(id) = identity(&relay.auth_contents)
        && let Some(provider) = gateway.providers.iter_mut().find(|provider| {
            identity(&provider.relay.auth_contents).as_deref() == Some(id.as_str())
        })
    {
        provider.relay.auth_contents = relay.auth_contents.clone();
        if !relay.model_list.trim().is_empty() {
            provider.relay.model_list = relay.model_list.clone();
        }
        let name = provider.relay.name.clone();
        store.save(&gateway)?;
        return Ok(format!("已更新「{name}」的登录。"));
    }
    let name = relay.name.clone();
    let count = relay
        .model_list
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    apply::add_provider(&store, relay, &format!("login:{kind}"))?;
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
) -> CommandResult<Option<GatewayPayload>> {
    let result = async {
        let relay = codex_plus_core::gateway::accounts::copilot_finish(&code).await?;
        add_account_provider(relay, "copilot", "login")
    }
    .await;
    match refresh_agents(result) {
        Ok(message) => ok(&message, current_payload()),
        Err(error) => failed(&format!("登录失败：{error:#}"), current_payload()),
    }
}

/// 在网关里登录 SuperGrok：浏览器登录后加进网关池。与 Grok CLI 自己的登录互不影响。
#[tauri::command]
pub async fn login_gateway_grok() -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let result = async {
        let tokens = accounts::grok_login(open_in_browser).await?;
        let models = accounts::grok_models(&tokens.access_token).await;
        add_account_provider(accounts::grok_relay(&tokens, &models), "grok", "email")
    }
    .await;
    match refresh_agents(result) {
        Ok(message) => ok(&message, current_payload()),
        Err(error) => failed(&format!("登录失败：{error:#}"), current_payload()),
    }
}

/// 在网关里登录 ChatGPT API（OpenAI 的「用 ChatGPT 登录」）：套餐额度直接用在 api.openai.com 上，
/// Agent 的指令原样保留。与 ChatGPT（Codex 订阅）是两种独立的登录。
#[tauri::command]
pub async fn login_gateway_chatgpt_api() -> CommandResult<Option<GatewayPayload>> {
    use codex_plus_core::gateway::accounts;
    let result = async {
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
        )
    }
    .await;
    match refresh_agents(result) {
        Ok(message) => ok(&message, current_payload()),
        Err(error) => failed(&format!("登录失败：{error:#}"), current_payload()),
    }
}

/// 清掉某个供应商的 Key 冷却与熔断状态。
#[tauri::command]
pub fn reset_gateway_key_health(relay_id: String) -> CommandResult<Option<GatewayPayload>> {
    key_pool::reset(&relay_id);
    ok("已重置该供应商的 Key 状态。", current_payload())
}

/// 管理器启动时：有 Agent 开着网关就拉起常驻 helper，并在端口变化或退出时恢复过直连的情况下重写配置。
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
    let result = if gateway.restore_on_exit {
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

/// 管理器退出时：用户要求的话把走网关的 Agent 恢复成直连。
pub fn restore_for_exit() {
    let settings = SettingsStore::default().load().unwrap_or_default();
    if let Err(error) = apply::restore_all_for_exit(&context(), &settings) {
        let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
            "gateway.exit_restore_failed",
            serde_json::json!({ "message": format!("{error:#}") }),
        );
    }
}
