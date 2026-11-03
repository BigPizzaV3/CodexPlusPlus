//! 通用 Agent 的接入、断开与漂移检测。
//!
//! - 接入：调规格里的写入函数，把改到的文件落盘；落盘前先存档（每个文件接入前的原文 + 改到的键），
//!   存档放在 `<状态目录>/backups/agent-<id>-before-gateway.json`，只给本人读写。
//! - 断开：按存档把网关改过的键改回去，接入期间 Agent 自己写的其它内容保留。
//! - 漂移：存档还在（Codex++ 认为它接在网关上），但配置文件里已经找不到网关地址。

use std::path::PathBuf;

use crate::gateway::agents::files::{self, AgentStash, FileSet};
use crate::gateway::agents::{AgentSpec, ModelEntry, Wiring};
use crate::gateway::apply::{ApplyContext, gateway_base_url};
use crate::gateway::drift::Drift;
use crate::gateway::restore as stash;
use crate::gateway::route;
use crate::gateway::store::{AgentBinding, GatewaySettings};
use crate::tools::ToolId;

pub fn stash_path(ctx: &ApplyContext, id: &str) -> PathBuf {
    ctx.backup_root.join(format!("agent-{id}-before-gateway.json"))
}

/// 写配置要用的地址、token 和模型列表（与 Grok / Codex 的合成供应商取法一致）。
pub fn wiring(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    id: &str,
    port: u16,
) -> Wiring {
    let default_model = route::binding_default_model(gateway, binding);
    let mut ids = vec![default_model.clone()];
    for model in route::pool_models(gateway, binding)
        .into_iter()
        .chain(binding.declared_models())
    {
        if !model.is_empty() && !ids.contains(&model) {
            ids.push(model);
        }
    }
    let models = ids
        .into_iter()
        .filter(|model| !model.is_empty())
        .map(|model| ModelEntry {
            context_window: route::model_entry_window(gateway, binding, &model)
                .or(binding.context_window),
            reasoning_levels: route::model_entry_reasoning_levels(gateway, binding, &model),
            id: model,
        })
        .collect();
    let small_model = Some(binding.claude_roles.haiku.trim())
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| default_model.clone());
    Wiring {
        agent: id.to_string(),
        root: gateway_base_url(port, &ToolId::Unknown(id.to_string())),
        token: binding.token.clone(),
        default_model,
        small_model,
        models,
    }
}

/// 把 Agent 的配置指向网关。已经接着时再写一遍（模型列表变了），改到的键追加进原有存档。
pub fn write(
    ctx: &ApplyContext,
    spec: &AgentSpec,
    gateway: &GatewaySettings,
    binding: &AgentBinding,
) -> anyhow::Result<()> {
    let wiring = wiring(gateway, binding, spec.id, ctx.port);
    let mut set = FileSet::new();
    (spec.write)(&ctx.agent_env, &wiring, &mut set)?;
    let changes = set.changes()?;
    if changes.is_empty() {
        return Ok(());
    }
    let path = stash_path(ctx, spec.id);
    let mut agent_stash: AgentStash = stash::load_stash(&path).unwrap_or_default();
    for change in &changes {
        agent_stash.record(change)?;
    }
    // 先存档再改配置：存不下就不改。
    stash::save_stash(&path, &agent_stash)?;
    for change in &changes {
        files::write_file(&change.path, change.after.as_deref())?;
    }
    Ok(())
}

/// 断开：把网关改过的键改回接入前的值，然后删掉存档。没有存档时什么都不做。
pub fn restore(ctx: &ApplyContext, spec: &AgentSpec) -> anyhow::Result<()> {
    let path = stash_path(ctx, spec.id);
    let Some(agent_stash) = stash::load_stash::<AgentStash>(&path) else {
        return Ok(());
    };
    for (file, record) in &agent_stash.files {
        let current = stash::read_optional(file)?;
        let restored = if matches!(spec.id, "vscode" | "vscode-insiders" | "vscodium")
            && file.file_name().is_some_and(|name| name == "chatLanguageModels.json")
            && record.format == files::FileFormat::Raw
        {
            super::catalog_apps::restore_vscode_models(
                record.stash.before.as_deref(),
                current.as_deref(),
            )?
        } else if matches!(spec.id, "workbuddy" | "workbuddy-ai" | "codebuddy")
            && file.file_name().is_some_and(|name| name == "models.json")
            && matches!(record.format, files::FileFormat::Raw | files::FileFormat::Json)
        {
            super::catalog_tools::restore_buddy_models(&record.stash, current.as_deref())?
        } else if spec.id == "dsh"
            && file.file_name().is_some_and(|name| name == "cordis.patch.yml")
            && record.format == files::FileFormat::Raw
        {
            super::catalog_tools::restore_dsh_patch(
                file,
                record.stash.before.as_deref(),
                current.as_deref(),
            )?
        } else {
            files::restored_text(record, current.as_deref())?
        };
        if restored != current {
            files::write_file(file, restored.as_deref())?;
        }
    }
    stash::remove_stash(&path);
    Ok(())
}

/// 配置已被别的工具改走时的断开：不碰 Agent 的文件，只丢掉存档。
pub fn forget(ctx: &ApplyContext, spec: &AgentSpec) {
    stash::remove_stash(&stash_path(ctx, spec.id));
}

/// Codex++ 的记录里它还接在网关上（存档还在）。
pub fn recorded(ctx: &ApplyContext, spec: &AgentSpec) -> bool {
    stash_path(ctx, spec.id).exists()
}

/// 配置文件里还有这个 Agent 的网关地址。
pub fn points_at_gateway(ctx: &ApplyContext, spec: &AgentSpec) -> bool {
    let mut paths: Vec<PathBuf> = stash::load_stash::<AgentStash>(&stash_path(ctx, spec.id))
        .map(|agent_stash| agent_stash.files.into_keys().collect())
        .unwrap_or_default();
    paths.push((spec.config_path)(&ctx.agent_env));
    paths.iter().any(|path| {
        std::fs::read_to_string(path).is_ok_and(|text| mentions_gateway(&text, spec.id))
    })
}

/// 文本里出现了 `/gw/<id>`（后面不能紧跟 id 里的字符，`/gw/qoder` 不算 `/gw/qoder-cn`）。
pub fn mentions_gateway(text: &str, id: &str) -> bool {
    let needle = format!("/gw/{id}");
    text.match_indices(&needle).any(|(index, _)| {
        !text[index + needle.len()..]
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    })
}

/// 接着网关、配置里却找不到网关地址：被别的工具改掉了。
pub fn drift(ctx: &ApplyContext, spec: &AgentSpec) -> Option<Drift> {
    if !recorded(ctx, spec) || points_at_gateway(ctx, spec) {
        return None;
    }
    Some(Drift {
        by: None,
        endpoint: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_mention_respects_id_boundary() {
        assert!(mentions_gateway("http://127.0.0.1:1/gw/qoder/v1", "qoder"));
        assert!(mentions_gateway("\"http://127.0.0.1:1/gw/qoder\"", "qoder"));
        assert!(!mentions_gateway("http://127.0.0.1:1/gw/qoder-cn/v1", "qoder"));
        assert!(!mentions_gateway("http://127.0.0.1:1/gw/other", "qoder"));
    }
}
