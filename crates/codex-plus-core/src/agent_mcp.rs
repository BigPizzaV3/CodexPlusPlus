//! Claude Code 与 Grok 的 MCP 服务器读写，供「全局管理 · MCP」同时管理多个 Agent。
//!
//! Codex 的 MCP 仍走 Codex++ 的公共配置（context entries），这里只管另外两家。
//! 对前端统一用 Codex 规范的 `[mcp_servers.<id>]` 表体（TOML 文本）交换，表单与解析逻辑
//! 全部复用 [`crate::mcp_config`]；格式差异在这里换算：
//!
//! | | Codex 规范（交换格式） | Claude Code `~/.claude.json` | Grok `~/.grok/config.toml` |
//! |---|---|---|---|
//! | 位置 | — | 顶层 `mcpServers` 对象 | `[mcp_servers.<id>]` 表 |
//! | HTTP 头 | `http_headers` | `headers` | `headers` |
//! | Bearer | `bearer_token` | 写成 `Authorization` 头 | 写成 `Authorization` 头 |
//! | 停用 | `enabled = false` | 无此概念：停用即移除 | `enabled = false` |
//!
//! 写入只改动目标那一条，其余内容（包括同一条里本工具不认识的键）原样保留；
//! 写前备份，写时若文件已被其他程序改过就拒绝覆盖（复用 Computer Use 的写入路径）。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Serialize;
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, InlineTable, Item, Table};

use crate::computer_use::{claude_code_config_path, parse_json_object, read_text, write_with_backup};
use crate::mcp_config::{McpKeyValue, McpServerForm, McpTransport, build_mcp_toml_body, parse_mcp_toml_body};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpAgent {
    Claude,
    Grok,
}

impl McpAgent {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "claude" => Ok(Self::Claude),
            "grok" => Ok(Self::Grok),
            other => bail!("不支持的 Agent：{other}"),
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Grok => "grok",
        }
    }

    /// 用户级 MCP 配置文件的默认位置。
    pub fn config_path(self) -> PathBuf {
        match self {
            Self::Claude => claude_code_config_path(),
            Self::Grok => crate::grok_config::default_grok_home_dir().join("config.toml"),
        }
    }
}

/// 一条 MCP 服务器：`toml_body` 是 Codex 规范的表体，可直接交给 `parse_mcp_entry` 解析成表单。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpEntry {
    pub id: String,
    pub toml_body: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMcpConfig {
    pub agent: String,
    pub config_path: String,
    pub exists: bool,
    pub entries: Vec<AgentMcpEntry>,
}

pub fn load(agent: McpAgent) -> anyhow::Result<AgentMcpConfig> {
    load_at(agent, &agent.config_path())
}

pub fn load_at(agent: McpAgent, path: &Path) -> anyhow::Result<AgentMcpConfig> {
    let text = read_text(path)?;
    let entries = match (&text, agent) {
        (None, _) => Vec::new(),
        (Some(text), McpAgent::Claude) => read_claude(text, path)?,
        (Some(text), McpAgent::Grok) => read_grok(text, path)?,
    };
    Ok(AgentMcpConfig {
        agent: agent.id().to_string(),
        config_path: path.to_string_lossy().into_owned(),
        exists: text.is_some(),
        entries,
    })
}

/// 写入或移除一条。`toml_body` 为 `None` 表示移除；返回备份文件位置（没有改动或新建文件时为空）。
pub fn save(agent: McpAgent, id: &str, toml_body: Option<&str>) -> anyhow::Result<Option<PathBuf>> {
    save_at(agent, &agent.config_path(), id, toml_body)
}

pub fn save_at(agent: McpAgent, path: &Path, id: &str, toml_body: Option<&str>) -> anyhow::Result<Option<PathBuf>> {
    validate_id(id)?;
    let form = toml_body.map(parse_mcp_toml_body).transpose()?;
    let before = read_text(path)?;
    let rendered = match agent {
        McpAgent::Claude => render_claude(before.as_deref(), path, id, form.as_ref())?,
        McpAgent::Grok => render_grok(before.as_deref(), path, id, form.as_ref())?,
    };
    let Some(rendered) = rendered else {
        return Ok(None);
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("创建 {} 失败", parent.display()))?;
    }
    write_with_backup(path, before.as_deref(), &rendered)
}

fn validate_id(id: &str) -> anyhow::Result<()> {
    let id = id.trim();
    if id.is_empty() {
        bail!("服务器名称不能为空");
    }
    if id.chars().any(|ch| ch.is_control() || ch == '"' || ch == '\\') {
        bail!("服务器名称不能包含引号、反斜杠或控制字符：{id}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Claude Code：~/.claude.json 顶层 mcpServers
// ---------------------------------------------------------------------------

fn read_claude(text: &str, path: &Path) -> anyhow::Result<Vec<AgentMcpEntry>> {
    let root = parse_json_object(text, path)?;
    let Some(servers) = root.get("mcpServers").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (id, value) in servers {
        let Some(object) = value.as_object() else { continue };
        let form = claude_to_form(object);
        entries.push(AgentMcpEntry { id: id.clone(), toml_body: build_mcp_toml_body(&form)?, enabled: true });
    }
    Ok(entries)
}

fn claude_to_form(object: &Map<String, Value>) -> McpServerForm {
    let text = |key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let pairs = |key: &str| -> Vec<McpKeyValue> {
        object
            .get(key)
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(key, value)| McpKeyValue {
                        key: key.clone(),
                        value: value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let url = text("url");
    let kind = text("type");
    let http = !url.is_empty() || kind == "http" || kind == "sse";
    McpServerForm {
        transport: if http { McpTransport::Http } else { McpTransport::Stdio },
        command: text("command"),
        args: object
            .get("args")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        env: pairs("env"),
        url,
        http_headers: pairs("headers"),
        enabled: true,
        ..Default::default()
    }
}

fn render_claude(before: Option<&str>, path: &Path, id: &str, form: Option<&McpServerForm>) -> anyhow::Result<Option<String>> {
    let mut root = parse_json_object(before.unwrap_or("{}"), path)?;
    // Claude Code 没有「停用」：停用等同于移除。
    let form = form.filter(|form| form.enabled);
    let changed = match form {
        None => {
            let servers = root.get_mut("mcpServers").and_then(Value::as_object_mut);
            servers.is_some_and(|servers| servers.remove(id).is_some())
        }
        Some(form) => {
            let servers = root
                .entry("mcpServers".to_string())
                .or_insert_with(|| Value::Object(Map::new()))
                .as_object_mut()
                .with_context(|| format!("{} 的 mcpServers 不是对象，未修改", path.display()))?;
            let mut entry = servers.get(id).and_then(Value::as_object).cloned().unwrap_or_default();
            apply_claude_form(&mut entry, form);
            let next = Value::Object(entry);
            if servers.get(id) == Some(&next) {
                false
            } else {
                servers.insert(id.to_string(), next);
                true
            }
        }
    };
    Ok(changed.then(|| format!("{}\n", serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default())))
}

/// 在已有条目上改写本工具管理的键；其余键（例如 oauth 之类）原样保留。
fn apply_claude_form(entry: &mut Map<String, Value>, form: &McpServerForm) {
    for key in ["command", "args", "env", "url", "headers"] {
        entry.remove(key);
    }
    match form.transport {
        McpTransport::Stdio => {
            entry.insert("type".into(), Value::String("stdio".into()));
            entry.insert("command".into(), Value::String(form.command.clone()));
            if !form.args.is_empty() {
                entry.insert("args".into(), Value::Array(form.args.iter().cloned().map(Value::String).collect()));
            }
            if !form.env.is_empty() {
                entry.insert("env".into(), pairs_to_json(&form.env));
            }
        }
        McpTransport::Http => {
            // 原来就是 sse 的保持不变，其余一律写 http。
            if entry.get("type").and_then(Value::as_str) != Some("sse") {
                entry.insert("type".into(), Value::String("http".into()));
            }
            entry.insert("url".into(), Value::String(form.url.clone()));
            let headers = headers_with_bearer(form);
            if !headers.is_empty() {
                entry.insert("headers".into(), pairs_to_json(&headers));
            }
        }
    }
}

fn pairs_to_json(pairs: &[McpKeyValue]) -> Value {
    Value::Object(pairs.iter().map(|pair| (pair.key.clone(), Value::String(pair.value.clone()))).collect())
}

/// Codex 的 `bearer_token` 在 Claude / Grok 里没有对应字段，换成 Authorization 头（已有同名头时不覆盖）。
fn headers_with_bearer(form: &McpServerForm) -> Vec<McpKeyValue> {
    let mut headers = form.http_headers.clone();
    let token = form.bearer_token.trim();
    if !token.is_empty() && !headers.iter().any(|pair| pair.key.eq_ignore_ascii_case("authorization")) {
        headers.push(McpKeyValue { key: "Authorization".into(), value: format!("Bearer {token}") });
    }
    headers
}

// ---------------------------------------------------------------------------
// Grok：~/.grok/config.toml 的 [mcp_servers.<id>]，与 Codex 同构，HTTP 头叫 headers
// ---------------------------------------------------------------------------

fn read_grok(text: &str, path: &Path) -> anyhow::Result<Vec<AgentMcpEntry>> {
    let doc: DocumentMut = text.parse().with_context(|| format!("{} 不是合法 TOML", path.display()))?;
    let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table_like) else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for (id, item) in servers.iter() {
        let Some(table) = item.as_table_like() else { continue };
        let mut body = DocumentMut::new();
        for (key, value) in table.iter() {
            let key = if key == "headers" { "http_headers" } else { key };
            body.as_table_mut().insert(key, value.clone());
        }
        let enabled = table.get("enabled").and_then(Item::as_bool).unwrap_or(true);
        entries.push(AgentMcpEntry { id: id.to_string(), toml_body: body.to_string(), enabled });
    }
    Ok(entries)
}

fn render_grok(before: Option<&str>, path: &Path, id: &str, form: Option<&McpServerForm>) -> anyhow::Result<Option<String>> {
    let mut doc: DocumentMut = before
        .unwrap_or("")
        .parse()
        .with_context(|| format!("{} 不是合法 TOML，未修改", path.display()))?;
    let changed = match form {
        None => doc
            .get_mut("mcp_servers")
            .and_then(Item::as_table_like_mut)
            .is_some_and(|servers| servers.remove(id).is_some()),
        Some(form) => {
            let next = grok_table(form)?;
            if !doc.contains_key("mcp_servers") {
                let mut servers = Table::new();
                servers.set_implicit(true);
                doc.insert("mcp_servers", Item::Table(servers));
            }
            let servers = doc["mcp_servers"]
                .as_table_mut()
                .with_context(|| format!("{} 的 mcp_servers 不是表，未修改", path.display()))?;
            let previous = servers.get(id).map(|item| item.to_string());
            // 保留旧表里本工具不认识的键（例如 tool_timeouts），再用表单覆盖认识的键。
            let mut merged = servers.get(id).and_then(Item::as_table).cloned().unwrap_or_default();
            for key in ["command", "args", "env", "cwd", "url", "headers", "startup_timeout_sec", "enabled"] {
                merged.remove(key);
            }
            for (key, value) in next.iter() {
                merged.insert(key, value.clone());
            }
            merged.set_implicit(false);
            let changed = previous.as_deref() != Some(&Item::Table(merged.clone()).to_string());
            servers.insert(id, Item::Table(merged));
            changed
        }
    };
    Ok(changed.then(|| doc.to_string()))
}

fn grok_table(form: &McpServerForm) -> anyhow::Result<Table> {
    let mut codex_form = form.clone();
    // Grok 没有 bearer_token / http_headers：合成 headers 后单独写。
    let headers = headers_with_bearer(form);
    codex_form.bearer_token.clear();
    codex_form.http_headers.clear();
    let body = build_mcp_toml_body(&codex_form)?;
    let parsed: DocumentMut = body.parse().context("MCP 配置不是合法的 TOML")?;
    let mut table = Table::new();
    for (key, value) in parsed.as_table().iter() {
        table.insert(key, value.clone());
    }
    if matches!(form.transport, McpTransport::Http) && !headers.is_empty() {
        let mut inline = InlineTable::new();
        for pair in &headers {
            inline.insert(&pair.key, pair.value.as_str().into());
        }
        table.insert("headers", Item::Value(toml_edit::Value::InlineTable(inline)));
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STDIO: &str = "command = \"npx\"\nargs = [\"-y\", \"@modelcontextprotocol/server-github\"]\nenv = { GITHUB_TOKEN = \"t\" }\n";
    const HTTP: &str = "url = \"https://mcp.example.com/mcp\"\nbearer_token = \"abc\"\nhttp_headers = { X-Team = \"core\" }\n";

    #[test]
    fn claude_write_read_round_trip_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        std::fs::write(&path, r#"{"numStartups": 3, "mcpServers": {"keep": {"type": "stdio", "command": "keep-me"}}}"#).unwrap();

        let backup = save_at(McpAgent::Claude, &path, "github", Some(STDIO)).unwrap();
        assert!(backup.is_some(), "改动已有文件前要备份");
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["numStartups"], 3, "其它顶层键保留");
        assert_eq!(root["mcpServers"]["keep"]["command"], "keep-me", "其它服务器保留");
        assert_eq!(root["mcpServers"]["github"]["type"], "stdio");
        assert_eq!(root["mcpServers"]["github"]["args"][1], "@modelcontextprotocol/server-github");
        assert_eq!(root["mcpServers"]["github"]["env"]["GITHUB_TOKEN"], "t");

        let config = load_at(McpAgent::Claude, &path).unwrap();
        let github = config.entries.iter().find(|entry| entry.id == "github").unwrap();
        let form = parse_mcp_toml_body(&github.toml_body).unwrap();
        assert_eq!(form.command, "npx");
        assert_eq!(form.env[0].key, "GITHUB_TOKEN");
    }

    #[test]
    fn claude_http_bearer_becomes_authorization_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        save_at(McpAgent::Claude, &path, "remote", Some(HTTP)).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let entry = &root["mcpServers"]["remote"];
        assert_eq!(entry["type"], "http");
        assert_eq!(entry["headers"]["Authorization"], "Bearer abc");
        assert_eq!(entry["headers"]["X-Team"], "core");
        // 读回来时 headers 落在 http_headers
        let config = load_at(McpAgent::Claude, &path).unwrap();
        let form = parse_mcp_toml_body(&config.entries[0].toml_body).unwrap();
        assert!(form.http_headers.iter().any(|pair| pair.key == "Authorization"));
    }

    #[test]
    fn claude_disabled_or_none_removes_entry_and_unchanged_write_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude.json");
        save_at(McpAgent::Claude, &path, "github", Some(STDIO)).unwrap();
        assert!(save_at(McpAgent::Claude, &path, "github", Some(STDIO)).unwrap().is_none(), "内容没变就不写");
        save_at(McpAgent::Claude, &path, "github", Some(&format!("enabled = false\n{STDIO}"))).unwrap();
        assert!(load_at(McpAgent::Claude, &path).unwrap().entries.is_empty(), "停用即移除");
        save_at(McpAgent::Claude, &path, "github", Some(STDIO)).unwrap();
        save_at(McpAgent::Claude, &path, "github", None).unwrap();
        assert!(load_at(McpAgent::Claude, &path).unwrap().entries.is_empty());
    }

    #[test]
    fn grok_uses_headers_and_keeps_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ui]\ntheme = \"dark\"\n\n[mcp_servers.remote]\nurl = \"https://old\"\ntool_timeouts = { slow_op = 120 }\n").unwrap();

        save_at(McpAgent::Grok, &path, "remote", Some(HTTP)).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["ui"]["theme"].as_str(), Some("dark"), "其它表保留");
        let remote = &doc["mcp_servers"]["remote"];
        assert_eq!(remote["url"].as_str(), Some("https://mcp.example.com/mcp"));
        assert_eq!(remote["headers"]["Authorization"].as_str(), Some("Bearer abc"));
        assert!(remote.get("http_headers").is_none() && remote.get("bearer_token").is_none(), "不写 Grok 不认识的键");
        assert_eq!(remote["tool_timeouts"]["slow_op"].as_integer(), Some(120), "保留本工具不认识的键");

        let config = load_at(McpAgent::Grok, &path).unwrap();
        let form = parse_mcp_toml_body(&config.entries[0].toml_body).unwrap();
        assert_eq!(form.transport, McpTransport::Http);
        assert!(form.http_headers.iter().any(|pair| pair.key == "X-Team"));
    }

    #[test]
    fn grok_disabled_is_kept_with_enabled_false_and_none_removes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save_at(McpAgent::Grok, &path, "github", Some(&format!("enabled = false\n{STDIO}"))).unwrap();
        let config = load_at(McpAgent::Grok, &path).unwrap();
        assert_eq!(config.entries.len(), 1);
        assert!(!config.entries[0].enabled);
        save_at(McpAgent::Grok, &path, "github", None).unwrap();
        assert!(load_at(McpAgent::Grok, &path).unwrap().entries.is_empty());
    }

    #[test]
    fn missing_file_reads_empty_and_rejects_bad_names() {
        let dir = tempfile::tempdir().unwrap();
        let config = load_at(McpAgent::Grok, &dir.path().join("none.toml")).unwrap();
        assert!(!config.exists && config.entries.is_empty());
        assert!(save_at(McpAgent::Claude, &dir.path().join("x.json"), "bad\"name", Some(STDIO)).is_err());
    }
}
