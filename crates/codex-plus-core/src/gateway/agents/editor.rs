//! 读取和保存适配器允许用户编辑的偏好，不改变网关绑定。

use std::path::Path;

use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{PreferenceKind, Preferences, files, jsonc_edit, spec};
use crate::gateway::{apply::ApplyContext, restore};

/// 编辑器显示的配置来源、可编辑字段和输入格式。没有开放字段时只显示来源。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    /// 保存时应原样传回的来源和内容。
    #[serde(flatten)]
    pub snapshot: ConfigSnapshot,
    /// 草稿使用的格式；只读来源没有输入格式。
    pub input_format: &'static str,
    /// 当前来源是否开放可编辑字段。
    pub editable: bool,
}

/// 打开编辑器时读取的快照，用于拒绝写入已切换的配置档或覆盖后续修改。
#[derive(Deserialize, Serialize)]
pub struct ConfigSnapshot {
    /// 由后端解析的配置来源，保存时不将它作为写入路径。
    pub source: String,
    /// 当时展示的可编辑内容，不包含同一文件中的其他字段。
    pub contents: String,
}

impl ConfigSnapshot {
    fn check(&self, source: &str, contents: &str) -> anyhow::Result<()> {
        ensure!(self.source == source, "配置来源已变更，请重新打开");
        ensure!(self.contents == contents, "配置已变更，请重新打开");
        Ok(())
    }
}

impl PreferenceKind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Text => value.as_str().is_some_and(|text| !text.trim().is_empty()),
            Self::Bool => value.is_boolean(),
            Self::PositiveNumber => value.as_f64().is_some_and(|number| number > 0.0),
            Self::AutoUpdate => value.is_boolean() || value.as_str() == Some("notify"),
        }
    }
}

fn source_only(
    source: String,
    edit: Option<(&ConfigSnapshot, &str)>,
) -> anyhow::Result<ConfigView> {
    ensure!(edit.is_none(), "此配置没有可在这里修改的字段");
    Ok(ConfigView {
        snapshot: ConfigSnapshot {
            source,
            contents: String::new(),
        },
        input_format: "",
        editable: false,
    })
}

fn write_preferences(path: &Path, before: Option<&str>, text: &str) -> anyhow::Result<()> {
    // 与本次保存解析的原文比较；回读和写入之间仍可能有其他进程修改文件。
    ensure!(
        restore::read_optional(path)?.as_deref() == before,
        "保存期间配置已变更，请重新打开"
    );
    files::write_file(path, Some(text))?;
    ensure!(
        restore::read_optional(path)?.as_deref() == Some(text),
        "保存后回读不一致，请重新打开"
    );
    Ok(())
}

fn preferences(
    path: &Path,
    agent: &str,
    section: Option<&str>,
    fields: &[(&str, PreferenceKind)],
    edit: Option<(&ConfigSnapshot, &str)>,
) -> anyhow::Result<ConfigView> {
    let source = match section {
        Some(key) => format!("{} · {key}", path.display()),
        None if matches!(agent, "vscode" | "vscode-insiders" | "vscodium") => {
            format!(
                "{} · 默认 User/settings.json（不跟随当前 profile）",
                path.display()
            )
        }
        None => path.display().to_string(),
    };
    let toml = match path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => true,
        Some("json" | "jsonc") => false,
        _ => return source_only(source, edit),
    };
    let before = restore::read_optional(path)?;
    let raw = before.as_deref().unwrap_or_default();
    let mut doc = if toml {
        restore::toml_value(before.as_deref())?
    } else {
        jsonc_edit::read_preferences(raw)?
    };
    let mut root = doc
        .as_object_mut()
        .context("配置根节点必须是对象，未改动文件")?;
    if let Some(key) = section {
        root = root
            .entry(key)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .with_context(|| format!("{key} 不是配置对象，未改动文件"))?;
    }
    for &(key, kind) in fields {
        ensure!(
            root.get(key).is_none_or(|value| kind.accepts(value)),
            "{key} 当前的格式不受支持，未改动文件"
        );
    }
    let visible: Map<String, Value> = fields
        .iter()
        .filter_map(|(key, _)| root.get(*key).map(|value| (key.to_string(), value.clone())))
        .collect();
    let format_view = |values: &Map<String, Value>| -> anyhow::Result<String> {
        if toml {
            Ok(toml::to_string_pretty(values)?)
        } else {
            Ok(serde_json::to_string_pretty(values)?)
        }
    };
    let mut contents = format_view(&visible)?;
    if let Some((original, next)) = edit {
        original.check(&source, &contents)?;
        let parsed = if toml {
            restore::toml_value(Some(next))?
        } else {
            Value::Object(jsonc_edit::preference_draft(next)?)
        };
        let draft = parsed.as_object().context("设置必须是键值对象")?;
        validate_draft(fields, draft)?;
        if draft != &visible {
            contents = format_view(draft)?;
            let text = if toml {
                let mut doc = restore::parse_toml("配置", raw)?;
                let pending = restore::parse_toml("配置", &contents)?;
                let table = if let Some(section) = section {
                    doc.as_table_mut()
                        .entry(section)
                        .or_insert_with(|| toml_edit::Item::Table(toml_edit::Table::new()))
                        .as_table_like_mut()
                        .context("目标不是 TOML 表，未改动文件")?
                } else {
                    doc.as_table_mut()
                };
                for &(key, _) in fields {
                    if draft.get(key) == visible.get(key) {
                        continue;
                    }
                    let Some(item) = pending.get(key) else {
                        table.remove(key);
                        continue;
                    };
                    let mut value = item
                        .as_value()
                        .with_context(|| format!("{key} 的值无效"))?
                        .clone();
                    if let Some(previous) =
                        table.get_mut(key).and_then(toml_edit::Item::as_value_mut)
                    {
                        *value.decor_mut() = previous.decor().clone();
                        *previous = value;
                    } else {
                        table.insert(key, toml_edit::Item::Value(value));
                    }
                }
                doc.to_string()
            } else {
                for key in visible.keys() {
                    root.remove(key);
                }
                root.extend(draft.clone());
                jsonc_edit::render(before.as_deref(), &doc)?
            };
            write_preferences(path, before.as_deref(), &text)?;
        }
    }
    Ok(ConfigView {
        snapshot: ConfigSnapshot { source, contents },
        input_format: if toml { "TOML" } else { "JSON / JSONC" },
        editable: true,
    })
}

fn validate_draft(
    fields: &[(&str, PreferenceKind)],
    draft: &Map<String, Value>,
) -> anyhow::Result<()> {
    for (key, value) in draft {
        let (_, kind) = fields
            .iter()
            .find(|(name, _)| *name == key.as_str())
            .with_context(|| format!("此处不支持修改 {key}"))?;
        ensure!(kind.accepts(value), "{key} 的值无效");
    }
    Ok(())
}

/// 读取指定 Agent 的配置。传入原始快照和草稿时，只保存开放的字段并返回更新后的视图。
pub fn config(
    ctx: &ApplyContext,
    agent: &str,
    edit: Option<(&ConfigSnapshot, &str)>,
) -> anyhow::Result<ConfigView> {
    let _guard = crate::gateway::apply::AGENT_CONFIG_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    match agent {
        "claude" => {
            return preferences(
                &ctx.claude.claude_home.join("settings.json"),
                agent,
                None,
                &[("language", PreferenceKind::Text)],
                edit,
            );
        }
        "grok" => {
            return source_only(
                ctx.grok_home.join("config.toml").display().to_string(),
                edit,
            );
        }
        _ => {}
    }
    let spec = spec(agent).context("未知的 Agent")?;
    let main_path = (spec.config_path)(&ctx.agent_env);
    let path = match agent {
        "ante" | "codebuddy" | "vscode" | "vscode-insiders" | "vscodium" => {
            main_path.with_file_name("settings.json")
        }
        "openchamber" if !main_path.try_exists()? => main_path.with_file_name("settings.json"),
        "zcode" => ctx.agent_env.home_path(".zcode/v2/setting.json"),
        "claude-desktop-local" => return source_only("Claude Desktop · 进程环境".into(), edit),
        _ => main_path,
    };
    let Preferences::Editable { section, fields } = spec.preferences else {
        return source_only(path.display().to_string(), edit);
    };
    preferences(&path, agent, section, fields, edit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_preferences_checks_exact_source_and_existence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(write_preferences(&path, Some(""), "{}").is_err());
        assert!(!path.exists());

        write_preferences(&path, None, "").unwrap();
        assert!(write_preferences(&path, None, "{}").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"");

        write_preferences(&path, Some(""), "{}\r\n").unwrap();
        assert!(write_preferences(&path, Some("{}\n"), "{}").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{}\r\n");
    }
}
