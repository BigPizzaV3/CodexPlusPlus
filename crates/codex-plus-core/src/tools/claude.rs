//! Claude Code 的供应商切换：把供应商写进 `~/.claude/settings.json`。
//!
//! 写入规则参考 CC Switch（MIT，见 THIRD_PARTY_NOTICES.md）的「关键字段」投影：
//!
//! - **关键字段**回答「请求发到哪、凭什么鉴权、用哪个模型名、说哪种协议」，完全归供应商：
//!   切换时一律清空，再写目标供应商的值；
//! - **独有字段**是上游的兼容开关和窗口值：切入时写入，切走时只删上一家带进来、而且值没被
//!   用户改过的；
//! - 其余键（hooks、permissions、插件、状态栏……）归用户和 Claude Code，这里不写也不删。
//!
//! 供应商列表存在 Codex++ 状态目录的 `claude-providers.json`，不进 `settings.json` 的
//! `tools` 分片：那份文件由前端整份保存，独立文件不会被它覆盖。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 网关模式托管的供应商 id：开启 Claude Code 网关时写入、关闭时删掉。
pub const GATEWAY_PROVIDER_ID: &str = "codex-plus-gateway";

const SETTINGS_FILE: &str = "settings.json";
const STORE_FILE: &str = "claude-providers.json";
const STORE_VERSION: u32 = 1;

/// Claude Code 的协议选择器。`CLAUDE_CODE_USE_` 不能按前缀匹配：同一前缀下还有
/// `USE_POWERSHELL_TOOL` 这类与供应商无关的功能开关。
const PROTOCOL_SELECTORS: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_GATEWAY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
];

/// `env` 里整个前缀都属于连接和鉴权的前缀：地址、凭据、各档模型名、自定义头，
/// Bedrock 的区域与凭据，Vertex 的分模型区域。
const FLOOR_ENV_PREFIXES: &[&str] = &["ANTHROPIC_", "AWS_", "VERTEX_REGION_"];

/// `env` 里按名字列出的关键字段（协议选择器之外）。
const FLOOR_ENV_KEYS: &[&str] = &[
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "CLOUD_ML_REGION",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_API_KEY_HELPER_TTL_MS",
];

/// 顶层的关键字段。`model` 是 `/model` 保存的选择，属于当时所在的那一家。
const FLOOR_TOP: &[&str] = &[
    "apiKeyHelper",
    "apiBaseUrl",
    "primaryModel",
    "smallFastModel",
    "apiKey",
    "model",
    "fallbackModel",
    "modelOverrides",
    "modelPicker",
    "advisorModel",
    "awsAuthRefresh",
    "awsCredentialExport",
    "gcpAuthRefresh",
];

/// `env` 里的供应商独有字段：兼容开关与窗口值。用户也可能把它们设成全局，所以不当关键字段。
const EXCLUSIVE_ENV: &[&str] = &[
    "CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS",
    "CLAUDE_CODE_DISABLE_ARTIFACT",
    "ENABLE_TOOL_SEARCH",
    "CLAUDE_CODE_DISABLE_THINKING",
    "DISABLE_INTERLEAVED_THINKING",
    "CLAUDE_CODE_ALWAYS_ENABLE_EFFORT",
    "CLAUDE_CODE_EXTRA_BODY",
    "CLAUDE_CODE_ENABLE_FINE_GRAINED_TOOL_STREAMING",
    "CLAUDE_CODE_AUTO_MODE_SERVER",
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
    "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
    "CLAUDE_CODE_DISABLE_1M_CONTEXT",
    "CLAUDE_CODE_DISABLE_UNKNOWN_MODEL_WINDOW_ENFORCEMENT",
    "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY",
];

pub fn is_floor_env(key: &str) -> bool {
    FLOOR_ENV_PREFIXES.iter().any(|prefix| key.starts_with(prefix))
        || PROTOCOL_SELECTORS.contains(&key)
        || FLOOR_ENV_KEYS.contains(&key)
        || (key.starts_with("CLAUDE_CODE_SKIP_") && key.ends_with("_AUTH"))
}

pub fn is_floor_top(key: &str) -> bool {
    FLOOR_TOP.contains(&key)
}

pub fn is_exclusive_env(key: &str) -> bool {
    EXCLUSIVE_ENV.contains(&key)
}

/// 一个 Claude 供应商。`settings_config` 与 `settings.json` 同形：`{ "env": {...}, 顶层键... }`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProvider {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub website_url: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "empty_settings_config")]
    pub settings_config: Value,
    #[serde(default)]
    pub created_at: u64,
    /// 从 CC Switch 导入时的来源 id，重复导入时据此跳过。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ccs_source_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProviderStore {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub current_id: String,
    #[serde(default)]
    pub providers: Vec<ClaudeProvider>,
}

/// `settings.json` 当前状态的摘要。不带 Key 原文。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLiveSummary {
    pub settings_path: String,
    pub exists: bool,
    /// 文件存在但无法解析时的错误；此时切换会被拒绝，不会覆盖用户文件。
    pub parse_error: Option<String>,
    pub base_url: String,
    pub model: String,
    pub auth_configured: bool,
    /// live 的关键字段是否与当前供应商一致；`false` 说明 settings.json 被外部改过。
    pub matches_current: bool,
    pub has_key_fields: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProvidersPayload {
    pub store_path: String,
    pub claude_home: String,
    pub cli_path: Option<String>,
    pub current_id: String,
    pub providers: Vec<ClaudeProvider>,
    pub live: ClaudeLiveSummary,
    /// 本次写 settings.json 前的备份。
    pub backup_path: Option<String>,
    /// 导入类操作新增的条数。
    pub imported: usize,
}

/// 读写所需的路径。命令层用 [`ClaudeContext::default_paths`]，测试指向临时目录。
#[derive(Debug, Clone)]
pub struct ClaudeContext {
    pub store_path: PathBuf,
    pub claude_home: PathBuf,
    pub backup_root: PathBuf,
}

impl ClaudeContext {
    pub fn default_paths() -> Self {
        let state = crate::paths::default_app_state_dir();
        Self {
            store_path: state.join(STORE_FILE),
            claude_home: super::default_claude_home_dir(),
            backup_root: state.join("backups"),
        }
    }

    fn settings_path(&self) -> PathBuf {
        self.claude_home.join(SETTINGS_FILE)
    }
}

fn empty_settings_config() -> Value {
    serde_json::json!({ "env": {} })
}

/// 一个供应商在 `settings.json` 里拥有的键。
#[derive(Debug, Clone, Default, PartialEq)]
struct Projection {
    top: Map<String, Value>,
    env: Map<String, Value>,
    exclusive: Map<String, Value>,
}

impl Projection {
    fn of(settings: &Value) -> Self {
        let mut projection = Self::default();
        if let Some(root) = settings.as_object() {
            for (key, value) in root {
                if is_floor_top(key) {
                    projection.top.insert(key.clone(), value.clone());
                }
            }
        }
        if let Some(env) = settings.get("env").and_then(Value::as_object) {
            for (key, value) in env {
                if is_floor_env(key) {
                    projection.env.insert(key.clone(), value.clone());
                } else if is_exclusive_env(key) {
                    projection.exclusive.insert(key.clone(), value.clone());
                }
            }
        }
        projection
    }

    fn is_empty(&self) -> bool {
        self.top.is_empty() && self.env.is_empty() && self.exclusive.is_empty()
    }

    /// 投影成供应商行：只含关键字段和独有字段。
    fn to_settings_config(&self) -> Value {
        let mut root = self.top.clone();
        let mut env = self.env.clone();
        env.extend(self.exclusive.clone());
        root.insert("env".to_string(), Value::Object(env));
        Value::Object(root)
    }
}

/// 以 live 为底切到 `target`，原地改写 `live`：
/// - 关键字段：目标没有的删掉，目标有的写入（已存在的键保持原位置）；
/// - 独有字段：`prev` 带进来、值没被改过、目标又不写的删掉，再写目标的；
/// - 其余键不动。
fn apply_projection(
    live: &mut Value,
    prev: Option<&Projection>,
    target: &Projection,
) -> anyhow::Result<()> {
    let root = live
        .as_object_mut()
        .context("settings.json 顶层不是 JSON 对象")?;

    // 用 retain 删：生产构建开了 preserve_order，`remove` 会把末尾的键换到删掉的位置。
    root.retain(|key, _| !is_floor_top(key) || target.top.contains_key(key));
    for (key, value) in &target.top {
        root.insert(key.clone(), value.clone());
    }

    let wants_env = !target.env.is_empty() || !target.exclusive.is_empty();
    if root.get("env").is_none() && !wants_env {
        return Ok(());
    }
    let env = root
        .entry("env")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .context("settings.json 里的 env 不是 JSON 对象")?;

    env.retain(|key, value| {
        if is_floor_env(key) {
            return target.env.contains_key(key);
        }
        let outgoing = prev.is_some_and(|prev| prev.exclusive.get(key) == Some(value));
        !outgoing || target.exclusive.contains_key(key)
    });
    for (key, value) in target.env.iter().chain(&target.exclusive) {
        env.insert(key.clone(), value.clone());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 对外操作
// ---------------------------------------------------------------------------

pub fn load(ctx: &ClaudeContext) -> anyhow::Result<ClaudeProvidersPayload> {
    let store = read_store(&ctx.store_path)?;
    Ok(payload(ctx, store, None, 0))
}

/// 新增或更新一个供应商。改的是当前供应商时，同步重写 settings.json。
pub fn save_provider(
    ctx: &ClaudeContext,
    mut provider: ClaudeProvider,
) -> anyhow::Result<ClaudeProvidersPayload> {
    validate_provider(&provider)?;
    provider.name = provider.name.trim().to_string();
    provider.website_url = provider.website_url.trim().to_string();
    let mut store = read_store(&ctx.store_path)?;
    if provider.id.trim().is_empty() {
        provider.id = new_provider_id(&store);
    }
    if provider.created_at == 0 {
        provider.created_at = now_millis();
    }

    let previous = store
        .providers
        .iter()
        .position(|candidate| candidate.id == provider.id);
    let mut backup_path = None;
    if store.current_id == provider.id {
        let prev = previous.map(|index| Projection::of(&store.providers[index].settings_config));
        backup_path = write_live(ctx, prev.as_ref(), &Projection::of(&provider.settings_config))?;
    }
    match previous {
        Some(index) => store.providers[index] = provider,
        None => store.providers.push(provider),
    }
    write_store(&ctx.store_path, &store)?;
    Ok(payload(ctx, store, backup_path, 0))
}

pub fn delete_provider(ctx: &ClaudeContext, id: &str) -> anyhow::Result<ClaudeProvidersPayload> {
    let mut store = read_store(&ctx.store_path)?;
    if store.current_id == id {
        bail!("不能删除正在使用的供应商，请先切换到其他供应商");
    }
    let before = store.providers.len();
    store.providers.retain(|provider| provider.id != id);
    if store.providers.len() == before {
        bail!("供应商不存在：{id}");
    }
    write_store(&ctx.store_path, &store)?;
    Ok(payload(ctx, store, None, 0))
}

/// 切换到 `id`：先写 settings.json，成功后再改当前指针。
///
/// 还没有当前供应商、而 settings.json 里已有关键字段时，先把它存成一个供应商，
/// 避免切换把用户原本手填的地址和 Key 清掉。
pub fn switch_provider(ctx: &ClaudeContext, id: &str) -> anyhow::Result<ClaudeProvidersPayload> {
    let mut store = read_store(&ctx.store_path)?;
    let target = store
        .providers
        .iter()
        .find(|provider| provider.id == id)
        .cloned()
        .with_context(|| format!("供应商不存在：{id}"))?;

    let mut imported = 0;
    let current = store
        .providers
        .iter()
        .find(|provider| provider.id == store.current_id)
        .cloned();
    let prev = match current {
        Some(current) => Some(Projection::of(&current.settings_config)),
        None => {
            let live = read_live(&ctx.settings_path())?;
            let projection = Projection::of(&live);
            if !projection.is_empty() && find_by_projection(&store, &projection).is_none() {
                let snapshot = ClaudeProvider {
                    id: new_provider_id(&store),
                    name: "切换前的配置".to_string(),
                    website_url: String::new(),
                    notes: "首次切换时由 Codex++ 从 settings.json 自动保存。".to_string(),
                    settings_config: projection.to_settings_config(),
                    created_at: now_millis(),
                    ccs_source_id: String::new(),
                };
                store.providers.push(snapshot);
                imported = 1;
            }
            Some(projection)
        }
    };

    let backup_path = write_live(ctx, prev.as_ref(), &Projection::of(&target.settings_config))?;
    store.current_id = target.id;
    write_store(&ctx.store_path, &store)?;
    Ok(payload(ctx, store, backup_path, imported))
}

/// 当前供应商 id（没有时为空串）。
pub fn current_provider_id(ctx: &ClaudeContext) -> anyhow::Result<String> {
    Ok(read_store(&ctx.store_path)?.current_id)
}

/// 清掉当前供应商写进 settings.json 的关键字段与独有字段，并清空当前指针。
///
/// 网关模式关闭、而开启前没有可切回的供应商时用：settings.json 回到「没配供应商」的样子，
/// 用户的 hooks、permissions 等其余键不动。
pub fn clear_current(ctx: &ClaudeContext) -> anyhow::Result<ClaudeProvidersPayload> {
    let mut store = read_store(&ctx.store_path)?;
    let prev = store
        .providers
        .iter()
        .find(|provider| provider.id == store.current_id)
        .map(|provider| Projection::of(&provider.settings_config));
    let backup_path = write_live(ctx, prev.as_ref(), &Projection::default())?;
    store.current_id.clear();
    write_store(&ctx.store_path, &store)?;
    Ok(payload(ctx, store, backup_path, 0))
}

/// 只清空当前指针，不动 settings.json。
///
/// 网关断开时 settings.json 已经被别的工具改走：它现在的样子是用户的新选择，不能再写回，
/// 只把 Codex++ 自己记的「当前供应商」清掉。
pub fn forget_current(ctx: &ClaudeContext) -> anyhow::Result<()> {
    let mut store = read_store(&ctx.store_path)?;
    if store.current_id.is_empty() {
        return Ok(());
    }
    store.current_id.clear();
    write_store(&ctx.store_path, &store)
}

/// 把 settings.json 现有的关键字段存成一个供应商，并把它设为当前供应商（live 本来就是它）。
pub fn import_live(ctx: &ClaudeContext) -> anyhow::Result<ClaudeProvidersPayload> {
    let mut store = read_store(&ctx.store_path)?;
    let live = read_live(&ctx.settings_path())?;
    let projection = Projection::of(&live);
    if projection.is_empty() {
        bail!("settings.json 里没有供应商相关的配置（ANTHROPIC_* 等），无需导入");
    }
    if let Some(existing) = find_by_projection(&store, &projection) {
        bail!("当前配置已存在于供应商「{}」中", existing.name);
    }
    let base_url = projection
        .env
        .get("ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let provider = ClaudeProvider {
        id: new_provider_id(&store),
        name: host_label(base_url).unwrap_or_else(|| "当前配置".to_string()),
        website_url: String::new(),
        notes: String::new(),
        settings_config: projection.to_settings_config(),
        created_at: now_millis(),
        ccs_source_id: String::new(),
    };
    store.current_id = provider.id.clone();
    store.providers.push(provider);
    write_store(&ctx.store_path, &store)?;
    Ok(payload(ctx, store, None, 1))
}

/// 从 CC Switch 的数据库导入 Claude 供应商。只读打开数据库，已导入过的来源跳过。
pub fn import_from_cc_switch(
    ctx: &ClaudeContext,
    db_path: &Path,
) -> anyhow::Result<ClaudeProvidersPayload> {
    let rows = list_cc_switch_claude_rows(db_path)?;
    let mut store = read_store(&ctx.store_path)?;
    let mut imported = 0;
    for row in rows {
        if store
            .providers
            .iter()
            .any(|provider| provider.ccs_source_id == row.source_id)
        {
            continue;
        }
        let provider = ClaudeProvider {
            id: new_provider_id(&store),
            name: row.name,
            website_url: row.website_url,
            notes: row.notes,
            settings_config: Projection::of(&row.settings_config).to_settings_config(),
            created_at: now_millis(),
            ccs_source_id: row.source_id,
        };
        store.providers.push(provider);
        imported += 1;
    }
    if imported > 0 {
        write_store(&ctx.store_path, &store)?;
    }
    Ok(payload(ctx, store, None, imported))
}

// ---------------------------------------------------------------------------
// 内部实现
// ---------------------------------------------------------------------------

struct CcSwitchRow {
    source_id: String,
    name: String,
    website_url: String,
    notes: String,
    settings_config: Value,
}

fn list_cc_switch_claude_rows(path: &Path) -> anyhow::Result<Vec<CcSwitchRow>> {
    if !path.is_file() {
        bail!("没有找到 CC Switch 数据库：{}", path.display());
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("无法打开 CC Switch 数据库 {}", path.display()))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, settings_config, website_url, notes
         FROM providers
         WHERE app_type = 'claude'
         ORDER BY COALESCE(sort_index, 999999), created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (source_id, name, settings_config, website_url, notes) = row?;
        // 单行 JSON 坏掉不影响其它行。
        let Ok(settings_config) = serde_json::from_str::<Value>(&settings_config) else {
            continue;
        };
        if !settings_config.is_object() {
            continue;
        }
        result.push(CcSwitchRow {
            source_id,
            name,
            website_url: website_url.unwrap_or_default(),
            notes: notes.unwrap_or_default(),
            settings_config,
        });
    }
    Ok(result)
}

fn validate_provider(provider: &ClaudeProvider) -> anyhow::Result<()> {
    if provider.name.trim().is_empty() {
        bail!("供应商名称不能为空");
    }
    let root = provider
        .settings_config
        .as_object()
        .context("供应商配置必须是 JSON 对象")?;
    if let Some(env) = root.get("env") {
        let env = env.as_object().context("供应商配置里的 env 必须是 JSON 对象")?;
        for (key, value) in env {
            if !matches!(value, Value::String(_) | Value::Number(_) | Value::Bool(_)) {
                bail!("env.{key} 必须是字符串、数字或布尔值");
            }
        }
    }
    Ok(())
}

/// 关键字段完全相同的供应商（独有字段不比：用户可能把它们设成了全局）。
fn find_by_projection<'a>(
    store: &'a ClaudeProviderStore,
    projection: &Projection,
) -> Option<&'a ClaudeProvider> {
    store.providers.iter().find(|provider| {
        let other = Projection::of(&provider.settings_config);
        other.env == projection.env && other.top == projection.top
    })
}

fn payload(
    ctx: &ClaudeContext,
    store: ClaudeProviderStore,
    backup_path: Option<PathBuf>,
    imported: usize,
) -> ClaudeProvidersPayload {
    let current = store
        .providers
        .iter()
        .find(|provider| provider.id == store.current_id);
    let live = live_summary(&ctx.settings_path(), current);
    ClaudeProvidersPayload {
        store_path: ctx.store_path.to_string_lossy().to_string(),
        claude_home: ctx.claude_home.to_string_lossy().to_string(),
        cli_path: find_claude_cli(&ctx.claude_home).map(|path| path.to_string_lossy().to_string()),
        current_id: store.current_id,
        providers: store.providers,
        live,
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
        imported,
    }
}

fn live_summary(path: &Path, current: Option<&ClaudeProvider>) -> ClaudeLiveSummary {
    let mut summary = ClaudeLiveSummary {
        settings_path: path.to_string_lossy().to_string(),
        exists: path.is_file(),
        ..ClaudeLiveSummary::default()
    };
    let live = match read_live(path) {
        Ok(value) => value,
        Err(error) => {
            summary.parse_error = Some(format!("{error:#}"));
            return summary;
        }
    };
    let projection = Projection::of(&live);
    let env_string = |key: &str| {
        projection
            .env
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    summary.base_url = env_string("ANTHROPIC_BASE_URL");
    summary.model = env_string("ANTHROPIC_MODEL");
    summary.auth_configured =
        !env_string("ANTHROPIC_AUTH_TOKEN").is_empty() || !env_string("ANTHROPIC_API_KEY").is_empty();
    summary.has_key_fields = !projection.top.is_empty() || !projection.env.is_empty();
    summary.matches_current = current.is_some_and(|current| {
        let expected = Projection::of(&current.settings_config);
        expected.top == projection.top && expected.env == projection.env
    });
    summary
}

/// 读 settings.json；不存在视作空对象。
fn read_live(path: &Path) -> anyhow::Result<Value> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Map::new()));
        }
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let value: Value = serde_json::from_str(&text)
        .with_context(|| format!("{} 不是有效的 JSON，请先修好再切换", path.display()))?;
    if !value.is_object() {
        bail!("{} 顶层不是 JSON 对象", path.display());
    }
    Ok(value)
}

/// 把投影写进 settings.json，返回备份路径（原文件不存在时没有备份）。
fn write_live(
    ctx: &ClaudeContext,
    prev: Option<&Projection>,
    target: &Projection,
) -> anyhow::Result<Option<PathBuf>> {
    let path = ctx.settings_path();
    let mut live = read_live(&path)?;
    let before = live.clone();
    apply_projection(&mut live, prev, target)?;
    if live == before && path.is_file() {
        return Ok(None);
    }

    let existed = path.is_file();
    let backup = if existed {
        Some(backup_file(&path, &ctx.backup_root)?)
    } else {
        None
    };
    let mut text = serde_json::to_string_pretty(&live)?;
    text.push('\n');
    crate::settings::atomic_write(&path, text.as_bytes())
        .with_context(|| format!("无法写入 {}", path.display()))?;
    // 里面有 Key：新建的文件只给本人读写。已有文件沿用原权限。
    if !existed {
        set_private(&path)?;
    }
    Ok(backup)
}

fn backup_file(path: &Path, backup_root: &Path) -> anyhow::Result<PathBuf> {
    let dir = backup_root.join("claude").join(now_nanos().to_string());
    fs::create_dir_all(&dir).with_context(|| format!("无法创建备份目录 {}", dir.display()))?;
    set_private(&dir)?;
    let target = dir.join(SETTINGS_FILE);
    fs::copy(path, &target).with_context(|| format!("无法备份 {}", path.display()))?;
    set_private(&target)?;
    Ok(target)
}

fn read_store(path: &Path) -> anyhow::Result<ClaudeProviderStore> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ClaudeProviderStore::default());
        }
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(ClaudeProviderStore::default());
    }
    serde_json::from_str(&text).with_context(|| format!("无法解析 {}", path.display()))
}

fn write_store(path: &Path, store: &ClaudeProviderStore) -> anyhow::Result<()> {
    let mut store = store.clone();
    store.version = STORE_VERSION;
    let existed = path.is_file();
    let mut text = serde_json::to_string_pretty(&store)?;
    text.push('\n');
    crate::settings::atomic_write(path, text.as_bytes())
        .with_context(|| format!("无法写入 {}", path.display()))?;
    if !existed {
        set_private(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o700 } else { 0o600 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("无法设置权限 {}", path.display()))
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn new_provider_id(store: &ClaudeProviderStore) -> String {
    let base = format!("claude-{:x}", now_nanos());
    let mut id = base.clone();
    let mut suffix = 2;
    while store.providers.iter().any(|provider| provider.id == id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

/// 用 Base URL 的主机名当导入条目的名字，比「当前配置」好认。
fn host_label(base_url: &str) -> Option<String> {
    let rest = base_url.trim().split("://").nth(1)?;
    let host = rest.split(['/', ':', '?']).next()?.trim();
    (!host.is_empty()).then(|| host.to_string())
}

fn find_claude_cli(claude_home: &Path) -> Option<PathBuf> {
    let binary = if cfg!(windows) { "claude.exe" } else { "claude" };
    let mut candidates = vec![claude_home.join("local").join(binary)];
    if let Some(dirs) = directories::BaseDirs::new() {
        candidates.push(dirs.home_dir().join(".local").join("bin").join(binary));
    }
    if let Some(found) = candidates.into_iter().find(|path| path.is_file()) {
        return Some(found);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|path| path.join(binary))
        .find(|path| path.is_file())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        _temp: tempfile::TempDir,
        ctx: ClaudeContext,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let ctx = ClaudeContext {
            store_path: temp.path().join("state").join(STORE_FILE),
            claude_home: temp.path().join("claude"),
            backup_root: temp.path().join("state").join("backups"),
        };
        Fixture { _temp: temp, ctx }
    }

    fn write_settings(ctx: &ClaudeContext, value: &Value) {
        fs::create_dir_all(&ctx.claude_home).unwrap();
        fs::write(ctx.settings_path(), serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    fn read_settings(ctx: &ClaudeContext) -> Value {
        serde_json::from_str(&fs::read_to_string(ctx.settings_path()).unwrap()).unwrap()
    }

    fn provider(name: &str, settings_config: Value) -> ClaudeProvider {
        ClaudeProvider {
            id: String::new(),
            name: name.to_string(),
            website_url: String::new(),
            notes: String::new(),
            settings_config,
            created_at: 0,
            ccs_source_id: String::new(),
        }
    }

    fn kimi() -> Value {
        json!({ "env": {
            "ANTHROPIC_BASE_URL": "https://kimi.example",
            "ANTHROPIC_AUTH_TOKEN": "sk-kimi",
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "262144"
        }})
    }

    fn glm() -> Value {
        json!({ "env": {
            "ANTHROPIC_BASE_URL": "https://glm.example",
            "ANTHROPIC_AUTH_TOKEN": "sk-glm",
            "ANTHROPIC_MODEL": "glm-5"
        }})
    }

    fn add(ctx: &ClaudeContext, name: &str, config: Value) -> String {
        let payload = save_provider(ctx, provider(name, config)).unwrap();
        payload.providers.last().unwrap().id.clone()
    }

    #[test]
    fn projection_splits_key_exclusive_and_user_fields() {
        let projection = Projection::of(&json!({
            "model": "picked",
            "hooks": { "Stop": [] },
            "env": {
                "ANTHROPIC_BASE_URL": "https://a.example",
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1",
                "ENABLE_TOOL_SEARCH": "true",
                "API_TIMEOUT_MS": "300000"
            }
        }));
        assert_eq!(Value::Object(projection.top), json!({ "model": "picked" }));
        assert_eq!(
            Value::Object(projection.env),
            json!({ "ANTHROPIC_BASE_URL": "https://a.example", "CLAUDE_CODE_USE_BEDROCK": "1" })
        );
        assert_eq!(Value::Object(projection.exclusive), json!({ "ENABLE_TOOL_SEARCH": "true" }));
    }

    #[test]
    fn switching_replaces_key_fields_and_keeps_user_settings() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let glm_id = add(&f.ctx, "GLM", glm());
        write_settings(&f.ctx, &json!({
            "model": "opus",
            "hooks": { "Stop": [] },
            "permissions": { "allow": ["Bash(ls)"] },
            "env": { "API_TIMEOUT_MS": "300000", "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1" }
        }));

        // 首次切换：live 里的 `model` 被存成「切换前的配置」，再写入 Kimi。
        let result = switch_provider(&f.ctx, &kimi_id).unwrap();
        assert_eq!(result.imported, 1);
        assert_eq!(result.current_id, kimi_id);
        assert!(result.backup_path.is_some());
        let live = read_settings(&f.ctx);
        assert_eq!(live["hooks"], json!({ "Stop": [] }));
        assert_eq!(live["permissions"], json!({ "allow": ["Bash(ls)"] }));
        assert!(live.get("model").is_none());
        assert_eq!(live["env"]["API_TIMEOUT_MS"], "300000");
        assert_eq!(live["env"]["CLAUDE_CODE_USE_POWERSHELL_TOOL"], "1");
        assert_eq!(live["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-kimi");
        assert_eq!(live["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "262144");
        assert!(result.live.matches_current);

        // 切到 GLM：Kimi 的地址、Key、窗口值都走，用户的设置都留。
        let result = switch_provider(&f.ctx, &glm_id).unwrap();
        assert_eq!(result.imported, 0);
        let live = read_settings(&f.ctx);
        assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://glm.example");
        assert_eq!(live["env"]["ANTHROPIC_MODEL"], "glm-5");
        assert!(live["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none());
        assert_eq!(live["env"]["API_TIMEOUT_MS"], "300000");
        assert_eq!(live["hooks"], json!({ "Stop": [] }));
    }

    #[test]
    fn exclusive_field_edited_by_user_is_kept_on_switch() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let glm_id = add(&f.ctx, "GLM", glm());
        switch_provider(&f.ctx, &kimi_id).unwrap();
        let mut live = read_settings(&f.ctx);
        live["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"] = json!("500000");
        write_settings(&f.ctx, &live);

        switch_provider(&f.ctx, &glm_id).unwrap();
        assert_eq!(read_settings(&f.ctx)["env"]["CLAUDE_CODE_MAX_CONTEXT_TOKENS"], "500000");
    }

    #[test]
    fn official_provider_clears_all_key_fields() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let official_id = add(&f.ctx, "Claude Official", json!({ "env": {} }));
        switch_provider(&f.ctx, &kimi_id).unwrap();
        switch_provider(&f.ctx, &official_id).unwrap();
        let live = read_settings(&f.ctx);
        assert_eq!(live, json!({ "env": {} }));
    }

    #[test]
    fn editing_current_provider_rewrites_settings() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let payload = switch_provider(&f.ctx, &kimi_id).unwrap();
        let mut edited = payload.providers.iter().find(|p| p.id == kimi_id).unwrap().clone();
        edited.settings_config = json!({ "env": {
            "ANTHROPIC_BASE_URL": "https://kimi-new.example",
            "ANTHROPIC_API_KEY": "sk-new"
        }});
        save_provider(&f.ctx, edited).unwrap();
        let live = read_settings(&f.ctx);
        assert_eq!(live["env"]["ANTHROPIC_BASE_URL"], "https://kimi-new.example");
        assert_eq!(live["env"]["ANTHROPIC_API_KEY"], "sk-new");
        assert!(live["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
        // 编辑前的行带进来的窗口值，值没改过，跟着走。
        assert!(live["env"].get("CLAUDE_CODE_MAX_CONTEXT_TOKENS").is_none());
    }

    #[test]
    fn editing_other_provider_does_not_touch_settings() {
        let f = fixture();
        let glm_id = add(&f.ctx, "GLM", glm());
        add(&f.ctx, "Kimi", kimi());
        assert!(!f.ctx.settings_path().exists());
        let payload = load(&f.ctx).unwrap();
        let mut glm = payload.providers.iter().find(|p| p.id == glm_id).unwrap().clone();
        glm.name = "GLM 2".to_string();
        save_provider(&f.ctx, glm).unwrap();
        assert!(!f.ctx.settings_path().exists());
    }

    #[test]
    fn cannot_delete_current_provider() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let glm_id = add(&f.ctx, "GLM", glm());
        switch_provider(&f.ctx, &kimi_id).unwrap();
        assert!(delete_provider(&f.ctx, &kimi_id).is_err());
        let payload = delete_provider(&f.ctx, &glm_id).unwrap();
        assert_eq!(payload.providers.len(), 1);
    }

    #[test]
    fn invalid_settings_json_is_never_overwritten() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        fs::create_dir_all(&f.ctx.claude_home).unwrap();
        fs::write(f.ctx.settings_path(), "{ broken").unwrap();
        assert!(switch_provider(&f.ctx, &kimi_id).is_err());
        assert_eq!(fs::read_to_string(f.ctx.settings_path()).unwrap(), "{ broken");
        let payload = load(&f.ctx).unwrap();
        assert!(payload.current_id.is_empty());
        assert!(payload.live.parse_error.is_some());
    }

    #[test]
    fn import_live_creates_current_provider_once() {
        let f = fixture();
        write_settings(&f.ctx, &json!({
            "hooks": {},
            "env": { "ANTHROPIC_BASE_URL": "https://gw.example.com/api", "ANTHROPIC_AUTH_TOKEN": "sk" }
        }));
        let payload = import_live(&f.ctx).unwrap();
        assert_eq!(payload.providers.len(), 1);
        assert_eq!(payload.providers[0].name, "gw.example.com");
        assert_eq!(payload.current_id, payload.providers[0].id);
        assert!(payload.providers[0].settings_config.get("hooks").is_none());
        assert!(payload.live.matches_current);
        assert!(import_live(&f.ctx).is_err());
    }

    #[test]
    fn new_settings_file_is_private() {
        let f = fixture();
        let kimi_id = add(&f.ctx, "Kimi", kimi());
        let result = switch_provider(&f.ctx, &kimi_id).unwrap();
        assert!(result.backup_path.is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(f.ctx.settings_path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let mode = fs::metadata(&f.ctx.store_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn validation_rejects_bad_configs() {
        let f = fixture();
        assert!(save_provider(&f.ctx, provider(" ", json!({}))).is_err());
        assert!(save_provider(&f.ctx, provider("A", json!([]))).is_err());
        assert!(save_provider(&f.ctx, provider("A", json!({ "env": [] }))).is_err());
        assert!(save_provider(&f.ctx, provider("A", json!({ "env": { "X": {} } }))).is_err());
    }

    #[test]
    fn imports_claude_rows_from_cc_switch_once() {
        let f = fixture();
        let db = f.ctx.claude_home.parent().unwrap().join("cc-switch.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE providers (
                id TEXT NOT NULL, app_type TEXT NOT NULL, name TEXT NOT NULL,
                settings_config TEXT NOT NULL, website_url TEXT, category TEXT,
                created_at INTEGER, sort_index INTEGER, notes TEXT,
                PRIMARY KEY (id, app_type));",
        )
        .unwrap();
        let rows = [
            ("a", "claude", "Kimi", r#"{"env":{"ANTHROPIC_BASE_URL":"https://kimi.example","ANTHROPIC_AUTH_TOKEN":"sk"},"hooks":{"Stop":[]}}"#),
            ("b", "claude", "Broken", "not json"),
            ("c", "codex", "Codex", r#"{"auth":{}}"#),
        ];
        for (id, app, name, config) in rows {
            conn.execute(
                "INSERT INTO providers (id, app_type, name, settings_config, website_url) VALUES (?1, ?2, ?3, ?4, 'https://kimi.example')",
                rusqlite::params![id, app, name, config],
            )
            .unwrap();
        }
        drop(conn);

        let payload = import_from_cc_switch(&f.ctx, &db).unwrap();
        assert_eq!(payload.imported, 1);
        assert_eq!(payload.providers[0].name, "Kimi");
        assert_eq!(payload.providers[0].ccs_source_id, "a");
        assert!(payload.providers[0].settings_config.get("hooks").is_none());
        assert_eq!(import_from_cc_switch(&f.ctx, &db).unwrap().imported, 0);
    }
}
