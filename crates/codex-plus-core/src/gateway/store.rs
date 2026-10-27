//! 网关模式的持久状态：`<状态目录>/gateway.json`。
//!
//! 网关模式与原来的直连模式完全独立：供应商池、路由组都只存在这里，
//! 不读也不写 `settings.json` 里 Codex / Claude / Grok 各自的供应商。从直连模式「导入」是复制一份，
//! 之后两边互不影响。文件里有上游 Key 和本地 token，新建时只给本人读写。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::settings::RelayProfile;
use crate::tools::ToolId;

const STORE_FILE: &str = "gateway.json";
const STORE_VERSION: u32 = 1;
/// 客户端 token 前缀；日志里看到它就知道是网关签发的本地凭据，不是上游 Key。
pub const CLIENT_TOKEN_PREFIX: &str = "cpx-gw-";

/// 路由组当作模型名时的前缀：`group/<组 id>`。
pub const GROUP_MODEL_PREFIX: &str = "group/";

/// 网关供应商池里的一个供应商。上游配置复用 [`RelayProfile`] 的结构（协议、地址、多 Key 等），
/// 但只存在网关里；`relay.id` 同时是它的短名，`<id>/<模型>` 可以指定只发给它。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayProvider {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    /// 来源：内置预设的 key，或 `codex:<id>` / `claude:<id>` / `grok:<id>`，界面用来判断「已添加」。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// 每百万 token 的价格，填了才在「用量」里估算费用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<ProviderPrice>,
    /// 这个供应商单独走的 HTTP 代理；空串表示不单独走代理（沿用系统代理）。
    /// 与直连模式的「供应商独立代理」互不相干。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy: String,
    pub relay: RelayProfile,
}

/// 每百万 token 的价格。命中缓存的输入单独计价。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPrice {
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub cached_input: f64,
    #[serde(default)]
    pub output: f64,
}

fn enabled_by_default() -> bool {
    true
}

/// 路由组的成员怎么排先后。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupMode {
    /// 依次：按成员顺序，前一个答不了才换下一个。
    #[default]
    InOrder,
    /// 轮换：每个请求从下一个成员开始。
    Rotate,
    /// 按权重：按成员权重分配第一个尝试的成员。
    Weighted,
}

/// 路由组成员：`<供应商 id>/<模型>`、裸模型名（池里所有提供它的供应商），或 `group/<组 id>`（嵌套）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupMember {
    pub model: String,
    #[serde(default = "default_weight")]
    pub weight: u32,
}

fn default_weight() -> u32 {
    1
}

/// 网关自己的路由组：Agent 在模型列表里把它当一个模型（`group/<id>`）来选。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mode: GroupMode,
    #[serde(default)]
    pub members: Vec<GroupMember>,
}

impl GatewayGroup {
    pub fn model_id(&self) -> String {
        format!("{GROUP_MODEL_PREFIX}{}", self.id)
    }
}

/// 请求里的模型不在绑定的模型集合里时怎么办。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnknownModelPolicy {
    /// 改写成绑定的主模型。Claude Code 的后台任务会固定请求 claude-haiku-* 之类的
    /// 模型名，第三方上游没有，原样转发只会报错。
    #[default]
    MapToDefault,
    /// 原样转发。
    Passthrough,
}

/// Claude Code 的分档模型（opus / sonnet / haiku / 子代理）。空串表示沿用主模型。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeRoleModels {
    #[serde(default)]
    pub opus: String,
    #[serde(default)]
    pub sonnet: String,
    #[serde(default)]
    pub haiku: String,
    #[serde(default)]
    pub subagent: String,
}

/// 开启网关前的现场，关闭时据此恢复。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayRestoreState {
    /// 开启前 Claude Code 正在用的供应商 id（`claude-providers.json` 里）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub claude_previous_provider_id: String,
    /// 开启前 Codex 正在用的供应商 id（直连模式 settings.json 里的 `active_relay_id`）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub codex_previous_relay_id: String,
    /// 写进各 Agent 配置时用的网关端口；端口变化后要重写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_port: Option<u16>,
}

/// 一个 Agent 的网关绑定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBinding {
    #[serde(default)]
    pub enabled: bool,
    /// 主模型：写进 Agent 配置的默认模型，也是未知模型的改写目标。
    /// 为空时用池里第一个供应商的默认模型。
    #[serde(default)]
    pub model: String,
    /// 额外暴露给 Agent 的模型（每行一个，可带 `[1M]` 窗口后缀）。
    #[serde(default)]
    pub extra_models: Vec<String>,
    #[serde(default)]
    pub claude_roles: ClaudeRoleModels,
    #[serde(default)]
    pub unknown_model: UnknownModelPolicy,
    /// 写给 Agent 的上下文窗口（Claude Code 的 `CLAUDE_CODE_MAX_CONTEXT_TOKENS`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// 非 claude-* 模型名时 Claude Code 默认不发 effort，打开后强制启用。
    #[serde(default)]
    pub claude_enable_effort: bool,
    /// 这个 Agent 不使用的池内供应商（「挑选」里取消勾选的）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pool_exclude: Vec<String>,
    /// 网关签发给该 Agent 的本地 token，用来鉴权本地请求。
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub restore: GatewayRestoreState,
}

impl Default for AgentBinding {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            extra_models: Vec::new(),
            claude_roles: ClaudeRoleModels::default(),
            unknown_model: UnknownModelPolicy::default(),
            context_window: None,
            claude_enable_effort: false,
            pool_exclude: Vec::new(),
            token: String::new(),
            restore: GatewayRestoreState::default(),
        }
    }
}

impl AgentBinding {
    /// 绑定里声明过的模型名（主模型、分档模型、额外模型），去掉窗口后缀、去重保序。
    pub fn declared_models(&self) -> Vec<String> {
        let mut models = Vec::new();
        let roles = [
            &self.claude_roles.opus,
            &self.claude_roles.sonnet,
            &self.claude_roles.haiku,
            &self.claude_roles.subagent,
        ];
        let candidates = std::iter::once(&self.model)
            .chain(roles)
            .chain(self.extra_models.iter());
        for raw in candidates {
            let (slug, _) = crate::model_suffix::parse_model_suffix(raw.trim());
            let slug = slug.trim().to_string();
            if !slug.is_empty() && !models.contains(&slug) {
                models.push(slug);
            }
        }
        models
    }
}

/// Key 轮换策略（多 Key 供应商）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeyRotationMode {
    /// 会话粘性：同一会话固定用一个 Key，出错才换。保护上游 prompt cache。
    #[default]
    Sticky,
    /// 每个请求轮换下一个 Key。
    RoundRobin,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyPolicy {
    #[serde(default)]
    pub mode: KeyRotationMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewaySettings {
    #[serde(default)]
    pub version: u32,
    /// 键是 Agent 标识（`claude` / `grok`）。
    #[serde(default)]
    pub bindings: BTreeMap<String, AgentBinding>,
    #[serde(default)]
    pub key_policy: KeyPolicy,
    /// 退出 Codex++ 时把走网关的 Agent 恢复成直连，避免网关不在时连不上。
    #[serde(default)]
    pub restore_on_exit: bool,
    /// 网关的供应商池（顺序即同一模型多家时的尝试顺序）。
    #[serde(default)]
    pub providers: Vec<GatewayProvider>,
    /// 网关的路由组。
    #[serde(default)]
    pub groups: Vec<GatewayGroup>,
    /// 左键点托盘图标弹出快捷面板（默认开）；关掉时托盘保持原来的点击行为。
    #[serde(default = "enabled_by_default")]
    pub tray_panel: bool,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            version: 0,
            bindings: BTreeMap::new(),
            key_policy: KeyPolicy::default(),
            restore_on_exit: false,
            providers: Vec::new(),
            groups: Vec::new(),
            tray_panel: true,
        }
    }
}

impl GatewaySettings {
    pub fn provider(&self, id: &str) -> Option<&GatewayProvider> {
        self.providers
            .iter()
            .find(|provider| provider.relay.id == id)
    }

    pub fn group(&self, id: &str) -> Option<&GatewayGroup> {
        self.groups.iter().find(|group| group.id == id)
    }

    /// 给新供应商 / 路由组生成不重复的短名：按名字转小写、空白换成 `-`，重名时加 `-2`、`-3`。
    /// `group` 留给路由组前缀，供应商不能用。
    pub fn unique_slug(&self, name: &str, for_group: bool) -> String {
        let base = slugify(name);
        let base = if !for_group && base == "group" {
            "group-provider".to_string()
        } else {
            base
        };
        let taken = |candidate: &str| {
            if for_group {
                self.groups.iter().any(|group| group.id == candidate)
            } else {
                self.providers
                    .iter()
                    .any(|provider| provider.relay.id == candidate)
            }
        };
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|index| format!("{base}-{index}"))
            .find(|candidate| !taken(candidate))
            .expect("无限序列总能找到空位")
    }

    pub fn binding(&self, agent: &ToolId) -> Option<&AgentBinding> {
        self.bindings.get(agent.as_str())
    }

    pub fn active_binding(&self, agent: &ToolId) -> Option<&AgentBinding> {
        self.binding(agent).filter(|binding| binding.enabled)
    }

    pub fn any_enabled(&self) -> bool {
        self.bindings.values().any(|binding| binding.enabled)
    }
}

/// 网关状态文件的位置。命令层用 [`GatewayStore::default_path`]，测试指向临时目录。
#[derive(Debug, Clone)]
pub struct GatewayStore {
    pub path: PathBuf,
}

impl Default for GatewayStore {
    fn default() -> Self {
        Self {
            path: Self::default_path(),
        }
    }
}

impl GatewayStore {
    pub fn default_path() -> PathBuf {
        crate::paths::default_app_state_dir().join(STORE_FILE)
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 读状态；文件不存在或为空视作全部关闭。
    pub fn load(&self) -> anyhow::Result<GatewaySettings> {
        read_store(&self.path)
    }

    pub fn save(&self, settings: &GatewaySettings) -> anyhow::Result<()> {
        write_store(&self.path, settings)
    }
}

fn read_store(path: &Path) -> anyhow::Result<GatewaySettings> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GatewaySettings::default());
        }
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    if text.trim().is_empty() {
        return Ok(GatewaySettings::default());
    }
    let mut settings: GatewaySettings =
        serde_json::from_str(&text).with_context(|| format!("无法解析 {}", path.display()))?;
    // 地址、默认模型、Key 落盘时存在 config / auth 文本里，读回来要像 settings.json 一样还原出来。
    for provider in &mut settings.providers {
        let mut normalized = provider.relay.clone();
        if crate::relay_config::normalize_relay_profile_for_storage(&mut normalized).is_ok() {
            provider.relay = normalized;
        }
        // 发请求时只认网关自己的代理设置，不按 id 去查直连模式的代理表。
        provider.relay.gateway_proxy = Some(provider.proxy.clone());
    }
    Ok(settings)
}

fn write_store(path: &Path, settings: &GatewaySettings) -> anyhow::Result<()> {
    let mut settings = settings.clone();
    settings.version = STORE_VERSION;
    let existed = path.is_file();
    let mut text = serde_json::to_string_pretty(&settings)?;
    text.push('\n');
    crate::settings::atomic_write(path, text.as_bytes())
        .with_context(|| format!("无法写入 {}", path.display()))?;
    // 里面有本地 token：新建的文件只给本人读写。
    if !existed {
        set_private(path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("无法设置权限 {}", path.display()))
}

#[cfg(not(unix))]
fn set_private(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

/// 名字 → 短名：小写，空白、`_`、`/` 换成 `-`，去掉首尾 `-`；空了用 `unnamed`。
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_whitespace() || ch == '_' || ch == '/' {
            if !slug.ends_with('-') {
                slug.push('-');
            }
        } else {
            slug.push(ch);
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "unnamed".to_string()
    } else {
        slug.to_string()
    }
}

/// 为 Agent 签发一个新的本地 token。
pub fn new_client_token(agent: &ToolId) -> String {
    format!(
        "{CLIENT_TOKEN_PREFIX}{}-{}",
        agent.as_str(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 常量时间比较，避免按响应时间猜 token。
pub fn token_matches(expected: &str, provided: &str) -> bool {
    let expected = expected.as_bytes();
    let provided = provided.as_bytes();
    if expected.is_empty() || expected.len() != provided.len() {
        return false;
    }
    expected
        .iter()
        .zip(provided)
        .fold(0_u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reads_as_all_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let settings = store.load().unwrap();
        assert!(!settings.any_enabled());
        assert!(settings.tray_panel, "托盘快捷面板默认开");
        let old: GatewaySettings = serde_json::from_str("{}").unwrap();
        assert!(old.tray_panel, "旧文件没有这个字段时也是开");
        assert!(settings.active_binding(&ToolId::Claude).is_none());
    }

    #[test]
    fn round_trip_keeps_bindings_and_writes_version() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut settings = GatewaySettings::default();
        settings.bindings.insert(
            "claude".to_string(),
            AgentBinding {
                enabled: true,
                model: "deepseek-v4-pro".to_string(),
                token: new_client_token(&ToolId::Claude),
                ..AgentBinding::default()
            },
        );
        store.save(&settings).unwrap();
        let text = std::fs::read_to_string(&store.path).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        let loaded = store.load().unwrap();
        assert_eq!(loaded.bindings, settings.bindings);
        assert!(loaded.any_enabled());
    }

    #[test]
    fn declared_models_strip_window_suffix_and_dedupe() {
        let binding = AgentBinding {
            model: "deepseek-v4-pro[1M]".to_string(),
            extra_models: vec!["deepseek-v4-pro".to_string(), "glm-5.3".to_string()],
            claude_roles: ClaudeRoleModels {
                haiku: "deepseek-v4-flash".to_string(),
                ..ClaudeRoleModels::default()
            },
            ..AgentBinding::default()
        };
        assert_eq!(
            binding.declared_models(),
            vec!["deepseek-v4-pro", "deepseek-v4-flash", "glm-5.3"]
        );
    }

    #[test]
    fn older_files_with_codex_references_still_load() {
        // 早期版本把绑定指向 Codex 的供应商 / 聚合，现在这些字段直接忽略。
        let text = r#"{"version":1,"bindings":{"claude":{"enabled":true,"target":{"kind":"group","id":"x"},"model":"m","token":"t"}},"pool":["a"]}"#;
        let settings: GatewaySettings = serde_json::from_str(text).unwrap();
        assert!(settings.any_enabled());
        assert!(settings.providers.is_empty());
    }

    #[test]
    fn slugs_are_unique_and_keep_group_prefix_free() {
        let mut settings = GatewaySettings::default();
        assert_eq!(settings.unique_slug("Daily coding", true), "daily-coding");
        assert_eq!(settings.unique_slug(" 哈基米 国模 ", false), "哈基米-国模");
        assert_eq!(settings.unique_slug("Group", false), "group-provider");
        settings.providers.push(GatewayProvider {
            enabled: true,
            source: String::new(),
            price: None,
            proxy: String::new(),
            relay: RelayProfile {
                id: "deepseek".to_string(),
                ..RelayProfile::default()
            },
        });
        assert_eq!(settings.unique_slug("DeepSeek", false), "deepseek-2");
        assert_eq!(settings.unique_slug("", true), "unnamed");
    }

    #[test]
    fn token_comparison_requires_exact_match() {
        let token = new_client_token(&ToolId::Claude);
        assert!(token.starts_with("cpx-gw-claude-"));
        assert!(token_matches(&token, &token.clone()));
        assert!(!token_matches(&token, "cpx-gw-claude-x"));
        assert!(!token_matches("", ""));
    }
}
