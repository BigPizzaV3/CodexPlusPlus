//! 网关模式的持久状态：`<状态目录>/gateway.json`。
//!
//! 网关模式与原来的直连模式完全独立：供应商池、路由组都只存在这里，
//! 不读也不写 `settings.json` 里 Codex / Claude / Grok 各自的供应商。从直连模式「导入」是复制一份，
//! 之后两边互不影响。文件里有上游 Key 和本地 token，新建时只给本人读写。

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::settings::RelayProfile;
use crate::tools::ToolId;

const STORE_FILE: &str = "gateway.json";
/// 3：`auto_models` 默认改为开启；4：`restore_on_exit` 默认改为关闭（见 [`read_store`] 里的迁移）。
const STORE_VERSION: u32 = 4;
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
    /// 每百万 token 的价格，填了才在「用量」里估算费用。按模型的价格见 `model_prices`，
    /// 这里是没单独填价的模型用的默认价（旧版只有这一组）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price: Option<ProviderPrice>,
    /// 按模型的价格：键是上游模型名（不带供应商前缀）。同一家不同模型价格往往差好几倍。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_prices: BTreeMap<String, ProviderPrice>,
    /// 按模型的上下文窗口（token）：键是上游模型名（不带供应商前缀）。写进 Agent 的模型资料，
    /// 路由时请求比窗口还长的成员排到最后。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_windows: BTreeMap<String, u64>,
    /// 没单独写窗口的模型用的窗口；都不填就交给模型自带的资料。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// 这个供应商单独走的 HTTP 代理；空串表示不单独走代理（沿用系统代理）。
    /// 与直连模式的「供应商独立代理」互不相干。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy: String,
    /// 上次检查上游时，上游有、列表里没有的模型（界面提示「有新模型」，保存模型列表后清掉）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_new: Vec<String>,
    /// 上次检查上游时，列表里有、上游已经没有的模型。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upstream_gone: Vec<String>,
    /// 上次检查上游模型的时间（毫秒），0 表示没检查过。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub models_checked_at_ms: u64,
    pub relay: RelayProfile,
    /// 不认识的字段（更新版本写的）原样保留，保存时写回，回滚后再升级不丢数据。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl GatewayProvider {
    /// 某个上游模型的价格：先按模型找（忽略大小写和 `[1M]` 这类窗口后缀），找不到用默认价。
    pub fn price_for(&self, model: &str) -> Option<&ProviderPrice> {
        let (slug, _) = crate::model_suffix::parse_model_suffix(model.trim());
        let slug = slug.trim();
        self.model_prices
            .iter()
            .find(|(name, _)| {
                let (name, _) = crate::model_suffix::parse_model_suffix(name.trim());
                name.trim().eq_ignore_ascii_case(slug)
            })
            .map(|(_, price)| price)
            .or(self.price.as_ref())
    }
}

impl GatewayProvider {
    /// 某个上游模型的窗口：单独写的优先（忽略大小写和 `[1M]` 后缀），其次是这家的默认窗口；
    /// 都没有返回 None（交给模型自带的资料）。
    pub fn window_for(&self, model: &str) -> Option<u64> {
        let (slug, _) = crate::model_suffix::parse_model_suffix(model.trim());
        let slug = slug.trim();
        self.model_windows
            .iter()
            .find(|(name, _)| {
                crate::model_suffix::parse_model_suffix(name.trim())
                    .0
                    .trim()
                    .eq_ignore_ascii_case(slug)
            })
            .map(|(_, window)| *window)
            .or(self.context_window)
            .filter(|window| *window > 0)
    }

    /// 上游配置里带的窗口（从直连模式导入的、或模型列表里写了 `[1M]` 后缀的）收进网关自己的
    /// 字段，再从上游配置里清掉：之后只认网关字段，界面上删掉的窗口不会被旧记录带回来。
    pub fn adopt_relay_windows(&mut self) {
        let legacy: BTreeMap<String, String> =
            serde_json::from_str(&self.relay.model_windows).unwrap_or_default();
        for (model, token) in legacy {
            let Some(window) = crate::model_suffix::parse_window_token(&token) else {
                continue;
            };
            let (slug, _) = crate::model_suffix::parse_model_suffix(model.trim());
            if !self
                .model_windows
                .keys()
                .any(|known| known.eq_ignore_ascii_case(slug.trim()))
            {
                self.model_windows.insert(slug.trim().to_string(), window);
            }
        }
        if self.context_window.is_none() {
            self.context_window =
                crate::model_suffix::parse_window_token(&self.relay.context_window);
        }
        self.relay.model_windows.clear();
        self.relay.context_window.clear();
    }
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

/// 宽松读取枚举：认不出的取值（更新版本加的新选项）按默认值处理，不让整份 gateway.json 读失败。
/// 原值在保存时由 [`preserve_raw_unknowns`] 写回，不会被默认值覆盖掉。
///
/// 没有用 `#[serde(other)]` 兜底变体：新增变体会让各处按枚举穷举的 `match` 都要改，
/// 而且写回时只能写成兜底变体的名字，原值还是丢了。
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
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
    /// 额度优先：订阅账号里剩余额度最多的先试；没有额度数据的排在有余量的后面，用完的排最后。
    Usage,
    /// 速度优先：最近响应最快的先试；还没测过的先试一次。
    Fast,
    /// 手动：只发给选中的那个成员（`pick`，没选时是第一个），它答不了也不换。
    Manual,
}

/// 同一个会话在组里固定用哪个成员：固定才能命中上游的提示词缓存。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupSticky {
    /// 缓存有效期内固定：上次回答的成员一段时间内（见 `route::CACHE_STICKY_TTL`）继续先试。
    #[default]
    Cache,
    /// 整个会话固定：会话里一直先试上次回答的成员，除非它答不了。
    Session,
    /// 不固定：每个请求按模式重新排。
    Off,
}

/// 模型组成员：`<供应商 id>/<模型>`、裸模型名（池里所有提供它的供应商）、`group/<组 id>`（嵌套），
/// 或带 `*` 的通配符（按 `<供应商 id>/<模型>` 匹配池里现有的模型，上游新增的模型自动算进来）。
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayGroup {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "lenient")]
    pub mode: GroupMode,
    #[serde(default, deserialize_with = "lenient")]
    pub sticky: GroupSticky,
    #[serde(default)]
    pub members: Vec<GroupMember>,
    /// 隐藏成员：组里的模型不再单独出现在 Agent 的模型列表里，只能通过这个组来选（直接写名字仍能用）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_members: bool,
    /// 手动模式下选中的成员（成员的模型名）；空串表示第一个成员。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pick: String,
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl GatewayGroup {
    pub fn model_id(&self) -> String {
        format!("{GROUP_MODEL_PREFIX}{}", self.id)
    }
}

/// 自动组上用户改过的设置。成员不存：总是池里提供这个模型的那几家，按池顺序。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoGroupSettings {
    #[serde(default, deserialize_with = "lenient")]
    pub mode: GroupMode,
    #[serde(default, deserialize_with = "lenient")]
    pub sticky: GroupSticky,
    /// 隐藏这个自动组：不出现在 Agent 的模型列表里，不带供应商的模型名也不再走它（回到按池顺序依次试）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hide_members: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pick: String,
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
    /// 退出 / 更新前还原成直连后，Agent 配置里连的地址。下次启动重新接回网关前比对：
    /// 变了说明用户在这期间用别的工具改过，不再接回（会盖掉用户的新选择）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restored_endpoint: Option<String>,
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
    #[serde(default, deserialize_with = "lenient")]
    pub unknown_model: UnknownModelPolicy,
    /// 审批模型（#2703，目前只有 Codex 用）：Codex「替我审批」固定请求 `codex-auto-review`，
    /// 默认按「未知模型」策略改写成主模型；主模型是 ChatGPT 账号或含账号的模型组时会消耗订阅额度。
    /// 填了就把审批请求改发到这个模型（`<供应商 id>/<模型>`、裸模型名或 `group/<组 id>`）。
    /// 空串表示未设置，保持原有行为。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub auto_review_model: String,
    /// 起名模型（目前只有 Codex 用）：Codex 给会话起名 / 定期重新起名时另开临时线程，
    /// 用它内置的轻量模型（如 gpt-6-luna）或会话当前模型，池里有 ChatGPT 账号时会落到订阅额度上。
    /// 填了就把识别出的起名请求改发到这个模型（写法同审批模型）；空串表示不改，保持原有行为。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title_model: String,
    /// 写给 Agent 的上下文窗口（Claude Code 的 `CLAUDE_CODE_MAX_CONTEXT_TOKENS`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// 非 claude-* 模型名时 Claude Code 默认不发 effort，打开后强制启用。
    #[serde(default)]
    pub claude_enable_effort: bool,
    /// 这个 Agent 不使用的池内供应商（「挑选」里取消勾选的）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pool_exclude: Vec<String>,
    /// 往池里添加供应商时，默认也给这个 Agent 用；关掉后新供应商默认记进 `pool_exclude`。
    #[serde(default = "enabled_by_default")]
    pub auto_join_providers: bool,
    /// 网关签发给该 Agent 的本地 token，用来鉴权本地请求。
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub restore: GatewayRestoreState,
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for AgentBinding {
    fn default() -> Self {
        Self {
            enabled: false,
            model: String::new(),
            extra_models: Vec::new(),
            claude_roles: ClaudeRoleModels::default(),
            unknown_model: UnknownModelPolicy::default(),
            auto_review_model: String::new(),
            title_model: String::new(),
            context_window: None,
            claude_enable_effort: false,
            pool_exclude: Vec::new(),
            auto_join_providers: true,
            token: String::new(),
            restore: GatewayRestoreState::default(),
            extra: Map::new(),
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
    #[serde(default, deserialize_with = "lenient")]
    pub mode: KeyRotationMode,
    /// 不认识的字段原样保留。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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
    ///
    /// 默认关：Agent 一直指着网关，下次打开 Codex++ 就能接着用，不用每次退出、启动都改一遍 Agent 的配置。
    /// 安装更新前不看这个开关，照样还原（见 [`crate::gateway::apply::restore_all_for_exit`]）。
    /// 2.0.0 发布前的开发版默认开、文件里总会写出 `true`，读的时候统一迁移成关，见 [`read_store`]。
    #[serde(default)]
    pub restore_on_exit: bool,
    /// 退出或安装更新前已经把开着的 Agent 恢复成直连（绑定仍是开启）：下次启动要重新接回网关。
    #[serde(default, skip_serializing_if = "is_false")]
    pub restored_at_exit: bool,
    /// 网关的供应商池（顺序即同一模型多家时的尝试顺序）。
    #[serde(default)]
    pub providers: Vec<GatewayProvider>,
    /// 网关的路由组。
    #[serde(default)]
    pub groups: Vec<GatewayGroup>,
    /// 左键点托盘图标弹出快捷面板（默认开）；关掉时托盘保持原来的点击行为。
    #[serde(default = "enabled_by_default")]
    pub tray_panel: bool,
    /// 关闭管理器主窗口时进入轻量模式：销毁主窗口（连同 WebView 渲染进程）释放内存，
    /// 后端、网关、托盘照常运行，再打开时按原配置重建窗口（#2677）。
    /// 默认关（opt-in）：关时是只隐藏窗口的旧行为。和 `tray_panel` 一样放在这里，由管理器的专用命令读写。
    #[serde(default)]
    pub lightweight_on_close: bool,
    /// 自动建组：多家都有的模型自动成一个组（Agent 选不带供应商的模型名时走它）。默认开；
    /// 旧文件（version < 3）里的 false 是当时的默认值，读的时候迁移成开，见 [`read_store`]。
    #[serde(default = "enabled_by_default")]
    pub auto_models: bool,
    /// 多家都有的模型，在 Codex 的模型列表和输入框里名字后面带不带「（供应商）」。默认带。
    /// 关掉时名字后面换成看不见的字符：分组脚本照样能分清是哪一家，界面上只显示模型名。
    #[serde(default = "enabled_by_default")]
    pub model_suffix: bool,
    /// 自动组上改过的设置（键是小写模型名）。没改过的自动组不落盘，跟着供应商池自动变化。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub auto_groups: BTreeMap<String, AutoGroupSettings>,
    /// 收藏的模型（管理器模型下拉里点的星标）：写给 Codex 的模型目录时标出来，Codex 的选择器里也显示星标。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub favorite_models: Vec<String>,
    /// 各 Agent 的生图模型（键是 Agent id）：没有或空串是自动（见 [`crate::gateway::images`]），
    /// `off` 是关闭，其余是 `<供应商 id>/<模型>`。不放在 [`AgentBinding`] 里：界面保存绑定时整份替换。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_models: BTreeMap<String, String>,
    /// 生图结果只给链接时原样转发、不下载成图片数据的 Agent（默认都转，见 [`Self::inline_image_urls_for`]）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_keep_urls: Vec<String>,
    /// 旧版的全局生图模型，读的时候迁移给 Codex（见 [`read_store`]），之后不再写。
    #[serde(default, skip_serializing)]
    pub image_model: String,
    /// 最后写这个文件的 Codex++ 版本。降级到旧版时旧版不会跑这里的代码，只留个记录方便排查。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app_version: String,
    /// 不认识的字段（更新版本写的）原样保留，保存时写回。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            version: 0,
            bindings: BTreeMap::new(),
            key_policy: KeyPolicy::default(),
            restore_on_exit: false,
            restored_at_exit: false,
            providers: Vec::new(),
            groups: Vec::new(),
            tray_panel: true,
            lightweight_on_close: false,
            auto_models: true,
            model_suffix: true,
            auto_groups: BTreeMap::new(),
            favorite_models: Vec::new(),
            image_models: BTreeMap::new(),
            image_keep_urls: Vec::new(),
            image_model: String::new(),
            app_version: String::new(),
            extra: Map::new(),
        }
    }
}

impl GatewaySettings {
    pub fn provider(&self, id: &str) -> Option<&GatewayProvider> {
        self.providers
            .iter()
            .find(|provider| provider.relay.id == id)
    }

    /// 某个 Agent 的生图模型设置（空串是自动）。
    pub fn image_model_for(&self, agent: &ToolId) -> &str {
        self.image_models
            .get(agent.as_str())
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// 这个 Agent 的生图结果只给链接时，要不要下载下来补成 `b64_json`。
    pub fn inline_image_urls_for(&self, agent: &ToolId) -> bool {
        !self.image_keep_urls.iter().any(|id| id == agent.as_str())
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
        let _guard = self.lock();
        write_store(&self.path, settings)
    }

    /// 带锁的读-改-写：同一进程里对同一个文件的 `update` 依次进行，后台任务（检查模型、
    /// 刷新登录 token）与界面命令同时改时不会互相覆盖。`edit` 出错时不写盘；改完和读到的
    /// 一样时也不写盘。返回改后的状态与 `edit` 的返回值。
    ///
    /// `edit` 里不能再调用这个 store 的 `save` / `update`（同一把锁，会死锁）。
    pub fn update<T>(
        &self,
        edit: impl FnOnce(&mut GatewaySettings) -> anyhow::Result<T>,
    ) -> anyhow::Result<(GatewaySettings, T)> {
        let _guard = self.lock();
        let before = read_store(&self.path)?;
        let mut settings = before.clone();
        let value = edit(&mut settings)?;
        if settings != before {
            write_store(&self.path, &settings)?;
        }
        Ok((settings, value))
    }

    /// 这个文件在进程内的写锁。按路径区分，测试里各自的临时文件互不阻塞。
    fn lock(&self) -> std::sync::MutexGuard<'static, ()> {
        static LOCKS: OnceLock<Mutex<HashMap<PathBuf, &'static Mutex<()>>>> = OnceLock::new();
        let lock: &'static Mutex<()> = {
            let mut locks = LOCKS
                .get_or_init(Mutex::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            locks
                .entry(self.path.clone())
                // 每个路径只泄漏一把锁，进程里用到的 gateway.json 路径屈指可数。
                .or_insert_with(|| Box::leak(Box::new(Mutex::new(()))))
        };
        lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
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
    // 版本 4 之前只有发布前的开发版，`restoreOnExit` 总会写出来，是当时的默认值（开）而不是用户选的：
    // 统一迁移成关；用户想要，再打开一次即可（之后的文件是版本 4，不再迁移）。
    if settings.version < 4 {
        settings.restore_on_exit = false;
    }
    // 版本 2 的 `autoModels: false` 是那时「多家自动切换」的默认值；自动建组改成默认开以后统一迁移成开。
    if settings.version < 3 {
        settings.auto_models = true;
    }
    // 旧版只有一个全局生图模型，当时只有 Codex 会发生图请求：迁移成 Codex 的。
    let legacy_image = std::mem::take(&mut settings.image_model);
    if !legacy_image.trim().is_empty() {
        settings
            .image_models
            .entry(ToolId::Codex.as_str().to_string())
            .or_insert(legacy_image);
    }
    // 地址、默认模型、Key 落盘时存在 config / auth 文本里，读回来要像 settings.json 一样还原出来。
    for provider in &mut settings.providers {
        let mut normalized = provider.relay.clone();
        if crate::relay_config::normalize_relay_profile_for_storage(&mut normalized).is_ok() {
            provider.relay = normalized;
        }
        // 导入或手写 `[1M]` 带进上游配置的窗口，收进网关自己的按模型窗口。
        provider.adopt_relay_windows();
        // 发请求时只认网关自己的代理设置，不按 id 去查直连模式的代理表。
        provider.relay.gateway_proxy = Some(provider.proxy.clone());
    }
    Ok(settings)
}

fn write_store(path: &Path, settings: &GatewaySettings) -> anyhow::Result<()> {
    let mut settings = settings.clone();
    settings.version = STORE_VERSION;
    settings.app_version = crate::version::VERSION.to_string();
    let existed = path.is_file();
    let mut value = serde_json::to_value(&settings)?;
    // 磁盘上原有的、这一版不认识的取值（供应商配置里的新字段、枚举的新选项）照原样带回去。
    if let Some(old) = fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
    {
        preserve_raw_unknowns(&mut value, &old);
    }
    let mut text = serde_json::to_string_pretty(&value)?;
    text.push('\n');
    crate::settings::atomic_write(path, text.as_bytes())
        .with_context(|| format!("无法写入 {}", path.display()))?;
    // 里面有本地 token：新建的文件只给本人读写。
    if !existed {
        set_private(path)?;
    }
    Ok(())
}

/// 结构体自己的未知字段由 `extra` 保留；这里补上两类 `extra` 管不到的：
/// - 供应商的 `relay`（[`RelayProfile`]，与直连模式共用的结构）里不认识的字段；
/// - 枚举字段里不认识的取值：读的时候按默认值处理了，只要还是默认值（用户没改过）就写回原值。
fn preserve_raw_unknowns(new: &mut Value, old: &Value) {
    // 供应商：按 `relay.id` 对上。
    let old_providers = old.get("providers").and_then(Value::as_array);
    if let (Some(providers), Some(old_providers)) = (
        new.get_mut("providers").and_then(Value::as_array_mut),
        old_providers,
    ) {
        for provider in providers {
            let Some(id) = provider.pointer("/relay/id").and_then(Value::as_str) else {
                continue;
            };
            let Some(old_relay) = old_providers
                .iter()
                .find(|old| old.pointer("/relay/id").and_then(Value::as_str) == Some(id))
                .and_then(|old| old.get("relay"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            let Some(relay) = provider.get_mut("relay").and_then(Value::as_object_mut) else {
                continue;
            };
            for (key, value) in old_relay {
                if !relay.contains_key(key)
                    && !crate::settings::RELAY_PROFILE_KNOWN_FIELDS.contains(&key.as_str())
                {
                    relay.insert(key.clone(), value.clone());
                }
            }
        }
    }
    // 模型组：按 `id` 对上。
    let old_groups = old.get("groups").and_then(Value::as_array);
    if let (Some(groups), Some(old_groups)) = (
        new.get_mut("groups").and_then(Value::as_array_mut),
        old_groups,
    ) {
        for group in groups {
            let id = group.get("id").cloned();
            let Some(old_group) = old_groups.iter().find(|old| old.get("id") == id.as_ref()) else {
                continue;
            };
            keep_unknown_enum::<GroupMode>(group, old_group, "mode");
            keep_unknown_enum::<GroupSticky>(group, old_group, "sticky");
        }
    }
    if let (Some(bindings), Some(old_bindings)) = (
        new.get_mut("bindings").and_then(Value::as_object_mut),
        old.get("bindings").and_then(Value::as_object),
    ) {
        for (agent, binding) in bindings {
            if let Some(old_binding) = old_bindings.get(agent) {
                keep_unknown_enum::<UnknownModelPolicy>(binding, old_binding, "unknownModel");
            }
        }
    }
    if let (Some(policy), Some(old_policy)) = (new.get_mut("keyPolicy"), old.get("keyPolicy")) {
        keep_unknown_enum::<KeyRotationMode>(policy, old_policy, "mode");
    }
}

/// `old[key]` 是这一版认不出的取值、`new[key]` 还是默认值时，写回原值。
fn keep_unknown_enum<T>(new: &mut Value, old: &Value, key: &str)
where
    T: DeserializeOwned + Serialize + Default,
{
    let Some(old_value) = old.get(key) else {
        return;
    };
    if serde_json::from_value::<T>(old_value.clone()).is_ok() {
        return;
    }
    let default = serde_json::to_value(T::default()).unwrap_or(Value::Null);
    if let Some(object) = new.as_object_mut()
        && object.get(key).is_none_or(|value| *value == default)
    {
        object.insert(key.to_string(), old_value.clone());
    }
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
    fn legacy_global_image_model_moves_to_codex() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("gateway.json");
        fs::write(&path, r#"{ "version": 3, "imageModel": "jo/gpt-image-2" }"#).unwrap();
        let store = GatewayStore::at(path.clone());
        let settings = store.load().unwrap();
        assert_eq!(settings.image_model_for(&ToolId::Codex), "jo/gpt-image-2");
        assert_eq!(settings.image_model_for(&ToolId::Grok), "");
        assert!(settings.inline_image_urls_for(&ToolId::Codex));
        store.save(&settings).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("\"imageModel\""), "旧字段不再写：{text}");
        assert!(text.contains("\"imageModels\""), "{text}");
    }

    #[test]
    fn missing_file_reads_as_all_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let settings = store.load().unwrap();
        assert!(!settings.any_enabled());
        assert!(settings.tray_panel, "托盘快捷面板默认开");
        let old: GatewaySettings = serde_json::from_str("{}").unwrap();
        assert!(old.tray_panel, "旧文件没有这个字段时也是开");
        assert!(!settings.lightweight_on_close, "关窗进入轻量模式默认关（opt-in）");
        assert!(!old.lightweight_on_close, "旧文件没有这个字段时也是关");
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
        assert!(text.contains("\"version\": 4"), "{text}");
        assert!(
            text.contains(&format!("\"appVersion\": \"{}\"", crate::version::VERSION)),
            "记下写文件的版本：{text}"
        );
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
            model_prices: Default::default(),
            model_windows: Default::default(),
            context_window: None,
            proxy: String::new(),
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            relay: RelayProfile {
                id: "deepseek".to_string(),
                ..RelayProfile::default()
            },
            extra: Default::default(),
        });
        assert_eq!(settings.unique_slug("DeepSeek", false), "deepseek-2");
        assert_eq!(settings.unique_slug("", true), "unnamed");
    }

    /// 回滚到这一版后：更新版本写的未知字段、枚举新取值都要能读，保存后原样还在。
    #[test]
    fn unknown_fields_and_enum_values_survive_load_and_save() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let text = r#"{
          "version": 7,
          "restoreOnExit": false,
          "futureTopLevel": {"a": 1},
          "keyPolicy": {"mode": "leastUsed", "futurePolicy": true},
          "bindings": {
            "claude": {
              "enabled": true,
              "model": "m",
              "token": "t",
              "unknownModel": "askUser",
              "futureBinding": [1, 2],
              "claudeRoles": {"opus": "o", "futureRole": "x"},
              "restore": {"appliedPort": 1, "futureRestore": "y"}
            }
          },
          "providers": [{
            "enabled": true,
            "futureProvider": "keep",
            "relay": {"id": "p", "name": "P", "futureRelayField": {"deep": true}}
          }],
          "groups": [
            {"id": "g", "name": "G", "mode": "smartest", "sticky": "forever", "futureGroup": 3, "members": []},
            {"id": "h", "name": "H", "mode": "rotate", "sticky": "off"}
          ]
        }"#;
        std::fs::write(&store.path, text).unwrap();
        let mut settings = store.load().unwrap();
        // 认不出的取值按默认处理。
        assert_eq!(settings.groups[0].mode, GroupMode::InOrder);
        assert_eq!(settings.groups[0].sticky, GroupSticky::Cache);
        assert_eq!(settings.groups[1].mode, GroupMode::Rotate);
        assert_eq!(settings.key_policy.mode, KeyRotationMode::Sticky);
        let binding = settings.binding(&ToolId::Claude).unwrap();
        assert_eq!(binding.unknown_model, UnknownModelPolicy::MapToDefault);
        assert!(
            !settings.restore_on_exit,
            "版本 4 之后的文件里显式关掉的保持关"
        );

        // 改一点别的再保存。
        settings.groups[1].mode = GroupMode::Weighted;
        settings.tray_panel = false;
        settings.lightweight_on_close = true;
        store.save(&settings).unwrap();
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&store.path).unwrap()).unwrap();
        assert_eq!(saved["futureTopLevel"]["a"], 1);
        assert_eq!(saved["keyPolicy"]["mode"], "leastUsed");
        assert_eq!(saved["keyPolicy"]["futurePolicy"], true);
        let claude = &saved["bindings"]["claude"];
        assert_eq!(claude["unknownModel"], "askUser");
        assert_eq!(claude["futureBinding"][1], 2);
        assert_eq!(claude["claudeRoles"]["futureRole"], "x");
        assert_eq!(claude["restore"]["futureRestore"], "y");
        assert_eq!(saved["providers"][0]["futureProvider"], "keep");
        assert_eq!(
            saved["providers"][0]["relay"]["futureRelayField"]["deep"],
            true
        );
        assert_eq!(saved["groups"][0]["mode"], "smartest");
        assert_eq!(saved["groups"][0]["sticky"], "forever");
        assert_eq!(saved["groups"][0]["futureGroup"], 3);
        assert_eq!(saved["groups"][1]["mode"], "weighted", "改过的照新值写");
        assert_eq!(saved["trayPanel"], false);
        assert_eq!(saved["lightweightOnClose"], true);
        // 再读一遍还是同样的内容。
        let again = store.load().unwrap();
        assert_eq!(again.groups[0].extra["futureGroup"], 3);
    }

    #[test]
    fn restore_on_exit_defaults_off_and_old_files_migrate() {
        assert!(!GatewaySettings::default().restore_on_exit);
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        // 版本 4 之前的文件总会写出 true（开发版的默认值），读的时候迁移成关。
        std::fs::write(&store.path, r#"{"version":3,"restoreOnExit":true}"#).unwrap();
        assert!(!store.load().unwrap().restore_on_exit);
        std::fs::write(&store.path, r#"{"bindings":{}}"#).unwrap();
        assert!(!store.load().unwrap().restore_on_exit);
        // 迁移后用户再打开：存成版本 4，之后保持开。
        let mut settings = store.load().unwrap();
        settings.restore_on_exit = true;
        store.save(&settings).unwrap();
        assert!(store.load().unwrap().restore_on_exit);
    }

    #[test]
    fn auto_groups_default_on_and_version_two_files_migrate() {
        assert!(GatewaySettings::default().auto_models);
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        // 版本 2 写的 false 是当时「多家自动切换」的默认值：迁移成开。
        std::fs::write(&store.path, r#"{"version":2,"autoModels":false}"#).unwrap();
        assert!(store.load().unwrap().auto_models);
        // 迁移后用户再关掉：存成版本 3，之后保持关。
        let mut settings = store.load().unwrap();
        settings.auto_models = false;
        store.save(&settings).unwrap();
        assert!(!store.load().unwrap().auto_models);
    }

    #[test]
    fn update_serializes_concurrent_writers() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let threads: Vec<_> = (0..8)
            .map(|thread| {
                let store = store.clone();
                std::thread::spawn(move || {
                    for index in 0..10 {
                        store
                            .update(|gateway| {
                                gateway.groups.push(GatewayGroup {
                                    id: format!("g-{thread}-{index}"),
                                    name: String::new(),
                                    mode: GroupMode::default(),
                                    sticky: GroupSticky::default(),
                                    members: Vec::new(),
                                    ..GatewayGroup::default()
                                });
                                Ok(())
                            })
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(store.load().unwrap().groups.len(), 80, "并发写入一条都不丢");

        // 出错时不写盘。
        let failed = store.update(|gateway| -> anyhow::Result<()> {
            gateway.groups.clear();
            anyhow::bail!("不保存")
        });
        assert!(failed.is_err());
        assert_eq!(store.load().unwrap().groups.len(), 80);
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
