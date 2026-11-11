//! Agent 自带的模型：不经网关、由 Agent 用自己的登录或 Key 直接向厂商要的模型。
//!
//! 网关的模型下拉里，Codex++ 池里的模型之外再列一组「直连，不经 Codex++」：
//! - Codex：断开网关后会回到的直连供应商的模型。官方登录读 Codex 为 ChatGPT 账号缓存的
//!   `models_cache.json`，厂商记为 OpenAI；聚合供应商列它发布给 Codex 的模型目录；别的供应商读它
//!   自己的模型列表。后两种厂商记为供应商名。
//! - Claude Code：它自己认的别名（opus / sonnet / haiku，Claude Code 解析成当前最新的那一个），
//!   厂商是 Anthropic，或用户自己配的 `ANTHROPIC_BASE_URL` 的主机名。
//!
//! 选中其中一个（[`use_own_model`]）= 断开网关并还原接入前的配置（与「断开并还原」同一条路），
//! 再把模型写进 Agent 自己的配置：Codex 写 `config.toml` 根上的 `model`（与 Codex 自己的
//! `/model` 写的是同一个键），Claude Code 写 `settings.json` 的 `model`。

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::gateway::apply::{
    self, ApplyContext, CODEX_GATEWAY_RELAY_ID, agent_points_at_gateway, claude_stash_path,
};
use crate::gateway::restore::{self as stash, FileStash};
use crate::gateway::store::{AgentBinding, GatewaySettings};
use crate::settings::{AggregateRelayProfile, BackendSettings, RelayMode, RelayProfile};
use crate::tools::ToolId;
use crate::tools::claude::{self, GATEWAY_PROVIDER_ID};

/// 一个自带模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnModel {
    /// 写进 Agent 配置的值。
    pub id: String,
    /// 显示名（有官方显示名时用它，否则同 id）。
    pub label: String,
}

/// 一个 Agent 的自带模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnModels {
    /// 直连的对象：OpenAI、Anthropic、直连模式供应商的名字或自定义地址的主机名。
    pub vendor: String,
    pub models: Vec<OwnModel>,
    /// 没接入网关时 Agent 现在用的模型（下拉里打勾用）；接入着或没设时为空。
    pub current: String,
}

/// 模型下拉里列不列这个 Agent 的自带模型（gateway.json 的 `hideOwnModels`）。
/// 关掉后下拉只列池里的模型；Agent 现在用着的那个自带模型照样显示，不会凭空消失。
pub fn set_shown(
    store: &crate::gateway::store::GatewayStore,
    agent: &ToolId,
    shown: bool,
) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        gateway.hide_own_models.retain(|id| id != agent.as_str());
        if !shown {
            gateway.hide_own_models.push(agent.as_str().to_string());
        }
        Ok(())
    })?;
    Ok(gateway)
}

/// Claude Code 自己认的模型别名，与它 `/model` 菜单一致。
const CLAUDE_ALIASES: &[(&str, &str)] = &[
    ("opus", "Opus"),
    ("opus[1m]", "Opus · 1M"),
    ("sonnet", "Sonnet"),
    ("sonnet[1m]", "Sonnet · 1M"),
    ("haiku", "Haiku"),
];

/// 这个 Agent 断开网关后能直连用的模型；没有（不支持的 Agent、无处可回、列不出模型）时为 None。
pub fn own_models(
    ctx: &ApplyContext,
    agent: &ToolId,
    binding: Option<&AgentBinding>,
) -> Option<OwnModels> {
    match agent {
        ToolId::Codex => codex_own_models(ctx, binding),
        ToolId::Claude => Some(claude_own_models(ctx, binding)),
        _ => None,
    }
}

/// 选一个自带模型：接入着就先断开并还原，再把模型写进 Agent 自己的配置。
pub fn use_own_model(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
    model: &str,
) -> anyhow::Result<GatewaySettings> {
    let model = model.trim();
    if model.is_empty() {
        bail!("没有选模型");
    }
    if !matches!(agent, ToolId::Codex | ToolId::Claude) {
        bail!("{} 没有可直连的自带模型", agent.as_str());
    }
    let connected = ctx
        .gateway
        .load()
        .ok()
        .and_then(|gateway| gateway.binding(agent).map(|binding| binding.enabled))
        .unwrap_or(false);
    if connected || agent_points_at_gateway(ctx, agent) {
        apply::disable_binding(ctx, settings, agent)?;
    }
    match agent {
        ToolId::Codex => write_codex_model(ctx, model)?,
        _ => write_claude_model(ctx, model)?,
    }
    ctx.gateway.load()
}

/// Codex 断开后回到的直连供应商。
enum CodexTarget<'a> {
    Relay(&'a RelayProfile),
    Aggregate(&'a AggregateRelayProfile),
}

fn find_codex_target<'a>(settings: &'a BackendSettings, id: &str) -> Option<CodexTarget<'a>> {
    if id.is_empty() || id == CODEX_GATEWAY_RELAY_ID {
        return None;
    }
    // 聚合供应商在普通列表里也有一份同 id 的影子（模式 aggregate），以聚合的定义为准。
    if let Some(aggregate) = settings
        .aggregate_relay_profiles
        .iter()
        .find(|profile| profile.id == id)
    {
        return Some(CodexTarget::Aggregate(aggregate));
    }
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == id && profile.relay_mode != RelayMode::Aggregate)
        .map(CodexTarget::Relay)
}

/// 接入着时，「Codex 自己的模型」取哪一份配置。
///
/// 1. 接入前那份能用（不是聚合）就用它：就是用户原来的供应商，与断开后的还原一致；
/// 2. 否则优先 Codex 自己的官方登录（`models_cache.json` 那套 sign-in 目录）；
/// 3. 再退回列表里第一个普通供应商。
///
/// 聚合**不在这里展开**：它会把几家供应商的模型混成一大串列成"自带模型"，
/// 看起来像 Codex 内置的（#系统默认 30 个），实际全是聚合成员。
fn pick_codex_target<'a>(settings: &'a BackendSettings, previous: &str) -> Option<CodexTarget<'a>> {
    if let Some(CodexTarget::Relay(profile)) = find_codex_target(settings, previous.trim()) {
        return Some(CodexTarget::Relay(profile));
    }
    if let Some(profile) = settings
        .relay_profiles
        .iter()
        .find(|profile| crate::official_login::is_official_login_profile(profile))
    {
        return Some(CodexTarget::Relay(profile));
    }
    settings
        .relay_profiles
        .iter()
        .find(|profile| {
            profile.id != CODEX_GATEWAY_RELAY_ID && profile.relay_mode != RelayMode::Aggregate
        })
        .map(CodexTarget::Relay)
}

/// 接入着时是「自己的那份」（见 [`pick_codex_target`]），没接入时是当前那一个。
fn codex_target<'a>(
    settings: &'a BackendSettings,
    binding: Option<&AgentBinding>,
) -> Option<CodexTarget<'a>> {
    if settings.active_relay_id != CODEX_GATEWAY_RELAY_ID {
        return find_codex_target(settings, settings.active_relay_id.trim());
    }
    let previous = binding
        .map(|binding| binding.restore.codex_previous_relay_id.trim())
        .unwrap_or_default();
    pick_codex_target(settings, previous)
}

fn codex_own_models(ctx: &ApplyContext, binding: Option<&AgentBinding>) -> Option<OwnModels> {
    let settings = ctx.settings.load().ok()?;
    let (vendor, models) = match codex_target(&settings, binding)? {
        CodexTarget::Relay(profile) => {
            let official = crate::official_login::is_official_login_profile(profile);
            let mut models = if official {
                codex_cached_models(&ctx.codex_home)
            } else {
                Vec::new()
            };
            if models.is_empty() {
                models = plain_models(crate::gateway::route::chat_models(profile));
            }
            let vendor = if official {
                "OpenAI".to_string()
            } else {
                profile.name.trim().to_string()
            };
            (vendor, models)
        }
        // 聚合供应商：列它发布给 Codex 的模型目录（与 Codex 自己的选择器里看到的一致）。
        CodexTarget::Aggregate(aggregate) => (
            aggregate.name.trim().to_string(),
            crate::aggregate_catalog::build_catalog_json(&settings, aggregate)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .map(|catalog| catalog_models(&catalog))
                .unwrap_or_default(),
        ),
    };
    if models.is_empty() {
        return None;
    }
    let current = if settings.active_relay_id == CODEX_GATEWAY_RELAY_ID {
        String::new()
    } else {
        std::fs::read_to_string(ctx.codex_home.join("config.toml"))
            .ok()
            .and_then(|text| crate::relay_config::root_key_string(&text, "model"))
            .map(|model| model.trim().to_string())
            .unwrap_or_default()
    };
    Some(OwnModels {
        vendor,
        models,
        current,
    })
}

fn plain_models(ids: Vec<String>) -> Vec<OwnModel> {
    ids.into_iter()
        .map(|id| OwnModel {
            label: id.clone(),
            id,
        })
        .collect()
}

/// Codex 为登录的 ChatGPT 账号缓存的官方模型（`models_cache.json`），带官方显示名。
fn codex_cached_models(codex_home: &std::path::Path) -> Vec<OwnModel> {
    std::fs::read_to_string(codex_home.join("models_cache.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|cache| catalog_models(&cache))
        .unwrap_or_default()
}

/// Codex 模型目录（`{"models":[{"slug","display_name","visibility"}]}`）里列出的模型；
/// 标成隐藏的（自动审批模型等）不算。
fn catalog_models(catalog: &Value) -> Vec<OwnModel> {
    let mut models: Vec<OwnModel> = Vec::new();
    for entry in catalog
        .get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(slug) = entry
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|slug| !slug.is_empty())
        else {
            continue;
        };
        let listed = entry
            .get("visibility")
            .and_then(Value::as_str)
            .is_none_or(|visibility| visibility.is_empty() || visibility == "list");
        if !listed || models.iter().any(|known| known.id == slug) {
            continue;
        }
        let label = entry
            .get("display_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(slug);
        models.push(OwnModel {
            id: slug.to_string(),
            label: label.to_string(),
        });
    }
    models
}

fn claude_own_models(ctx: &ApplyContext, binding: Option<&AgentBinding>) -> OwnModels {
    let settings_path = ctx.claude.claude_home.join("settings.json");
    let live = read_json(&settings_path);
    let on_gateway = claude::current_provider_id(&ctx.claude)
        .is_ok_and(|current| current == GATEWAY_PROVIDER_ID);
    let base_url = if on_gateway {
        claude_base_after_restore(ctx, binding)
    } else {
        env_string(&live, "ANTHROPIC_BASE_URL")
    };
    let vendor = host_of(&base_url).unwrap_or_else(|| "Anthropic".to_string());
    let current = if on_gateway {
        String::new()
    } else {
        live.get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let mut models: Vec<OwnModel> = CLAUDE_ALIASES
        .iter()
        .map(|(id, label)| OwnModel {
            id: (*id).to_string(),
            label: (*label).to_string(),
        })
        .collect();
    // 现在用的是一个具体的模型 id（不是别名）：放在最前面，下拉里才看得到它打着勾。
    if !current.is_empty() && !models.iter().any(|model| model.id.eq_ignore_ascii_case(&current))
    {
        models.insert(
            0,
            OwnModel {
                id: current.clone(),
                label: current.clone(),
            },
        );
    }
    OwnModels {
        vendor,
        models,
        current,
    }
}

/// 断开后 Claude Code 会连的地址：接入时存过 settings.json 的，以存档里的为准（还原会改回它）；
/// 否则是接入前那个供应商的；都没有就是没配（直连 Anthropic）。
fn claude_base_after_restore(ctx: &ApplyContext, binding: Option<&AgentBinding>) -> String {
    if let Some(saved) = stash::load_stash::<FileStash>(&claude_stash_path(ctx)) {
        let before = saved
            .before
            .as_deref()
            .and_then(|text| serde_json::from_str::<Value>(text).ok())
            .unwrap_or(Value::Null);
        return env_string(&before, "ANTHROPIC_BASE_URL");
    }
    let previous = binding
        .map(|binding| binding.restore.claude_previous_provider_id.trim())
        .unwrap_or_default();
    if previous.is_empty() {
        return String::new();
    }
    claude::load(&ctx.claude)
        .ok()
        .and_then(|payload| {
            payload
                .providers
                .into_iter()
                .find(|provider| provider.id == previous)
        })
        .map(|provider| env_string(&provider.settings_config, "ANTHROPIC_BASE_URL"))
        .unwrap_or_default()
}

/// 写 Codex `config.toml` 根上的 `model`。断开后还停在托管供应商上说明没还原成，不写。
fn write_codex_model(ctx: &ApplyContext, model: &str) -> anyhow::Result<()> {
    if ctx.settings.load()?.active_relay_id == CODEX_GATEWAY_RELAY_ID {
        bail!("Codex 还接在 Codex++ 网关上，没能断开");
    }
    let path = ctx.codex_home.join("config.toml");
    let text = stash::read_optional(&path)?.unwrap_or_default();
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .context("Codex 的 config.toml 不是有效的 TOML")?;
    doc["model"] = toml_edit::value(model);
    std::fs::create_dir_all(&ctx.codex_home)?;
    crate::settings::atomic_write(&path, doc.to_string().as_bytes())
        .context("写 Codex 的 config.toml 失败")
}

/// 写 Claude Code `settings.json` 的 `model`。供应商用 `ANTHROPIC_MODEL` 指定了模型时它优先，
/// 一起改掉，否则选了不生效。
fn write_claude_model(ctx: &ApplyContext, model: &str) -> anyhow::Result<()> {
    let path = ctx.claude.claude_home.join("settings.json");
    let mut object: Map<String, Value> = match stash::read_optional(&path)? {
        Some(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} 不是有效的 JSON 对象，请先修好再选择模型", path.display()))?,
        None => Map::new(),
    };
    object.insert("model".into(), Value::String(model.to_string()));
    if let Some(env) = object.get_mut("env").and_then(Value::as_object_mut)
        && env.contains_key("ANTHROPIC_MODEL")
    {
        env.insert("ANTHROPIC_MODEL".into(), Value::String(model.to_string()));
    }
    std::fs::create_dir_all(&ctx.claude.claude_home)?;
    crate::settings::atomic_write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&Value::Object(object))?).as_bytes(),
    )
    .context("写 Claude Code 的 settings.json 失败")
}

fn read_json(path: &std::path::Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null)
}

fn env_string(settings: &Value, key: &str) -> String {
    settings
        .pointer(&format!("/env/{key}"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}

/// 地址的主机名（不含端口）；空或认不出时为 None。
fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map_or(url.trim(), |(_, rest)| rest);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = host.split(':').next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AggregateRelayProfile, BackendSettings, RelayMode, RelayProfile};

    fn profile(id: &str, mode: RelayMode) -> RelayProfile {
        RelayProfile {
            id: id.into(),
            name: id.into(),
            relay_mode: mode,
            ..RelayProfile::default()
        }
    }

    fn settings() -> BackendSettings {
        let mut settings = BackendSettings::default();
        settings.relay_profiles = vec![
            profile("aggregate-shadow", RelayMode::Aggregate),
            profile("official", RelayMode::Official),
            profile("relay", RelayMode::PureApi),
        ];
        settings.aggregate_relay_profiles = vec![AggregateRelayProfile {
            id: "aggregate-shadow".into(),
            name: "系统默认".into(),
            session_provider: Default::default(),
            model_aggregation_enabled: true,
            strategy: Default::default(),
            members: Vec::new(),
            routes: Vec::new(),
            default_relay_id: String::new(),
            sync_default_member_login: false,
            auto_review_relay_id: String::new(),
        }];
        settings
    }

    fn target_id(target: Option<CodexTarget<'_>>) -> String {
        match target {
            Some(CodexTarget::Relay(profile)) => profile.id.clone(),
            Some(CodexTarget::Aggregate(aggregate)) => format!("aggregate:{}", aggregate.id),
            None => String::new(),
        }
    }

    #[test]
    fn own_models_follow_the_previous_profile_when_it_is_a_real_provider() {
        let settings = settings();
        assert_eq!(target_id(pick_codex_target(&settings, "relay")), "relay");
    }

    #[test]
    fn own_models_never_expand_an_aggregate_into_its_members() {
        let settings = settings();
        // 接入前是聚合：不展开成员，改用它自己的官方登录那套。
        assert_eq!(
            target_id(pick_codex_target(&settings, "aggregate-shadow")),
            "official"
        );
    }

    #[test]
    fn own_models_fall_back_to_the_first_provider_without_a_sign_in() {
        let mut settings = settings();
        settings.relay_profiles.remove(1);
        assert_eq!(target_id(pick_codex_target(&settings, "")), "relay");
        assert_eq!(
            target_id(pick_codex_target(&settings, "aggregate-shadow")),
            "relay"
        );
    }
}
