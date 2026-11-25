//! Claude Code 直连共享供应商里的一家：不经 Codex++，地址和 Key 写进它自己的 `~/.claude/settings.json`。
//!
//! Claude 页的「直连」：共享供应商（和 Codex 共用的那一份）里 Anthropic 协议、用 Key 的那几家，
//! Claude Code 可以不经本地服务直接连，关掉 Codex++ 照样能用。做法和「断开并直连」同一条路：
//! 接入着就先断开并还原（[`apply::disable_binding`]），再把这一家存成 Claude 供应商列表里的一项
//! （id `pool-<共享供应商 id>`，重复选同一家就地更新）并切过去。OpenAI 协议、订阅账号这些
//! 要本地服务转换或代登录的不能直连，只能聚合。

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};

use crate::gateway::apply::{self, ApplyContext, agent_points_at_gateway};
use crate::gateway::store::{GatewayProvider, GatewaySettings};
use crate::settings::{BackendSettings, RelayProtocol};
use crate::tools::ToolId;
use crate::tools::claude::{self, ClaudeProvider};

/// Claude 供应商列表里，直连共享供应商的那几项的 id 前缀。
pub const POOL_PROVIDER_PREFIX: &str = "pool-";

/// 直连这一家时，Claude 供应商列表里那一项的 id。
pub fn claude_provider_id(pool_id: &str) -> String {
    format!("{POOL_PROVIDER_PREFIX}{pool_id}")
}

/// Claude Code 现在直连的共享供应商 id（当前 Claude 供应商是 `pool-<id>` 时）；不是时为 None。
pub fn current_pool_provider(ctx: &ApplyContext) -> Option<String> {
    let current = claude::current_provider_id(&ctx.claude).ok()?;
    current
        .strip_prefix(POOL_PROVIDER_PREFIX)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// 这一家不能给 Claude Code 直连的原因；能直连时为 None。
pub fn rejection(provider: &GatewayProvider) -> Option<&'static str> {
    let relay = &provider.relay;
    if relay.protocol != RelayProtocol::AnthropicMessages {
        return Some("只有 Anthropic 协议的供应商能直连，OpenAI 协议的要经 Codex++ 转换");
    }
    if crate::gateway::accounts::relay_account_kind(relay).is_some()
        || crate::official_login::member_uses_official_login_transport(relay)
    {
        return Some("订阅账号要经 Codex++ 代登录，不能直连");
    }
    if claude_code_base_url(&relay.base_url).is_none() {
        return Some("这家的地址 Claude Code 用不了（没有地址，或地址末尾带 # 不让补 /v1）");
    }
    if !relay.uses_no_auth() && api_key(provider).is_empty() {
        return Some("这家没有 API Key");
    }
    None
}

/// 共享供应商的地址换成 Claude Code 的 `ANTHROPIC_BASE_URL`：它自己会补 `/v1/messages`，
/// 所以去掉末尾的 `/messages`、`/v1`。末尾带 `#`（不补版本前缀）的地址它表达不了，返回 None。
pub fn claude_code_base_url(base_url: &str) -> Option<String> {
    let base = base_url.trim();
    if base.is_empty() || base.ends_with('#') {
        return None;
    }
    let mut base = base.trim_end_matches('/').to_string();
    for suffix in ["/messages", "/v1"] {
        if base.to_ascii_lowercase().ends_with(suffix) {
            base.truncate(base.len() - suffix.len());
            base = base.trim_end_matches('/').to_string();
        }
    }
    (!base.is_empty()).then_some(base)
}

fn api_key(provider: &GatewayProvider) -> String {
    provider
        .relay
        .selected_api_key()
        .map(|entry| entry.api_key.trim().to_string())
        .filter(|key| !key.is_empty())
        .unwrap_or_else(|| provider.relay.api_key.trim().to_string())
}

/// 直连用的模型：选了就用选的（要在这家的模型列表里），没选用这家的默认模型或第一个聊天模型。
fn pick_model(provider: &GatewayProvider, model: Option<&str>) -> anyhow::Result<String> {
    let models = crate::gateway::route::chat_models(&provider.relay);
    if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
        if !models.iter().any(|known| known == model) {
            bail!("{} 没有模型 {model}", provider.relay.name.trim());
        }
        return Ok(model.to_string());
    }
    let preferred = provider.relay.model.trim();
    if !preferred.is_empty() && models.iter().any(|known| known == preferred) {
        return Ok(preferred.to_string());
    }
    models
        .into_iter()
        .next()
        .with_context(|| format!("{} 没有可用的聊天模型", provider.relay.name.trim()))
}

/// 这一家写给 Claude Code 的配置（Claude 供应商列表里的一项）。
pub fn claude_provider(provider: &GatewayProvider, model: Option<&str>) -> anyhow::Result<ClaudeProvider> {
    if let Some(reason) = rejection(provider) {
        bail!("{}：{reason}", provider.relay.name.trim());
    }
    let model = pick_model(provider, model)?;
    let base_url = claude_code_base_url(&provider.relay.base_url).unwrap_or_default();
    let mut env = Map::new();
    env.insert("ANTHROPIC_BASE_URL".into(), Value::String(base_url.clone()));
    if !provider.relay.uses_no_auth() {
        env.insert("ANTHROPIC_AUTH_TOKEN".into(), Value::String(api_key(provider)));
    }
    env.insert("ANTHROPIC_MODEL".into(), Value::String(model.clone()));
    // 第三方的 Anthropic 兼容接口不认 Claude 的模型名：后台的小模型请求也指到这个模型，否则会报模型不存在。
    if !base_url.to_ascii_lowercase().contains("anthropic.com") {
        env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), Value::String(model));
    }
    Ok(ClaudeProvider {
        id: claude_provider_id(&provider.relay.id),
        name: provider.relay.name.trim().to_string(),
        website_url: String::new(),
        notes: "Codex++ 共享供应商直连：在 Codex++ 里改地址、Key、模型后重新选一次就更新。".to_string(),
        settings_config: json!({ "env": Value::Object(env) }),
        created_at: 0,
        ccs_source_id: String::new(),
    })
}

/// 让 Claude Code 直连共享供应商 `pool_id`：接入着就先断开并还原，再写它自己的 settings.json。
pub fn use_pool_provider(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    pool_id: &str,
    model: Option<&str>,
) -> anyhow::Result<GatewaySettings> {
    let gateway = ctx.gateway.load()?;
    let provider = gateway
        .provider(pool_id.trim())
        .with_context(|| format!("共享供应商不存在：{pool_id}"))?;
    if !provider.enabled {
        bail!("{} 已停用", provider.relay.name.trim());
    }
    // 先把要写的配置算好：不能直连就别动现在的接入。
    let target = claude_provider(provider, model)?;
    let connected = gateway
        .binding(&ToolId::Claude)
        .is_some_and(|binding| binding.enabled);
    if connected || agent_points_at_gateway(ctx, &ToolId::Claude) {
        apply::disable_binding(ctx, settings, &ToolId::Claude)?;
    }
    let id = target.id.clone();
    claude::save_provider(&ctx.claude, target)?;
    claude::switch_provider(&ctx.claude, &id)?;
    ctx.gateway.load()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_drops_the_version_and_messages_suffix() {
        assert_eq!(claude_code_base_url("https://api.kimi.test/anthropic").as_deref(), Some("https://api.kimi.test/anthropic"));
        assert_eq!(claude_code_base_url("https://api.kimi.test/anthropic/v1/").as_deref(), Some("https://api.kimi.test/anthropic"));
        assert_eq!(claude_code_base_url("https://x.test/v1/messages").as_deref(), Some("https://x.test"));
        assert_eq!(claude_code_base_url("https://x.test/api#"), None);
        assert_eq!(claude_code_base_url("  "), None);
    }
}
