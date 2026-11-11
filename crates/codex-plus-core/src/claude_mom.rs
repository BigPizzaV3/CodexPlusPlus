//! ClaudeMom 的上游配置：多组 Base URL + Key，直连给 Claude Desktop 完整功能模式用。
//!
//! 不经 Codex++ 网关：选中的那组通过 [`claude_local_backend::set_direct_upstream`] 交给本地兼容服务，
//! 请求直接发到 `{base}/v1/messages`（与 `ANTHROPIC_BASE_URL` 同一语义）。
//!
//! 三种来源，导入后都只是普通的一组 Base URL + Key：
//! - **cc-switch**：读 `~/.cc-switch/cc-switch.db` 里 `app_type = 'claude'` 的供应商（只收 Anthropic 格式）；
//! - **Magpie**：把 Magpie 的本机网关当上游，地址读 `~/.config/magpie/gateway.json`，
//!   token 用 `magpie-claude-desktop`（Magpie 只校验非本机请求，后缀用来归属用量）；
//! - **Codex++**：把 Codex++ 的本机网关当上游，用它签给 Claude Desktop / Claude Code 的 token。
//!
//! [`claude_local_backend::set_direct_upstream`]: crate::claude_local_backend::set_direct_upstream

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::claude_local_backend::DirectUpstream;

pub const SOURCE_MANUAL: &str = "manual";
pub const SOURCE_CC_SWITCH: &str = "ccswitch";
pub const SOURCE_MAGPIE: &str = "magpie";
pub const SOURCE_CODEX_PLUS_PLUS: &str = "codexpp";

/// Magpie 网关默认端口（`settings.DefaultPort`）。
const MAGPIE_DEFAULT_ADDR: &str = "127.0.0.1:3425";
/// Magpie 按 `magpie-<agent>` 归属用量；本机请求不校验，值只是让客户端有个 token 可带。
const MAGPIE_TOKEN: &str = "magpie-claude-desktop";
const BOOK_FILE: &str = "upstreams.json";

/// 一组上游。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpstreamProfile {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub small_model: String,
    pub models: Vec<String>,
    pub context_window: Option<u64>,
    /// 模型显示名（上游 `/v1/models` 的 `display_name`）。Magpie 的路由组 id 不可读，全靠它。
    pub model_names: BTreeMap<String, String>,
    /// `manual` / `ccswitch` / `magpie` / `codexpp`。
    pub source: String,
    /// 来源里的标识（cc-switch 的供应商 id 等）；再次导入时据此更新而不是重复添加。
    pub source_id: String,
}

impl UpstreamProfile {
    pub fn direct(&self) -> DirectUpstream {
        DirectUpstream {
            base_url: self.base_url.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            small_model: self.small_model.clone(),
            models: self.models.clone(),
            context_window: self.context_window,
            model_names: self.model_names.clone(),
        }
        .normalized()
    }
}

/// 全部上游与当前选中的那组。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UpstreamBook {
    pub profiles: Vec<UpstreamProfile>,
    pub active_id: Option<String>,
}

impl UpstreamBook {
    pub fn active(&self) -> Option<&UpstreamProfile> {
        let id = self.active_id.as_deref()?;
        self.profiles.iter().find(|profile| profile.id == id)
    }

    /// 新增或更新一组：没有 id 的算新增；第一组自动选中。返回它的 id。
    pub fn upsert(&mut self, mut profile: UpstreamProfile) -> String {
        profile.base_url = profile.base_url.trim().trim_end_matches('/').to_string();
        profile.name = profile.name.trim().to_string();
        if profile.name.is_empty() {
            profile.name = host_of(&profile.base_url).unwrap_or_else(|| "未命名".into());
        }
        if profile.source.is_empty() {
            profile.source = SOURCE_MANUAL.into();
        }
        if profile.id.is_empty() {
            profile.id = uuid::Uuid::new_v4().to_string();
        }
        let id = profile.id.clone();
        match self.profiles.iter_mut().find(|known| known.id == id) {
            Some(known) => *known = profile,
            None => self.profiles.push(profile),
        }
        if self.active().is_none() {
            self.active_id = Some(id.clone());
        }
        id
    }

    /// 导入：同一来源同一标识的已有条目只更新地址和 Key，保留用户选过的模型与名字；否则新增。
    /// 返回 `(新增数, 更新数)`。
    pub fn merge_imported(&mut self, imported: Vec<UpstreamProfile>) -> (usize, usize) {
        let (mut added, mut updated) = (0, 0);
        for profile in imported {
            let existing = self.profiles.iter_mut().find(|known| {
                known.source == profile.source && known.source_id == profile.source_id
            });
            match existing {
                Some(known) => {
                    known.base_url = profile.base_url;
                    known.api_key = profile.api_key;
                    if known.model.is_empty() {
                        known.model = profile.model;
                        known.small_model = profile.small_model;
                    }
                    for model in profile.models {
                        if !known.models.contains(&model) {
                            known.models.push(model);
                        }
                    }
                    known.model_names.extend(profile.model_names);
                    updated += 1;
                }
                None => {
                    self.upsert(UpstreamProfile { id: String::new(), ..profile });
                    added += 1;
                }
            }
        }
        (added, updated)
    }

    pub fn remove(&mut self, id: &str) {
        self.profiles.retain(|profile| profile.id != id);
        if self.active().is_none() {
            self.active_id = self.profiles.first().map(|profile| profile.id.clone());
        }
    }
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_string)
}

pub fn default_book_path() -> PathBuf {
    crate::paths::default_app_state_dir().join("claude-mom").join(BOOK_FILE)
}

pub fn load_book(path: &Path) -> Result<UpstreamBook> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("上游配置文件损坏：{}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(UpstreamBook::default()),
        Err(error) => Err(error).with_context(|| format!("读取上游配置失败：{}", path.display())),
    }
}

/// 存盘（含 Key，Unix 上权限收紧到仅本人可读写）。
pub fn save_book(path: &Path, book: &UpstreamBook) -> Result<()> {
    crate::settings::atomic_write(path, &serde_json::to_vec_pretty(book)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 把选中的那组交给 Claude 本地兼容服务。没有选中时也切到直连（空上游），
/// 免得 ClaudeMom 悄悄回落到网关；启动时会提示先配置上游。
pub fn apply_active(book: &UpstreamBook) {
    let upstream = book.active().map(UpstreamProfile::direct).unwrap_or_default();
    crate::claude_local_backend::set_direct_upstream(Some(upstream));
}

// ── cc-switch ──────────────────────────────────────────────────────────

pub fn default_cc_switch_db() -> PathBuf {
    crate::ccs_import::default_ccs_db_path()
}

/// cc-switch 里的 Claude 供应商。OpenAI / Gemini 格式的要经它自己的代理转换，这里不收。
pub fn cc_switch_profiles(db: &Path) -> Result<Vec<UpstreamProfile>> {
    if !db.is_file() {
        bail!("没有找到 cc-switch 的数据库：{}", db.display());
    }
    let connection = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("打开 cc-switch 数据库失败：{}", db.display()))?;
    let mut statement = connection.prepare(
        "SELECT id, name, settings_config, COALESCE(meta, '{}') FROM providers \
         WHERE app_type = 'claude' ORDER BY COALESCE(sort_index, 0), created_at",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        let (id, name, settings, meta) = row?;
        if let Some(profile) = cc_switch_profile(&id, &name, &settings, &meta) {
            profiles.push(profile);
        }
    }
    Ok(profiles)
}

fn cc_switch_profile(id: &str, name: &str, settings: &str, meta: &str) -> Option<UpstreamProfile> {
    let format = serde_json::from_str::<Value>(meta)
        .ok()
        .and_then(|meta| meta.get("apiFormat").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "anthropic".into());
    if format != "anthropic" {
        return None;
    }
    let settings: Value = serde_json::from_str(settings).ok()?;
    let env = settings.get("env")?;
    let text = |key: &str| {
        env.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let base_url = text("ANTHROPIC_BASE_URL")?;
    let api_key = text("ANTHROPIC_AUTH_TOKEN")
        .or_else(|| text("ANTHROPIC_API_KEY"))
        .unwrap_or_default();
    let model = text("ANTHROPIC_MODEL")
        .or_else(|| text("ANTHROPIC_DEFAULT_OPUS_MODEL"))
        .or_else(|| text("ANTHROPIC_DEFAULT_SONNET_MODEL"))
        .unwrap_or_default();
    let small_model = text("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .or_else(|| text("ANTHROPIC_SMALL_FAST_MODEL"))
        .unwrap_or_default();
    let mut models = Vec::new();
    for key in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_SMALL_FAST_MODEL",
    ] {
        if let Some(model) = text(key)
            && !models.contains(&model)
        {
            models.push(model);
        }
    }
    Some(UpstreamProfile {
        name: name.trim().to_string(),
        base_url,
        api_key,
        model,
        small_model,
        models,
        source: SOURCE_CC_SWITCH.into(),
        source_id: id.to_string(),
        ..UpstreamProfile::default()
    })
}

// ── Magpie ─────────────────────────────────────────────────────────────

pub fn default_magpie_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".config").join("magpie"))
        .unwrap_or_else(|| PathBuf::from(".config/magpie"))
}

/// 把 Magpie 的本机网关当上游。发现顺序同 Magpie 自己：`MAGPIE_ADDR`、`gateway.json`、默认端口。
pub fn magpie_profile(dir: &Path, env_addr: Option<&str>) -> UpstreamProfile {
    let recorded = std::fs::read(dir.join("gateway.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("addr").and_then(Value::as_str).map(str::to_string));
    let addr = env_addr
        .map(str::trim)
        .filter(|addr| !addr.is_empty())
        .map(str::to_string)
        .or(recorded)
        .unwrap_or_else(|| MAGPIE_DEFAULT_ADDR.into());
    UpstreamProfile {
        name: "Magpie".into(),
        base_url: format!("http://{}", loopback_addr(&addr)),
        api_key: MAGPIE_TOKEN.into(),
        source: SOURCE_MAGPIE.into(),
        source_id: "gateway".into(),
        ..UpstreamProfile::default()
    }
}

/// 监听在全部网卡（`0.0.0.0` / `[::]` / 只有端口）时，本机连 127.0.0.1。
fn loopback_addr(addr: &str) -> String {
    let addr = addr.trim().trim_start_matches("http://");
    let (host, port) = match addr.rsplit_once(':') {
        Some((host, port)) => (host, port),
        None => return MAGPIE_DEFAULT_ADDR.into(),
    };
    match host {
        "" | "0.0.0.0" | "[::]" | "::" => format!("127.0.0.1:{port}"),
        _ => format!("{host}:{port}"),
    }
}

// ── Codex++ ────────────────────────────────────────────────────────────

/// 把 Codex++ 的本机网关当上游：优先用它给 Claude Desktop 的接入，其次 Claude Code。
/// 两个都没接入时报错，让用户先在 Codex++ 里接入。
pub fn codex_plus_plus_profile(state_dir: &Path) -> Result<UpstreamProfile> {
    let store = crate::gateway::store::GatewayStore::at(state_dir.join("gateway.json"));
    let gateway = store
        .load()
        .with_context(|| format!("读取 Codex++ 网关配置失败：{}", state_dir.display()))?;
    let port = crate::protocol_proxy::protocol_proxy_port_in(state_dir);
    for agent in ["claude-desktop", "claude"] {
        let tool = crate::tools::ToolId::parse(agent);
        if let Some(binding) = gateway.active_binding(&tool)
            && !binding.token.trim().is_empty()
        {
            let model = binding.model.trim().to_string();
            return Ok(UpstreamProfile {
                name: "Codex++".into(),
                base_url: crate::gateway::apply::gateway_base_url(port, &tool),
                api_key: binding.token.clone(),
                models: if model.is_empty() { Vec::new() } else { vec![model.clone()] },
                model,
                source: SOURCE_CODEX_PLUS_PLUS.into(),
                source_id: "gateway".into(),
                ..UpstreamProfile::default()
            });
        }
    }
    bail!("Codex++ 里还没有接入 Claude：先在 Codex++ 的「Agent 接入」里接入 Claude Code 或 Claude Desktop")
}

// ── 模型列表 ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamModel {
    pub id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u64>,
}

/// `GET {base}/v1/models`（Anthropic 格式，Magpie / Codex++ 网关都支持）。
pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<UpstreamModel>> {
    let base = base_url.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(&format!("{base}/v1/models"))
        .with_context(|| format!("Base URL 不是有效地址：{base_url}"))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1" | "[::1]"));
    let mut client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(20));
    if loopback {
        // 本机网关不走系统代理，否则请求会被代理转发出去。
        client = client.no_proxy();
    }
    let response = client
        .build()?
        .get(url)
        .header("x-api-key", api_key.trim())
        .bearer_auth(api_key.trim())
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .context("请求模型列表失败")?;
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let detail: String = body.chars().take(200).collect();
        bail!("上游返回 {status}：{detail}");
    }
    parse_models(&body)
}

fn parse_models(body: &str) -> Result<Vec<UpstreamModel>> {
    let value: Value = serde_json::from_str(body).context("模型列表不是 JSON")?;
    let entries = value
        .get("data")
        .and_then(Value::as_array)
        .context("模型列表里没有 data 字段")?;
    let mut seen = BTreeMap::new();
    let mut models = Vec::new();
    for entry in entries {
        let Some(id) = entry.get("id").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if id.is_empty() || seen.insert(id.to_string(), ()).is_some() {
            continue;
        }
        models.push(UpstreamModel {
            id: id.to_string(),
            display_name: entry
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty() && *name != id)
                .map(str::to_string),
            context_window: entry.get("context_window").and_then(Value::as_u64).filter(|v| *v > 0),
        });
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upsert_assigns_ids_names_and_selects_the_first_profile() {
        let mut book = UpstreamBook::default();
        let id = book.upsert(UpstreamProfile {
            base_url: "https://api.example.com/".into(),
            ..UpstreamProfile::default()
        });
        assert_eq!(book.active_id.as_deref(), Some(id.as_str()));
        let first = book.active().unwrap();
        assert_eq!(first.base_url, "https://api.example.com");
        assert_eq!(first.name, "api.example.com");
        assert_eq!(first.source, SOURCE_MANUAL);
        // 第二组不抢选中；改名按 id 更新。
        let second = book.upsert(UpstreamProfile { name: "B".into(), ..UpstreamProfile::default() });
        assert_eq!(book.active_id.as_deref(), Some(id.as_str()));
        book.upsert(UpstreamProfile { id: second.clone(), name: "B2".into(), ..UpstreamProfile::default() });
        assert_eq!(book.profiles.len(), 2);
        assert_eq!(book.profiles[1].name, "B2");
        // 删掉选中的那组，选中落到剩下的第一组。
        book.remove(&id);
        assert_eq!(book.active_id.as_deref(), Some(second.as_str()));
    }

    #[test]
    fn reimport_updates_address_and_key_but_keeps_user_choices() {
        let mut book = UpstreamBook::default();
        let imported = |key: &str| UpstreamProfile {
            name: "Kimi".into(),
            base_url: "https://kimi.example".into(),
            api_key: key.into(),
            model: "kimi-k3".into(),
            models: vec!["kimi-k3".into()],
            source: SOURCE_CC_SWITCH.into(),
            source_id: "p1".into(),
            ..UpstreamProfile::default()
        };
        assert_eq!(book.merge_imported(vec![imported("old")]), (1, 0));
        book.profiles[0].name = "我的 Kimi".into();
        book.profiles[0].model = "kimi-k3-turbo".into();
        assert_eq!(book.merge_imported(vec![imported("new")]), (0, 1));
        let profile = &book.profiles[0];
        assert_eq!(profile.api_key, "new");
        assert_eq!(profile.name, "我的 Kimi");
        assert_eq!(profile.model, "kimi-k3-turbo");
    }

    #[test]
    fn book_round_trips_through_disk_and_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claude-mom").join(BOOK_FILE);
        assert_eq!(load_book(&path).unwrap(), UpstreamBook::default());
        let mut book = UpstreamBook::default();
        book.upsert(UpstreamProfile { base_url: "https://a.example".into(), api_key: "k".into(), ..UpstreamProfile::default() });
        save_book(&path, &book).unwrap();
        assert_eq!(load_book(&path).unwrap(), book);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn cc_switch_claude_providers_are_read_and_non_anthropic_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cc-switch.db");
        let connection = rusqlite::Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE providers (id TEXT, app_type TEXT, name TEXT, settings_config TEXT, \
                 meta TEXT DEFAULT '{}', sort_index INTEGER, created_at INTEGER, is_current BOOLEAN);",
            )
            .unwrap();
        let insert = |id: &str, app: &str, name: &str, settings: Value, meta: Value| {
            connection
                .execute(
                    "INSERT INTO providers (id, app_type, name, settings_config, meta, sort_index, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)",
                    rusqlite::params![id, app, name, settings.to_string(), meta.to_string()],
                )
                .unwrap();
        };
        insert(
            "a",
            "claude",
            "GLM",
            json!({"env": {
                "ANTHROPIC_BASE_URL": "https://open.bigmodel.example/api/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "tok",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "glm-5",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "glm-5-air"
            }}),
            json!({}),
        );
        insert(
            "b",
            "claude",
            "OpenRouter",
            json!({"env": {"ANTHROPIC_BASE_URL": "https://openrouter.example", "ANTHROPIC_API_KEY": "x"}}),
            json!({"apiFormat": "openai_chat"}),
        );
        insert("c", "codex", "Codex", json!({"env": {"ANTHROPIC_BASE_URL": "https://x"}}), json!({}));
        insert("d", "claude", "无地址", json!({"env": {}}), json!({}));
        drop(connection);

        let profiles = cc_switch_profiles(&db).unwrap();
        assert_eq!(profiles.len(), 1);
        let glm = &profiles[0];
        assert_eq!(glm.name, "GLM");
        assert_eq!(glm.api_key, "tok");
        assert_eq!(glm.model, "glm-5");
        assert_eq!(glm.small_model, "glm-5-air");
        assert_eq!(glm.models, ["glm-5", "glm-5-air"]);
        assert_eq!((glm.source.as_str(), glm.source_id.as_str()), (SOURCE_CC_SWITCH, "a"));
        assert!(cc_switch_profiles(&dir.path().join("missing.db")).is_err());
    }

    #[test]
    fn magpie_gateway_is_discovered_like_magpie_does() {
        let dir = tempfile::tempdir().unwrap();
        // 没有 gateway.json：默认端口。
        assert_eq!(magpie_profile(dir.path(), None).base_url, "http://127.0.0.1:3425");
        std::fs::write(dir.path().join("gateway.json"), r#"{"addr":"0.0.0.0:4111","pid":1}"#).unwrap();
        let profile = magpie_profile(dir.path(), None);
        assert_eq!(profile.base_url, "http://127.0.0.1:4111");
        assert_eq!(profile.api_key, "magpie-claude-desktop");
        assert_eq!(profile.source, SOURCE_MAGPIE);
        // MAGPIE_ADDR 优先。
        assert_eq!(magpie_profile(dir.path(), Some("127.0.0.1:5000")).base_url, "http://127.0.0.1:5000");
    }

    #[test]
    fn codex_plus_plus_gateway_needs_a_claude_binding() {
        let dir = tempfile::tempdir().unwrap();
        assert!(codex_plus_plus_profile(dir.path()).is_err(), "没接入 Claude 时报错");
        std::fs::write(dir.path().join("protocol-proxy-port"), "58001\n").unwrap();
        std::fs::write(
            dir.path().join("gateway.json"),
            json!({"bindings": {"claude": {"enabled": true, "token": "cpp-token", "model": "deepseek/v4"}}})
                .to_string(),
        )
        .unwrap();
        let profile = codex_plus_plus_profile(dir.path()).unwrap();
        assert_eq!(profile.base_url, "http://127.0.0.1:58001/gw/claude");
        assert_eq!(profile.api_key, "cpp-token");
        assert_eq!(profile.model, "deepseek/v4");
        assert_eq!(profile.source, SOURCE_CODEX_PLUS_PLUS);
    }

    #[test]
    fn model_lists_parse_anthropic_shape_with_optional_windows() {
        let models = parse_models(
            &json!({"data": [
                {"id": "a/claude-opus", "context_window": 1_000_000, "display_name": "Claude Opus · A"},
                {"id": "b/glm-5"},
                {"id": "b/glm-5"},
                {"id": ""}
            ]})
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            models,
            [
                UpstreamModel { id: "a/claude-opus".into(), display_name: Some("Claude Opus · A".into()), context_window: Some(1_000_000) },
                UpstreamModel { id: "b/glm-5".into(), display_name: None, context_window: None },
            ]
        );
        assert!(parse_models("{}").is_err());
    }
}
