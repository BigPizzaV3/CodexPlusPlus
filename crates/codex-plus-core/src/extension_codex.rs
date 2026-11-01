//! 拓展声明的 Codex 原生能力：技能、子 agent 角色、外部 MCP 服务器、全局指令。
//!
//! 清单（codexpp.json）里的写法：
//! - `skills`：包内含 `SKILL.md` 的目录列表；省略时自动收 `skills/*/SKILL.md`。装进 `~/.codex/skills/<拓展>-<目录名>`。
//! - `agents`：`[{ name, description, instructions?, model?, reasoningEffort?, nicknames? }]`，写成 Codex 的
//!   `[agents.<拓展>-<name>]`，角色配置文件放在 Codex++ 数据目录。`instructions` 可以是包内 .md 路径或文字。
//! - `mcpServers`：`{ 名字: { command, args?, env?, cwd? } | { url, headers? } }`，写成 `[mcp_servers.<拓展>-<名字>]`。
//!   字符串里的 `${extensionDir}` 换成包目录，`${config.键}` 换成拓展配置值。
//! - `instructions`：包内 .md 路径或文字，合并进 `~/.codex/AGENTS.md` 里 Codex++ 托管的一段（官方直连也生效）。
//! - `prompts` / `resources`：经 Codex++ 拓展 MCP 提供的提示词模板与只读资源（不写进 Codex home）。
//!
//! 同步只动自己写过的东西：技能目录带标记文件，配置项与 AGENTS.md 段落记在状态文件里；同名的用户自有条目不覆盖。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{Map, Value, json};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::user_scripts::UserScriptManager;

/// 技能目录里的归属标记文件，内容是拓展 id。
pub const SKILL_MARKER: &str = ".codexpp-extension";
const INSTRUCTIONS_START: &str = "<!-- codexpp-extensions:start（由 Codex++ 拓展管理，请勿手改这一段） -->";
const INSTRUCTIONS_END: &str = "<!-- codexpp-extensions:end -->";
/// 单个技能目录的大小上限。
const SKILL_MAX_BYTES: u64 = 4 * 1024 * 1024;
const INSTRUCTIONS_MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionCodexAssets {
    /// 含 SKILL.md 的包内目录（绝对路径）。
    pub skills: Vec<PathBuf>,
    pub agents: Vec<ExtensionAgent>,
    pub mcp_servers: Vec<ExtensionMcpServer>,
    /// 合并进全局 AGENTS.md 的文字。
    pub instructions: Option<String>,
    /// 经 Codex++ 拓展 MCP 提供的提示词模板（prompts/list、prompts/get）。
    pub prompts: Vec<ExtensionPrompt>,
    /// 经 Codex++ 拓展 MCP 提供的只读资源（resources/list、resources/read）。
    pub resources: Vec<ExtensionResource>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionPrompt {
    pub name: String,
    pub description: String,
    /// [{ name, description, required }]
    pub arguments: Vec<Value>,
    /// 模板文字，`{{参数名}}` 换成调用时给的值。
    pub template: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionResource {
    pub name: String,
    pub description: String,
    pub mime_type: String,
    /// 包内文件（绝对路径）。
    pub file: PathBuf,
}

/// 单个资源文件的大小上限。
pub const RESOURCE_MAX_BYTES: u64 = 1024 * 1024;

impl ExtensionCodexAssets {
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty() && self.agents.is_empty() && self.mcp_servers.is_empty() && self.instructions.is_none() && self.prompts.is_empty() && self.resources.is_empty()
    }

    /// 给清单摘要用的计数与名字。
    pub fn summary(&self) -> Value {
        json!({
            "skills": self.skills.iter().filter_map(|dir| dir.file_name().map(|name| name.to_string_lossy().to_string())).collect::<Vec<_>>(),
            "agents": self.agents.iter().map(|agent| agent.name.as_str()).collect::<Vec<_>>(),
            "mcpServers": self.mcp_servers.iter().map(|server| server.name.as_str()).collect::<Vec<_>>(),
            "instructions": self.instructions.is_some(),
            "prompts": self.prompts.iter().map(|prompt| prompt.name.as_str()).collect::<Vec<_>>(),
            "resources": self.resources.iter().map(|resource| resource.name.as_str()).collect::<Vec<_>>(),
        })
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionAgent {
    pub name: String,
    pub description: String,
    pub instructions: Option<String>,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub nicknames: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionMcpServer {
    pub name: String,
    /// 原样的声明（command / args / env / cwd 或 url / headers），同步时再展开占位符。
    pub spec: Map<String, Value>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 48 && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// 包内路径或文字：以 .md / .txt 结尾且文件存在时读文件，否则当文字。
fn text_or_file(root: &Path, text: &str, label: &str) -> anyhow::Result<String> {
    let looks_like_path = !text.contains('\n') && (text.ends_with(".md") || text.ends_with(".txt"));
    if !looks_like_path {
        return Ok(text.to_string());
    }
    let path = crate::user_scripts::resolve_package_path(root, text)?;
    let metadata = std::fs::metadata(&path).with_context(|| format!("{label} 指向的文件不存在：{text}"))?;
    if metadata.len() > INSTRUCTIONS_MAX_BYTES {
        anyhow::bail!("{label} 文件超过 64 KiB：{text}");
    }
    std::fs::read_to_string(&path).with_context(|| format!("无法读取 {label} 文件 {text}"))
}

/// 解析清单里的 skills / agents / mcpServers / instructions。
pub fn parse(root: &Path, raw: &Map<String, Value>) -> anyhow::Result<ExtensionCodexAssets> {
    let mut assets = ExtensionCodexAssets::default();
    assets.skills = match raw.get("skills") {
        None => {
            let mut found = Vec::new();
            if let Ok(entries) = std::fs::read_dir(root.join("skills")) {
                let mut entries = entries.flatten().collect::<Vec<_>>();
                entries.sort_by_key(|entry| entry.file_name());
                for entry in entries {
                    let path = entry.path();
                    let is_dir = std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
                    if is_dir && path.join("SKILL.md").is_file() {
                        found.push(path);
                    }
                }
            }
            found
        }
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let relative = item.as_str().context("skills 只能是包内目录的字符串数组")?;
                let path = crate::user_scripts::resolve_package_path(root, relative)?;
                if !path.join("SKILL.md").is_file() {
                    anyhow::bail!("skills 里的 {relative} 没有 SKILL.md");
                }
                Ok(path)
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("skills 只能是包内目录的字符串数组"),
    };
    for skill in &assets.skills {
        let name = skill.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        if !valid_name(&name) {
            anyhow::bail!("技能目录名不合法：{name}（只含字母、数字、- 和 _）");
        }
    }
    assets.agents = match raw.get("agents") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let item = item.as_object().context("agents 的每一项必须是对象")?;
                let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
                let name = text("name").context("agents 的每一项需要 name")?;
                if !valid_name(&name) {
                    anyhow::bail!("子 agent 名字不合法：{name}（只含字母、数字、- 和 _）");
                }
                let description = text("description").context(format!("子 agent {name} 需要 description（Codex 据此决定何时派它）"))?;
                let instructions = text("instructions").map(|text| text_or_file(root, &text, "agents.instructions")).transpose()?;
                let nicknames = match item.get("nicknames") {
                    None => Vec::new(),
                    Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
                    Some(_) => anyhow::bail!("agents.nicknames 只能是字符串数组"),
                };
                Ok(ExtensionAgent { name, description, instructions, model: text("model"), reasoning_effort: text("reasoningEffort"), nicknames })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("agents 只能是数组"),
    };
    assets.mcp_servers = match raw.get("mcpServers") {
        None => Vec::new(),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(name, spec)| {
                if !valid_name(name) {
                    anyhow::bail!("MCP 服务器名字不合法：{name}（只含字母、数字、- 和 _）");
                }
                let spec = spec.as_object().with_context(|| format!("mcpServers.{name} 必须是对象"))?;
                let has_command = spec.get("command").and_then(Value::as_str).is_some_and(|command| !command.trim().is_empty());
                let has_url = spec.get("url").and_then(Value::as_str).is_some_and(|url| url.starts_with("http://") || url.starts_with("https://"));
                if has_command == has_url {
                    anyhow::bail!("mcpServers.{name} 需要 command（本地进程）或 url（HTTP），二选一");
                }
                for key in spec.keys() {
                    if !matches!(key.as_str(), "command" | "args" | "env" | "cwd" | "url" | "headers" | "startupTimeoutSec" | "toolTimeoutSec") {
                        anyhow::bail!("mcpServers.{name} 不支持 {key}");
                    }
                }
                Ok(ExtensionMcpServer { name: name.clone(), spec: spec.clone() })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("mcpServers 必须是对象：{{ \"名字\": {{ \"command\": \"node\", \"args\": [...] }} }}"),
    };
    assets.prompts = match raw.get("prompts") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let item = item.as_object().context("prompts 的每一项必须是对象")?;
                let name = item.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
                if !valid_name(&name) {
                    anyhow::bail!("提示词名字不合法：{name:?}（只含字母、数字、- 和 _）");
                }
                let template = item.get("template").and_then(Value::as_str).context(format!("提示词 {name} 需要 template（文字或包内 .md 路径）"))?;
                let arguments = match item.get("arguments") {
                    None => Vec::new(),
                    Some(Value::Array(list)) => list
                        .iter()
                        .map(|argument| {
                            let argument_name = argument.get("name").and_then(Value::as_str).filter(|text| valid_name(text)).context(format!("提示词 {name} 的参数需要合法的 name"))?;
                            Ok(json!({
                                "name": argument_name,
                                "description": argument.get("description").and_then(Value::as_str).unwrap_or_default(),
                                "required": argument.get("required").and_then(Value::as_bool).unwrap_or(false),
                            }))
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?,
                    Some(_) => anyhow::bail!("prompts.arguments 只能是数组"),
                };
                Ok(ExtensionPrompt {
                    description: item.get("description").and_then(Value::as_str).unwrap_or_default().to_string(),
                    template: text_or_file(root, template, "prompts.template")?,
                    name,
                    arguments,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("prompts 只能是数组"),
    };
    assets.resources = match raw.get("resources") {
        None => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                let item = item.as_object().context("resources 的每一项必须是对象")?;
                let relative = item.get("file").and_then(Value::as_str).context("resources 的每一项需要 file（包内路径）")?;
                let file = crate::user_scripts::resolve_package_path(root, relative)?;
                if !file.is_file() {
                    anyhow::bail!("resources 的 {relative} 不是文件");
                }
                let name = item.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| relative.to_string());
                let mime_type = item.get("mimeType").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
                    match relative.rsplit('.').next().unwrap_or_default() {
                        "md" => "text/markdown",
                        "json" => "application/json",
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "csv" => "text/csv",
                        _ => "text/plain",
                    }
                    .to_string()
                });
                Ok(ExtensionResource { name, description: item.get("description").and_then(Value::as_str).unwrap_or_default().to_string(), mime_type, file })
            })
            .collect::<anyhow::Result<Vec<_>>>()?,
        Some(_) => anyhow::bail!("resources 只能是数组"),
    };
    assets.instructions = match raw.get("instructions") {
        None => None,
        Some(Value::String(text)) if text.trim().is_empty() => None,
        Some(Value::String(text)) => Some(text_or_file(root, text, "instructions")?),
        Some(_) => anyhow::bail!("instructions 只能是包内 .md 路径或文字"),
    };
    Ok(assets)
}

/// 一个启用中的拓展要同步的内容。
pub struct EnabledCodexAssets {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub config: Value,
    pub assets: ExtensionCodexAssets,
}

/// 展开 `${extensionDir}`、`${config.键}`。
fn expand(text: &str, root: &Path, config: &Value) -> String {
    let mut out = text.replace("${extensionDir}", &root.to_string_lossy());
    while let Some(start) = out.find("${config.") {
        let Some(end) = out[start..].find('}').map(|end| start + end) else {
            break;
        };
        let key = &out[start + "${config.".len()..end];
        let replacement = match config.get(key) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        out.replace_range(start..=end, &replacement);
    }
    out
}

fn expand_value(value: &Value, root: &Path, config: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(expand(text, root, config)),
        Value::Array(items) => Value::Array(items.iter().map(|item| expand_value(item, root, config)).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(key, item)| (key.clone(), expand_value(item, root, config))).collect()),
        other => other.clone(),
    }
}

fn json_to_toml(value: &Value) -> Option<toml_edit::Value> {
    Some(match value {
        Value::String(text) => text.as_str().into(),
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => match number.as_i64() {
            Some(int) => int.into(),
            None => number.as_f64()?.into(),
        },
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(json_to_toml(item)?);
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(map) => {
            let mut table = toml_edit::InlineTable::new();
            for (key, item) in map {
                table.insert(key, json_to_toml(item)?);
            }
            toml_edit::Value::InlineTable(table)
        }
        Value::Null => return None,
    })
}

fn snake_case(key: &str) -> String {
    let mut out = String::new();
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 同步状态：记下自己写过的配置项名，下次只增删这些。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SyncState {
    #[serde(default)]
    mcp_servers: Vec<String>,
    #[serde(default)]
    agents: Vec<String>,
}

fn sync_dir(manager: &UserScriptManager) -> PathBuf {
    manager.tool_exec_dir().with_file_name(".codex-sync")
}

/// 把 config.toml 文本里 Codex++ 拥有的 agents / mcp_servers 条目调整成 `wanted`。
/// 返回新文本与新的归属列表；同名的用户条目不动，并记进 `conflicts`。
fn sync_config_text(
    contents: &str,
    state: &SyncState,
    agents: &BTreeMap<String, Table>,
    servers: &BTreeMap<String, Table>,
    conflicts: &mut Vec<String>,
) -> anyhow::Result<(String, SyncState)> {
    let mut document = contents.parse::<DocumentMut>().map_err(|_| anyhow::anyhow!("Codex 配置不是有效 TOML，未修改"))?;
    let mut next = SyncState::default();
    for (section, owned, wanted, out) in [
        ("agents", &state.agents, agents, &mut next.agents),
        ("mcp_servers", &state.mcp_servers, servers, &mut next.mcp_servers),
    ] {
        if document.get(section).is_none() {
            if wanted.is_empty() {
                continue;
            }
            let mut table = Table::new();
            table.set_implicit(true);
            document.insert(section, Item::Table(table));
        }
        let table = document
            .get_mut(section)
            .and_then(Item::as_table_mut)
            .with_context(|| format!("{section} 必须是 TOML 表，未修改配置"))?;
        for name in owned {
            if !wanted.contains_key(name) {
                table.remove(name);
            }
        }
        for (name, entry) in wanted {
            let ours = owned.contains(name);
            if table.contains_key(name) && !ours {
                conflicts.push(format!("{section}.{name}"));
                continue;
            }
            // 已有且内容一致就不动，保留原有格式。
            let same = table.get(name).and_then(Item::as_table).is_some_and(|existing| existing.to_string() == entry.to_string());
            if !same {
                table.insert(name, Item::Table(entry.clone()));
            }
            out.push(name.clone());
        }
    }
    Ok((document.to_string(), next))
}

/// AGENTS.md 里替换（或去掉）Codex++ 托管的一段。
pub fn sync_instructions_text(contents: &str, block: Option<&str>) -> String {
    let mut text = contents.to_string();
    if let (Some(start), Some(end)) = (text.find(INSTRUCTIONS_START), text.find(INSTRUCTIONS_END)) {
        if end > start {
            let mut end = end + INSTRUCTIONS_END.len();
            if text[end..].starts_with('\n') {
                end += 1;
            }
            text.replace_range(start..end, "");
        }
    }
    let Some(block) = block else {
        return text.trim_end().to_string() + if text.trim().is_empty() { "" } else { "\n" };
    };
    let mut out = text.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(INSTRUCTIONS_START);
    out.push('\n');
    out.push_str(block.trim());
    out.push('\n');
    out.push_str(INSTRUCTIONS_END);
    out.push('\n');
    out
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

/// 复制目录（跳过符号链接）。
fn copy_dir(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)?.flatten() {
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)?;
        let target = to.join(entry.file_name());
        if meta.is_dir() {
            copy_dir(&path, &target)?;
        } else if meta.is_file() {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

/// 目录内容的指纹：相对路径 + 大小 + 修改时间。
fn dir_fingerprint(path: &Path) -> String {
    let mut parts = Vec::new();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                let modified = meta.modified().ok().and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map(|time| time.as_nanos()).unwrap_or_default();
                parts.push(format!("{}|{}|{modified}", entry.path().strip_prefix(path).unwrap_or(&entry.path()).display(), meta.len()));
            }
        }
    }
    parts.sort();
    parts.join("\n")
}

fn sync_skills(home: &Path, wanted: &[(String, PathBuf)], report: &mut Map<String, Value>) -> anyhow::Result<()> {
    let skills_dir = home.join("skills");
    let mut installed = Vec::new();
    let mut errors = Vec::new();
    // 去掉不再需要的（只认带标记文件的目录）。
    for entry in std::fs::read_dir(&skills_dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if !path.join(SKILL_MARKER).is_file() || wanted.iter().any(|(wanted, _)| *wanted == name) {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            errors.push(format!("{name}：{error}"));
        }
    }
    for (name, source) in wanted {
        let target = skills_dir.join(name);
        let marker = target.join(SKILL_MARKER);
        if target.exists() && !marker.is_file() {
            errors.push(format!("{name}：~/.codex/skills 里已有同名的技能，没有覆盖"));
            continue;
        }
        if dir_size(source) > SKILL_MAX_BYTES {
            errors.push(format!("{name}：技能目录超过 4 MiB"));
            continue;
        }
        let fingerprint = dir_fingerprint(source);
        if std::fs::read_to_string(&marker).ok().is_some_and(|text| text.lines().nth(1) == Some(&fingerprint.replace('\n', "\u{1f}"))) {
            installed.push(name.clone());
            continue;
        }
        if target.exists() {
            std::fs::remove_dir_all(&target).with_context(|| format!("无法更新技能 {name}"))?;
        }
        copy_dir(source, &target).with_context(|| format!("无法安装技能 {name}"))?;
        std::fs::write(&marker, format!("{}\n{}\n", source.display(), fingerprint.replace('\n', "\u{1f}")))?;
        installed.push(name.clone());
    }
    report.insert("skills".into(), json!(installed));
    if !errors.is_empty() {
        report.insert("skillErrors".into(), json!(errors));
    }
    Ok(())
}

/// 把启用中拓展的技能、子 agent、MCP 服务器、全局指令同步进 Codex home。
pub fn sync(home: &Path, manager: &UserScriptManager, enabled: &[EnabledCodexAssets]) -> anyhow::Result<Value> {
    let mut report = Map::new();
    let state_dir = sync_dir(manager);
    let state_path = state_dir.join("state.json");
    let state: SyncState = std::fs::read_to_string(&state_path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();

    // 技能。
    let skills = enabled
        .iter()
        .flat_map(|item| item.assets.skills.iter().map(move |dir| (format!("{}-{}", item.id, dir.file_name().unwrap_or_default().to_string_lossy()), dir.clone())))
        .collect::<Vec<_>>();
    sync_skills(home, &skills, &mut report)?;

    // 子 agent：角色配置文件写在 Codex++ 数据目录，config.toml 里只放描述和路径。
    let agents_dir = state_dir.join("agents");
    let mut agents = BTreeMap::new();
    for item in enabled {
        for agent in &item.assets.agents {
            let name = format!("{}-{}", item.id, agent.name);
            let mut role = DocumentMut::new();
            if let Some(model) = &agent.model {
                role["model"] = value(model.as_str());
            }
            if let Some(effort) = &agent.reasoning_effort {
                role["model_reasoning_effort"] = value(effort.as_str());
            }
            if let Some(instructions) = &agent.instructions {
                role["developer_instructions"] = value(expand(instructions, &item.root, &item.config));
            }
            std::fs::create_dir_all(&agents_dir)?;
            let file = agents_dir.join(format!("{name}.toml"));
            let text = role.to_string();
            if std::fs::read_to_string(&file).ok().as_deref() != Some(text.as_str()) {
                crate::settings::atomic_write(&file, text.as_bytes())?;
            }
            let mut entry = Table::new();
            entry["description"] = value(agent.description.as_str());
            entry["config_file"] = value(file.to_string_lossy().as_ref());
            if !agent.nicknames.is_empty() {
                let mut list = toml_edit::Array::new();
                for nickname in &agent.nicknames {
                    list.push(nickname.as_str());
                }
                entry["nickname_candidates"] = value(list);
            }
            agents.insert(name, entry);
        }
    }
    // 不再需要的角色文件顺手删掉（只删自己目录里的单个文件）。
    for entry in std::fs::read_dir(&agents_dir).into_iter().flatten().flatten() {
        let stem = entry.path().file_stem().map(|stem| stem.to_string_lossy().to_string()).unwrap_or_default();
        if !agents.contains_key(&stem) {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    // MCP 服务器。
    let mut servers = BTreeMap::new();
    for item in enabled {
        for server in &item.assets.mcp_servers {
            let spec = expand_value(&Value::Object(server.spec.clone()), &item.root, &item.config);
            let mut entry = Table::new();
            for (key, field) in spec.as_object().into_iter().flatten() {
                if let Some(converted) = json_to_toml(field) {
                    entry[snake_case(key).as_str()] = value(converted);
                }
            }
            servers.insert(format!("{}-{}", item.id, server.name), entry);
        }
    }
    let path = home.join("config.toml");
    let before = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("无法读取 Codex 配置"),
    };
    let mut conflicts = Vec::new();
    let (updated, next_state) = sync_config_text(&before, &state, &agents, &servers, &mut conflicts)?;
    if updated != before {
        if std::fs::read_to_string(&path).unwrap_or_default() != before {
            anyhow::bail!("Codex 配置在同步期间被修改，未覆盖");
        }
        crate::settings::atomic_write(&path, updated.as_bytes()).context("无法写入 Codex 配置")?;
    }
    std::fs::create_dir_all(&state_dir)?;
    crate::settings::atomic_write(&state_path, serde_json::to_string_pretty(&next_state)?.as_bytes())?;
    report.insert("agents".into(), json!(next_state.agents));
    report.insert("mcpServers".into(), json!(next_state.mcp_servers));
    if !conflicts.is_empty() {
        report.insert("conflicts".into(), json!(conflicts));
    }

    // 全局指令：AGENTS.md 托管段。
    let blocks = enabled
        .iter()
        .filter_map(|item| item.assets.instructions.as_ref().map(|text| format!("## {}\n\n{}", item.name, expand(text, &item.root, &item.config).trim())))
        .collect::<Vec<_>>();
    let agents_md = home.join("AGENTS.md");
    let before = std::fs::read_to_string(&agents_md).unwrap_or_default();
    let block = (!blocks.is_empty()).then(|| blocks.join("\n\n"));
    if block.is_some() || before.contains(INSTRUCTIONS_START) {
        let after = sync_instructions_text(&before, block.as_deref());
        if after != before {
            if after.trim().is_empty() && !agents_md.exists() {
                // 没有原文件、也没有要写的：什么都不做。
            } else {
                crate::settings::atomic_write(&agents_md, after.as_bytes()).context("无法写入 AGENTS.md")?;
            }
        }
    }
    report.insert("instructions".into(), json!(blocks.len()));
    Ok(Value::Object(report))
}

/// 拓展资源的 MCP 地址：`codexpp://<拓展 id>/<包内路径>`。
pub fn resource_uri(extension: &str, root: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
    format!("codexpp://{extension}/{relative}")
}

/// 把模板里的 `{{参数名}}` 换成值；没给的换成空串。
pub fn render_prompt(template: &str, arguments: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start + 2..].find("}}").map(|end| start + 2 + end) else {
            break;
        };
        out.push_str(&rest[..start]);
        let key = rest[start + 2..end].trim();
        match arguments.get(key) {
            Some(Value::String(text)) => out.push_str(text),
            Some(Value::Null) | None => {}
            Some(other) => out.push_str(&other.to_string()),
        }
        rest = &rest[end + 2..];
    }
    out.push_str(rest);
    out
}
