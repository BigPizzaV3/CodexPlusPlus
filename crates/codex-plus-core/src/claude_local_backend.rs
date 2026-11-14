//! Codex++ 的 Claude Desktop 完整功能模式。
//!
//! 核心（本地兼容服务、证书、profile 切换、启动 / 停止）在 claude-local（ClaudeMom 仓库，两边共用一份）；
//! 这里是 Codex++ 的宿主实现：设置存在 settings.json 的 `claudeDesktopLocal`，上游走 Codex++ 网关的
//! `claude-desktop-local` 绑定，听写走常驻 helper，关掉完整功能模式时经网关启动 3p 模式。
//! 对外仍是原来的 `status` / `configure` / `perform` 等函数，调用方不用改。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

pub use claude_local::{ClaudeLocalResult, ClaudeLocalStatus, Host, Upstream};

use crate::gateway::agents::connect::wiring;
use crate::gateway::store::{AgentBinding, GatewaySettings, GatewayStore, new_client_token};
use crate::settings::{ClaudeLocalSettings, SettingsStore};
use crate::tools::ToolId;

pub const AGENT_ID: &str = "claude-desktop-local";

fn ensure_host() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        claude_local::install_host(Arc::new(CodexPlusHost));
    });
}

pub fn status() -> Result<ClaudeLocalStatus> {
    ensure_host();
    claude_local::status()
}

pub fn configure(config: ClaudeLocalSettings) -> Result<ClaudeLocalStatus> {
    ensure_host();
    claude_local::configure(config)
}

pub fn perform(action: &str) -> Result<ClaudeLocalResult> {
    ensure_host();
    claude_local::perform(action)
}

pub fn stop_backend() {
    ensure_host();
    claude_local::stop_backend();
}

pub fn resume_attached_proxy() -> Result<Option<u16>> {
    ensure_host();
    claude_local::resume_attached_proxy()
}

pub fn reap_stale_proxy(root: &Path) {
    ensure_host();
    claude_local::reap_stale_proxy(root);
}

struct CodexPlusHost;

impl Host for CodexPlusHost {
    fn state_dir(&self) -> PathBuf {
        crate::paths::default_app_state_dir()
    }

    fn ca_name(&self) -> &str {
        "Codex++ Claude Local CA"
    }

    fn load_config(&self) -> Result<ClaudeLocalSettings> {
        Ok(SettingsStore::default().load()?.claude_desktop_local)
    }

    fn save_config(&self, config: &ClaudeLocalSettings) -> Result<()> {
        save_config(&SettingsStore::default(), config.clone())
    }

    /// 只读：不写绑定、不起网关。状态里报的是「交给 Claude 的那份列表」，不是网关全量。
    fn preview_upstream(&self, config: &ClaudeLocalSettings) -> Result<Upstream> {
        let gateway = GatewayStore::default().load()?.with_extension_providers();
        if available_models(&gateway).is_empty() {
            anyhow::bail!("供应商池中还没有可用的聊天模型");
        }
        let binding = local_binding(&gateway, config)?;
        let port = crate::protocol_proxy::protocol_proxy_port();
        Ok(upstream_for(&gateway, &binding, config, port))
    }

    fn prepare_upstream(&self, config: &ClaudeLocalSettings) -> Result<Upstream> {
        let port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
        crate::resident_helper::ensure_resident_helper(port)?;
        let (gateway, binding) = bind_gateway(&GatewayStore::default(), config)?;
        Ok(upstream_for(&gateway, &binding, config, port))
    }

    /// 与拆分前一致：可用模型、交给 Claude 的窗口、选中的模型变了才要重启（令牌、端口不算）。
    fn upstream_fingerprint(&self, config: &ClaudeLocalSettings) -> Value {
        let Ok(gateway) = GatewayStore::default().load() else {
            return Value::Null;
        };
        let gateway = gateway.with_extension_providers();
        let models = available_models(&gateway);
        let binding = local_binding(&gateway, config).ok();
        let windows = binding
            .as_ref()
            .map(|binding| available_model_windows(&gateway, binding, config.list_all_models))
            .unwrap_or_default();
        let model = binding.map(|binding| binding.model);
        fingerprint(&models, &windows, &model)
    }

    /// Code 页签默认和命令行共用 ~/.claude（与 ClaudeMom 一致）：平时建的会话直接能打开。
    fn default_code_share_home(&self) -> bool {
        true
    }

    fn dictation_ready(&self, _config: &ClaudeLocalSettings) -> bool {
        let Ok(settings) = SettingsStore::default().load() else {
            return false;
        };
        let effective = crate::dictation::effective_settings(&settings);
        crate::dictation::ensure_local_sensevoice_available(&effective).is_ok()
    }

    /// 听写走常驻 helper：它在状态目录写 dictation-helper.json，本地兼容服务按凭据连过去。
    fn prepare_dictation_service(&self) -> Result<()> {
        let port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
        crate::resident_helper::ensure_resident_helper(port)
    }

    fn launch_three_p(&self, source: &Path, root: &Path, config: &ClaudeLocalSettings) -> Result<()> {
        launch_three_p(source, root, config)
    }

    fn diagnostic(&self, event: &str, detail: Value) {
        let _ = crate::diagnostic_log::append_diagnostic_log(event, detail);
    }
}

fn fingerprint(models: &[String], windows: &BTreeMap<String, u64>, model: &Option<String>) -> Value {
    json!({ "models": models, "windows": windows, "model": model })
}

fn upstream_for(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    config: &ClaudeLocalSettings,
    port: u16,
) -> Upstream {
    let wiring = wiring(gateway, binding, AGENT_ID, port);
    let (models, hidden_models) = exposed_models(gateway, binding, config.list_all_models);
    let extra_env = crate::gateway::agents::catalog_apps::desktop_capabilities(&wiring)
        .map(|capabilities| vec![("CLAUDE_CODE_MODEL_CAPABILITIES".to_string(), capabilities)])
        .unwrap_or_default();
    Upstream {
        model_names: model_names(gateway, &models),
        model_windows: available_model_windows(gateway, binding, config.list_all_models),
        // 选中模型的窗口也单独给一份：逐模型表里没有它时，界面还知道该显示多长，
        // 与启动 Claude Code 时写进 CLAUDE_CODE_MAX_CONTEXT_TOKENS 的值同源。
        default_window: context_tokens(gateway, binding, &binding.model),
        root: wiring.root.clone(),
        token: wiring.token.clone(),
        default_model: wiring.default_model.clone(),
        small_model: wiring.small_model.clone(),
        models,
        hidden_models,
        extra_env,
    }
}

fn available_models(gateway: &GatewaySettings) -> Vec<String> {
    let mut models = Vec::new();
    for provider in gateway.providers.iter().filter(|provider| provider.enabled) {
        for model in crate::gateway::route::chat_models(&provider.relay) {
            if is_internal_review_model(&model) {
                continue;
            }
            let model = format!("{}/{model}", provider.relay.id);
            if !models.contains(&model) {
                models.push(model);
            }
        }
    }
    for group in gateway
        .groups
        .iter()
        .filter(|group| !group.members.is_empty())
    {
        models.push(group.model_id());
    }
    models
}

/// Claude 模型选择器的显示名：有组显示名用组名，否则把 slug 拼得像样点（不再是 `jojocodemax/gpt-6.1-sol`）；
/// 副标题放供应商显示名（取不到就用前缀原文）。发给网关的 model id 不变。
fn model_names(
    gateway: &GatewaySettings,
    models: &[String],
) -> std::collections::BTreeMap<String, Value> {
    models
        .iter()
        .map(|id| {
            let friendly = crate::gateway::route::model_display_name(gateway, id);
            let slug = id.rsplit('/').next().unwrap_or(id);
            let name = if friendly != id.as_str() {
                friendly.to_string()
            } else {
                claude_local::friendly_model_name(slug)
            };
            let vendor = id.split('/').next().unwrap_or_default();
            let provider = gateway
                .providers
                .iter()
                .find(|provider| provider.relay.id == vendor)
                .map(|provider| provider.relay.name.clone())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| vendor.to_string());
            (id.clone(), json!({"name": name, "description": provider}))
        })
        .collect()
}

/// 自动评审模型（`codex-auto-review`）是 Codex 内部子代理用的，不该出现在
/// 人的模型选择器里：选中它当聊天模型没有意义，只会白烧额度。
fn is_internal_review_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("auto-review")
}

/// 交给 Claude 的模型：`(可见, 因窗口更小而隐藏)`。
///
/// Claude Code 的上下文窗口是一份**全局环境变量**（`CLAUDE_CODE_MAX_CONTEXT_TOKENS`），
/// 启动时按默认模型算一次；在 Claude 里切到窗口更小的模型时它不会跟着变小，
/// auto-compact 会一直等到大窗口才触发，请求先被上游顶回来。所以只暴露窗口
/// 不小于默认模型的模型，其余的仍留在 Codex++ 的默认模型选择器里。
fn exposed_models(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    list_all: bool,
) -> (Vec<String>, Vec<String>) {
    let models = available_models(gateway);
    if list_all {
        // 用户显式要求全列：不按窗口过滤，界面里自己留意切到小窗口模型的风险。
        return (models, Vec::new());
    }
    let Some(selected) = selected_model_window(gateway, binding, &binding.model) else {
        return (models, Vec::new());
    };
    models
        .into_iter()
        .partition(|model| {
            selected_model_window(gateway, binding, model).is_some_and(|window| window >= selected)
        })
}

fn available_model_windows(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    list_all: bool,
) -> BTreeMap<String, u64> {
    exposed_models(gateway, binding, list_all)
        .0
        .into_iter()
        .filter_map(|model| {
            selected_model_window(gateway, binding, &model).map(|window| (model, window))
        })
        .collect()
}

fn selected_model_window(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<u64> {
    crate::gateway::route::model_entry_window(gateway, binding, model)
        .or(binding.context_window)
        .filter(|window| *window > 0)
}

fn context_tokens(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    model: &str,
) -> Option<u64> {
    claude_local::pick_context_tokens(
        selected_model_window(gateway, binding, model),
        claude_local::inherited_context_tokens(),
    )
}

fn bind_gateway(
    store: &GatewayStore,
    config: &ClaudeLocalSettings,
) -> Result<(GatewaySettings, AgentBinding)> {
    let (_, binding) = store.update(|gateway| {
        let view = gateway.clone().with_extension_providers();
        let binding = local_binding(&view, config)?;
        gateway.bindings.insert(AGENT_ID.into(), binding.clone());
        Ok(binding)
    })?;
    // 不走 enable_binding：它会改 Desktop 的部署模式，原版本地服务只需要网关路由。
    Ok((store.load()?.with_extension_providers(), binding))
}

/// Claude 用哪个模型：`Agent 接入` 里那份绑定选过就用它；否则退回配置里的旧值；
/// 两者都没有才默认第一个可用模型。**选过但已经不可用**时留空，交给调用方报错，
/// 不静默换成别的模型。
fn chosen_model(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    config: &ClaudeLocalSettings,
) -> String {
    let routable = |model: &str| {
        !model.is_empty() && crate::gateway::route::model_is_routable(gateway, binding, model)
    };
    // 「Agent 接入」页里 Claude Desktop 那一行写的是 `claude-desktop` 绑定；本地模式用的是
    // `claude-desktop-local`。用户只看得见前者，所以以它为准，本地绑定自己的值退到第二位。
    let sibling = gateway
        .binding(&ToolId::Unknown("claude-desktop".into()))
        .map(|binding| binding.model.trim().to_string())
        .unwrap_or_default();
    let bound = binding.model.trim();
    let configured = config.model.trim();
    if routable(&sibling) {
        sibling
    } else if routable(bound) {
        bound.to_string()
    } else if routable(configured) {
        configured.to_string()
    } else if !bound.is_empty() {
        // 选过但上游已经不提供它了：原样留着，让下面的 ensure 明确报错，不静默换模型。
        bound.to_string()
    } else if !configured.is_empty() {
        configured.to_string()
    } else {
        binding.extra_models.first().cloned().unwrap_or_default()
    }
}

fn local_binding(gateway: &GatewaySettings, config: &ClaudeLocalSettings) -> Result<AgentBinding> {
    let agent = ToolId::Unknown(AGENT_ID.into());
    let mut binding = gateway.binding(&agent).cloned().unwrap_or_default();
    binding.pool_exclude.clear();
    binding.extra_models = available_models(gateway);
    // 模型以「Agent 接入」里这份绑定的选择为准：那边一处选好，Codex / Claude / Grok 都按它走。
    // 配置里的 `model` 只是老版本的遗留值，绑定还没选过模型时才拿来兜底。
    binding.model = chosen_model(gateway, &binding, &config);
    let model = crate::gateway::route::binding_default_model(gateway, &binding);
    ensure!(
        !model.is_empty() && crate::gateway::route::model_is_routable(gateway, &binding, &model),
        "请先在供应商池中添加可用模型，并为 Claude 本地模式选择模型"
    );
    binding.model = model;
    binding.enabled = true;
    if binding.token.is_empty() {
        binding.token = new_client_token(&agent);
    }
    Ok(binding)
}

fn save_config(store: &SettingsStore, config: ClaudeLocalSettings) -> Result<()> {
    let mut settings = store.load()?;
    settings.claude_desktop_local = config;
    store.save(&settings)?;
    Ok(())
}

fn launch_three_p(source: &Path, root: &Path, config: &ClaudeLocalSettings) -> Result<()> {
    // 本地服务已停、真实 profile 已恢复（claude-local 在调用前做好）。
    GatewayStore::default().update(|gateway| {
        if let Some(binding) = gateway.bindings.get_mut(AGENT_ID) {
            binding.enabled = false;
        }
        Ok(())
    })?;
    let port = crate::protocol_proxy::ensure_protocol_proxy_port_available();
    crate::resident_helper::ensure_resident_helper(port)?;
    let context = crate::gateway::apply::ApplyContext::default_paths(port);
    let agent = ToolId::Unknown("claude-desktop".into());
    let gateway = context.gateway.load()?.with_extension_providers();
    let mut binding = gateway.binding(&agent).cloned().unwrap_or_default();
    binding.pool_exclude.clear();
    binding.extra_models = available_models(&gateway);
    // 与完整功能模式同一条规则：绑定里选过就用它，配置里的旧值只作兜底。
    binding.model = chosen_model(&gateway, &binding, config);
    let default_model = crate::gateway::route::binding_default_model(&gateway, &binding);
    ensure!(
        !default_model.is_empty()
            && crate::gateway::route::model_is_routable(&gateway, &binding, &default_model),
        "请先在供应商池中添加可用模型，并为 Claude 选择模型"
    );
    binding.model = default_model;
    let gateway = crate::gateway::apply::enable_binding(&context, &agent, binding)?;
    let binding = gateway
        .binding(&agent)
        .context("Claude Desktop 网关配置缺失")?;
    let wiring = wiring(&gateway, binding, "claude-desktop", port);
    claude_local::private_dir(root)?;
    claude_local::private_dir(&root.join("claude-code-3p"))?;
    let window = context_tokens(&gateway, binding, &binding.model);
    let capabilities = crate::gateway::agents::catalog_apps::desktop_capabilities(&wiring);
    let no_proxy = merged_no_proxy([
        std::env::var("NO_PROXY").ok(),
        std::env::var("no_proxy").ok(),
    ]);
    claude_local::launch_claude(source, |command| {
        if let Some(window) = window {
            command.env("CLAUDE_CODE_MAX_CONTEXT_TOKENS", window.to_string());
        }
        command
            .env("CLAUDE_CONFIG_DIR", root.join("claude-code-3p"))
            .env("ANTHROPIC_BASE_URL", &wiring.root)
            .env("ANTHROPIC_API_KEY", &wiring.token)
            .env("ANTHROPIC_AUTH_TOKEN", &wiring.token)
            .env("NO_PROXY", &no_proxy)
            .env("no_proxy", &no_proxy);
        if let Some(capabilities) = capabilities {
            command.env("CLAUDE_CODE_MODEL_CAPABILITIES", capabilities);
        }
    })
}

/// 网关地址在回环上，必须绕过代理；但不能把用户自己配的排除项（内网网段、内部域名）
/// 覆盖掉，否则 Claude 访问内网也被送进代理，报 403（#3102）。
/// 这里在继承的 NO_PROXY / no_proxy 之后补上回环地址，按出现顺序去重。
fn merged_no_proxy(inherited: [Option<String>; 2]) -> String {
    let mut entries: Vec<String> = Vec::new();
    let values = inherited.into_iter().flatten();
    for entry in values
        .flat_map(|value| {
            value
                .split(',')
                .map(|entry| entry.trim().to_string())
                .collect::<Vec<_>>()
        })
        .chain(["localhost".to_string(), "127.0.0.1".to_string()])
    {
        if !entry.is_empty() && !entries.iter().any(|seen| seen.eq_ignore_ascii_case(&entry)) {
            entries.push(entry);
        }
    }
    entries.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_local::pick_context_tokens;

    fn provider(id: &str, models: &str) -> crate::gateway::store::GatewayProvider {
        serde_json::from_value(json!({
            "relay": {
                "id": id,
                "name": id,
                "modelList": models,
            },
        }))
        .unwrap()
    }

    #[test]
    fn three_p_launch_keeps_user_no_proxy_entries() {
        // #3102：用户配的内网排除项要保留，回环地址补在后面且不重复。
        assert_eq!(
            merged_no_proxy([
                Some("10.0.0.0/8, *.internal,localhost".into()),
                Some("10.0.0.0/8,intranet.example".into()),
            ]),
            "10.0.0.0/8,*.internal,localhost,intranet.example,127.0.0.1"
        );
        assert_eq!(merged_no_proxy([None, None]), "localhost,127.0.0.1");
        assert_eq!(
            merged_no_proxy([Some(" , ".into()), None]),
            "localhost,127.0.0.1"
        );
    }

    #[test]
    fn internal_review_models_are_not_offered_as_chat_models() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![provider("openai", "gpt-6\ncodex-auto-review")];
        assert_eq!(available_models(&gateway), vec!["openai/gpt-6"]);
    }

    #[test]
    fn local_mode_follows_the_claude_desktop_binding_model() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("relay", "gpt-6-astra\ngpt-6.1-sol"),
        ];
        gateway.bindings.insert(
            "claude-desktop".into(),
            AgentBinding {
                model: "relay/gpt-6.1-sol".into(),
                ..AgentBinding::default()
            },
        );
        gateway.bindings.insert(
            AGENT_ID.into(),
            AgentBinding {
                model: "relay/gpt-6-astra".into(),
                ..AgentBinding::default()
            },
        );
        let binding = local_binding(&gateway, &ClaudeLocalSettings::default()).unwrap();
        assert_eq!(
            binding.model, "relay/gpt-6.1-sol",
            "用户在「Agent 接入」的 Claude Desktop 行选的模型说了算"
        );
    }

    #[test]
    fn agent_binding_model_wins_over_legacy_local_model() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("xai", "grok-4"),
        ];
        // 「Agent 接入」里已经给 Claude 选过模型：本地模式跟着它走。
        gateway.bindings.insert(
            AGENT_ID.into(),
            AgentBinding {
                model: "anthropic/claude-sonnet-4-6".into(),
                ..AgentBinding::default()
            },
        );
        let binding = local_binding(
            &gateway,
            &ClaudeLocalSettings {
                model: "xai/grok-4".into(),
                ..ClaudeLocalSettings::default()
            },
        )
        .unwrap();
        assert_eq!(binding.model, "anthropic/claude-sonnet-4-6");
    }

    #[test]
    fn exposed_models_hide_windows_smaller_than_the_selected_one() {
        let mut gateway = GatewaySettings::default();
        let mut relay = provider("relay", "gpt-6-astra\nsmall-model");
        relay.model_windows.insert("gpt-6-astra".into(), 1_100_000);
        relay.model_windows.insert("small-model".into(), 128_000);
        gateway.providers.push(relay);

        let binding = AgentBinding {
            model: "relay/gpt-6-astra".into(),
            ..AgentBinding::default()
        };
        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert_eq!(visible, vec!["relay/gpt-6-astra"]);
        assert_eq!(hidden, vec!["relay/small-model"], "小窗口模型不交给 Claude");

        // 默认模型自己没声明窗口时不过滤：Claude 那边按 200k 兜底，也不会更差。
        let binding = AgentBinding {
            model: "relay/unknown-model".into(),
            ..AgentBinding::default()
        };
        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert!(hidden.is_empty());
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn list_all_models_skips_the_window_filter() {
        let mut gateway = GatewaySettings::default();
        let mut relay = provider("relay", "gpt-6-astra\nsmall-model");
        relay.model_windows.insert("gpt-6-astra".into(), 1_100_000);
        relay.model_windows.insert("small-model".into(), 128_000);
        gateway.providers.push(relay);
        let binding = AgentBinding {
            model: "relay/gpt-6-astra".into(),
            ..AgentBinding::default()
        };

        let (visible, hidden) = exposed_models(&gateway, &binding, false);
        assert_eq!(visible, vec!["relay/gpt-6-astra"]);
        assert_eq!(hidden, vec!["relay/small-model"]);

        // 显式要求全列：小窗口模型也交给 Claude（界面自己提示风险）
        let (visible, hidden) = exposed_models(&gateway, &binding, true);
        assert_eq!(visible.len(), 2);
        assert!(hidden.is_empty());
    }

    #[test]
    fn all_pool_models_keep_provider_identity_for_each_family() {
        let mut gateway = GatewaySettings::default();
        gateway.providers = vec![
            provider("anthropic", "claude-sonnet-4-6"),
            provider("xai", "grok-4"),
            provider("openai", "gpt-6\nshared-model"),
            provider("relay", "shared-model"),
        ];
        assert_eq!(
            available_models(&gateway),
            vec![
                "anthropic/claude-sonnet-4-6",
                "xai/grok-4",
                "openai/gpt-6",
                "openai/shared-model",
                "relay/shared-model",
            ]
        );
        let snapshot = gateway.clone();
        let binding = local_binding(
            &gateway,
            &ClaudeLocalSettings {
                model: "xai/grok-4".into(),
                ..ClaudeLocalSettings::default()
            },
        )
        .unwrap();
        assert_eq!(binding.model, "xai/grok-4");
        assert_eq!(binding.extra_models, available_models(&gateway));
        assert!(binding.enabled && !binding.token.is_empty());
        assert_eq!(
            gateway, snapshot,
            "合成本地绑定不应写应用配置或修改其它网关绑定"
        );
        gateway.providers[1].enabled = false;
        assert!(
            local_binding(
                &gateway,
                &ClaudeLocalSettings {
                    model: "xai/grok-4".into(),
                    ..ClaudeLocalSettings::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn context_tokens_prefers_gateway_window_then_inherited_value() {
        assert_eq!(
            pick_context_tokens(Some(1_100_000), Some(200_000)),
            Some(1_100_000),
            "网关声明压过环境里的旧值"
        );
        assert_eq!(
            pick_context_tokens(None, Some(1_000_000)),
            Some(1_000_000),
            "网关没声明时沿用用户自己设的窗口"
        );
        assert_eq!(pick_context_tokens(None, None), None, "都没有就不设，交给 Claude Code");
        assert_eq!(pick_context_tokens(Some(0), Some(0)), None, "0 不是窗口");
        assert_eq!(pick_context_tokens(Some(0), Some(272_000)), Some(272_000));
    }

    #[test]
    fn saving_mode_changes_preserves_enhanced_copy_settings() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(directory.path().join("settings.json"));
        let mut settings = crate::settings::BackendSettings::default();
        settings.claude_app_localization = false;
        settings.claude_app_voice_input = false;
        settings.claude_app_computer_use = true;
        settings.claude_app_extensions = true;
        store.save(&settings).unwrap();
        for enabled in [true, false] {
            let config = ClaudeLocalSettings {
                enabled,
                model: "relay/model".into(),
                ..ClaudeLocalSettings::default()
            };
            save_config(&store, config.clone()).unwrap();
            let saved = store.load().unwrap();
            assert_eq!(saved.claude_desktop_local, config);
            assert!(!saved.claude_app_localization && !saved.claude_app_voice_input);
            assert!(saved.claude_app_computer_use && saved.claude_app_extensions);
        }
    }

    #[test]
    fn local_agent_never_writes_desktop_deployment_configuration() {
        let env = crate::gateway::agents::AgentEnv::at(PathBuf::from("/synthetic-home"))
            .with_os(crate::gateway::agents::Os::Mac);
        let mut files = crate::gateway::agents::FileSet::default();
        let gateway = GatewaySettings::default();
        let binding = AgentBinding::default();
        let wiring = wiring(&gateway, &binding, AGENT_ID, 57321);
        (crate::gateway::agents::catalog_apps::CLAUDE_DESKTOP_LOCAL.write)(
            &env, &wiring, &mut files,
        )
        .unwrap();
        assert!(files.changes().unwrap().is_empty());
        assert!(crate::gateway::agents::spec(AGENT_ID).is_some());
        assert_eq!(
            wiring.root,
            "http://127.0.0.1:57321/gw/claude-desktop-local"
        );
    }

    /// 选择器显示名：组模型用组名、普通模型拼成产品名、副标题用供应商显示名。
    #[test]
    fn model_names_prefers_friendly_names_and_vendor_label() {
        let mut gateway = crate::gateway::store::GatewaySettings::default();
        let mut provider = provider("jojo", "gpt-6.1-sol");
        provider.relay.name = "Jojo Codemax".into();
        gateway.providers.push(provider);
        let names = model_names(&gateway, &["jojo/gpt-6.1-sol".to_string()]);
        let entry = &names["jojo/gpt-6.1-sol"];
        assert_eq!(entry["name"], "GPT-6.1 Sol");
        assert_eq!(entry["description"], "Jojo Codemax");
    }



    #[test]
    fn selected_model_window_prefers_provider_metadata_and_hash_tracks_changes() {
        let mut gateway = GatewaySettings::default();
        let mut provider = provider("openai", "gpt-6\nsmall-model");
        provider.model_windows.insert("gpt-6".into(), 1_100_000);
        gateway.providers.push(provider);
        let binding = AgentBinding {
            model: "openai/gpt-6".into(),
            context_window: Some(200_000),
            ..AgentBinding::default()
        };

        assert_eq!(
            selected_model_window(&gateway, &binding, &binding.model),
            Some(1_100_000)
        );
        assert_eq!(
            selected_model_window(&gateway, &binding, "openai/small-model"),
            Some(200_000)
        );

        let models = vec!["openai/gpt-6".to_string()];
        let bound = Some("openai/gpt-6".to_string());
        let initial = fingerprint(&models, &BTreeMap::from([("openai/gpt-6".into(), 1_100_000)]), &bound);
        let changed_window = fingerprint(&models, &BTreeMap::from([("openai/gpt-6".into(), 300_000)]), &bound);
        assert_ne!(initial, changed_window, "窗口变了要重启代理");
        let changed_model = fingerprint(
            &models,
            &BTreeMap::from([("openai/gpt-6".into(), 1_100_000)]),
            &Some("openai/small-model".to_string()),
        );
        assert_ne!(initial, changed_model, "「Agent 接入」换了模型也要重启代理");
    }
}
