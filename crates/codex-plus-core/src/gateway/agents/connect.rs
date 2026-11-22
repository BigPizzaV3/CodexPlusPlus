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
    let models = route::binding_exposed_models(gateway, binding)
        .into_iter()
        .filter(|model| !model.is_empty())
        .map(|model| ModelEntry {
            display_name: route::model_display_name(gateway, &model).to_string(),
            context_window: route::model_entry_window(gateway, binding, &model)
                .or(binding.context_window),
            reasoning_levels: route::model_entry_reasoning_levels(gateway, binding, &model),
            id: model,
        })
        .collect();
    let small_model = Some(binding.claude_roles.haiku.trim())
        .filter(|model| !model.is_empty() && !route::is_disabled_group_model(gateway, model))
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
    use crate::gateway::agents::{Os, spec};
    use crate::gateway::apply::{GroupInput, save_group};
    use crate::gateway::store::{GatewayStore, GroupMember};
    use serde_json::Value;

    fn model_label(value: &Value, agent: &str, model: &str) -> Option<String> {
        match value {
            Value::Object(object) => {
                let row = match agent {
                    "qoder" | "qoder-cn" if object.contains_key("models") => None,
                    "opencode" | "openchamber" | "mimo-code" | "deveco-code"
                    | "command-code" | "minimax-code" | "zcode" | "copilot-jetbrains" => {
                        object.get(model)
                    }
                    _ => {
                        let key = match agent {
                            "droid" | "qoder" | "qoder-cn" => "model",
                            "zed" => "name",
                            "t3code" => "slug",
                            _ => "id",
                        };
                        (object.get(key).and_then(Value::as_str) == Some(model)).then_some(value)
                    }
                };
                if let Some(row) = row {
                    let label = match agent {
                        "droid" | "qoder" | "qoder-cn" => &row["displayName"],
                        "zed" | "ante" => &row["display_name"],
                        "copilot-jetbrains" => &row["modelCapabilities"]["name"],
                        _ => &row["name"],
                    };
                    if agent == "droid" {
                        assert_eq!(row["id"], format!("custom:codexplus/{model}"));
                    }
                    return Some(label.as_str().unwrap().to_string());
                }
                for (key, value) in object {
                    if key == "app_models" {
                        if let Some(label) = model_label(
                            &serde_json::from_str::<Value>(value.as_str().unwrap()).unwrap(),
                            agent,
                            model,
                        ) {
                            return Some(label);
                        }
                    } else if let Some(label) = model_label(value, agent, model) {
                        return Some(label);
                    }
                }
                None
            }
            Value::Array(array) => array.iter().find_map(|value| model_label(value, agent, model)),
            _ => None,
        }
    }

    #[test]
    fn renamed_groups_refresh_client_display_labels_and_keep_route_ids() {
        let display_agents = [
            "opencode",
            "openchamber",
            "mimo-code",
            "deveco-code",
            "pi",
            "omo",
            "prime-agent",
            "stepcode",
            "aside",
            "omp",
            "mothx",
            "crush",
            "command-code",
            "qwen-code",
            "dsh",
            "empryo",
            "minimax-code",
            "droid",
            "cline",
            "qoder",
            "qoder-cn",
            "grok-app",
            "zcode",
            "workbuddy",
            "workbuddy-ai",
            "codebuddy",
            "ante",
            "zed",
            "vscode",
            "vscode-insiders",
            "vscodium",
            "copilot-jetbrains",
            "pencil",
            "raycast",
            "t3code",
            "openclaw",
        ];
        let ids = ["聚合供应商-1", "second"];
        for id in display_agents {
            let spec = spec(id).unwrap();
            let temp = tempfile::tempdir().unwrap();
            let env =
                crate::gateway::agents::AgentEnv::at(&temp.path().join("home")).with_os(Os::Linux);
            let store = GatewayStore::at(temp.path().join("gateway.json"));
            if spec.id == "zcode" {
                let legacy = (spec.config_path)(&env).with_file_name("config.json");
                std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
                std::fs::write(legacy, "{}").unwrap();
            }
            for names in [
                ["6.1 Sol", "工作组"],
                ["日常编码", "备用 Sol"],
                ["Sol again", "另一个组"],
            ] {
                for (id, name) in ids.into_iter().zip(names) {
                    save_group(
                        &store,
                        &GroupInput {
                            id: id.to_string(),
                            name: name.to_string(),
                            members: vec![GroupMember {
                                model: "gpt-6.1-sol".to_string(),
                                weight: 1,
                            }],
                            ..GroupInput::default()
                        },
                    )
                    .unwrap();
                }
                let gateway = store.load().unwrap();
                let wiring = wiring(
                    &gateway,
                    &AgentBinding {
                        model: "group/聚合供应商-1".to_string(),
                        ..AgentBinding::default()
                    },
                    spec.id,
                    57321,
                );
                assert_eq!(wiring.default_model, "group/聚合供应商-1");
                let mut set = FileSet::new();
                (spec.write)(&env, &wiring, &mut set).unwrap();
                let mut labels = Vec::new();
                for change in set.changes().unwrap() {
                    let text = change.after.as_deref().unwrap();
                    let value = match change.format {
                        files::FileFormat::Json | files::FileFormat::Toml => Some(
                            files::content_value(change.format, Some(text)).unwrap(),
                        ),
                        files::FileFormat::Raw
                            if change.path.extension().is_some_and(|ext| ext == "json") =>
                        {
                            Some(files::parse_jsonc(text).unwrap())
                        }
                        files::FileFormat::Yaml | files::FileFormat::Raw => {
                            let lines = text
                                .lines()
                                .map(|line| line.trim_start().trim_start_matches("- "))
                                .collect::<Vec<_>>()
                                .join("\n");
                            for (id, name) in ids.into_iter().zip(names) {
                                let model = format!("group/{id}");
                                let expected = if spec.id == "minimax-code" {
                                    files::yaml_block(
                                        &model,
                                        &serde_json::json!({ "name": name }),
                                    )
                                    .lines()
                                    .map(str::trim_start)
                                    .collect::<Vec<_>>()
                                    .join("\n")
                                } else {
                                    format!(
                                        "id: {}\nname: {}",
                                        serde_json::json!(model),
                                        serde_json::json!(name),
                                    )
                                };
                                // dsh 的条目在 id 和 name 之间还有 input 等字段（#3042 起声明可收图），
                                // 所以只要求 name 出现在同一条目里：id 行之后、下一个 id 行之前。
                                let (id_line, name_line) = expected.split_once('\n').unwrap();
                                let in_same_entry = || {
                                    let mut rows = lines.lines().skip_while(|row| *row != id_line);
                                    rows.next().is_some()
                                        && rows
                                            .take_while(|row| !row.starts_with("id: "))
                                            .any(|row| row == name_line)
                                };
                                if lines.contains(&expected) || (spec.id == "dsh" && in_same_entry()) {
                                    labels.push((model, name.to_string()));
                                }
                            }
                            None
                        }
                        files::FileFormat::Env => None,
                    };
                    if let Some(value) = value {
                        for id in ids {
                            let model = format!("group/{id}");
                            if let Some(label) = model_label(&value, spec.id, &model) {
                                labels.push((model, label));
                            }
                        }
                    }
                    files::write_file(&change.path, change.after.as_deref()).unwrap();
                }
                for (id, name) in ids.into_iter().zip(names) {
                    assert!(
                        labels.contains(&(format!("group/{id}"), name.to_string())),
                        "{} 没有在稳定模型 ID group/{id} 上写入当前显示名 {name}：{labels:?}",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn gateway_mention_respects_id_boundary() {
        assert!(mentions_gateway("http://127.0.0.1:1/gw/qoder/v1", "qoder"));
        assert!(mentions_gateway("\"http://127.0.0.1:1/gw/qoder\"", "qoder"));
        assert!(!mentions_gateway("http://127.0.0.1:1/gw/qoder-cn/v1", "qoder"));
        assert!(!mentions_gateway("http://127.0.0.1:1/gw/other", "qoder"));
    }
}
