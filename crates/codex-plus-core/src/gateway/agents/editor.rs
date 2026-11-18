use std::path::Path;

use anyhow::{Context, ensure};
use serde::Serialize;
use serde_json::{Map, Value};
use toml_edit::DocumentMut;

use super::{files, jsonc_edit, spec};
use crate::gateway::{apply::ApplyContext, restore};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub source: String,
    pub contents: String,
    pub keys: Vec<String>,
    pub editable: bool,
}

const COMMON_KEYS: &[&str] = &[
    "developer_instructions",
    "model_instructions_file",
    "compact_prompt",
    "personality",
    "web_search",
    "model_reasoning_effort",
    "model_reasoning_summary",
    "model_verbosity",
    "approval_policy",
    "sandbox_mode",
    "features",
    "tui",
    "notify",
    "tool_output_token_limit",
    "include_environment_context",
    "include_environment_context_time",
    "include_permissions_instructions",
    "include_apps_instructions",
    "include_collaboration_mode_instructions",
];

fn common_doc(text: &str) -> anyhow::Result<DocumentMut> {
    text.parse()
        .map_err(|_: toml_edit::TomlError| anyhow::anyhow!("通用配置不是合法的 TOML"))
}

fn codex(ctx: &ApplyContext, edit: Option<(&str, &str)>) -> anyhow::Result<ConfigView> {
    let mut settings = ctx.settings.load()?;
    let mut full = common_doc(&settings.relay_common_config_contents)?;
    let mut visible = full.clone();
    visible
        .as_table_mut()
        .retain(|key, _| COMMON_KEYS.contains(&key));
    if let Some((original, next)) = edit {
        ensure!(visible.to_string() == original, "配置已变更，请重新打开");
        let draft = common_doc(next)?;
        for (key, item) in draft.iter() {
            ensure!(COMMON_KEYS.contains(&key), "此处不支持修改 {key}");
            if visible
                .get(key)
                .is_some_and(|previous| previous.to_string() == item.to_string())
            {
                continue;
            }
            let valid = match key {
                "features" => item
                    .as_table_like()
                    .is_some_and(|table| table.iter().all(|(_, value)| value.is_bool())),
                "tui" => item.is_table_like(),
                "notify" => item
                    .as_array()
                    .is_some_and(|args| args.iter().all(|arg| arg.is_str())),
                "tool_output_token_limit" => item.as_integer().is_some_and(|value| value > 0),
                key if key.starts_with("include_") => item.is_bool(),
                "web_search" => matches!(item.as_str(), Some("disabled" | "cached" | "live")),
                "sandbox_mode" => matches!(
                    item.as_str(),
                    Some("read-only" | "workspace-write" | "danger-full-access")
                ),
                "approval_policy" => matches!(
                    item.as_str(),
                    Some("untrusted" | "on-failure" | "on-request" | "never")
                ),
                "personality" => matches!(item.as_str(), Some("none" | "friendly" | "pragmatic")),
                "model_verbosity" => matches!(item.as_str(), Some("low" | "medium" | "high")),
                "model_reasoning_effort" => matches!(
                    item.as_str(),
                    Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh")
                ),
                "model_reasoning_summary" => matches!(
                    item.as_str(),
                    Some("auto" | "concise" | "detailed" | "none")
                ),
                _ => item.is_str(),
            };
            ensure!(valid, "{key} 的值无效");
        }
        if full.len() == visible.len() {
            full = draft;
        } else {
            for (key, _) in visible.iter() {
                if !draft.contains_key(key) {
                    full.remove(key);
                }
            }
            for (key, item) in draft.iter() {
                full[key] = item.clone();
            }
        }
        settings.relay_common_config_contents = full.to_string();
        ctx.settings.save(&settings)?;
        crate::settings::notify_external_settings_change();
        visible = full;
        visible
            .as_table_mut()
            .retain(|key, _| COMMON_KEYS.contains(&key));
    }
    Ok(ConfigView {
        source: "Codex++ · relayCommonConfigContents (TOML)".into(),
        contents: visible.to_string(),
        keys: COMMON_KEYS.iter().map(|key| key.to_string()).collect(),
        editable: true,
    })
}

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Bool,
    PositiveNumber,
    AutoUpdate,
}

impl Kind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Text => value.as_str().is_some_and(|text| !text.trim().is_empty()),
            Self::Bool => value.is_boolean(),
            Self::PositiveNumber => value.as_f64().is_some_and(|number| number > 0.0),
            Self::AutoUpdate => value.is_boolean() || value.as_str() == Some("notify"),
        }
    }
}

const DISPLAY_FIELDS: &[(&str, Kind)] = &[
    ("theme", Kind::Text),
    ("language", Kind::Text),
    ("locale", Kind::Text),
    ("fontSize", Kind::PositiveNumber),
    ("font_size", Kind::PositiveNumber),
    ("showLineNumbers", Kind::Bool),
    ("showTimestamps", Kind::Bool),
];

fn source_only(source: String, edit: Option<(&str, &str)>) -> anyhow::Result<ConfigView> {
    ensure!(edit.is_none(), "此配置没有可在这里修改的字段");
    Ok(ConfigView {
        source,
        contents: String::new(),
        keys: vec![],
        editable: false,
    })
}

fn preferences(
    path: &Path,
    section: Option<&str>,
    declared: &[(&str, Kind)],
    edit: Option<(&str, &str)>,
) -> anyhow::Result<ConfigView> {
    let source = match section {
        Some(key) => format!("{} · {key}", path.display()),
        None => path.display().to_string(),
    };
    let before = restore::read_optional(path)?;
    let raw = before.as_deref().unwrap_or_default();
    let extension = path.extension().and_then(|ext| ext.to_str());
    let mut doc =
        match extension {
            Some("toml") => restore::toml_value(before.as_deref())?,
            Some("yaml" | "yml") => {
                let doc = files::YamlDoc::parse(raw);
                let mut values = Map::new();
                for &(key, _) in declared.iter().chain(DISPLAY_FIELDS) {
                    if let Some(text) = doc.scalar(key) {
                        let block = doc.block(key).expect("scalar comes from this block");
                        if block.lines().skip(1).all(|line| {
                            line.trim().is_empty() || line.trim_start().starts_with('#')
                        }) && !text.starts_with(['[', '{', '&', '*', '!', '|', '>'])
                        {
                            values.insert(
                                key.into(),
                                serde_json::from_str(&text).unwrap_or(Value::String(text)),
                            );
                        } else if declared.iter().any(|(name, _)| *name == key) {
                            anyhow::bail!("{key} 不是可编辑的标量");
                        }
                    } else if doc.has(key) && declared.iter().any(|(name, _)| *name == key) {
                        anyhow::bail!("{key} 不是可编辑的标量");
                    }
                }
                Value::Object(values)
            }
            Some("json" | "jsonc" | "json5") => files::parse_jsonc(raw)?,
            _ => return source_only(source, edit),
        };
    let root = doc
        .as_object_mut()
        .context("配置根节点必须是对象，未改动文件")?;
    let root = match section {
        Some(key) => root
            .entry(key)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .with_context(|| format!("{key} 不是配置对象，未改动文件"))?,
        None => root,
    };
    for &(key, kind) in declared {
        ensure!(
            root.get(key).is_none_or(|value| kind.accepts(value)),
            "{key} 当前的格式不受支持，未改动文件"
        );
    }
    let mut fields = declared.to_vec();
    for &(key, kind) in DISPLAY_FIELDS {
        if root.get(key).is_some_and(|value| kind.accepts(value))
            && !declared.iter().any(|(name, _)| *name == key)
        {
            fields.push((key, kind));
        }
    }
    if fields.is_empty() {
        return source_only(source, edit);
    }
    let visible: Map<String, Value> = fields
        .iter()
        .filter_map(|(key, _)| root.get(*key).map(|value| (key.to_string(), value.clone())))
        .collect();
    let mut contents = serde_json::to_string_pretty(&visible)?;
    if let Some((original, next)) = edit {
        ensure!(original == contents, "配置已变更，请重新打开");
        let draft: Map<String, Value> =
            serde_json::from_str(next).context("请输入合法的 JSON 对象")?;
        for (key, value) in &draft {
            let (_, kind) = fields
                .iter()
                .find(|(name, _)| *name == key.as_str())
                .with_context(|| format!("此处不支持修改 {key}"))?;
            ensure!(kind.accepts(value), "{key} 的值无效");
        }
        if draft != visible {
            let text = match extension {
                Some("toml") => {
                    let mut doc: DocumentMut = raw.parse().context("无法解析 TOML 配置")?;
                    for &(key, _) in &fields {
                        if draft.get(key) == visible.get(key) {
                            continue;
                        }
                        match draft.get(key) {
                            Some(Value::String(text)) => doc[key] = toml_edit::value(text.as_str()),
                            Some(Value::Bool(value)) => doc[key] = toml_edit::value(*value),
                            Some(Value::Number(value)) => {
                                doc[key] = if let Some(integer) = value.as_i64() {
                                    toml_edit::value(integer)
                                } else {
                                    toml_edit::value(value.as_f64().context("数字超出范围")?)
                                }
                            }
                            None => {
                                doc.remove(key);
                            }
                            _ => anyhow::bail!("{key} 的值无效"),
                        }
                    }
                    doc.to_string()
                }
                Some("yaml" | "yml") => {
                    let mut doc = files::YamlDoc::parse(raw);
                    for &(key, _) in &fields {
                        if draft.get(key) == visible.get(key) {
                            continue;
                        }
                        if let Some(value) = draft.get(key) {
                            doc.set(key, value);
                        } else {
                            doc.remove(key);
                        }
                    }
                    doc.render()
                }
                _ => {
                    for key in visible.keys() {
                        root.remove(key);
                    }
                    root.extend(draft.clone());
                    jsonc_edit::render(before.as_deref(), &doc)?
                }
            };
            files::write_file(path, Some(&text))?;
            contents = serde_json::to_string_pretty(&draft)?;
        }
    }
    Ok(ConfigView {
        source,
        contents,
        keys: fields.iter().map(|(key, _)| key.to_string()).collect(),
        editable: true,
    })
}

pub fn config(
    ctx: &ApplyContext,
    agent: &str,
    edit: Option<(&str, &str)>,
) -> anyhow::Result<ConfigView> {
    let _guard = crate::gateway::apply::AGENT_CONFIG_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if agent == "codex" {
        return codex(ctx, edit);
    }
    let mut path = match agent {
        "claude" => ctx.claude.claude_home.join("settings.json"),
        "grok" => ctx.grok_home.join("config.toml"),
        other => (spec(other).context("未知的 Agent")?.config_path)(&ctx.agent_env),
    };
    if agent == "openpencil" && edit.is_some() {
        ensure!(
            !super::catalog_apps::process_running(&ctx.agent_env, "openpencil-desktop"),
            "请先退出 OpenPencil 再修改配置"
        );
    }
    let mut section = None;
    let fields: &[(&str, Kind)] = match agent {
        "pi" | "omo" | "prime-agent" => &[("theme", Kind::Text), ("hideThinkingBlock", Kind::Bool)],
        "opencode" | "mimo-code" | "deveco-code" => &[("autoupdate", Kind::AutoUpdate)],
        "claude" => &[("language", Kind::Text)],
        "gemini-cli" | "qwen-code" => {
            section = Some("ui");
            &[("theme", Kind::Text), ("hideBanner", Kind::Bool)]
        }
        "omp" => &[("theme", Kind::Text)],
        "droid" => &[("includeCoAuthoredByDroid", Kind::Bool)],
        "jetbrains-air" => {
            path = super::catalog_apps::air_opencode_config(&ctx.agent_env);
            &[("autoupdate", Kind::AutoUpdate)]
        }
        "ante" | "codebuddy" => {
            path.set_file_name("settings.json");
            &[]
        }
        "openchamber" if path.exists() => {
            let before = restore::read_optional(&path)?.context("偏好文件已移动，请重新打开")?;
            let mut doc = files::parse_jsonc(&before)?;
            ensure!(
                doc.get("version") == Some(&Value::from(1)),
                "不支持此偏好文件版本"
            );
            let fields = doc
                .get_mut("fields")
                .and_then(Value::as_object_mut)
                .context("偏好文件缺少 fields")?;
            let mut view = Map::new();
            if let Some(value) = fields.get("theme") {
                let theme = value
                    .get("value")
                    .and_then(Value::as_str)
                    .context("theme 格式无效")?;
                view.insert("theme".into(), Value::from(theme));
            }
            let mut contents = serde_json::to_string_pretty(&view)?;
            if let Some((original, next)) = edit {
                ensure!(original == contents, "配置已变更，请重新打开");
                let draft: Map<String, Value> =
                    serde_json::from_str(next).context("请输入合法的 JSON 对象")?;
                ensure!(
                    draft
                        .iter()
                        .all(|(key, value)| key == "theme" && Kind::Text.accepts(value)),
                    "只允许修改 theme"
                );
                if draft != view {
                    if let Some(theme) = draft.get("theme") {
                        super::catalog_cli::set_openchamber_pref(fields, "theme", theme.clone());
                    } else {
                        fields.remove("theme");
                    }
                    files::write_file(&path, Some(&jsonc_edit::render(Some(&before), &doc)?))?;
                    contents = serde_json::to_string_pretty(&draft)?;
                }
            }
            return Ok(ConfigView {
                source: path.display().to_string(),
                keys: vec!["theme".into()],
                contents,
                editable: true,
            });
        }
        "openchamber" => {
            path.set_file_name("settings.json");
            &[("theme", Kind::Text)]
        }
        "zcode" => {
            path = ctx.agent_env.home_path(".zcode/v2/setting.json");
            &[]
        }
        "zed" => &[
            ("ui_font_size", Kind::PositiveNumber),
            ("buffer_font_size", Kind::PositiveNumber),
        ],
        "vscode" | "vscode-insiders" | "vscodium" => {
            path.set_file_name("settings.json");
            &[
                ("editor.fontSize", Kind::PositiveNumber),
                ("editor.minimap.enabled", Kind::Bool),
            ]
        }
        "claude-desktop-local" => return source_only("Claude Desktop · 进程环境".into(), edit),
        "copilot-jetbrains" | "aside" | "dsh" | "pencil" | "snow" | "zoo-code" | "workbuddy"
        | "workbuddy-ai" | "cline" | "raycast" | "claude-desktop" => {
            return source_only(path.display().to_string(), edit);
        }
        _ => &[],
    };
    preferences(&path, section, fields, edit)
}
