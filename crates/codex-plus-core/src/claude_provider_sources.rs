//! Claude Desktop 供应商的外部来源：CC Switch 里的 Claude / Claude Desktop 供应商、ClaudeMom 的上游。
//!
//! 列出来给网关的「导入」用；导入时复制成网关供应商（之后两边互不影响），只读，不改原数据。
//! CC Switch 的规则照 ClaudeMom（`apps/claude-mom/src-tauri/src/upstreams.rs`）：Claude Desktop 的条目 id 加
//! `claude-desktop:` 前缀免得和 Claude Code 的重名；`meta.apiFormat` 是 `anthropic`（默认）或 `openai_chat`
//! 的才收——网关能把 Anthropic 请求转成 Chat Completions，所以后者也能用；别的格式跳过。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::settings::{RelayProfile, RelayProtocol};
use crate::tools::claude::ClaudeProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEntry {
    pub id: String,
    pub name: String,
}

// ---------------------------------------------------------------------------
// CC Switch
// ---------------------------------------------------------------------------

struct CcSwitchRow {
    id: String,
    name: String,
    settings: Value,
    protocol: RelayProtocol,
}

fn cc_switch_rows(db: &Path) -> anyhow::Result<Vec<CcSwitchRow>> {
    if !db.is_file() {
        bail!("没有找到 CC Switch 的数据库：{}", db.display());
    }
    let connection = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("打开 CC Switch 数据库失败：{}", db.display()))?;
    let mut statement = connection.prepare(
        "SELECT id, app_type, name, settings_config, COALESCE(meta, '{}') FROM providers \
         WHERE app_type IN ('claude', 'claude-desktop') \
         ORDER BY app_type = 'claude-desktop', COALESCE(sort_index, 0), created_at",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (id, app_type, name, settings, meta) = row?;
        let format = serde_json::from_str::<Value>(&meta)
            .ok()
            .and_then(|meta| meta.get("apiFormat").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| "anthropic".into());
        let protocol = match format.as_str() {
            "anthropic" => RelayProtocol::AnthropicMessages,
            "openai_chat" => RelayProtocol::ChatCompletions,
            _ => continue,
        };
        // 单行 JSON 坏掉不影响其它行。
        let Ok(settings) = serde_json::from_str::<Value>(&settings) else { continue };
        if settings.get("env").and_then(Value::as_object).is_none() {
            continue;
        }
        let id = if app_type == "claude" { id } else { format!("{app_type}:{id}") };
        result.push(CcSwitchRow { id, name, settings, protocol });
    }
    Ok(result)
}

/// CC Switch 里能导入的 Claude 供应商；没装 CC Switch 或读不了时为空。
pub fn cc_switch_entries() -> Vec<SourceEntry> {
    cc_switch_entries_at(&crate::ccs_import::default_ccs_db_path())
}

fn cc_switch_entries_at(db: &Path) -> Vec<SourceEntry> {
    cc_switch_rows(db)
        .map(|rows| rows.into_iter().map(|row| SourceEntry { id: row.id, name: row.name }).collect())
        .unwrap_or_default()
}

pub fn cc_switch_relay(id: &str) -> anyhow::Result<RelayProfile> {
    cc_switch_relay_at(&crate::ccs_import::default_ccs_db_path(), id)
}

fn cc_switch_relay_at(db: &Path, id: &str) -> anyhow::Result<RelayProfile> {
    let row = cc_switch_rows(db)?
        .into_iter()
        .find(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("CC Switch 里没有这个 Claude 供应商：{id}"))?;
    let mut profile = crate::gateway::apply::copy_claude_provider(&provider(&row.name, row.settings))?;
    if row.protocol != RelayProtocol::AnthropicMessages {
        profile.protocol = row.protocol;
        profile.anthropic_auth_header = String::new();
    }
    Ok(profile)
}

fn provider(name: &str, settings: Value) -> ClaudeProvider {
    ClaudeProvider {
        id: String::new(),
        name: name.to_string(),
        website_url: String::new(),
        notes: String::new(),
        settings_config: settings,
        created_at: 0,
        ccs_source_id: String::new(),
    }
}

// ---------------------------------------------------------------------------
// ClaudeMom
// ---------------------------------------------------------------------------

/// ClaudeMom 的 `upstreams.json`（只读用到的字段）。
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct UpstreamProfile {
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    model: String,
    small_model: String,
    models: Vec<String>,
    context_window: Option<u64>,
    source: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct UpstreamBook {
    profiles: Vec<UpstreamProfile>,
}

/// ClaudeMom 的状态目录：`CLAUDE_MOM_STATE_DIR` 覆盖，否则 `~/.claude-mom`（与 ClaudeMom 的 host.rs 一致）。
fn claude_mom_book() -> PathBuf {
    let dir = std::env::var_os("CLAUDE_MOM_STATE_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| crate::computer_use::home_dir().join(".claude-mom"));
    dir.join("upstreams.json")
}

/// 能导入的上游：从 Codex++ 本地服务导过去的（指回我们自己）不算。
fn claude_mom_profiles(path: &Path) -> Vec<UpstreamProfile> {
    std::fs::read(path)
        .ok()
        .and_then(|raw| serde_json::from_slice::<UpstreamBook>(&raw).ok())
        .map(|book| {
            book.profiles
                .into_iter()
                .filter(|profile| !profile.id.is_empty() && !profile.base_url.trim().is_empty() && profile.source != "codexpp")
                .collect()
        })
        .unwrap_or_default()
}

pub fn claude_mom_entries() -> Vec<SourceEntry> {
    claude_mom_entries_at(&claude_mom_book())
}

fn claude_mom_entries_at(path: &Path) -> Vec<SourceEntry> {
    claude_mom_profiles(path)
        .into_iter()
        .map(|profile| SourceEntry {
            name: if profile.name.trim().is_empty() { profile.base_url.clone() } else { profile.name },
            id: profile.id,
        })
        .collect()
}

pub fn claude_mom_relay(id: &str) -> anyhow::Result<RelayProfile> {
    claude_mom_relay_at(&claude_mom_book(), id)
}

/// ClaudeMom 的上游都是 Anthropic 格式直连（ClaudeMom 不做协议转换），按 Claude 供应商复制：
/// 默认模型、快速模型和模型列表都带上，统一窗口当这家的默认窗口。
fn claude_mom_relay_at(path: &Path, id: &str) -> anyhow::Result<RelayProfile> {
    let profile = claude_mom_profiles(path)
        .into_iter()
        .find(|profile| profile.id == id)
        .ok_or_else(|| anyhow::anyhow!("ClaudeMom 里没有这个上游：{id}"))?;
    let name = if profile.name.trim().is_empty() { profile.base_url.clone() } else { profile.name.clone() };
    let mut env = serde_json::Map::new();
    env.insert("ANTHROPIC_BASE_URL".into(), json!(profile.base_url.trim()));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), json!(profile.api_key.trim()));
    if !profile.model.trim().is_empty() {
        env.insert("ANTHROPIC_MODEL".into(), json!(profile.model.trim()));
    }
    if !profile.small_model.trim().is_empty() {
        env.insert("ANTHROPIC_DEFAULT_HAIKU_MODEL".into(), json!(profile.small_model.trim()));
    }
    if let Some(window) = profile.context_window.filter(|window| *window > 0) {
        env.insert("CLAUDE_CODE_MAX_CONTEXT_TOKENS".into(), json!(window.to_string()));
    }
    let mut relay = crate::gateway::apply::copy_claude_provider(&provider(&name, json!({ "env": env })))?;
    // 环境变量里只放得下两个模型，其余的补进模型列表（去重，保持顺序）。
    let mut models: Vec<String> = relay.model_list.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect();
    for model in &profile.models {
        let model = model.trim();
        if !model.is_empty() && !models.iter().any(|known| known == model) {
            models.push(model.to_string());
        }
    }
    relay.model_list = models.join("\n");
    if relay.model.is_empty() {
        relay.model = models.first().cloned().unwrap_or_default();
    }
    Ok(relay)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cc_switch_db(dir: &Path) -> PathBuf {
        let path = dir.join("cc-switch.db");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE providers (id TEXT, app_type TEXT, name TEXT, settings_config TEXT, meta TEXT, sort_index INTEGER, created_at INTEGER);
                 INSERT INTO providers VALUES ('k', 'claude', 'Kimi', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://api.moonshot.cn/anthropic\",\"ANTHROPIC_AUTH_TOKEN\":\"sk-k\",\"ANTHROPIC_MODEL\":\"kimi-k2\"}}', NULL, 0, 1);
                 INSERT INTO providers VALUES ('k', 'claude-desktop', 'Kimi 桌面', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://api.moonshot.cn/anthropic\",\"ANTHROPIC_API_KEY\":\"sk-d\"}}', '{\"apiFormat\":\"anthropic\"}', 0, 2);
                 INSERT INTO providers VALUES ('o', 'claude', 'OpenRouter', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://openrouter.ai/api/v1\",\"ANTHROPIC_AUTH_TOKEN\":\"sk-o\",\"ANTHROPIC_MODEL\":\"x/y\"}}', '{\"apiFormat\":\"openai_chat\"}', 0, 3);
                 INSERT INTO providers VALUES ('g', 'claude', 'Gemini', '{\"env\":{\"ANTHROPIC_BASE_URL\":\"https://g\"}}', '{\"apiFormat\":\"gemini\"}', 0, 4);
                 INSERT INTO providers VALUES ('c', 'codex', 'Codex 的', '{}', NULL, 0, 5);
                 INSERT INTO providers VALUES ('b', 'claude', '坏的', 'not json', NULL, 0, 6);",
            )
            .unwrap();
        path
    }

    #[test]
    fn cc_switch_lists_claude_and_desktop_rows_in_supported_formats() {
        let dir = tempfile::tempdir().unwrap();
        let db = cc_switch_db(dir.path());
        let entries = cc_switch_entries_at(&db);
        assert_eq!(
            entries.iter().map(|entry| (entry.id.as_str(), entry.name.as_str())).collect::<Vec<_>>(),
            [("k", "Kimi"), ("o", "OpenRouter"), ("claude-desktop:k", "Kimi 桌面")],
        );
        assert!(cc_switch_entries_at(&dir.path().join("missing.db")).is_empty());

        let kimi = cc_switch_relay_at(&db, "k").unwrap();
        assert_eq!(kimi.protocol, RelayProtocol::AnthropicMessages);
        assert_eq!(kimi.model, "kimi-k2");
        let openrouter = cc_switch_relay_at(&db, "o").unwrap();
        assert_eq!(openrouter.protocol, RelayProtocol::ChatCompletions, "OpenAI 格式交给网关转换");
        assert!(openrouter.anthropic_auth_header.is_empty());
        assert!(cc_switch_relay_at(&db, "claude-desktop:k").is_ok());
        assert!(cc_switch_relay_at(&db, "g").is_err());
    }

    #[test]
    fn claude_mom_profiles_copy_models_and_skip_ones_pointing_back_at_us() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upstreams.json");
        std::fs::write(&path, serde_json::to_vec(&json!({
            "activeId": "a",
            "profiles": [
                { "id": "a", "name": "DeepSeek", "baseUrl": "https://api.deepseek.com/anthropic", "apiKey": "sk-a",
                  "model": "deepseek-chat", "smallModel": "deepseek-lite", "models": ["deepseek-chat", "deepseek-reasoner"],
                  "contextWindow": 128000, "source": "manual" },
                { "id": "gw", "name": "Codex++", "baseUrl": "http://127.0.0.1:1/gw/claude-desktop", "apiKey": "t", "source": "codexpp" },
                { "id": "", "name": "没 id", "baseUrl": "https://x" }
            ]
        })).unwrap()).unwrap();
        assert_eq!(claude_mom_entries_at(&path), [SourceEntry { id: "a".into(), name: "DeepSeek".into() }]);
        let relay = claude_mom_relay_at(&path, "a").unwrap();
        assert_eq!(relay.protocol, RelayProtocol::AnthropicMessages);
        assert_eq!(relay.model, "deepseek-chat");
        assert_eq!(relay.model_list.lines().collect::<Vec<_>>(), ["deepseek-chat", "deepseek-lite", "deepseek-reasoner"]);
        assert!(claude_mom_relay_at(&path, "gw").is_err());
        assert!(claude_mom_entries_at(&dir.path().join("none.json")).is_empty());
    }
}
