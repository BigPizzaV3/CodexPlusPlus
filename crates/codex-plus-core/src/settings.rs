use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;
use sha2::{Digest, Sha256};
use serde::Deserialize;
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item};

use crate::tools::{ToolConfig, ToolId};

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayProfile {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing)]
    pub model: String,
    #[serde(default = "default_relay_base_url", skip_serializing)]
    pub base_url: String,
    #[serde(rename = "upstreamBaseUrl", default)]
    pub upstream_base_url: String,
    #[serde(
        default,
        skip_serializing,
        deserialize_with = "deserialize_profile_api_key"
    )]
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub api_keys: Vec<RelayApiKey>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub active_api_key_id: String,
    #[serde(default)]
    pub protocol: RelayProtocol,
    #[serde(rename = "relayMode", default)]
    pub relay_mode: RelayMode,
    #[serde(rename = "officialMixApiKey", default)]
    pub official_mix_api_key: bool,
    #[serde(rename = "noAuth", default)]
    pub no_auth: bool,
    #[serde(rename = "hideOfficialUsageAlert", default)]
    pub hide_official_usage_alert: bool,
    #[serde(rename = "testModel", default)]
    pub test_model: String,
    #[serde(rename = "configContents", default)]
    pub config_contents: String,
    #[serde(rename = "authContents", default)]
    pub auth_contents: String,
    #[serde(rename = "useCommonConfig", default = "default_true")]
    pub use_common_config: bool,
    #[serde(rename = "contextWindow", default)]
    pub context_window: String,
    #[serde(rename = "autoCompactLimit", default)]
    pub auto_compact_limit: String,
    #[serde(rename = "modelInsertMode", default)]
    pub model_insert_mode: RelayModelInsertMode,
    #[serde(rename = "modelList", default)]
    pub model_list: String,
    #[serde(
        rename = "modelWindows",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub model_windows: String,
    /// 每模型自动压缩百分比（JSON map: slug -> 百分比字符串，如 "90" 或 "90%"）。
    /// 为空时保持 Codex 原有的默认自动压缩行为，不向 catalog 写入覆盖值。
    #[serde(
        rename = "modelAutoCompact",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub model_auto_compact: String,
    /// 每模型元数据覆盖（JSON map: slug -> 字段覆盖）。
    #[serde(
        rename = "modelMetadata",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub model_metadata: String,
    #[serde(rename = "modelVlm", default, skip_serializing_if = "String::is_empty")]
    pub model_vlm: String,
    #[serde(
        rename = "vlmApiKey",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub vlm_api_key: String,
    #[serde(rename = "vlmModel", default)]
    pub vlm_model: String,
    #[serde(rename = "vlmBaseUrl", default)]
    pub vlm_base_url: String,
    #[serde(
        rename = "userAgent",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub user_agent: String,
    #[serde(rename = "sub2apiEnabled", default)]
    pub sub2api_enabled: bool,
    #[serde(
        rename = "sub2apiMultiplier",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub sub2api_multiplier: String,
    #[serde(rename = "modelRoutes", default, skip_serializing_if = "Vec::is_empty")]
    pub model_routes: Vec<RelayModelRoute>,
    /// 自定义上游请求头（有序列表，保住用户写的顺序）。
    /// 传输头（Host / Content-Length 等）与跳头由协议层掌控，写入前会被校验拦下。
    #[serde(
        rename = "customHeaders",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub custom_headers: Vec<RelayHeaderKeyValue>,
    // 上游审查要求：导出 round-trip 不改变既有 provider；为 false 时不写出该字段。
    #[serde(
        rename = "standardOpenaiProtocol",
        default,
        skip_serializing_if = "is_false"
    )]
    pub standard_openai_protocol: bool,
    #[serde(
        rename = "webSearchHistoryCompat",
        default,
        skip_serializing_if = "is_false"
    )]
    pub web_search_history_compat: bool,
    /// 只保留用户指定外部目录中的 Lite 声明，不为未知模型猜测能力。
    #[serde(default, skip_serializing_if = "is_false")]
    pub trust_upstream_catalog_lite: bool,
    /// 明确选择严格 Responses 历史契约；默认不改写历史。
    #[serde(default, skip_serializing_if = "is_false")]
    pub strict_responses_reasoning: bool,
    /// Anthropic 上游的认证字段：`x-api-key`（默认，Anthropic 原生）或
    /// `authorization`（多数 Claude 系中转网关用 Bearer）。
    #[serde(
        rename = "anthropicAuthHeader",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub anthropic_auth_header: String,
    /// Anthropic 的 `max_tokens` 是必填项：Codex 请求没带上限时用这个值兜底。
    /// 一旦填了（非 0），它优先于请求自带的上限。
    #[serde(
        rename = "anthropicMaxTokens",
        default,
        skip_serializing_if = "is_zero_u64"
    )]
    pub anthropic_max_tokens: u64,
    /// 注入 Anthropic 提示缓存断点（`cache_control`），默认开启。
    #[serde(rename = "anthropicPromptCache", default = "default_true")]
    pub anthropic_prompt_cache: bool,
    /// 伪装 Claude Code 客户端：只在网关限制「仅 Claude Code 可用」时才需要。
    #[serde(
        rename = "anthropicImpersonateClaudeCode",
        default,
        skip_serializing_if = "is_false"
    )]
    pub anthropic_impersonate_claude_code: bool,
    /// 思考模式：空/`auto`（新代模型透传等级、旧代回落预算）、`adaptive`（强制透传）、
    /// `budget`（强制经典预算）、`off`（不发 thinking）。
    #[serde(
        rename = "anthropicThinkingMode",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub anthropic_thinking_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayApiKey {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayModelRoute {
    pub model: String,
    #[serde(rename = "targetRelayId")]
    pub target_relay_id: String,
    #[serde(
        rename = "targetModel",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub target_model: String,
}

/// 供应商自定义上游请求头。
///
/// 用有序 Vec 而不是 map：与 MCP 的 env / http_headers 一致，保住用户书写顺序，
/// 也让设置文件的 diff 稳定。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayHeaderKeyValue {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AggregateRelayStrategy {
    #[default]
    Failover,
    ConversationRoundRobin,
    RequestRoundRobin,
    WeightedRoundRobin,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateRelayMember {
    #[serde(rename = "relayId")]
    pub relay_id: String,
    #[serde(default = "default_aggregate_member_weight")]
    pub weight: u32,
}

/// 聚合供应商按模型名路由规则：model 匹配 pattern 时转发到指定成员
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateRelayRoute {
    /// 模型匹配模式，如 "deepseek-*" / "gpt-*" / "*"；仅支持 * 通配符
    pub pattern: String,
    /// 目标聚合成员 relayId（必须是本聚合 members 之一）
    #[serde(rename = "relayId")]
    pub relay_id: String,
    /// 数字越大越优先，缺省 0；同 priority 按数组顺序（稳定优先）
    #[serde(default)]
    pub priority: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum RelaySessionProvider {
    #[default]
    Custom,
    Openai,
}

impl RelaySessionProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Custom => "custom",
            Self::Openai => "openai",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateRelayProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub session_provider: RelaySessionProvider,
    /// 将成员模型合并到 Codex 模型目录，并按模型 ID 固定绑定成员。
    #[serde(rename = "modelAggregationEnabled", default)]
    pub model_aggregation_enabled: bool,
    #[serde(default)]
    pub strategy: AggregateRelayStrategy,
    #[serde(default)]
    pub members: Vec<AggregateRelayMember>,
    #[serde(default)]
    pub routes: Vec<AggregateRelayRoute>,
    /// 默认供应商：解析不到成员的普通模型请求（旧会话/旧目录写下的模型名、
    /// 回合前自动压缩用的模型名等）交给该聚合成员按请求里的模型名转发。
    /// 空串表示未设置，此时保持原有的 400 报错行为。
    #[serde(default)]
    pub default_relay_id: String,
    /// 把默认成员的官方登录同步进 Codex（auth.json 的 `tokens` 段）。
    ///
    /// 聚合的请求都发往本地代理，默认切换只写一个代理 Key，于是 Codex 侧显示"未登录"，
    /// 官方成员的账号态与额度都不在 Codex 里体现。打开这个开关后，切换聚合时会把默认
    /// 成员的 ChatGPT 凭据一并写进 live auth.json（代理 Key 与其它字段保留），账号在
    /// Codex 里就是登录态。默认关：它会把某个账号的凭据写进 Codex 的凭据文件，
    /// 应当由用户显式选择。
    #[serde(rename = "syncDefaultMemberLogin", default)]
    pub sync_default_member_login: bool,
}

impl Default for RelayProfile {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: "默认中转".to_string(),
            model: String::new(),
            base_url: default_relay_base_url(),
            upstream_base_url: String::new(),
            api_key: String::new(),
            api_keys: Vec::new(),
            active_api_key_id: String::new(),
            protocol: RelayProtocol::Responses,
            relay_mode: RelayMode::Official,
            official_mix_api_key: false,
            no_auth: false,
            hide_official_usage_alert: false,
            test_model: String::new(),
            config_contents: String::new(),
            auth_contents: String::new(),
            use_common_config: true,
            context_window: String::new(),
            auto_compact_limit: String::new(),
            model_insert_mode: RelayModelInsertMode::Patch,
            model_list: String::new(),
            model_windows: String::new(),
            model_auto_compact: String::new(),
            model_metadata: String::new(),
            model_vlm: String::new(),
            vlm_api_key: String::new(),
            vlm_model: String::new(),
            vlm_base_url: String::new(),
            user_agent: String::new(),
            sub2api_enabled: false,
            sub2api_multiplier: String::new(),
            model_routes: Vec::new(),
            custom_headers: Vec::new(),
            standard_openai_protocol: false,
            web_search_history_compat: false,
            trust_upstream_catalog_lite: false,
            strict_responses_reasoning: false,
            anthropic_auth_header: String::new(),
            anthropic_max_tokens: 0,
            anthropic_prompt_cache: true,
            anthropic_impersonate_claude_code: false,
            anthropic_thinking_mode: String::new(),
        }
    }
}

impl RelayProfile {
    pub fn selected_api_key(&self) -> Option<&RelayApiKey> {
        self.api_keys.iter().find(|entry| entry.id == self.active_api_key_id)
            .or_else(|| self.api_keys.first())
    }

    pub fn uses_no_auth(&self) -> bool {
        self.relay_mode == RelayMode::PureApi && self.no_auth
    }

    /// 独立代理由本地转发层消费；纯官方 OAuth 不接管其网络请求。
    pub fn uses_independent_proxy(&self) -> bool {
        (self.relay_mode != RelayMode::Official || self.official_mix_api_key)
            && crate::relay_proxy::configured_proxy_for_profile(&self.id).is_some()
    }

    pub fn web_search_history_compat_enabled(&self) -> bool {
        self.web_search_history_compat
            && self.protocol == RelayProtocol::Responses
            && self.relay_mode != RelayMode::Aggregate
            && (self.relay_mode != RelayMode::Official || self.official_mix_api_key)
    }

    pub fn has_model_routes(&self) -> bool {
        self.model_routes
            .iter()
            .any(|route| !route.model.trim().is_empty() && !route.target_relay_id.trim().is_empty())
    }

    pub fn model_reasoning_parameters(&self, model: &str) -> &str {
        // 只接受明确已知策略，旧配置与无效值维持自动兼容。
        let Ok(entries) = serde_json::from_str::<BTreeMap<String, Value>>(&self.model_metadata) else {
            return "auto";
        };
        let Some(metadata) = crate::model_suffix::lookup_model_map(&entries, model) else {
            return "auto";
        };
        match metadata.get("codex_plus_reasoning_parameters").and_then(Value::as_str) {
            Some("omit") => "omit",
            Some("passthrough") => "passthrough",
            _ => "auto",
        }
    }

    pub fn request_compatibility_enabled(&self) -> bool {
        if self.relay_mode == RelayMode::Official && !self.official_mix_api_key {
            return false;
        }
        if self.protocol == RelayProtocol::Responses && self.strict_responses_reasoning {
            return true;
        }
        if !self.model_metadata.contains("codex_plus_reasoning_parameters") {
            return false;
        }
        serde_json::from_str::<Value>(&self.model_metadata).ok().is_some_and(|map| {
            map.as_object().is_some_and(|map| map.values().any(|entry| {
                matches!(entry.get("codex_plus_reasoning_parameters").and_then(Value::as_str),
                    Some("omit" | "passthrough"))
            }))
        })
    }

    /// 上游协议是否需要本地协议代理转换（Codex 侧恒以 Responses 说话）。
    ///
    /// 与「Chat Completions 专有行为」区分开：`standard_openai_protocol`、
    /// 原生远程压缩、VLM 等开关只认 `ChatCompletions`，不要用本函数替代。
    pub fn requires_protocol_proxy_conversion(&self) -> bool {
        matches!(
            self.protocol,
            RelayProtocol::ChatCompletions | RelayProtocol::AnthropicMessages
        )
    }

    /// Anthropic 上游的认证字段，空值表示默认的 `x-api-key`。
    pub fn anthropic_auth_header(&self) -> &str {
        let value = self.anthropic_auth_header.trim();
        if value.is_empty() {
            crate::anthropic_bridge::ANTHROPIC_AUTH_HEADER_API_KEY
        } else {
            value
        }
    }

    /// Anthropic 的思考模式（空值按 `auto`）。
    pub fn anthropic_thinking_mode(&self) -> crate::anthropic_bridge::AnthropicThinkingMode {
        crate::anthropic_bridge::AnthropicThinkingMode::parse(&self.anthropic_thinking_mode)
    }

    /// Anthropic 的 `max_tokens` 兜底值（协议必填，Codex 未给上限时使用）。
    pub fn anthropic_max_tokens(&self) -> u64 {
        if self.anthropic_max_tokens == 0 {
            crate::anthropic_bridge::ANTHROPIC_DEFAULT_MAX_TOKENS
        } else {
            self.anthropic_max_tokens
        }
    }
}

/// 归一化 Anthropic 认证字段：只接受两种取值，其余（含空串）按默认 `x-api-key` 处理。
pub fn normalize_anthropic_auth_header(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "authorization" | "bearer" | "auth_token" | "anthropic_auth_token" => {
            crate::anthropic_bridge::ANTHROPIC_AUTH_HEADER_BEARER.to_string()
        }
        "x-api-key" | "x_api_key" | "api_key" | "anthropic_api_key" => {
            crate::anthropic_bridge::ANTHROPIC_AUTH_HEADER_API_KEY.to_string()
        }
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RelayModelInsertMode {
    ModelCatalog,
    #[default]
    Patch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RelayProtocol {
    #[default]
    Responses,
    ChatCompletions,
    /// Anthropic Messages（`/v1/messages`）：由本地协议代理转成 Responses 供 Codex 使用。
    AnthropicMessages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RelayMode {
    Official,
    #[default]
    MixedApi,
    PureApi,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinColors {
    pub background: String,
    pub panel: String,
    pub panel_alt: String,
    pub accent: String,
    pub accent_alt: String,
    pub secondary: String,
    pub highlight: String,
    pub text: String,
    pub muted: String,
    pub line: String,
}

impl Default for DreamSkinColors {
    fn default() -> Self {
        Self {
            background: "#F7F4F5".to_string(),
            panel: "#FFFFFF".to_string(),
            panel_alt: "#FFF7F8".to_string(),
            accent: "#E25563".to_string(),
            accent_alt: "#F07A86".to_string(),
            secondary: "#F3A8AF".to_string(),
            highlight: "#C93D4C".to_string(),
            text: "#2B2224".to_string(),
            muted: "#8A7A7D".to_string(),
            line: "rgba(196, 120, 128, .22)".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DreamSkinThemeConfig {
    #[serde(default = "default_dream_skin_schema_version")]
    pub schema_version: u8,
    #[serde(default = "default_dream_skin_id")]
    pub id: String,
    #[serde(default = "default_dream_skin_name")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub style_preset: String,
    #[serde(default = "default_dream_skin_brand_subtitle")]
    pub brand_subtitle: String,
    #[serde(default = "default_dream_skin_tagline")]
    pub tagline: String,
    #[serde(default = "default_dream_skin_project_prefix")]
    pub project_prefix: String,
    #[serde(default = "default_dream_skin_project_label")]
    pub project_label: String,
    #[serde(default = "default_dream_skin_status_text")]
    pub status_text: String,
    #[serde(default = "default_dream_skin_quote")]
    pub quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<DreamSkinColors>,
    #[serde(flatten)]
    pub extra_fields: Map<String, Value>,
}

impl Default for DreamSkinThemeConfig {
    fn default() -> Self {
        let mut extra_fields = Map::new();
        #[cfg(windows)]
        {
            extra_fields.insert(
                "image".to_string(),
                Value::String("dream-reference.jpg".to_string()),
            );
            extra_fields.insert("appearance".to_string(), Value::String("auto".to_string()));
            extra_fields.insert(
                "art".to_string(),
                serde_json::json!({
                    "focusX": 0.72,
                    "focusY": 0.45,
                    "safeArea": "left",
                    "taskMode": "ambient"
                }),
            );
        }
        #[cfg(not(windows))]
        {
            extra_fields.insert(
                "image".to_string(),
                Value::String("portal-hero.png".to_string()),
            );
            extra_fields.insert(
                "promoTitle".to_string(),
                Value::String("感谢 Passion8 赞助".to_string()),
            );
            extra_fields.insert(
                "promoSub".to_string(),
                Value::String("passion8.cc".to_string()),
            );
            extra_fields.insert(
                "promoUrl".to_string(),
                Value::String("https://passion8.cc/register?aff=TuPe".to_string()),
            );
        }
        Self {
            schema_version: default_dream_skin_schema_version(),
            id: default_dream_skin_id(),
            name: default_dream_skin_name(),
            style_preset: String::new(),
            brand_subtitle: default_dream_skin_brand_subtitle(),
            tagline: default_dream_skin_tagline(),
            project_prefix: default_dream_skin_project_prefix(),
            project_label: default_dream_skin_project_label(),
            status_text: default_dream_skin_status_text(),
            quote: default_dream_skin_quote(),
            colors: if cfg!(windows) {
                None
            } else {
                Some(DreamSkinColors::default())
            },
            extra_fields,
        }
    }
}

/// 独立的听写服务配置；不复用编码供应商的密钥。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DictationProvider {
    /// OpenAI 兼容的远程转写服务。
    #[default]
    Api,
    /// 本机 SenseVoiceSmall ONNX 模型。
    #[serde(rename = "sensevoice")]
    SenseVoice,
}

/// 独立的听写服务配置；不复用编码供应商的密钥。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DictationSettings {
    pub enabled: bool,
    pub provider: DictationProvider,
    pub base_url: String,
    pub api_key: String,
    pub api_key_env: String,
    pub model: String,
    pub language: String,
    pub timeout_seconds: u64,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: DictationProvider::Api,
            base_url: "https://api.groq.com/openai/v1".to_string(),
            api_key: String::new(),
            api_key_env: String::new(),
            model: "whisper-large-v3-turbo".to_string(),
            language: String::new(),
            timeout_seconds: 120,
        }
    }
}

impl DictationSettings {
    pub fn normalize(&mut self) {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        // 兼容早期 SenseVoice 设置：旧版保存预设时可能只写入了
        // `local://sensevoice`，没有保存 provider 字段。加载时恢复 provider，
        // 否则离线标记会被语音服务按 HTTP(S) URL 校验并报错。
        if self.provider == DictationProvider::Api
            && self.base_url.eq_ignore_ascii_case("local://sensevoice")
        {
            self.provider = DictationProvider::SenseVoice;
        }
        self.api_key = self.api_key.trim().to_string();
        self.api_key_env = self.api_key_env.trim().to_string();
        self.model = self.model.trim().to_string();
        self.language = self.language.trim().to_string();
        self.timeout_seconds = self.timeout_seconds.clamp(1, 600);
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BackendSettings {
    /// 读取快照的版本，只用于条件保存，不写入 settings.json。
    #[serde(rename = "revision", default, skip_serializing_if = "String::is_empty")]
    pub settings_revision: String,
    #[serde(default)]
    pub dictation: DictationSettings,
    #[serde(rename = "codexAppPath", default)]
    pub codex_app_path: String,
    /// 自动接管官方 App：开启后用户自己双击启动官方 App 时由 Codex++ 接管并注入。
    #[serde(rename = "codexAppAutoTakeover", default)]
    pub codex_app_auto_takeover: bool,
    /// 已在运行的官方 App 没有调试端口时、「疑似有人在用」的策略：
    /// `defer`（默认：先不动，等空闲或用户手动触发）或 `immediate`（立即重启接管）。
    ///
    /// 显式写默认值而不是留空串：留空串虽然 `ActivePolicy::parse` 也按 `defer` 处理，
    /// 但读配置的人（以及前端默认值）会以为「没设置」，两边容易漂移。
    #[serde(
        rename = "codexAppAutoTakeoverWhenActive",
        default = "default_auto_takeover_when_active"
    )]
    pub codex_app_auto_takeover_when_active: String,
    #[serde(rename = "codexExtraArgs", default)]
    pub codex_extra_args: Vec<String>,
    #[serde(rename = "providerSyncEnabled", default)]
    pub provider_sync_enabled: bool,
    #[serde(rename = "providerSyncSavedProviders", default)]
    pub provider_sync_saved_providers: Vec<String>,
    #[serde(rename = "providerSyncManualProviders", default)]
    pub provider_sync_manual_providers: Vec<String>,
    #[serde(rename = "providerSyncLastSelectedProvider", default)]
    pub provider_sync_last_selected_provider: String,
    #[serde(rename = "ccsDbPath", default)]
    pub ccs_db_path: String,
    #[serde(rename = "ccsImportSources", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ccs_import_sources: BTreeMap<String, crate::ccs_import::CcsImportOrigin>,
    #[serde(rename = "relayProfilesEnabled", default = "default_true")]
    pub relay_profiles_enabled: bool,
    #[serde(rename = "enhancementsEnabled", default = "default_true")]
    pub enhancements_enabled: bool,
    #[serde(rename = "codexAppStandaloneQuotaUnlock", default)]
    pub codex_app_standalone_quota_unlock: bool,
    #[serde(rename = "codexAppPluginMarketplaceUnlock", default = "default_true")]
    pub codex_app_plugin_marketplace_unlock: bool,
    #[serde(rename = "codexAppModelWhitelistUnlock", default = "default_true")]
    pub codex_app_model_whitelist_unlock: bool,
    #[serde(rename = "codexAppSessionDelete", default = "default_true")]
    pub codex_app_session_delete: bool,
    #[serde(rename = "codexAppMarkdownExport", default = "default_true")]
    pub codex_app_markdown_export: bool,
    #[serde(rename = "codexAppSessionShare", default = "default_true")]
    pub codex_app_session_share: bool,
    // 默认开启：纯展示层状态栏，不改变 Codex 的请求内容与本地数据，误开没有副作用。
    #[serde(rename = "codexAppUsageStatusLine", default = "default_true")]
    pub codex_app_usage_status_line: bool,
    // 默认开启：只在受管副本的页面里加一个语音入口，识别走 Codex++ 自己的听写服务，
    // 不改官方应用、不动输入框已有内容；关掉只是没有入口，已运行的副本需重启才生效。
    #[serde(rename = "claudeAppVoiceInput", default = "default_true")]
    pub claude_app_voice_input: bool,
    // 默认开启：受管副本注入中文词表并把外壳语言切到 zh-Hans。关掉后副本按原语言启动，
    // 与「准备副本」这件事互不影响——副本只是所有增强共同的前置条件。
    #[serde(rename = "claudeAppLocalization", default = "default_true")]
    pub claude_app_localization: bool,
    // 默认关闭：Computer Use 需要改写副本的 app.asar 并关掉 Electron 的 asar 完整性校验，
    // 功能本身还受订阅与组织策略约束；误开没有收益，显式勾选才做。
    #[serde(rename = "claudeAppComputerUse", default)]
    pub claude_app_computer_use: bool,
    #[serde(rename = "codexAppPasteFix", default)]
    pub codex_app_paste_fix: bool,
    // 默认关闭：撤下的那版会主动从 settings.json 删掉这个键，老用户升级后读不到，
    // 若默认开启等于替他们重新打开一个从没同意过的行为。要开请显式勾选。
    #[serde(rename = "codexAppForceChineseLocale", default)]
    pub codex_app_force_chinese_locale: bool,
    #[serde(
        rename = "codexAppTypingEffect",
        default = "default_typing_effect",
        deserialize_with = "deserialize_typing_effect"
    )]
    pub codex_app_typing_effect: String,
    #[serde(rename = "codexAppThreadIdBadge", default)]
    pub codex_app_thread_id_badge: bool,
    #[serde(rename = "codexAppConversationView", default)]
    pub codex_app_conversation_view: bool,
    #[serde(rename = "codexAppThreadScrollRestore", default = "default_true")]
    pub codex_app_thread_scroll_restore: bool,
    #[serde(rename = "codexAppNativeMenuPlacement", default = "default_true")]
    pub codex_app_native_menu_placement: bool,
    #[serde(rename = "codexAppNativeBrowserRequireIdentification", default)]
    pub codex_app_native_browser_require_identification: bool,
    #[serde(rename = "codexAppNativeFastUnlock", default)]
    pub codex_app_native_fast_unlock: bool,
    #[serde(rename = "codexAppPetRealMouseLook", default)]
    pub codex_app_pet_real_mouse_look: bool,
    #[serde(rename = "codexAppActivityOverlayRight", default)]
    pub codex_app_activity_overlay_right: bool,
    #[serde(rename = "codexAppCapacityAutoRetry", default)]
    pub codex_app_capacity_auto_retry: bool,
    #[serde(rename = "codexAppWhaleWidgetEnabled", default)]
    pub codex_app_whale_widget_enabled: bool,
    #[serde(
        rename = "codexAppWhaleBalanceProtocol",
        default = "default_whale_balance_protocol"
    )]
    pub codex_app_whale_balance_protocol: String,
    #[serde(rename = "codexAppWhaleBalancePath", default)]
    pub codex_app_whale_balance_path: String,
    #[serde(rename = "codexAppWhaleBalanceField", default)]
    pub codex_app_whale_balance_field: String,
    #[serde(
        rename = "codexAppWhaleBalanceCurrency",
        default = "default_whale_balance_currency"
    )]
    pub codex_app_whale_balance_currency: String,
    #[serde(
        rename = "codexAppWhaleBalanceScale",
        default = "default_whale_balance_scale"
    )]
    pub codex_app_whale_balance_scale: f64,
    #[serde(rename = "codexAppStepwiseEnabled", default)]
    pub codex_app_stepwise_enabled: bool,
    #[serde(
        rename = "codexAppStepwiseGenerationMode",
        default = "default_stepwise_generation_mode",
        deserialize_with = "deserialize_stepwise_generation_mode"
    )]
    pub codex_app_stepwise_generation_mode: String,
    #[serde(rename = "codexAppAnswerOutlineEnabled", default)]
    pub codex_app_answer_outline_enabled: bool,
    #[serde(rename = "codexAppStepwiseDirectSend", default)]
    pub codex_app_stepwise_direct_send: bool,
    #[serde(rename = "codexAppStepwiseBaseUrl", default)]
    pub codex_app_stepwise_base_url: String,
    #[serde(rename = "codexAppStepwiseApiKey", default)]
    pub codex_app_stepwise_api_key: String,
    #[serde(
        rename = "codexAppStepwiseApiKeyEnv",
        default = "default_stepwise_api_key_env",
        deserialize_with = "empty_as_default_stepwise_api_key_env"
    )]
    pub codex_app_stepwise_api_key_env: String,
    #[serde(
        rename = "codexAppStepwiseProtocol",
        default = "default_stepwise_protocol",
        deserialize_with = "deserialize_stepwise_protocol"
    )]
    pub codex_app_stepwise_protocol: String,
    #[serde(rename = "codexAppStepwiseModel", default)]
    pub codex_app_stepwise_model: String,
    #[serde(
        rename = "codexAppStepwiseMaxItems",
        default = "default_stepwise_max_items",
        deserialize_with = "deserialize_stepwise_max_items"
    )]
    pub codex_app_stepwise_max_items: u8,
    #[serde(
        rename = "codexAppStepwiseMaxInputChars",
        default = "default_stepwise_max_input_chars",
        deserialize_with = "deserialize_stepwise_max_input_chars"
    )]
    pub codex_app_stepwise_max_input_chars: u32,
    #[serde(
        rename = "codexAppStepwiseMaxOutputTokens",
        default = "default_stepwise_max_output_tokens",
        deserialize_with = "deserialize_stepwise_max_output_tokens"
    )]
    pub codex_app_stepwise_max_output_tokens: u32,
    #[serde(
        rename = "codexAppStepwiseTimeoutMs",
        default = "default_stepwise_timeout_ms",
        deserialize_with = "deserialize_stepwise_timeout_ms"
    )]
    pub codex_app_stepwise_timeout_ms: u64,
    #[serde(rename = "codexAppImageOverlayEnabled", default)]
    pub codex_app_image_overlay_enabled: bool,
    #[serde(rename = "codexAppImageOverlayPath", default)]
    pub codex_app_image_overlay_path: String,
    #[serde(
        rename = "codexAppImageOverlayOpacity",
        default = "default_image_overlay_opacity",
        deserialize_with = "deserialize_image_overlay_opacity"
    )]
    pub codex_app_image_overlay_opacity: u8,
    #[serde(
        rename = "codexAppImageOverlayFitMode",
        default = "default_image_overlay_fit_mode",
        deserialize_with = "deserialize_image_overlay_fit_mode"
    )]
    pub codex_app_image_overlay_fit_mode: String,
    #[serde(rename = "codexAppDreamSkinEnabled", default)]
    pub codex_app_dream_skin_enabled: bool,
    #[serde(rename = "codexAppDreamSkinPaused", default)]
    pub codex_app_dream_skin_paused: bool,
    #[serde(
        rename = "codexAppDreamSkinTheme",
        default = "default_dream_skin_theme",
        deserialize_with = "deserialize_dream_skin_theme"
    )]
    pub codex_app_dream_skin_theme: String,
    #[serde(rename = "codexAppDreamSkinThemeConfig", default)]
    pub codex_app_dream_skin_theme_config: DreamSkinThemeConfig,
    #[serde(rename = "codexAppDreamSkinImagePath", default)]
    pub codex_app_dream_skin_image_path: String,
    #[serde(rename = "codexGoalsEnabled", default)]
    pub codex_goals_enabled: bool,
    #[serde(rename = "weixinConnectEnabled", default)]
    pub weixin_connect_enabled: bool,
    #[serde(
        rename = "weixinConnectBaseUrl",
        default = "default_weixin_connect_base_url"
    )]
    pub weixin_connect_base_url: String,
    #[serde(rename = "weixinConnectToken", default)]
    pub weixin_connect_token: String,
    #[serde(rename = "weixinConnectAccountId", default)]
    pub weixin_connect_account_id: String,
    #[serde(rename = "weixinConnectAllowFrom", default)]
    pub weixin_connect_allow_from: String,
    #[serde(rename = "weixinConnectRouteTag", default)]
    pub weixin_connect_route_tag: String,
    #[serde(rename = "weixinConnectWorkDir", default)]
    pub weixin_connect_work_dir: String,
    #[serde(rename = "weixinConnectModel", default)]
    pub weixin_connect_model: String,
    #[serde(
        rename = "weixinConnectSandbox",
        default = "default_weixin_connect_sandbox"
    )]
    pub weixin_connect_sandbox: String,
    #[serde(rename = "weixinConnectCodexPath", default)]
    pub weixin_connect_codex_path: String,
    #[serde(rename = "relayBaseUrl", default = "default_relay_base_url")]
    pub relay_base_url: String,
    #[serde(rename = "relayApiKey", default)]
    pub relay_api_key: String,
    #[serde(rename = "relayProfiles", default = "default_relay_profiles")]
    pub relay_profiles: Vec<RelayProfile>,
    /// 本机配置只自动导入一次，删除或重命名后不再补回。
    #[serde(rename = "localConfigProviderImported", default)]
    pub local_config_provider_imported: bool,
    #[serde(rename = "relayCommonConfigContents", default)]
    pub relay_common_config_contents: String,
    #[serde(rename = "relayPreserveNonproviderSettings", default)]
    pub relay_preserve_nonprovider_settings: bool,
    #[serde(rename = "relayContextConfigContents", default)]
    pub relay_context_config_contents: String,
    #[serde(rename = "activeRelayId", default = "default_active_relay_id")]
    pub active_relay_id: String,
    #[serde(rename = "aggregateRelayProfiles", default)]
    pub aggregate_relay_profiles: Vec<AggregateRelayProfile>,
    #[serde(rename = "activeAggregateRelayId", default)]
    pub active_aggregate_relay_id: String,
    #[serde(rename = "relayTestModel", default = "default_relay_test_model")]
    pub relay_test_model: String,
    /// 协议代理自动重试：200 流内的过载/容量错误，以及用户在
    /// `protocolProxyAutoRetryStatusCodes` 里勾选的 HTTP 错误，先在本地代理内部
    /// 退避重试，不立刻把错误交给 Codex。只影响经过本地协议代理的请求。
    /// 默认开启：用户看到的失败横幅会显著减少；关闭后恢复「失败立即返回」行为。
    #[serde(rename = "protocolProxyAutoRetry", default = "default_true")]
    pub protocol_proxy_auto_retry: bool,
    /// 最多自动重试次数（不含首次请求）。0 与关闭开关等价。
    #[serde(
        rename = "protocolProxyAutoRetryMaxAttempts",
        default = "default_protocol_proxy_auto_retry_max_attempts"
    )]
    pub protocol_proxy_auto_retry_max_attempts: u32,
    /// 首次重试等待秒数；之后按 2 倍指数退避，单次上限 30 秒。
    #[serde(
        rename = "protocolProxyAutoRetryIntervalSeconds",
        default = "default_protocol_proxy_auto_retry_interval_seconds"
    )]
    pub protocol_proxy_auto_retry_interval_seconds: u64,
    /// 上游直接返回这些 HTTP 状态码时由代理退避重发（Retry-After 优先），
    /// 与重试次数、总等待预算共用。默认空列表：HTTP 错误一律直接交给 Codex，
    /// 需要的用户自行加入 429 等（issue #2653）。
    #[serde(
        rename = "protocolProxyAutoRetryStatusCodes",
        default = "default_protocol_proxy_auto_retry_status_codes",
        deserialize_with = "deserialize_protocol_proxy_auto_retry_status_codes"
    )]
    pub protocol_proxy_auto_retry_status_codes: Vec<u16>,
    /// 过载错误处理模式：
    /// - `streaming`（默认，流式；推理/工具调用参数先缓冲，首段可见输出后再等
    ///   `protocolProxyAutoRetryCommitDelayMs`，窗口内仍由代理重试）；
    /// - `streaming-legacy`（旧流式：遇到第一个输出事件立即提交，不再为工具
    ///   参数或首字窗口缓冲）；
    /// - `buffered`（整段缓冲后转发；任何时点的过载都由代理重试，但失去流式）。
    #[serde(
        rename = "protocolProxyAutoRetryMode",
        default = "default_protocol_proxy_auto_retry_mode"
    )]
    pub protocol_proxy_auto_retry_mode: String,
    /// 流式避震的「首字提交延迟」：出现第一段可见输出后，代理继续缓冲多少毫秒
    /// 再交给 Codex。窗口内遇到容量/过载错误仍可丢弃重试；0 表示一出现可见输出
    /// 就提交。推理与工具调用参数阶段本来就不向客户端提交，不占这个延迟，所以
    /// 工具调用可以整项缓冲到完成后再提交。注意 `streaming-legacy` 模式忽略该
    /// 字段，始终保持旧行为。
    #[serde(
        rename = "protocolProxyAutoRetryCommitDelayMs",
        default = "default_protocol_proxy_auto_retry_commit_delay_ms"
    )]
    pub protocol_proxy_auto_retry_commit_delay_ms: u64,
    /// 本地协议代理（helper）单次 HTTP 请求体上限，单位 MiB。
    ///
    /// 默认 128 MiB：1M 上下文窗口的长会话（带截图历史）实测请求体可到 ~50 MB，
    /// 旧的 32 MiB 硬上限会让长会话直接返回 413 并中断对话。范围 32~512 MiB；
    /// 环境变量 `CODEX_PLUS_MAX_HTTP_BODY_BYTES`（字节）仍可覆盖本设置。
    #[serde(
        rename = "helperMaxRequestBodyMib",
        default = "default_helper_max_request_body_mib"
    )]
    pub helper_max_request_body_mib: u32,
    /// 按工具分区的配置。`tools.codex` 是上面那批扁平字段的镜像（写盘时同步），
    /// 其它工具（Grok / 后续工具）只存在这里。见 `crate::tools`。
    #[serde(rename = "tools", default)]
    pub tools: BTreeMap<ToolId, ToolConfig>,
    /// UI 顶栏当前聚焦的工具。只影响管理器的展示，不影响 Codex 的启动配置。
    #[serde(rename = "activeTool", default)]
    pub active_tool: ToolId,
}

impl Default for BackendSettings {
    fn default() -> Self {
        Self {
            settings_revision: String::new(),
            dictation: DictationSettings::default(),
            codex_app_path: String::new(),
            codex_app_auto_takeover: false,
            codex_app_auto_takeover_when_active: default_auto_takeover_when_active(),
            codex_extra_args: Vec::new(),
            provider_sync_enabled: false,
            provider_sync_saved_providers: Vec::new(),
            provider_sync_manual_providers: Vec::new(),
            provider_sync_last_selected_provider: String::new(),
            ccs_db_path: String::new(),
            ccs_import_sources: BTreeMap::new(),
            relay_profiles_enabled: true,
            enhancements_enabled: true,
            codex_app_standalone_quota_unlock: false,
            codex_app_plugin_marketplace_unlock: true,
            codex_app_model_whitelist_unlock: true,
            codex_app_session_delete: true,
            codex_app_markdown_export: true,
            codex_app_session_share: true,
            codex_app_usage_status_line: true,
            claude_app_voice_input: true,
            claude_app_localization: true,
            claude_app_computer_use: false,
            codex_app_paste_fix: false,
            codex_app_force_chinese_locale: false,
            codex_app_typing_effect: default_typing_effect(),
            codex_app_thread_id_badge: false,
            codex_app_conversation_view: false,
            codex_app_thread_scroll_restore: true,
            codex_app_native_menu_placement: true,
            codex_app_native_browser_require_identification: false,
            codex_app_native_fast_unlock: false,
            codex_app_pet_real_mouse_look: false,
            codex_app_activity_overlay_right: false,
            codex_app_capacity_auto_retry: false,
            codex_app_whale_widget_enabled: false,
            codex_app_whale_balance_protocol: default_whale_balance_protocol(),
            codex_app_whale_balance_path: String::new(),
            codex_app_whale_balance_field: String::new(),
            codex_app_whale_balance_currency: default_whale_balance_currency(),
            codex_app_whale_balance_scale: default_whale_balance_scale(),
            codex_app_stepwise_enabled: false,
            codex_app_stepwise_generation_mode: default_stepwise_generation_mode(),
            codex_app_answer_outline_enabled: false,
            codex_app_stepwise_direct_send: false,
            codex_app_stepwise_base_url: String::new(),
            codex_app_stepwise_api_key: String::new(),
            codex_app_stepwise_api_key_env: default_stepwise_api_key_env(),
            codex_app_stepwise_protocol: default_stepwise_protocol(),
            codex_app_stepwise_model: String::new(),
            codex_app_stepwise_max_items: default_stepwise_max_items(),
            codex_app_stepwise_max_input_chars: default_stepwise_max_input_chars(),
            codex_app_stepwise_max_output_tokens: default_stepwise_max_output_tokens(),
            codex_app_stepwise_timeout_ms: default_stepwise_timeout_ms(),
            codex_app_image_overlay_enabled: false,
            codex_app_image_overlay_path: String::new(),
            codex_app_image_overlay_opacity: default_image_overlay_opacity(),
            codex_app_image_overlay_fit_mode: default_image_overlay_fit_mode(),
            codex_app_dream_skin_enabled: false,
            codex_app_dream_skin_paused: false,
            codex_app_dream_skin_theme: default_dream_skin_theme(),
            codex_app_dream_skin_theme_config: DreamSkinThemeConfig::default(),
            codex_app_dream_skin_image_path: String::new(),
            codex_goals_enabled: false,
            weixin_connect_enabled: false,
            weixin_connect_base_url: default_weixin_connect_base_url(),
            weixin_connect_token: String::new(),
            weixin_connect_account_id: String::new(),
            weixin_connect_allow_from: String::new(),
            weixin_connect_route_tag: String::new(),
            weixin_connect_work_dir: String::new(),
            weixin_connect_model: String::new(),
            weixin_connect_sandbox: default_weixin_connect_sandbox(),
            weixin_connect_codex_path: String::new(),
            relay_base_url: default_relay_base_url(),
            relay_api_key: String::new(),
            relay_profiles: default_relay_profiles(),
            local_config_provider_imported: false,
            relay_common_config_contents: String::new(),
            relay_preserve_nonprovider_settings: false,
            relay_context_config_contents: String::new(),
            active_relay_id: default_active_relay_id(),
            aggregate_relay_profiles: Vec::new(),
            active_aggregate_relay_id: String::new(),
            relay_test_model: default_relay_test_model(),
            protocol_proxy_auto_retry: true,
            protocol_proxy_auto_retry_max_attempts:
                default_protocol_proxy_auto_retry_max_attempts(),
            protocol_proxy_auto_retry_interval_seconds:
                default_protocol_proxy_auto_retry_interval_seconds(),
            protocol_proxy_auto_retry_status_codes:
                default_protocol_proxy_auto_retry_status_codes(),
            protocol_proxy_auto_retry_mode: default_protocol_proxy_auto_retry_mode(),
            protocol_proxy_auto_retry_commit_delay_ms:
                default_protocol_proxy_auto_retry_commit_delay_ms(),
            helper_max_request_body_mib: default_helper_max_request_body_mib(),
            tools: BTreeMap::new(),
            active_tool: ToolId::Codex,
        }
    }
}

impl BackendSettings {
    /// 当前设置对应的本地代理请求体上限（字节），已夹取到 32~512 MiB。
    pub fn helper_max_request_body_bytes(&self) -> usize {
        clamp_helper_max_request_body_mib(self.helper_max_request_body_mib) as usize * 1024 * 1024
    }

    pub fn active_relay_profile(&self) -> RelayProfile {
        if self.active_relay_id == default_active_relay_id()
            && self.relay_profiles.len() == 1
            && self.relay_profiles[0] == RelayProfile::default()
            && (!self.relay_api_key.is_empty() || self.relay_base_url != default_relay_base_url())
        {
            return RelayProfile {
                id: default_active_relay_id(),
                name: "默认中转".to_string(),
                model: String::new(),
                base_url: if self.relay_base_url.is_empty() {
                    default_relay_base_url()
                } else {
                    self.relay_base_url.clone()
                },
                upstream_base_url: if self.relay_base_url.is_empty() {
                    default_relay_base_url()
                } else {
                    self.relay_base_url.clone()
                },
                api_key: self.relay_api_key.clone(),
                api_keys: Vec::new(),
                active_api_key_id: String::new(),
                protocol: RelayProtocol::Responses,
                relay_mode: RelayMode::MixedApi,
                official_mix_api_key: true,
                no_auth: false,
                hide_official_usage_alert: false,
                test_model: String::new(),
                config_contents: String::new(),
                auth_contents: String::new(),
                use_common_config: true,
                context_window: String::new(),
                auto_compact_limit: String::new(),
                model_insert_mode: RelayModelInsertMode::Patch,
                model_list: String::new(),
                model_windows: String::new(),
                model_auto_compact: String::new(),
                model_metadata: String::new(),
                model_vlm: String::new(),
                vlm_api_key: String::new(),
                vlm_model: String::new(),
                vlm_base_url: String::new(),
                user_agent: String::new(),
                sub2api_enabled: false,
                sub2api_multiplier: String::new(),
                model_routes: Vec::new(),
                custom_headers: Vec::new(),
                standard_openai_protocol: false,
                web_search_history_compat: false,
                trust_upstream_catalog_lite: false,
                strict_responses_reasoning: false,
                anthropic_auth_header: String::new(),
                anthropic_max_tokens: 0,
                anthropic_prompt_cache: true,
                anthropic_impersonate_claude_code: false,
                anthropic_thinking_mode: String::new(),
            };
        }

        if let Some(profile) = self
            .relay_profiles
            .iter()
            .find(|profile| profile.id == self.active_relay_id)
        {
            return profile.clone();
        }

        RelayProfile {
            id: if self.active_relay_id.is_empty() {
                default_active_relay_id()
            } else {
                self.active_relay_id.clone()
            },
            name: "默认中转".to_string(),
            model: String::new(),
            base_url: if self.relay_base_url.is_empty() {
                default_relay_base_url()
            } else {
                self.relay_base_url.clone()
            },
            upstream_base_url: if self.relay_base_url.is_empty() {
                default_relay_base_url()
            } else {
                self.relay_base_url.clone()
            },
            api_key: self.relay_api_key.clone(),
            api_keys: Vec::new(),
            active_api_key_id: String::new(),
            protocol: RelayProtocol::Responses,
            relay_mode: RelayMode::Official,
            official_mix_api_key: false,
            no_auth: false,
            hide_official_usage_alert: false,
            test_model: String::new(),
            config_contents: String::new(),
            auth_contents: String::new(),
            use_common_config: true,
            context_window: String::new(),
            auto_compact_limit: String::new(),
            model_insert_mode: RelayModelInsertMode::Patch,
            model_list: String::new(),
            model_windows: String::new(),
            model_auto_compact: String::new(),
            model_metadata: String::new(),
            model_vlm: String::new(),
            vlm_api_key: String::new(),
            vlm_model: String::new(),
            vlm_base_url: String::new(),
            user_agent: String::new(),
            sub2api_enabled: false,
            sub2api_multiplier: String::new(),
            model_routes: Vec::new(),
            custom_headers: Vec::new(),
            standard_openai_protocol: false,
            web_search_history_compat: false,
            trust_upstream_catalog_lite: false,
            strict_responses_reasoning: false,
            anthropic_auth_header: String::new(),
            anthropic_max_tokens: 0,
            anthropic_prompt_cache: true,
            anthropic_impersonate_claude_code: false,
            anthropic_thinking_mode: String::new(),
        }
    }

    pub fn active_aggregate_relay_profile(&self) -> Option<AggregateRelayProfile> {
        let active_relay = self
            .relay_profiles
            .iter()
            .find(|profile| profile.id == self.active_relay_id)?;
        if active_relay.relay_mode != RelayMode::Aggregate {
            return None;
        }

        let active_aggregate_id = if self.active_aggregate_relay_id.trim().is_empty() {
            active_relay.id.as_str()
        } else {
            self.active_aggregate_relay_id.trim()
        };

        if active_aggregate_id != active_relay.id {
            return None;
        }

        self.aggregate_relay_profiles
            .iter()
            .find(|profile| profile.id == active_aggregate_id)
            .cloned()
    }

    pub fn active_relay_session_provider(&self) -> RelaySessionProvider {
        if let Some(profile) = self.active_aggregate_relay_profile() {
            return profile.session_provider;
        }
        if self
            .active_relay_profile()
            .config_contents
            .parse::<DocumentMut>()
            .ok()
            .is_some_and(|doc| {
                doc.get("model_provider")
                    .and_then(Item::as_str)
                    .map(str::trim)
                    .is_some_and(|provider| provider == "openai")
            })
        {
            RelaySessionProvider::Openai
        } else {
            RelaySessionProvider::Custom
        }
    }

    pub fn active_relay_transport_uses_protocol_proxy(&self) -> bool {
        self.active_aggregate_relay_profile().is_some()
            || self.active_relay_profile().requires_protocol_proxy_conversion()
            || self.active_relay_profile().has_model_routes()
            || self.active_relay_profile().uses_no_auth()
            || self.active_relay_profile().uses_independent_proxy()
            || self.active_relay_profile().web_search_history_compat_enabled()
            || self.active_relay_profile().request_compatibility_enabled()
    }

    pub fn active_relay_uses_protocol_proxy(&self) -> bool {
        self.active_relay_transport_uses_protocol_proxy()
            || self.active_relay_session_provider() == RelaySessionProvider::Openai
    }
}

pub fn default_typing_effect() -> String {
    "off".to_string()
}

pub fn normalize_typing_effect(value: &str) -> String {
    match value {
        "off" | "rainbow" | "fireworks" | "stars" => value.to_string(),
        _ => default_typing_effect(),
    }
}

pub fn default_stepwise_api_key_env() -> String {
    "CODEX_STEPWISE_API_KEY".to_string()
}

pub fn default_stepwise_protocol() -> String {
    "chat_completions".to_string()
}

pub fn normalize_stepwise_protocol(value: &str) -> String {
    match value.trim() {
        "chat_completions" | "responses" | "anthropic_messages" | "auto" => {
            value.trim().to_string()
        }
        _ => default_stepwise_protocol(),
    }
}

pub fn default_stepwise_generation_mode() -> String {
    "auto".to_string()
}

pub fn normalize_stepwise_generation_mode(value: &str) -> String {
    match value.trim() {
        "manual" => "manual".to_string(),
        _ => default_stepwise_generation_mode(),
    }
}

pub fn default_stepwise_max_items() -> u8 {
    4
}

pub fn default_stepwise_max_input_chars() -> u32 {
    6000
}

pub fn default_stepwise_max_output_tokens() -> u32 {
    500
}

pub fn default_stepwise_timeout_ms() -> u64 {
    8000
}

fn default_image_overlay_opacity() -> u8 {
    35
}

fn clamp_image_overlay_opacity(value: u8) -> u8 {
    value.clamp(1, 100)
}

pub fn default_image_overlay_fit_mode() -> String {
    "fit".to_string()
}

fn normalize_image_overlay_fit_mode(value: &str) -> String {
    match value {
        "fill" | "fit" | "stretch" | "tile" | "center" => value.to_string(),
        _ => default_image_overlay_fit_mode(),
    }
}

pub fn default_dream_skin_theme() -> String {
    "pink".to_string()
}

fn default_dream_skin_schema_version() -> u8 {
    1
}

#[cfg(windows)]
fn default_dream_skin_id() -> String {
    "preset-arina-hashimoto".to_string()
}

#[cfg(not(windows))]
fn default_dream_skin_id() -> String {
    "custom-1784123441349".to_string()
}

#[cfg(windows)]
fn default_dream_skin_name() -> String {
    "桥本有菜".to_string()
}

#[cfg(not(windows))]
fn default_dream_skin_name() -> String {
    "Dream Skin".to_string()
}

pub fn resolve_dream_skin_style_preset(id: &str, style_preset: &str) -> String {
    let style_preset = style_preset.trim();
    if !style_preset.is_empty() && style_preset != "dream-original" {
        return style_preset.to_string();
    }

    match id.trim() {
        "caishen-lite" => "caishen-lite",
        "caishen-max" => "caishen-max",
        "caishen-readable" => "caishen-readable",
        "export-night" => "export-night",
        "global-founder-bright" => "global-founder-bright",
        "mythic-guardian-noir" => "mythic-guardian-noir",
        "codex-snow-skin" => "codex-snow",
        "glass-vision" => "glass-vision",
        "preset-midnight-aurora" => "midnight-aurora",
        "preset-amber-dusk" => "amber-dusk",
        "preset-forest-mist" => "forest-mist",
        "preset-cyber-neon" => "cyber-neon",
        "preset-sakura-dawn" => "sakura-dawn",
        _ => "dream-original",
    }
    .to_string()
}

fn default_dream_skin_brand_subtitle() -> String {
    "CODEX DREAM SKIN".to_string()
}

#[cfg(windows)]
fn default_dream_skin_tagline() -> String {
    "把柔光与玫瑰带进今天的工作台。".to_string()
}

#[cfg(not(windows))]
fn default_dream_skin_tagline() -> String {
    "把喜欢的画面变成可交互的 Codex 工作台。".to_string()
}

fn default_dream_skin_project_prefix() -> String {
    "选择项目 · ".to_string()
}

fn default_dream_skin_project_label() -> String {
    "◉  选择项目".to_string()
}

#[cfg(windows)]
fn default_dream_skin_status_text() -> String {
    "DREAM SKIN ONLINE".to_string()
}

#[cfg(not(windows))]
fn default_dream_skin_status_text() -> String {
    "THEME ONLINE".to_string()
}

#[cfg(windows)]
fn default_dream_skin_quote() -> String {
    "MAKE SOMETHING WONDERFUL".to_string()
}

#[cfg(not(windows))]
fn default_dream_skin_quote() -> String {
    "Make something wonderful".to_string()
}

fn normalize_dream_skin_theme(value: &str) -> String {
    match value.trim() {
        "pink" | "luckyGod" | "redWhite" | "clearGlass" | "inspiration" | "purpleNight"
        | "miku" | "blackGold" => value.trim().to_string(),
        _ => default_dream_skin_theme(),
    }
}

pub fn clamp_stepwise_max_items(value: u8) -> u8 {
    value.min(6)
}

pub fn clamp_stepwise_max_input_chars(value: u32) -> u32 {
    value.clamp(1000, 24000)
}

pub fn clamp_stepwise_max_output_tokens(value: u32) -> u32 {
    value.clamp(100, 4000)
}

pub fn clamp_stepwise_timeout_ms(value: u64) -> u64 {
    value.clamp(1000, 60000)
}

pub fn default_true() -> bool {
    true
}

/// 「疑似有人在用」的默认策略：先不动，等空闲或用户手动触发。
pub fn default_auto_takeover_when_active() -> String {
    crate::auto_takeover::ActivePolicy::Defer.as_str().to_string()
}

fn default_whale_balance_protocol() -> String {
    "auto".to_string()
}

fn default_whale_balance_currency() -> String {
    "USD".to_string()
}

fn default_whale_balance_scale() -> f64 {
    1.0
}

pub fn default_relay_base_url() -> String {
    String::new()
}

fn default_weixin_connect_base_url() -> String {
    crate::connect::DEFAULT_WEIXIN_BASE_URL.to_string()
}

fn default_weixin_connect_sandbox() -> String {
    "read-only".to_string()
}

pub fn default_active_relay_id() -> String {
    "default".to_string()
}

pub fn default_relay_test_model() -> String {
    "gpt-5.4-mini".to_string()
}

/// 协议代理默认最多自动重试 5 次：2+4+8+16+30 秒封顶，合计约 60 秒。
pub fn default_protocol_proxy_auto_retry_max_attempts() -> u32 {
    5
}

pub fn clamp_protocol_proxy_auto_retry_max_attempts(value: u32) -> u32 {
    value.min(20)
}

pub fn default_protocol_proxy_auto_retry_interval_seconds() -> u64 {
    2
}

pub fn clamp_protocol_proxy_auto_retry_interval_seconds(value: u64) -> u64 {
    value.min(30)
}

/// 默认不重试任何 HTTP 状态码：HTTP 错误重试曾按使用反馈整体下线（bd4e90d1），
/// 现在改为由用户按需选择，升级不改变既有行为。
pub fn default_protocol_proxy_auto_retry_status_codes() -> Vec<u16> {
    Vec::new()
}

/// 只保留 400~599 的错误码，去重排序，最多 20 个；成功码与非法值直接丢弃。
pub fn normalize_protocol_proxy_auto_retry_status_codes(values: &[u64]) -> Vec<u16> {
    let mut codes: Vec<u16> = values
        .iter()
        .filter(|code| (400..=599).contains(*code))
        .map(|code| *code as u16)
        .collect();
    codes.sort_unstable();
    codes.dedup();
    codes.truncate(20);
    codes
}

/// 手改的 settings.json 可能写进字符串、负数或超出 u16 的值；
/// 这里逐项容错，不能让一个坏值把整份设置读失败。
fn deserialize_protocol_proxy_auto_retry_status_codes<'de, D>(
    deserializer: D,
) -> Result<Vec<u16>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let Some(Value::Array(items)) = Option::<Value>::deserialize(deserializer)? else {
        return Ok(default_protocol_proxy_auto_retry_status_codes());
    };
    Ok(normalize_protocol_proxy_auto_retry_status_codes(
        &status_code_values(&items),
    ))
}

fn status_code_values(items: &[Value]) -> Vec<u64> {
    items
        .iter()
        .filter_map(|item| {
            item.as_u64()
                .or_else(|| item.as_str().and_then(|text| text.trim().parse().ok()))
        })
        .collect()
}

pub fn default_protocol_proxy_auto_retry_mode() -> String {
    "streaming".to_string()
}

pub fn normalize_protocol_proxy_auto_retry_mode(value: &str) -> String {
    match value.trim() {
        "buffered" => "buffered".to_string(),
        // 旧版「流式」行为保留为独立选项，方便随时退回。
        "streaming-legacy" => "streaming-legacy".to_string(),
        _ => default_protocol_proxy_auto_retry_mode(),
    }
}

/// 流式避震默认在首段可见输出后再缓冲 1 秒：能覆盖「刚开始吐字上游就过载」
/// 这一大类，首字延迟仍控制在 1 秒内；短回复在 response.completed 到达时
/// 立即提交，不会白等满 1 秒。
pub fn default_protocol_proxy_auto_retry_commit_delay_ms() -> u64 {
    1000
}

pub fn clamp_protocol_proxy_auto_retry_commit_delay_ms(value: u64) -> u64 {
    value.min(10_000)
}

/// 本地协议代理请求体默认上限：128 MiB。
///
/// 依据：1M 上下文窗口的长会话（历史里带 Computer Use 截图）实测请求体约 33 MB
/// 且仍在增长，对应 1M tokens 约 50 MB；旧的 32 MiB 硬上限会在正常长会话里直接
/// 返回 413 并中断对话。128 MiB 给 1M 窗口留出约 2.5 倍余量。
pub const DEFAULT_HELPER_MAX_REQUEST_BODY_MIB: u32 = 128;
/// 请求体上限下限：低于 32 MiB 会误伤正常请求。
pub const MIN_HELPER_MAX_REQUEST_BODY_MIB: u32 = 32;
/// 请求体上限上限：helper 会整段缓冲请求体，再高内存开销不可控。
pub const MAX_HELPER_MAX_REQUEST_BODY_MIB: u32 = 512;

pub fn default_helper_max_request_body_mib() -> u32 {
    DEFAULT_HELPER_MAX_REQUEST_BODY_MIB
}

pub fn clamp_helper_max_request_body_mib(value: u32) -> u32 {
    value.clamp(
        MIN_HELPER_MAX_REQUEST_BODY_MIB,
        MAX_HELPER_MAX_REQUEST_BODY_MIB,
    )
}

pub fn default_relay_profiles() -> Vec<RelayProfile> {
    vec![RelayProfile::default()]
}

/// 判断一份供应商列表是否退化成了「仅剩默认 profile」。
///
/// 这是 `load` 失败后退回默认设置、再被原样回写磁盘的典型形态。
/// `RelayProfile::default()` 的 id 固定为 `default`，用户手工创建的条目不会长这样。
pub fn is_default_single_profile(profiles: &[RelayProfile]) -> bool {
    profiles.len() == 1 && profiles[0] == RelayProfile::default()
}

pub fn default_aggregate_member_weight() -> u32 {
    1
}

pub fn empty_as_default_stepwise_api_key_env<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .filter(|value| !value.is_empty())
        .unwrap_or_else(default_stepwise_api_key_env))
}

fn deserialize_typing_effect<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_str()
        .map(normalize_typing_effect)
        .unwrap_or_else(default_typing_effect))
}

fn deserialize_stepwise_protocol<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?
        .map(|value| normalize_stepwise_protocol(&value))
        .unwrap_or_else(default_stepwise_protocol))
}

fn deserialize_stepwise_generation_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?
        .map(|value| normalize_stepwise_generation_mode(&value))
        .unwrap_or_else(default_stepwise_generation_mode))
}

fn deserialize_image_overlay_opacity<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<u8>::deserialize(deserializer)?
        .map(clamp_image_overlay_opacity)
        .unwrap_or_else(default_image_overlay_opacity))
}

fn deserialize_image_overlay_fit_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?
        .map(|value| normalize_image_overlay_fit_mode(&value))
        .unwrap_or_else(default_image_overlay_fit_mode))
}

fn deserialize_dream_skin_theme<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?
        .map(|value| normalize_dream_skin_theme(&value))
        .unwrap_or_else(default_dream_skin_theme))
}

fn deserialize_stepwise_max_items<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<u8>::deserialize(deserializer)?
        .map(clamp_stepwise_max_items)
        .unwrap_or_else(default_stepwise_max_items))
}

fn deserialize_stepwise_max_input_chars<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<u32>::deserialize(deserializer)?
        .map(clamp_stepwise_max_input_chars)
        .unwrap_or_else(default_stepwise_max_input_chars))
}

fn deserialize_stepwise_max_output_tokens<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<u32>::deserialize(deserializer)?
        .map(clamp_stepwise_max_output_tokens)
        .unwrap_or_else(default_stepwise_max_output_tokens))
}

fn deserialize_stepwise_timeout_ms<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<u64>::deserialize(deserializer)?
        .map(clamp_stepwise_timeout_ms)
        .unwrap_or_else(default_stepwise_timeout_ms))
}

fn deserialize_profile_api_key<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

pub fn normalize_codex_extra_args(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.trim())
        .filter(|arg| !arg.is_empty())
        .map(ToString::to_string)
        .collect()
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

/// 配置不应承载无界的转义膨胀或文件内容。超限时只报错，不覆盖原文件。
pub const MAX_SETTINGS_FILE_BYTES: usize = 16 * 1024 * 1024;

fn serialize_settings_bounded(value: &impl serde::Serialize) -> anyhow::Result<Vec<u8>> {
    struct BoundedWriter(Vec<u8>);
    impl Write for BoundedWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_SETTINGS_FILE_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other(
                    "settings 超过 16 MiB 上限，原文件未修改",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter(Vec::new());
    serde_json::to_writer_pretty(&mut writer, value)?;
    Ok(writer.0)
}

impl Default for SettingsStore {
    fn default() -> Self {
        Self::new(crate::paths::default_settings_path())
    }
}

thread_local! {
    /// 设置归一化的重入深度，见 [`SettingsStore::load`]。
    static SETTINGS_NORMALIZE_DEPTH: Cell<u32> = const { Cell::new(0) };
}

fn settings_normalize_depth() -> u32 {
    SETTINGS_NORMALIZE_DEPTH.with(Cell::get)
}

/// 归一化作用域：只要还在里面，嵌套的 `SettingsStore::load` 就只解析、不再归一化，
/// 从而打断 `load → normalize → complete_relay_profile_config → load` 的递归。
struct SettingsNormalizeGuard;

impl SettingsNormalizeGuard {
    fn enter() -> Self {
        SETTINGS_NORMALIZE_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        Self
    }
}

impl Drop for SettingsNormalizeGuard {
    fn drop(&mut self) {
        SETTINGS_NORMALIZE_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

impl SettingsStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn write_lock(&self) -> anyhow::Result<File> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // 锁文件保持稳定，不能锁会被 atomic rename 替换的 settings inode。
        let path = self.path.with_extension("json.lock");
        let file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
        file.lock_exclusive().context("获取设置写锁失败")?;
        Ok(file)
    }

    pub fn revision(&self) -> anyhow::Result<String> {
        Ok(settings_revision(self.read_contents()?.as_deref().map(str::as_bytes)))
    }

    pub(crate) fn with_current_revision<T>(&self, expected: &str, operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
        let _lock = self.write_lock()?;
        if self.revision()? != expected { anyhow::bail!("settings_revision_conflict：恢复前设置已更新"); }
        operation()
    }

    /// 读取设置。**嵌套调用只解析、不再归一化**。
    ///
    /// 归一化会走到 `complete_relay_profile_config`，它对聚合供应商会回头读全量设置
    /// （拿聚合的 `session_provider`），而它本身又是在 `normalize_settings_config_sections`
    /// 里被调用的——不挡就是 `load → 归一化 → load → …` 无限递归，主线程栈溢出、
    /// 应用启动即崩。触发条件：聚合供应商的 `configContents` 非空（切换供应商时
    /// `backfill_relay_profile_from_live` 会把 live 配置回填进聚合档案）。
    pub fn load(&self) -> anyhow::Result<BackendSettings> {
        let contents = match self.read_contents()? {
            Some(contents) => contents,
            None => {
                let mut settings = BackendSettings::default();
                settings.sync_tool_shards();
                settings.settings_revision = settings_revision(None);
                return Ok(settings);
            }
        };

        let mut parsed: BackendSettings = serde_json::from_str(&contents).with_context(|| {
            format!(
                "settings {} 解析失败，拒绝退回默认设置以免覆盖用户配置",
                self.path.display()
            )
        })?;
        parsed.settings_revision = settings_revision(Some(contents.as_bytes()));
        if settings_normalize_depth() > 0 {
            // 外层已经在归一化了：这里再归一化就会递归回本函数。
            return Ok(parsed);
        }
        let _guard = SettingsNormalizeGuard::enter();
        Ok(normalize_settings_config_sections(parsed))
    }

    pub fn save(&self, settings: &BackendSettings) -> anyhow::Result<()> {
        self.save_versioned(settings).map(|_| ())
    }

    pub fn save_versioned(&self, settings: &BackendSettings) -> anyhow::Result<String> {
        self.save_with_commit(settings, |_| Ok(()))
    }

    /// 持有同一个跨进程锁发布关联产物；失败时仅恢复仍属于本提交的设置。
    pub fn save_with_commit(
        &self,
        settings: &BackendSettings,
        publish: impl FnOnce(&str) -> anyhow::Result<()>,
    ) -> anyhow::Result<String> {
        self.save_with_prepared_commit(settings, |_| Ok(()), publish)
    }

    pub(crate) fn save_with_prepared_commit(
        &self,
        settings: &BackendSettings,
        prepare: impl FnOnce(&str) -> anyhow::Result<()>,
        publish: impl FnOnce(&str) -> anyhow::Result<()>,
    ) -> anyhow::Result<String> {
        let _lock = self.write_lock()?;
        let revision = self.revision()?;
        if settings.settings_revision != revision
            && !(settings.settings_revision.is_empty() && revision == "missing")
        {
            anyhow::bail!("settings_revision_conflict：设置已被其他操作更新，请重载后合并；原文件未修改");
        }
        let before = self.snapshot_raw_bytes()?;
        let bytes = self.prepare_save_bytes(settings)?;
        let committed = settings_revision(Some(&bytes));
        prepare(&committed)?;
        atomic_write(&self.path, &bytes)?;
        if let Err(error) = publish(&committed) {
            if self.revision()? != committed {
                anyhow::bail!("关联产物发布失败：{error}；设置已被外部修改，未覆盖新数据");
            }
            match before {
                Some(bytes) => atomic_write(&self.path, &bytes)?,
                None => { fs::remove_file(&self.path)?; },
            }
            return Err(error).context("关联产物发布失败，设置已恢复");
        }
        Ok(committed)
    }

    fn prepare_save_bytes(&self, settings: &BackendSettings) -> anyhow::Result<Vec<u8>> {
        // `save()` 是整体覆盖写，manager 的 save_settings 走的就是这条路。
        // 历史事故：load 失败退回默认设置（只剩 1 条默认 profile），前端把它原样
        // 回写，用户的 N 条供应商配置被清空。这里在写盘前比一次条数。
        // 旧文件超限、不可读或 JSON 损坏时拒绝写入，不能把读取失败当成空配置。
        let raw = self.load_raw_object()?;
        validate_profile_updates(settings, &raw)?;
        let existing_profile_count = raw
            .get("relayProfiles")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        if settings.relay_profiles.is_empty() && existing_profile_count > 0 {
            anyhow::bail!(
                "拒绝写入 settings：磁盘上已有 {} 条供应商配置，本次保存的列表为空；为避免清空用户配置，已保持原文件不变",
                existing_profile_count
            );
        }
        if is_default_single_profile(&settings.relay_profiles) && existing_profile_count > 1 {
            anyhow::bail!(
                "拒绝写入 settings：磁盘上已有 {} 条供应商配置，本次保存退化为仅剩默认供应商；为避免覆盖用户配置，已保持原文件不变",
                existing_profile_count
            );
        }
        // 先限制输入序列化大小，再复制/归一化；转义后的体积也计入预算。
        serialize_settings_bounded(settings)?;
        let mut settings = normalize_settings_config_sections(settings.clone());
        settings.codex_extra_args = normalize_codex_extra_args(&settings.codex_extra_args);
        let mut next = settings_to_object(&settings);
        next.remove("revision");
        // 完整表单不能擦除新版本或拓展写入的未知字段；profile 以稳定 ID 对齐。
        preserve_unknown_settings_fields(&mut next, &raw)?;
        next.remove("revision");
        serialize_settings_bounded(&Value::Object(next))
    }

    /// 切换事务的原始字节快照，保留未识别字段；不得把读错伪装成缺失文件。
    pub(crate) fn snapshot_raw_bytes(&self) -> anyhow::Result<Option<Vec<u8>>> {
        let Some(contents) = self.read_contents()? else {
            return Ok(None);
        };
        let bytes = contents.into_bytes();
        validate_settings_snapshot(&bytes)?;
        Ok(Some(bytes))
    }

    /// 仅供已捕获原始状态的内部事务回滚，不经过普通保存的供应商塌缩判断。
    /// None 只能来自事务开始时文件不存在的快照，删除本次新建的 settings 文件。
    pub(crate) fn restore_raw_snapshot(&self, snapshot: Option<&[u8]>, expected_revision: &str) -> anyhow::Result<()> {
        let _lock = self.write_lock()?;
        if self.revision()? != expected_revision {
            anyhow::bail!("settings_revision_conflict：回滚期间设置已有新变化，未覆盖新数据");
        }
        // 本操作写出的文件也受预算约束；超限的当前文件可能是外部新写入，不能覆盖。
        self.read_contents()?;
        match snapshot {
            Some(bytes) => {
                validate_settings_snapshot(bytes)?;
                atomic_write(&self.path, bytes)
            }
            None => match fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).context("恢复缺失 settings 文件状态失败"),
            },
        }
    }

    pub fn update(&self, payload: Value) -> anyhow::Result<BackendSettings> {
        let Value::Object(payload) = payload else {
            return self.load();
        };
        let _lock = self.write_lock()?;
        if let Some(expected) = payload.get("revision") {
            let expected = expected.as_str().ok_or_else(|| anyhow::anyhow!("revision 必须是字符串"))?;
            if expected != self.revision()? {
                anyhow::bail!("settings_revision_conflict：设置已被其他操作更新，补丁未应用");
            }
        }

        serialize_settings_bounded(&payload)?;
        let mut raw = self.load_raw_object()?;
        let original_raw = raw.clone();
        let existing_profile_count = raw
            .get("relayProfiles")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        // 已移除的增强项不再接受更新；顺便清理旧版本留下的开关。
        for key in [
            "launchMode",
            "codexAppFastStartup",
            "codexAppZedRemoteOpen",
            "zedRemoteOpenStrategy",
            "zedRemoteProjectRegistryEnabled",
            "zedRemoteSyncToZedSettings",
            "codexAppUpstreamWorktreeCreate",
            "codexAppNativeMenuLocalization",
            "codexAppServiceTierControls",
            "codexAppCustomLayoutEnabled",
        ] {
            raw.remove(key);
        }
        merge_known_setting_fields(&mut raw, &payload);
        serialize_settings_bounded(&raw)?;
        let candidate: BackendSettings = serde_json::from_value(Value::Object(raw.clone()))?;
        validate_profile_updates(&candidate, &self.load_raw_object()?)?;
        let mut settings = normalize_settings_config_sections(
            candidate,
        );
        raw.insert(
            "relayCommonConfigContents".to_string(),
            Value::String(settings.relay_common_config_contents.clone()),
        );
        raw.insert(
            "relayContextConfigContents".to_string(),
            Value::String(settings.relay_context_config_contents.clone()),
        );
        // 归一化把扁平字段镜像进了 tools.codex，这里写回原始对象，
        // 否则 update 路径保存的分片会停留在迁移前的旧值。
        raw.insert(
            "tools".to_string(),
            serde_json::to_value(&settings.tools).unwrap_or_else(|_| Value::Object(Map::new())),
        );
        preserve_unknown_settings_fields(&mut raw, &original_raw)?;
        let new_profile_count = raw
            .get("relayProfiles")
            .and_then(Value::as_array)
            .map(|items| items.len())
            .unwrap_or(0);
        if new_profile_count == 0 && existing_profile_count > 0 {
            anyhow::bail!(
                "拒绝写入 settings：本次更新会把供应商列表从 {} 条清空为 0 条，已保持原文件不变",
                existing_profile_count
            );
        }
        raw.remove("revision");
        let bytes = serialize_settings_bounded(&Value::Object(raw))?;
        atomic_write(&self.path, &bytes)?;
        settings.settings_revision = settings_revision(Some(&bytes));
        Ok(settings)
    }

    fn load_raw_object(&self) -> anyhow::Result<Map<String, Value>> {
        let contents = match self.read_contents()? {
            Some(contents) => contents,
            None => {
                return Ok(settings_to_object(&BackendSettings::default()));
            }
        };

        match serde_json::from_str::<Value>(&contents) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(anyhow::anyhow!(
                "settings {} 顶层不是 JSON 对象，拒绝用默认值覆盖",
                self.path.display()
            )),
            Err(error) => Err(error).with_context(|| {
                format!(
                    "settings {} 解析失败，拒绝用默认值覆盖；请先修复该文件",
                    self.path.display()
                )
            }),
        }
    }

    fn read_contents(&self) -> anyhow::Result<Option<String>> {
        let file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("failed to open settings"),
        };
        let reject_large_file = || {
            anyhow::anyhow!(
                "settings {} 超过 16 MiB 上限，拒绝读取或覆盖；请保留原文件并先修复",
                self.path.display()
            )
        };
        let length = file.metadata()?.len();
        if length > MAX_SETTINGS_FILE_BYTES as u64 {
            return Err(reject_large_file());
        }
        let mut bytes = Vec::with_capacity(length as usize);
        // metadata 检查后文件仍可能增长，实际读取也必须有上限。
        file.take(MAX_SETTINGS_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SETTINGS_FILE_BYTES {
            return Err(reject_large_file());
        }
        Ok(Some(
            String::from_utf8(bytes).context("settings 不是有效 UTF-8，原文件未修改")?,
        ))
    }
}

fn validate_settings_snapshot(bytes: &[u8]) -> anyhow::Result<()> {
    if bytes.len() > MAX_SETTINGS_FILE_BYTES {
        anyhow::bail!("settings 快照超过 16 MiB 上限，原文件未修改");
    }
    if !serde_json::from_slice::<Value>(bytes)
        .context("settings 快照不是有效 JSON，原文件未修改")?
        .is_object()
    {
        anyhow::bail!("settings 快照顶层不是 JSON 对象，原文件未修改");
    }
    Ok(())
}

fn merge_known_setting_fields(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    if let Some(patch) = source.get("dictation").and_then(Value::as_object) {
        let mut dictation = target
            .get("dictation")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        merge_bool_setting(&mut dictation, patch, "enabled");
        for key in ["provider", "baseUrl", "apiKey", "apiKeyEnv", "model", "language"] {
            if let Some(value) = patch.get(key).and_then(Value::as_str) {
                dictation.insert(key.to_string(), Value::String(value.trim().to_string()));
            }
        }
        if let Some(value) = patch.get("timeoutSeconds").and_then(Value::as_u64) {
            dictation.insert(
                "timeoutSeconds".to_string(),
                Value::from(value.clamp(1, 600)),
            );
        }
        target.insert("dictation".to_string(), Value::Object(dictation));
    }
    merge_bool_setting(target, source, "relayPreserveNonproviderSettings");
    target.remove("codexAppPluginAutoExpand");
    target.remove("computerUseGuardEnabled");
    if let Some(value) = source.get("codexAppPath").and_then(Value::as_str) {
        target.insert("codexAppPath".to_string(), Value::String(value.to_string()));
    }
    // 自动接管的开关与策略也走同一份 settings.json；漏掉这两行会让开关「保存了但没生效」。
    if let Some(value) = source.get("codexAppAutoTakeover").and_then(Value::as_bool) {
        target.insert(
            "codexAppAutoTakeover".to_string(),
            Value::Bool(value),
        );
    }
    if let Some(value) = source
        .get("codexAppAutoTakeoverWhenActive")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppAutoTakeoverWhenActive".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source.get("codexExtraArgs").and_then(Value::as_array) {
        let args = value
            .iter()
            .filter_map(Value::as_str)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        target.insert(
            "codexExtraArgs".to_string(),
            Value::Array(
                normalize_codex_extra_args(&args)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
    }
    if let Some(value) = source.get("providerSyncEnabled").and_then(Value::as_bool) {
        target.insert("providerSyncEnabled".to_string(), Value::Bool(value));
    }
    if let Some(value) = source.get("ccsDbPath").and_then(Value::as_str) {
        target.insert(
            "ccsDbPath".to_string(),
            Value::String(value.trim().to_string()),
        );
    }
    if let Some(value) = source.get("relayProfilesEnabled").and_then(Value::as_bool) {
        target.insert("relayProfilesEnabled".to_string(), Value::Bool(value));
    }
    // 协议代理自动重试：管理器整份保存也会经过 normalize，这里补上部分更新路径，
    // 保证任何走 /settings/set 的调用都不会被静默丢弃。
    merge_bool_setting(target, source, "protocolProxyAutoRetry");
    if let Some(value) = source
        .get("protocolProxyAutoRetryMaxAttempts")
        .and_then(Value::as_u64)
    {
        target.insert(
            "protocolProxyAutoRetryMaxAttempts".to_string(),
            Value::from(clamp_protocol_proxy_auto_retry_max_attempts(
                value as u32,
            )),
        );
    }
    if let Some(value) = source
        .get("protocolProxyAutoRetryIntervalSeconds")
        .and_then(Value::as_u64)
    {
        target.insert(
            "protocolProxyAutoRetryIntervalSeconds".to_string(),
            Value::from(clamp_protocol_proxy_auto_retry_interval_seconds(value)),
        );
    }
    if let Some(items) = source
        .get("protocolProxyAutoRetryStatusCodes")
        .and_then(Value::as_array)
    {
        target.insert(
            "protocolProxyAutoRetryStatusCodes".to_string(),
            serde_json::json!(normalize_protocol_proxy_auto_retry_status_codes(
                &status_code_values(items)
            )),
        );
    }
    if let Some(value) = source
        .get("protocolProxyAutoRetryMode")
        .and_then(Value::as_str)
    {
        target.insert(
            "protocolProxyAutoRetryMode".to_string(),
            Value::String(normalize_protocol_proxy_auto_retry_mode(value)),
        );
    }
    if let Some(value) = source
        .get("protocolProxyAutoRetryCommitDelayMs")
        .and_then(Value::as_u64)
    {
        target.insert(
            "protocolProxyAutoRetryCommitDelayMs".to_string(),
            Value::from(clamp_protocol_proxy_auto_retry_commit_delay_ms(value)),
        );
    }
    // 请求体上限也走同一份 settings.json；漏登记会让「保存了但没生效」。
    if let Some(value) = source.get("helperMaxRequestBodyMib").and_then(Value::as_u64) {
        let clamped = value.min(u64::from(MAX_HELPER_MAX_REQUEST_BODY_MIB)) as u32;
        target.insert(
            "helperMaxRequestBodyMib".to_string(),
            Value::from(clamp_helper_max_request_body_mib(clamped)),
        );
    }
    if let Some(value) = source.get("enhancementsEnabled").and_then(Value::as_bool) {
        target.insert("enhancementsEnabled".to_string(), Value::Bool(value));
    }
    merge_bool_setting(target, source, "codexAppStandaloneQuotaUnlock");
    merge_bool_setting(target, source, "codexAppPluginMarketplaceUnlock");
    merge_bool_setting(target, source, "codexAppModelWhitelistUnlock");
    merge_bool_setting(target, source, "codexAppSessionDelete");
    merge_bool_setting(target, source, "codexAppMarkdownExport");
    merge_bool_setting(target, source, "codexAppSessionShare");
    merge_bool_setting(target, source, "codexAppUsageStatusLine");
    merge_bool_setting(target, source, "codexAppPasteFix");
    merge_bool_setting(target, source, "codexAppForceChineseLocale");
    if let Some(value @ ("off" | "rainbow" | "fireworks" | "stars")) =
        source.get("codexAppTypingEffect").and_then(Value::as_str)
    {
        target.insert(
            "codexAppTypingEffect".to_string(),
            Value::String(value.to_string()),
        );
    }
    merge_bool_setting(target, source, "codexAppThreadIdBadge");
    merge_bool_setting(target, source, "codexAppConversationView");
    merge_bool_setting(target, source, "codexAppThreadScrollRestore");
    merge_bool_setting(target, source, "codexAppNativeMenuPlacement");
    merge_bool_setting(target, source, "codexAppNativeBrowserRequireIdentification");
    merge_bool_setting(target, source, "codexAppNativeFastUnlock");
    merge_bool_setting(target, source, "codexAppPetRealMouseLook");
    merge_bool_setting(target, source, "codexAppActivityOverlayRight");
    merge_bool_setting(target, source, "codexAppCapacityAutoRetry");
    merge_bool_setting(target, source, "codexAppWhaleWidgetEnabled");
    if let Some(value @ ("auto" | "custom" | "off")) =
        source.get("codexAppWhaleBalanceProtocol").and_then(Value::as_str)
    {
        target.insert(
            "codexAppWhaleBalanceProtocol".to_string(),
            Value::String(value.to_string()),
        );
    }
    for key in [
        "codexAppWhaleBalancePath",
        "codexAppWhaleBalanceField",
        "codexAppWhaleBalanceCurrency",
    ] {
        if let Some(value) = source.get(key).and_then(Value::as_str) {
            target.insert(key.to_string(), Value::String(value.trim().to_string()));
        }
    }
    if let Some(value) = source
        .get("codexAppWhaleBalanceScale")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        target.insert("codexAppWhaleBalanceScale".to_string(), Value::from(value));
    }
    merge_bool_setting(target, source, "codexAppStepwiseEnabled");
    if let Some(value) = source
        .get("codexAppStepwiseGenerationMode")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppStepwiseGenerationMode".to_string(),
            Value::String(normalize_stepwise_generation_mode(value)),
        );
    }
    merge_bool_setting(target, source, "codexAppAnswerOutlineEnabled");
    merge_bool_setting(target, source, "codexAppStepwiseDirectSend");
    if let Some(value) = source
        .get("codexAppStepwiseBaseUrl")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppStepwiseBaseUrl".to_string(),
            Value::String(value.trim().trim_end_matches('/').to_string()),
        );
    }
    if let Some(value) = source.get("codexAppStepwiseApiKey").and_then(Value::as_str) {
        target.insert(
            "codexAppStepwiseApiKey".to_string(),
            Value::String(value.trim().to_string()),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseApiKeyEnv")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppStepwiseApiKeyEnv".to_string(),
            Value::String(if value.trim().is_empty() {
                default_stepwise_api_key_env()
            } else {
                value.trim().to_string()
            }),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseProtocol")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppStepwiseProtocol".to_string(),
            Value::String(normalize_stepwise_protocol(value)),
        );
    }
    if let Some(value) = source.get("codexAppStepwiseModel").and_then(Value::as_str) {
        target.insert(
            "codexAppStepwiseModel".to_string(),
            Value::String(value.trim().to_string()),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseMaxItems")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
    {
        target.insert(
            "codexAppStepwiseMaxItems".to_string(),
            Value::Number(serde_json::Number::from(clamp_stepwise_max_items(value))),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseMaxInputChars")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        target.insert(
            "codexAppStepwiseMaxInputChars".to_string(),
            Value::Number(serde_json::Number::from(clamp_stepwise_max_input_chars(
                value,
            ))),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseMaxOutputTokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        target.insert(
            "codexAppStepwiseMaxOutputTokens".to_string(),
            Value::Number(serde_json::Number::from(clamp_stepwise_max_output_tokens(
                value,
            ))),
        );
    }
    if let Some(value) = source
        .get("codexAppStepwiseTimeoutMs")
        .and_then(Value::as_u64)
    {
        target.insert(
            "codexAppStepwiseTimeoutMs".to_string(),
            Value::Number(serde_json::Number::from(clamp_stepwise_timeout_ms(value))),
        );
    }
    merge_bool_setting(target, source, "codexAppImageOverlayEnabled");
    if let Some(value) = source
        .get("codexAppImageOverlayPath")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppImageOverlayPath".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source
        .get("codexAppImageOverlayOpacity")
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
    {
        target.insert(
            "codexAppImageOverlayOpacity".to_string(),
            Value::Number(serde_json::Number::from(clamp_image_overlay_opacity(value))),
        );
    }
    if let Some(value) = source
        .get("codexAppImageOverlayFitMode")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppImageOverlayFitMode".to_string(),
            Value::String(normalize_image_overlay_fit_mode(value)),
        );
    }
    merge_bool_setting(target, source, "codexAppDreamSkinEnabled");
    merge_bool_setting(target, source, "codexAppDreamSkinPaused");
    if let Some(value) = source.get("codexAppDreamSkinTheme").and_then(Value::as_str) {
        target.insert(
            "codexAppDreamSkinTheme".to_string(),
            Value::String(normalize_dream_skin_theme(value)),
        );
    }
    if let Some(value) = source.get("codexAppDreamSkinThemeConfig")
        && serde_json::from_value::<DreamSkinThemeConfig>(value.clone()).is_ok()
    {
        target.insert("codexAppDreamSkinThemeConfig".to_string(), value.clone());
    }
    if let Some(value) = source
        .get("codexAppDreamSkinImagePath")
        .and_then(Value::as_str)
    {
        target.insert(
            "codexAppDreamSkinImagePath".to_string(),
            Value::String(value.trim().to_string()),
        );
    }
    if let Some(value) = source.get("codexGoalsEnabled").and_then(Value::as_bool) {
        target.insert("codexGoalsEnabled".to_string(), Value::Bool(value));
    }
    merge_bool_setting(target, source, "weixinConnectEnabled");
    for key in [
        "weixinConnectBaseUrl",
        "weixinConnectToken",
        "weixinConnectAccountId",
        "weixinConnectAllowFrom",
        "weixinConnectRouteTag",
        "weixinConnectWorkDir",
        "weixinConnectModel",
        "weixinConnectSandbox",
        "weixinConnectCodexPath",
    ] {
        if let Some(value) = source.get(key).and_then(Value::as_str) {
            target.insert(key.to_string(), Value::String(value.trim().to_string()));
        }
    }
    if let Some(value) = source.get("relayBaseUrl").and_then(Value::as_str) {
        target.insert("relayBaseUrl".to_string(), Value::String(value.to_string()));
    }
    if let Some(value) = source.get("relayApiKey").and_then(Value::as_str) {
        target.insert("relayApiKey".to_string(), Value::String(value.to_string()));
    }
    if let Some(value) = source.get("relayProfiles").and_then(Value::as_array) {
        // 逐个反序列化：单个坏条目只丢自己，不能把整份供应商列表清空。
        // 历史实现用 serde_json::from_value::<Vec<RelayProfile>>(...).unwrap_or_default()，
        // 任一 profile 字段不匹配就会让整个数组变空，随后被 update 写回磁盘。
        let mut profiles = Vec::with_capacity(value.len());
        let mut dropped = 0usize;
        for entry in value {
            match serde_json::from_value::<RelayProfile>(entry.clone()) {
                Ok(profile) => profiles.push(profile),
                Err(_) => dropped += 1,
            }
        }
        if dropped > 0 {
            eprintln!(
                "codex-plus settings: {dropped} 个供应商条目反序列化失败已跳过（共 {} 个）",
                value.len()
            );
        }
        // 新列表为空、或退化为仅剩默认 profile，而现有列表非空时，保留现有列表。
        // 空列表和「默认单条」都是 load 失败后默认设置被原样回写的形态。
        let existing_count = target
            .get("relayProfiles")
            .and_then(Value::as_array)
            .map(|existing| existing.len())
            .unwrap_or(0);
        let should_reject = (profiles.is_empty() && existing_count > 0)
            || (is_default_single_profile(&profiles) && existing_count > 1);
        if should_reject {
            eprintln!(
                "codex-plus settings: 新供应商列表退化为 {} 条，现有 {} 条，已拒绝覆盖",
                profiles.len(),
                existing_count
            );
        } else {
            preserve_official_mix_bearer_tokens(&mut profiles, target);
            target.insert(
                "relayProfiles".to_string(),
                serde_json::to_value(profiles).unwrap_or_else(|_| Value::Array(Vec::new())),
            );
        }
    }
    if let Some(value) = source
        .get("relayCommonConfigContents")
        .and_then(Value::as_str)
    {
        target.insert(
            "relayCommonConfigContents".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source
        .get("relayContextConfigContents")
        .and_then(Value::as_str)
    {
        target.insert(
            "relayContextConfigContents".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source.get("activeRelayId").and_then(Value::as_str) {
        target.insert(
            "activeRelayId".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source
        .get("aggregateRelayProfiles")
        .and_then(Value::as_array)
    {
        target.insert(
            "aggregateRelayProfiles".to_string(),
            Value::Array(value.clone()),
        );
    }
    if let Some(value) = source.get("activeAggregateRelayId").and_then(Value::as_str) {
        target.insert(
            "activeAggregateRelayId".to_string(),
            Value::String(value.to_string()),
        );
    }
    if let Some(value) = source.get("relayTestModel").and_then(Value::as_str) {
        target.insert(
            "relayTestModel".to_string(),
            Value::String(if value.trim().is_empty() {
                default_relay_test_model()
            } else {
                value.trim().to_string()
            }),
        );
    }
    if let Some(value) = source.get("activeTool").and_then(Value::as_str) {
        target.insert(
            "activeTool".to_string(),
            Value::String(ToolId::parse(value).as_str().to_string()),
        );
    }
}

fn merge_bool_setting(target: &mut Map<String, Value>, source: &Map<String, Value>, key: &str) {
    if let Some(value) = source.get(key).and_then(Value::as_bool) {
        target.insert(key.to_string(), Value::Bool(value));
    }
}

fn preserve_official_mix_bearer_tokens(
    profiles: &mut [RelayProfile],
    previous: &Map<String, Value>,
) {
    let previous_tokens = previous
        .get("relayProfiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| serde_json::from_value::<RelayProfile>(value.clone()).ok())
        .filter_map(|profile| {
            if profile.relay_mode != RelayMode::Official || !profile.official_mix_api_key {
                return None;
            }
            let token = experimental_bearer_token_from_config_text(&profile.config_contents)?;
            Some((profile.id, token))
        })
        .collect::<HashMap<_, _>>();

    for profile in profiles {
        if profile.relay_mode != RelayMode::Official || !profile.official_mix_api_key {
            continue;
        }
        if experimental_bearer_token_from_config_text(&profile.config_contents).is_some() {
            continue;
        }
        let token = if profile.api_key.trim().is_empty() {
            previous_tokens.get(&profile.id).cloned()
        } else {
            Some(profile.api_key.trim().to_string())
        };
        let Some(token) = token else {
            continue;
        };
        profile.config_contents =
            set_or_replace_experimental_bearer_token(&profile.config_contents, &token);
    }
}

fn set_or_replace_experimental_bearer_token(contents: &str, token: &str) -> String {
    let mut doc = parse_toml_document(contents).unwrap_or_else(|_| DocumentMut::new());
    let session_provider_id =
        active_provider_id(&doc).unwrap_or_else(|| "codex-plus-relay".to_string());
    let transport_provider_id = if session_provider_id == "openai" {
        "custom"
    } else {
        session_provider_id.as_str()
    };
    doc["model_provider"] = toml_edit::value(session_provider_id.as_str());
    doc["model_providers"][transport_provider_id]["experimental_bearer_token"] =
        toml_edit::value(token.trim());
    ensure_text_newline(doc.to_string())
}

fn ensure_text_newline(mut value: String) -> String {
    if !value.is_empty() && !value.ends_with('\n') {
        value.push('\n');
    }
    value
}

fn experimental_bearer_token_from_config_text(contents: &str) -> Option<String> {
    let doc = parse_toml_document(contents).ok()?;
    let provider_id = active_provider_id(&doc)?;
    let token_from = |provider_id: &str| {
        doc.get("model_providers")
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(provider_id))
            .and_then(Item::as_table)
            .and_then(|provider| provider.get("experimental_bearer_token"))
            .and_then(Item::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    };
    token_from(&provider_id).or_else(|| {
        (provider_id == "openai")
            .then(|| token_from("custom"))
            .flatten()
    })
}

fn active_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
        .map(ToString::to_string)
}

fn parse_toml_document(contents: &str) -> anyhow::Result<DocumentMut> {
    let contents = contents.trim_start_matches('\u{feff}');
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .map_err(|error| anyhow::anyhow!("config.toml TOML 解析失败：{error}"))
    }
}

fn settings_to_object(settings: &BackendSettings) -> Map<String, Value> {
    match serde_json::to_value(settings).unwrap_or_else(|_| Value::Object(Map::new())) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn settings_revision(bytes: Option<&[u8]>) -> String {
    bytes.map(|bytes| format!("sha256:{:x}", Sha256::digest(bytes)))
        .unwrap_or_else(|| "missing".to_string())
}

fn validate_profile_updates(settings: &BackendSettings, raw: &Map<String, Value>) -> anyhow::Result<()> {
    let previous: BackendSettings = serde_json::from_value(Value::Object(raw.clone()))?;
    for profile in &settings.relay_profiles {
        let mut normalized = profile.clone();
        if let Err(error) = crate::relay_config::normalize_relay_profile_for_storage(&mut normalized) {
            // 旧的不支持快照允许原样保留，不能阻断用户修改无关设置。
            if previous.relay_profiles.iter().any(|old| old.id == profile.id && old == profile) {
                continue;
            }
            if profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key
                && previous.relay_profiles.iter().any(|old| old.id == profile.id && old.auth_contents == profile.auth_contents)
            {
                let mut without_legacy_auth = profile.clone();
                without_legacy_auth.auth_contents.clear();
                crate::relay_config::normalize_relay_profile_for_storage(&mut without_legacy_auth)?;
                continue;
            }
            return Err(error).with_context(|| format!("供应商 {} 配置无效，原存档未修改", profile.id));
        }
    }
    Ok(())
}

fn preserve_unknown_settings_fields(
    next: &mut Map<String, Value>,
    old: &Map<String, Value>,
) -> anyhow::Result<()> {
    const KNOWN: &[&str] = &[
        "id",
        "name",
        "model",
        "baseUrl",
        "upstreamBaseUrl",
        "apiKey",
        "apiKeys",
        "activeApiKeyId",
        "protocol",
        "relayMode",
        "officialMixApiKey",
        "noAuth",
        "hideOfficialUsageAlert",
        "testModel",
        "configContents",
        "authContents",
        "useCommonConfig",
        "contextWindow",
        "autoCompactLimit",
        "modelInsertMode",
        "modelList",
        "modelWindows",
        "modelAutoCompact",
        "modelMetadata",
        "modelVlm",
        "vlmApiKey",
        "vlmModel",
        "vlmBaseUrl",
        "userAgent",
        "sub2apiEnabled",
        "sub2apiMultiplier",
        "modelRoutes",
        "customHeaders",
        "standardOpenaiProtocol",
        "webSearchHistoryCompat",
        "trustUpstreamCatalogLite",
        "strictResponsesReasoning",
        "anthropicAuthHeader",
        "anthropicMaxTokens",
        "anthropicPromptCache",
        "anthropicImpersonateClaudeCode",
        "anthropicThinkingMode",
    ];
    fn mark_profiles(value: &mut Value) {
        match value {
            Value::Object(object) => {
                if let Some(profiles) = object
                    .get_mut("relayProfiles")
                    .and_then(Value::as_array_mut)
                {
                    for profile in profiles.iter_mut().filter_map(Value::as_object_mut) {
                        // skip_serializing 的旧凭据/地址也是已知字段，绝不能当扩展字段补回。
                        for key in KNOWN {
                            profile.entry((*key).to_string()).or_insert(Value::Null);
                        }
                    }
                }
                for child in object.values_mut() {
                    mark_profiles(child);
                }
            }
            Value::Array(values) => {
                for child in values {
                    mark_profiles(child);
                }
            }
            _ => {}
        }
    }
    fn identity(value: &Value) -> Option<String> {
        if let Some(id) = value.get("id").and_then(Value::as_str) {
            return Some(format!("id:{id}"));
        }
        let relay = value.get("relayId").and_then(Value::as_str)?;
        Some(format!(
            "relay:{relay}:{}",
            value.get("pattern").and_then(Value::as_str).unwrap_or("")
        ))
    }
    fn merge(target: &mut Value, old: &Value, known: &Value) {
        match (target, old, known) {
            (Value::Object(target), Value::Object(old), Value::Object(known)) => {
                for (key, original) in old {
                    if let Some(shape) = known.get(key) {
                        if let Some(value) = target.get_mut(key) {
                            merge(value, original, shape);
                        }
                    } else {
                        target
                            .entry(key.clone())
                            .or_insert_with(|| original.clone());
                    }
                }
            }
            (Value::Array(target), Value::Array(old), Value::Array(known)) => {
                for row in target {
                    let Some(id) = identity(row) else {
                        continue;
                    };
                    let matches: Vec<_> = old
                        .iter()
                        .enumerate()
                        .filter(|(_, old)| identity(old).as_ref() == Some(&id))
                        .collect();
                    if matches.len() != 1 {
                        continue;
                    }
                    let (index, original) = matches[0];
                    if let Some(shape) = known.get(index) {
                        merge(row, original, shape);
                    }
                }
            }
            _ => {}
        }
    }
    let previous: BackendSettings = serde_json::from_value(Value::Object(old.clone()))?;
    let mut known = serde_json::to_value(previous)?;
    known["ccsImportSources"] = known
        .get("ccsImportSources")
        .cloned()
        .unwrap_or(Value::Null);
    known["revision"] = Value::Null;
    for key in [
        "launchMode",
        "codexAppFastStartup",
        "codexAppZedRemoteOpen",
        "zedRemoteOpenStrategy",
        "zedRemoteProjectRegistryEnabled",
        "zedRemoteSyncToZedSettings",
        "codexAppUpstreamWorktreeCreate",
        "codexAppNativeMenuLocalization",
        "codexAppServiceTierControls",
        "codexAppCustomLayoutEnabled",
        "codexAppPluginAutoExpand",
        "computerUseGuardEnabled",
    ] {
        known[key] = Value::Null;
    }
    mark_profiles(&mut known);
    let mut target = Value::Object(std::mem::take(next));
    merge(&mut target, &Value::Object(old.clone()), &known);
    *next = target.as_object_mut().expect("settings object").clone();
    Ok(())
}
fn normalize_settings_config_sections(mut settings: BackendSettings) -> BackendSettings {
    settings.dictation.normalize();
    settings.codex_app_whale_balance_protocol =
        match settings.codex_app_whale_balance_protocol.trim() {
            "custom" => "custom",
            "off" => "off",
            _ => "auto",
        }
        .to_string();
    settings.codex_app_whale_balance_path = settings.codex_app_whale_balance_path.trim().to_string();
    settings.codex_app_whale_balance_field = settings.codex_app_whale_balance_field.trim().to_string();
    settings.codex_app_whale_balance_currency = settings
        .codex_app_whale_balance_currency
        .trim()
        .to_ascii_uppercase();
    if !settings.codex_app_whale_balance_scale.is_finite()
        || settings.codex_app_whale_balance_scale <= 0.0
    {
        settings.codex_app_whale_balance_scale = default_whale_balance_scale();
    }
    settings.codex_app_typing_effect = normalize_typing_effect(&settings.codex_app_typing_effect);
    settings.ccs_db_path = settings.ccs_db_path.trim().to_string();
    settings.ccs_import_sources.retain(|id, _| settings.relay_profiles.iter().any(|profile| &profile.id == id));
    let (common, extracted_context) =
        split_context_config_sections(&settings.relay_common_config_contents);
    let context = join_config_sections(&[
        settings.relay_context_config_contents.as_str(),
        extracted_context.as_str(),
    ]);
    settings.relay_common_config_contents = crate::relay_config::normalize_config_text(&common);
    settings.relay_context_config_contents = crate::relay_config::strip_legacy_skill_tables(
        &crate::relay_config::normalize_config_text(&context),
    );
    for profile in &mut settings.relay_profiles {
        let mut normalized = profile.clone();
        if crate::relay_config::normalize_relay_profile_for_storage(&mut normalized).is_ok() {
            *profile = normalized;
        }
    }
    settings.codex_app_image_overlay_opacity =
        clamp_image_overlay_opacity(settings.codex_app_image_overlay_opacity);
    settings.codex_app_image_overlay_fit_mode =
        normalize_image_overlay_fit_mode(&settings.codex_app_image_overlay_fit_mode);
    settings.codex_app_dream_skin_theme =
        normalize_dream_skin_theme(&settings.codex_app_dream_skin_theme);
    if settings.codex_app_dream_skin_theme_config == DreamSkinThemeConfig::default()
        && settings.codex_app_dream_skin_theme != default_dream_skin_theme()
    {
        settings.codex_app_dream_skin_theme_config.id = settings.codex_app_dream_skin_theme.clone();
    }
    settings.codex_app_dream_skin_image_path =
        settings.codex_app_dream_skin_image_path.trim().to_string();
    settings.codex_app_stepwise_base_url = settings
        .codex_app_stepwise_base_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    settings.codex_app_stepwise_api_key = settings.codex_app_stepwise_api_key.trim().to_string();
    settings.codex_app_stepwise_api_key_env =
        if settings.codex_app_stepwise_api_key_env.trim().is_empty() {
            default_stepwise_api_key_env()
        } else {
            settings.codex_app_stepwise_api_key_env.trim().to_string()
        };
    settings.codex_app_stepwise_protocol =
        normalize_stepwise_protocol(&settings.codex_app_stepwise_protocol);
    settings.codex_app_stepwise_generation_mode =
        normalize_stepwise_generation_mode(&settings.codex_app_stepwise_generation_mode);
    settings.codex_app_stepwise_model = settings.codex_app_stepwise_model.trim().to_string();
    settings.weixin_connect_base_url = settings
        .weixin_connect_base_url
        .trim()
        .trim_end_matches('/')
        .to_string();
    if settings.weixin_connect_base_url.is_empty() {
        settings.weixin_connect_base_url = default_weixin_connect_base_url();
    }
    settings.weixin_connect_token = settings.weixin_connect_token.trim().to_string();
    settings.weixin_connect_account_id = settings.weixin_connect_account_id.trim().to_string();
    settings.weixin_connect_allow_from = settings.weixin_connect_allow_from.trim().to_string();
    settings.weixin_connect_route_tag = settings.weixin_connect_route_tag.trim().to_string();
    settings.weixin_connect_work_dir = settings.weixin_connect_work_dir.trim().to_string();
    settings.weixin_connect_model = settings.weixin_connect_model.trim().to_string();
    settings.weixin_connect_sandbox = match settings.weixin_connect_sandbox.trim() {
        "workspace-write" => "workspace-write",
        "danger-full-access" => "danger-full-access",
        _ => "read-only",
    }
    .to_string();
    settings.weixin_connect_codex_path = settings.weixin_connect_codex_path.trim().to_string();
    // #2028 存量迁移：系统保护目录（WindowsApps）里的 CLI 无法被第三方进程执行，
    // 早期版本点「使用桌面版内置 CLI」写入过这类路径；置空后启动时会自动
    // 选用桌面版维护的标准 CLI，用户无需手动清理。
    if crate::app_paths::is_windows_store_cli_path(&settings.weixin_connect_codex_path) {
        settings.weixin_connect_codex_path = String::new();
    }
    settings.codex_app_stepwise_max_items =
        clamp_stepwise_max_items(settings.codex_app_stepwise_max_items);
    settings.codex_app_stepwise_max_input_chars =
        clamp_stepwise_max_input_chars(settings.codex_app_stepwise_max_input_chars);
    settings.codex_app_stepwise_max_output_tokens =
        clamp_stepwise_max_output_tokens(settings.codex_app_stepwise_max_output_tokens);
    settings.codex_app_stepwise_timeout_ms =
        clamp_stepwise_timeout_ms(settings.codex_app_stepwise_timeout_ms);
    settings.protocol_proxy_auto_retry_max_attempts =
        clamp_protocol_proxy_auto_retry_max_attempts(settings.protocol_proxy_auto_retry_max_attempts);
    settings.protocol_proxy_auto_retry_interval_seconds =
        clamp_protocol_proxy_auto_retry_interval_seconds(
            settings.protocol_proxy_auto_retry_interval_seconds,
        );
    settings.protocol_proxy_auto_retry_mode =
        normalize_protocol_proxy_auto_retry_mode(&settings.protocol_proxy_auto_retry_mode);
    settings.protocol_proxy_auto_retry_status_codes =
        normalize_protocol_proxy_auto_retry_status_codes(
            &settings
                .protocol_proxy_auto_retry_status_codes
                .iter()
                .map(|code| u64::from(*code))
                .collect::<Vec<_>>(),
        );
    settings.protocol_proxy_auto_retry_commit_delay_ms =
        clamp_protocol_proxy_auto_retry_commit_delay_ms(
            settings.protocol_proxy_auto_retry_commit_delay_ms,
        );
    settings.helper_max_request_body_mib =
        clamp_helper_max_request_body_mib(settings.helper_max_request_body_mib);
    // 扁平字段始终是 Codex 的唯一事实来源，这里把它镜像进 tools.codex；
    // 其它工具的分片原样保留。放在函数末尾，所有 load / save / update 路径
    // 都会经过，两边不会漂移。
    settings.sync_tool_shards();
    settings
}

fn split_context_config_sections(config: &str) -> (String, String) {
    let mut common = Vec::new();
    let mut context = Vec::new();
    let mut in_context_table = false;

    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_context_table = is_context_table_header(trimmed);
        }
        if in_context_table {
            context.push(line);
        } else {
            common.push(line);
        }
    }

    (
        normalize_text_config(common.join("\n")),
        normalize_text_config(context.join("\n")),
    )
}

fn is_context_table_header(header: &str) -> bool {
    header.starts_with("[mcp_servers.")
        || header.starts_with("[skills.")
        || header.starts_with("[plugins.")
}

fn join_config_sections(sections: &[&str]) -> String {
    let joined = sections
        .iter()
        .map(|section| section.trim())
        .filter(|section| !section.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    normalize_text_config(joined)
}

fn normalize_text_config(contents: String) -> String {
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

/// 逐级解析软链接（最多 16 层，防环），返回最终目标路径；不是软链接时返回 None。
fn resolve_symlink_target(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    let mut resolved = false;
    for _ in 0..16 {
        let Ok(metadata) = fs::symlink_metadata(&current) else { break };
        if !metadata.file_type().is_symlink() {
            break;
        }
        let Ok(target) = fs::read_link(&current) else { break };
        current = if target.is_absolute() {
            target
        } else {
            current.parent().map(|parent| parent.join(&target)).unwrap_or(target)
        };
        resolved = true;
    }
    resolved.then_some(current)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    atomic_write_with(path, |file| file.write_all(bytes))
}

/// 流式原子写入：内容由 `write_contents` 直接写进临时文件，调用方不必先把
/// 完整字节拼在内存里。大文件（如历史会话的 rollout JSONL）走这条路径。
pub fn atomic_write_with(
    path: &Path,
    write_contents: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> anyhow::Result<()> {
    // 目标是软链接时（常见于把 ~/.codex/config.toml 链到 dotfiles 仓库），
    // 原子替换要落在链接指向的真实文件上；直接 rename 会把链接本身换成普通文件，
    // 链接被悄悄断开，dotfiles 里的原文件不再更新。
    let resolved = resolve_symlink_target(path);
    let path = resolved.as_deref().unwrap_or(path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    // 保留原文件权限：临时文件默认权限不一定和它一致。
    let existing_permissions = match fs::metadata(path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read permissions for {}", path.display()));
        }
    };
    let temp_path = temp_path_for(path);
    let write_result = (|| -> std::io::Result<()> {
        let mut temp_file = File::create(&temp_path)?;
        write_contents(&mut temp_file)?;
        temp_file.flush()?;
        if let Some(permissions) = existing_permissions {
            temp_file.set_permissions(permissions)?;
        }
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error)
            .with_context(|| format!("failed to write temp file {}", temp_path.display()));
    }
    if let Err(error) = replace_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error).with_context(|| {
            format!(
                "failed to replace {} with {}",
                path.display(),
                temp_path.display()
            )
        });
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    use windows::core::PCWSTR;

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // Codex 客户端 / 杀毒软件可能短暂持有目标文件的读句柄，MoveFileExW 会以
    // ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION 失败。一次失败就让整次
    // 供应商切换报错对用户表现为「无法切换和保存供应商」（#2601 / #2500 /
    // #2612）。短退避重试几次，仍失败再把错误上抛。
    let mut last_error = None;
    for attempt in 0..6 {
        match unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(target.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("替换文件失败")))
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp_path = path.to_path_buf();
    let extension = path.extension().and_then(|value| value.to_str());
    temp_path.set_extension(match extension {
        Some(extension) => format!("{extension}.tmp"),
        None => "tmp".to_string(),
    });
    temp_path
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_symlink_and_updates_its_target() {
        let temp = tempfile::tempdir().unwrap();
        let dotfiles = temp.path().join("dotfiles");
        std::fs::create_dir_all(&dotfiles).unwrap();
        let target = dotfiles.join("config.toml");
        std::fs::write(&target, "model = \"old\"\n").unwrap();
        let home = temp.path().join(".codex");
        std::fs::create_dir_all(&home).unwrap();
        let link = home.join("config.toml");
        std::os::unix::fs::symlink("../dotfiles/config.toml", &link).unwrap();

        atomic_write(&link, b"model = \"new\"\n").unwrap();

        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "软链接不能被替换成普通文件");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "model = \"new\"\n");
        let leftovers: Vec<_> = std::fs::read_dir(&home).unwrap().filter_map(Result::ok)
            .map(|entry| entry.file_name()).collect();
        assert_eq!(leftovers.len(), 1, "链接所在目录不应残留临时文件：{leftovers:?}");
    }
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-plus-core-settings-test-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn claude_enhancement_switches_default_on_and_respect_explicit_off() {
        // 老用户的 settings.json 里没有这两个键：必须按默认开启读出来，否则升级后
        // 语音入口与中文界面会静默消失，用户会以为是坏了，而不是自己关掉的。
        let mut legacy = serde_json::to_value(BackendSettings::default()).unwrap();
        let legacy = legacy.as_object_mut().unwrap();
        legacy.remove("claudeAppVoiceInput");
        legacy.remove("claudeAppLocalization");
        let parsed: BackendSettings = serde_json::from_value(serde_json::Value::Object(legacy.clone())).unwrap();
        assert!(parsed.claude_app_voice_input, "缺少键时语音输入应按默认开启");
        assert!(parsed.claude_app_localization, "缺少键时界面汉化应按默认开启");

        let mut off = serde_json::to_value(BackendSettings::default()).unwrap();
        off["claudeAppVoiceInput"] = json!(false);
        off["claudeAppLocalization"] = json!(false);
        let parsed: BackendSettings = serde_json::from_value(off).unwrap();
        assert!(!parsed.claude_app_voice_input, "显式关闭必须被尊重");
        assert!(!parsed.claude_app_localization, "显式关闭必须被尊重");
    }

    #[test]
    fn atomic_write_replaces_existing_file_and_removes_temp_file() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, b"old").unwrap();

        atomic_write(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!dir.join("settings.json.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn settings_default_matches_expected_behavior() {
        let settings = BackendSettings::default();
        assert!(!settings.provider_sync_enabled);
        assert!(settings.relay_profiles_enabled);
        assert!(settings.enhancements_enabled);
        assert!(settings.codex_app_plugin_marketplace_unlock);
        assert!(!settings.codex_app_force_chinese_locale);
        assert!(!settings.codex_app_thread_id_badge);
        assert!(!settings.codex_goals_enabled);
        assert!(settings.codex_app_path.is_empty());
        assert!(settings.codex_extra_args.is_empty());
        assert_eq!(settings.relay_base_url, default_relay_base_url());
        assert!(settings.relay_api_key.is_empty());
        assert_eq!(settings.relay_profiles[0].relay_mode, RelayMode::Official);
        assert!(settings.relay_common_config_contents.is_empty());
        assert_eq!(settings.relay_test_model, default_relay_test_model());
        assert!(!settings.codex_app_stepwise_enabled);
        assert_eq!(settings.codex_app_stepwise_generation_mode, "auto");
        assert!(!settings.codex_app_answer_outline_enabled);
        assert!(!settings.codex_app_stepwise_direct_send);
        assert!(settings.codex_app_stepwise_base_url.is_empty());
        assert!(settings.codex_app_stepwise_api_key.is_empty());
        assert_eq!(
            settings.codex_app_stepwise_api_key_env,
            "CODEX_STEPWISE_API_KEY"
        );
        assert_eq!(settings.codex_app_stepwise_protocol, "chat_completions");
        assert!(settings.codex_app_stepwise_model.is_empty());
        assert_eq!(settings.codex_app_stepwise_max_items, 4);
        assert_eq!(settings.codex_app_stepwise_max_input_chars, 6000);
        assert_eq!(settings.codex_app_stepwise_max_output_tokens, 500);
        assert_eq!(settings.codex_app_stepwise_timeout_ms, 8000);
        assert!(!settings.weixin_connect_enabled);
        assert_eq!(
            settings.weixin_connect_base_url,
            crate::connect::DEFAULT_WEIXIN_BASE_URL
        );
        assert!(settings.weixin_connect_token.is_empty());
        assert_eq!(settings.weixin_connect_sandbox, "read-only");
    }

    #[test]
    fn settings_deserialize_normalizes_stepwise_protocol_and_supports_legacy_missing_field() {
        let defaults: BackendSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.codex_app_stepwise_protocol, "chat_completions");

        for protocol in [
            "chat_completions",
            "responses",
            "anthropic_messages",
            "auto",
        ] {
            let settings: BackendSettings = serde_json::from_value(json!({
                "codexAppStepwiseProtocol": format!(" {protocol} ")
            }))
            .unwrap();
            assert_eq!(settings.codex_app_stepwise_protocol, protocol);
        }

        let invalid: BackendSettings = serde_json::from_value(json!({
            "codexAppStepwiseProtocol": "unsupported"
        }))
        .unwrap();
        assert_eq!(invalid.codex_app_stepwise_protocol, "chat_completions");
    }

    #[test]
    fn settings_deserialize_defaults_stepwise_ui_settings() {
        let defaults: BackendSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.codex_app_stepwise_generation_mode, "auto");
        assert!(!defaults.codex_app_answer_outline_enabled);

        let explicitly_enabled: BackendSettings = serde_json::from_value(json!({
            "codexAppAnswerOutlineEnabled": true
        }))
        .unwrap();
        assert!(explicitly_enabled.codex_app_answer_outline_enabled);
    }

    #[test]
    fn settings_deserialize_normalizes_stepwise_generation_mode() {
        let manual: BackendSettings = serde_json::from_value(json!({
            "codexAppStepwiseGenerationMode": " manual "
        }))
        .unwrap();
        assert_eq!(manual.codex_app_stepwise_generation_mode, "manual");

        let invalid: BackendSettings = serde_json::from_value(json!({
            "codexAppStepwiseGenerationMode": "unsupported"
        }))
        .unwrap();
        assert_eq!(invalid.codex_app_stepwise_generation_mode, "auto");
    }

    #[test]
    fn settings_deserialize_ignores_removed_cli_wrapper_keys() {
        let settings: BackendSettings = serde_json::from_str(
            r#"{"codexAppPath":"C:\\Portable\\Codex\\app","providerSyncEnabled":true,"codexGoalsEnabled":true,"cliWrapperEnabled":true,"cliWrapperBaseUrl":"https://example.test","cliWrapperApiKey":"sk-test","cliWrapperApiKeyEnv":""}"#,
        )
        .unwrap();
        assert_eq!(settings.codex_app_path, r"C:\Portable\Codex\app");
        assert!(settings.provider_sync_enabled);
        assert!(settings.codex_goals_enabled);
        assert_eq!(settings.relay_base_url, default_relay_base_url());
        assert!(settings.codex_extra_args.is_empty());
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("cliWrapperEnabled").is_none());
        assert!(saved.get("cliWrapperBaseUrl").is_none());
        assert!(saved.get("cliWrapperApiKey").is_none());
        assert!(saved.get("cliWrapperApiKeyEnv").is_none());
    }

    #[test]
    fn settings_deserialize_keeps_plugin_marketplace_unlock_switch() {
        let settings: BackendSettings = serde_json::from_str(
            r#"{
                "codexAppPluginMarketplaceUnlock": true,
                "codexAppPluginAutoExpand": false
            }"#,
        )
        .unwrap();

        assert!(settings.codex_app_plugin_marketplace_unlock);
        let saved = serde_json::to_value(&settings).unwrap();
        assert!(saved.get("codexAppPluginAutoExpand").is_none());

        let legacy_settings: BackendSettings = serde_json::from_str(
            r#"{
                "codexAppForcePluginInstall": false
            }"#,
        )
        .unwrap();

        assert!(legacy_settings.codex_app_plugin_marketplace_unlock);
    }

    #[test]
    fn settings_deserialize_reads_codex_extra_args() {
        let settings: BackendSettings = serde_json::from_str(
            r#"{"codexExtraArgs":["--force_high_performance_gpu"," --ignored-trimmed-by-ui "]}"#,
        )
        .unwrap();

        assert_eq!(
            settings.codex_extra_args,
            vec![
                "--force_high_performance_gpu".to_string(),
                " --ignored-trimmed-by-ui ".to_string(),
            ]
        );
    }

    #[test]
    fn relay_profile_official_mix_api_key_defaults_to_false() {
        let profile: RelayProfile =
            serde_json::from_str(r#"{"id":"official","name":"官方","relayMode":"official"}"#)
                .unwrap();

        assert_eq!(profile.relay_mode, RelayMode::Official);
        assert!(!profile.official_mix_api_key);
        assert!(!profile.hide_official_usage_alert);
        assert!(profile.test_model.is_empty());
    }

    #[test]
    fn relay_profile_context_fields_default_to_empty() {
        let profile = RelayProfile::default();

        assert!(profile.use_common_config);
        assert!(profile.context_window.is_empty());
        assert!(profile.auto_compact_limit.is_empty());
        assert_eq!(profile.model_insert_mode, RelayModelInsertMode::Patch);
        assert!(profile.model_list.is_empty());
        assert!(profile.model_auto_compact.is_empty());
        assert!(profile.model_routes.is_empty());
        assert!(!profile.has_model_routes());
    }

    #[test]
    fn no_auth_relay_requires_protocol_proxy() {
        let settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                relay_mode: RelayMode::PureApi,
                no_auth: true,
                base_url: "https://relay.example.test/v1".to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        assert!(settings.active_relay_uses_protocol_proxy());
    }

    #[test]
    fn relay_profile_model_auto_compact_is_opt_in_and_round_trips() {
        let profile = RelayProfile::default();
        let serialized = serde_json::to_value(&profile).unwrap();
        assert!(serialized.get("modelAutoCompact").is_none());

        let profile: RelayProfile = serde_json::from_value(serde_json::json!({
            "id": "relay",
            "name": "Relay",
            "modelAutoCompact": "{\"gpt-5.6-sol\":\"84.329412%\"}"
        }))
        .unwrap();
        assert_eq!(
            profile.model_auto_compact,
            r#"{"gpt-5.6-sol":"84.329412%"}"#
        );
    }

    #[test]
    fn relay_profile_model_metadata_is_opt_in_and_round_trips() {
        let profile = RelayProfile::default();
        let serialized = serde_json::to_value(&profile).unwrap();
        assert!(serialized.get("modelMetadata").is_none());

        let profile: RelayProfile = serde_json::from_value(serde_json::json!({
            "id": "relay",
            "name": "Relay",
            "modelMetadata": "{\"gpt-5.6-sol\":{\"supports_search_tool\":true}}"
        }))
        .unwrap();
        assert_eq!(
            profile.model_metadata,
            r#"{"gpt-5.6-sol":{"supports_search_tool":true}}"#
        );
    }

    #[test]
    fn relay_profile_model_routes_roundtrip_in_camel_case() {
        let profile: RelayProfile = serde_json::from_str(
            r#"{
                "id":"relay-a",
                "name":"供应商 A",
                "modelRoutes":[{
                    "model":"gpt-5.6-luna",
                    "targetRelayId":"relay-b",
                    "targetModel":"provider-luna"
                }]
            }"#,
        )
        .unwrap();

        assert!(profile.has_model_routes());
        assert_eq!(profile.model_routes[0].model, "gpt-5.6-luna");
        assert_eq!(profile.model_routes[0].target_relay_id, "relay-b");
        assert_eq!(profile.model_routes[0].target_model, "provider-luna");

        let saved = serde_json::to_value(profile).unwrap();
        assert_eq!(saved["modelRoutes"][0]["targetRelayId"], "relay-b");
        assert_eq!(saved["modelRoutes"][0]["targetModel"], "provider-luna");
    }

    /// 旧版按供应商勾选上下文条目的 `contextSelection` / `contextSelectionInitialized`
    /// 已被上下文条目自身的 `enabled` 开关取代。历史 settings.json 里仍会带着这两个键，
    /// 反序列化必须容忍它们，否则老用户一升级配置就读不出来。
    #[test]
    fn relay_profile_context_fields_deserialize_from_camel_case() {
        let profile: RelayProfile = serde_json::from_str(
            r#"{
                "id":"relay-a",
                "name":"供应商 A",
                "contextSelection":{
                    "mcpServers":["context7"],
                    "skills":["writer"],
                    "plugins":["local"]
                },
                "contextSelectionInitialized":true,
                "useCommonConfig":false,
                "contextWindow":"200000",
                "autoCompactLimit":"160000",
                "modelInsertMode":"patch",
                "modelList":"qwen3-coder\ndeepseek-coder"
            }"#,
        )
        .unwrap();

        assert!(!profile.use_common_config);
        assert_eq!(profile.context_window, "200000");
        assert_eq!(profile.auto_compact_limit, "160000");
        assert_eq!(profile.model_insert_mode, RelayModelInsertMode::Patch);
        assert_eq!(profile.model_list, "qwen3-coder\ndeepseek-coder");
    }

    #[test]
    fn relay_profile_derived_fields_are_read_but_not_serialized() {
        let profile: RelayProfile = serde_json::from_str(
            r#"{
                "id":"relay-a",
                "name":"供应商 A",
                "model":"gpt-5.4",
                "baseUrl":"https://relay.example/v1",
                "apiKey":"sk-test",
                "configContents":"model = \"gpt-5.4\"\n",
                "authContents":"{\"OPENAI_API_KEY\":\"sk-test\"}"
            }"#,
        )
        .unwrap();

        assert_eq!(profile.model, "gpt-5.4");
        assert_eq!(profile.base_url, "https://relay.example/v1");
        assert_eq!(profile.api_key, "sk-test");

        let saved = serde_json::to_value(&profile).unwrap();
        assert!(saved.get("model").is_none());
        assert!(saved.get("baseUrl").is_none());
        assert!(saved.get("apiKey").is_none());
        assert_eq!(saved["configContents"], "model = \"gpt-5.4\"\n");
        assert_eq!(saved["authContents"], "{\"OPENAI_API_KEY\":\"sk-test\"}");
    }

    #[test]
    fn chat_protocol_profile_roundtrip_migrates_upstream_base_url_out_of_config() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        let settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                id: "relay-chat".to_string(),
                name: "DeepSeek".to_string(),
                protocol: RelayProtocol::ChatCompletions,
                relay_mode: RelayMode::PureApi,
                config_contents: r#"model = "deepseek-chat"
codex_plus_chat_base_url = "https://api.deepseek.com"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "http://127.0.0.1:57321/v1"
"#
                .to_string(),
                auth_contents: r#"{"OPENAI_API_KEY":"sk-test"}"#.to_string(),
                ..RelayProfile::default()
            }],
            active_relay_id: "relay-chat".to_string(),
            ..BackendSettings::default()
        };

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        let active = loaded.active_relay_profile();

        assert_eq!(active.protocol, RelayProtocol::ChatCompletions);
        assert_eq!(active.base_url, "https://api.deepseek.com");
        assert_eq!(active.upstream_base_url, "https://api.deepseek.com");
        assert_eq!(active.api_key, "sk-test");
        assert!(!active.config_contents.contains("codex_plus_chat_base_url"));

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        let profile = &saved["relayProfiles"][0];
        assert!(profile.get("baseUrl").is_none());
        assert_eq!(profile["upstreamBaseUrl"], "https://api.deepseek.com");
        assert!(profile.get("apiKey").is_none());
        assert!(
            !profile["configContents"]
                .as_str()
                .unwrap()
                .contains("codex_plus_chat_base_url")
        );
    }

    #[test]
    fn official_profile_without_mix_does_not_persist_api_config() {
        let settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                id: "official".to_string(),
                name: "官方".to_string(),
                relay_mode: RelayMode::Official,
                official_mix_api_key: false,
                hide_official_usage_alert: false,
                model: "gpt-5.5".to_string(),
                base_url: "https://relay.example/v1".to_string(),
                api_key: "sk-test".to_string(),
                config_contents: r#"model = "gpt-5.5"
model_provider = "custom"

[model_providers.custom]
requires_openai_auth = true
"#
                .to_string(),
                auth_contents: r#"{"OPENAI_API_KEY":"sk-test"}"#.to_string(),
                ..RelayProfile::default()
            }],
            active_relay_id: "official".to_string(),
            ..BackendSettings::default()
        };

        let value = settings_to_object(&normalize_settings_config_sections(settings));
        let profile = &value["relayProfiles"][0];
        assert_eq!(profile["relayMode"], "official");
        assert_eq!(profile["officialMixApiKey"], false);
        assert_eq!(profile["configContents"], "");
        assert_eq!(profile["authContents"], "");
        assert!(profile.get("model").is_none());
        assert!(profile.get("baseUrl").is_none());
        assert!(profile.get("apiKey").is_none());
    }

    #[test]
    fn official_mix_profile_keeps_key_in_config_not_auth() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        let settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                id: "official-mix".to_string(),
                name: "官方混入".to_string(),
                relay_mode: RelayMode::Official,
                official_mix_api_key: true,
                hide_official_usage_alert: false,
                model: "gpt-5.5".to_string(),
                base_url: "https://relay.example/v1".to_string(),
                api_key: "sk-mix".to_string(),
                config_contents: r#"model = "gpt-5.5"
model_provider = "custom"

[model_providers.custom]
requires_openai_auth = true
base_url = "https://relay.example/v1"
experimental_bearer_token = "sk-mix"
"#
                .to_string(),
                auth_contents: r#"{"OPENAI_API_KEY":"sk-mix","auth_mode":"chatgpt"}"#.to_string(),
                ..RelayProfile::default()
            }],
            active_relay_id: "official-mix".to_string(),
            ..BackendSettings::default()
        };

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        let profile = &loaded.relay_profiles[0];

        assert_eq!(profile.relay_mode, RelayMode::Official);
        assert!(profile.official_mix_api_key);
        assert_eq!(profile.api_key, "sk-mix");
        assert!(!profile.auth_contents.contains("OPENAI_API_KEY"));
        assert!(
            profile
                .config_contents
                .contains(r#"experimental_bearer_token = "sk-mix""#)
        );

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert!(saved["relayProfiles"][0].get("apiKey").is_none());
        assert!(
            !saved["relayProfiles"][0]["authContents"]
                .as_str()
                .unwrap()
                .contains("OPENAI_API_KEY")
        );
        assert!(
            saved["relayProfiles"][0]["configContents"]
                .as_str()
                .unwrap()
                .contains(r#"experimental_bearer_token = "sk-mix""#)
        );
    }

    #[test]
    fn settings_update_preserves_official_mix_key_when_payload_loses_it() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        store
            .save(&BackendSettings {
                relay_profiles: vec![RelayProfile {
                    id: "official-mix".to_string(),
                    name: "官方混入".to_string(),
                    relay_mode: RelayMode::Official,
                    official_mix_api_key: true,
                    hide_official_usage_alert: false,
                    config_contents: r#"model_provider = "custom"

[model_providers.other]
base_url = "https://other.example/v1"
experimental_bearer_token = "sk-other"

[model_providers.custom]
base_url = "https://relay.example/v1"
experimental_bearer_token = "sk-existing"
"#
                    .to_string(),
                    ..RelayProfile::default()
                }],
                active_relay_id: "official-mix".to_string(),
                ..BackendSettings::default()
            })
            .unwrap();

        let updated = store
            .update(json!({
                "relayProfiles": [{
                    "id": "official-mix",
                    "name": "官方混入",
                    "relayMode": "official",
                    "officialMixApiKey": true,
                    "configContents": "model_provider = \"custom\"\n\n[model_providers.other]\nbase_url = \"https://other.example/v1\"\nexperimental_bearer_token = \"sk-other\"\n\n[model_providers.custom]\nbase_url = \"https://relay.example/v1\"\nexperimental_bearer_token = \"\"\n",
                    "authContents": ""
                }],
                "activeRelayId": "official-mix"
            }))
            .unwrap();

        let profile = &updated.relay_profiles[0];
        assert_eq!(profile.api_key, "sk-existing");
        assert!(!profile.config_contents.contains("sk-other"));
        assert!(profile.config_contents.contains(
            r#"[model_providers.custom]
base_url = "https://relay.example/v1"
experimental_bearer_token = "sk-existing""#
        ));
    }

    #[test]
    fn official_mix_update_uses_api_key_when_config_token_missing() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayProfiles": [{
                    "id": "official-mix",
                    "name": "官方混入",
                    "relayMode": "official",
                    "officialMixApiKey": true,
                    "baseUrl": "https://relay.example/v1",
                    "apiKey": "sk-new",
                    "configContents": "model_provider = \"custom\"\n\n[model_providers.custom]\nbase_url = \"https://relay.example/v1\"\n",
                    "authContents": ""
                }],
                "activeRelayId": "official-mix"
            }))
            .unwrap();

        let profile = &updated.relay_profiles[0];
        assert_eq!(profile.api_key, "sk-new");
        assert!(
            profile
                .config_contents
                .contains(r#"experimental_bearer_token = "sk-new""#)
        );
        assert!(!profile.auth_contents.contains("OPENAI_API_KEY"));
    }

    #[test]
    fn settings_update_preserves_manual_official_mix_config_token() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayProfiles": [{
                    "id": "official-mix",
                    "name": "官方混入",
                    "relayMode": "official",
                    "officialMixApiKey": true,
                    "configContents": "model_provider = \"custom\"\n\n[model_providers.custom]\nbase_url = \"https://relay.example/v1\"\nexperimental_bearer_token = \"22222222222222222222222222222222222\"\n",
                    "authContents": ""
                }],
                "activeRelayId": "official-mix"
            }))
            .unwrap();

        let profile = &updated.relay_profiles[0];
        assert_eq!(profile.relay_mode, RelayMode::Official);
        assert!(profile.official_mix_api_key);
        assert_eq!(profile.api_key, "22222222222222222222222222222222222");
        assert!(
            profile
                .config_contents
                .contains(r#"experimental_bearer_token = "22222222222222222222222222222222222""#)
        );
        assert!(!profile.auth_contents.contains("OPENAI_API_KEY"));
    }

    fn normalized_default_settings() -> BackendSettings {
        // Keep expected values independent of the production normalizer.
        let mut expected = BackendSettings::default();
        expected.tools.insert(ToolId::Codex, ToolConfig::default());
        expected
    }

    #[test]
    fn settings_store_load_missing_file_returns_default() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let mut expected = normalized_default_settings();
        expected.settings_revision = "missing".into();
        assert_eq!(store.load().unwrap(), expected);
    }

    /// 坏 JSON 不再静默退回默认设置：解析失败必须报错，避免拿默认值把磁盘上的
    /// 供应商列表覆盖掉。原文件内容必须原样保留。
    #[test]
    fn settings_store_load_bad_json_returns_error_and_keeps_file() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{bad json").unwrap();
        let store = SettingsStore::new(path.clone());

        let error = store
            .load()
            .expect_err("坏 JSON 必须报错，而不是退回默认值");
        assert!(
            error.to_string().contains("解析失败"),
            "错误信息应指出解析失败，实际是: {error}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{bad json");
        assert!(store.save(&BackendSettings::default()).is_err());
        assert!(store.update(json!({"providerSyncEnabled": false})).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{bad json");
    }

    #[test]
    fn settings_store_rejects_oversized_existing_file_without_overwriting() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let mut file = File::create(&path).unwrap();
        file.write_all(b"keep-original").unwrap();
        file.set_len(MAX_SETTINGS_FILE_BYTES as u64 + 1).unwrap();
        let store = SettingsStore::new(path.clone());

        for error in [
            store.load().unwrap_err(),
            store.save(&BackendSettings::default()).unwrap_err(),
            store
                .update(json!({"providerSyncEnabled": false}))
                .unwrap_err(),
            store.snapshot_raw_bytes().unwrap_err(),
            store
                .restore_raw_snapshot(Some(b"{}".as_slice()), "invalid")
                .unwrap_err(),
        ] {
            assert!(error.to_string().contains("16 MiB"));
        }
        assert_eq!(
            fs::metadata(&path).unwrap().len(),
            MAX_SETTINGS_FILE_BYTES as u64 + 1
        );
        let mut prefix = [0_u8; 13];
        File::open(&path).unwrap().read_exact(&mut prefix).unwrap();
        assert_eq!(&prefix, b"keep-original");
    }

    #[test]
    fn settings_store_serialized_size_limit_counts_json_escaping() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let original = r#"{"providerSyncEnabled":false,"customField":"keep"}"#;
        fs::write(&path, original).unwrap();
        let store = SettingsStore::new(path.clone());
        let mut settings = store.load().unwrap();
        settings.codex_app_image_overlay_path = "\\".repeat(MAX_SETTINGS_FILE_BYTES / 2);

        assert!(
            store
                .save(&settings)
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        let error = store
            .update(json!({
                "codexAppImageOverlayPath": settings.codex_app_image_overlay_path,
            }))
            .unwrap_err();
        assert!(error.to_string().contains("16 MiB"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn settings_store_reads_file_at_size_limit() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let mut bytes = b"{\"providerSyncEnabled\":false}".to_vec();
        bytes.resize(MAX_SETTINGS_FILE_BYTES, b' ');
        fs::write(&path, bytes).unwrap();
        assert!(
            !SettingsStore::new(path)
                .load()
                .unwrap()
                .provider_sync_enabled
        );
    }

    /// 回归：一次把 relayProfiles 传空的 update，不得清掉磁盘上已有的供应商配置。
    /// 这就是「供应商配置一个都不剩」那次故障的核心防护。
    #[test]
    fn settings_store_update_does_not_wipe_existing_relay_profiles() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{
  "relayProfiles": [
    {"id": "relay-a", "name": "A", "relayMode": "pureApi"},
    {"id": "relay-b", "name": "B", "relayMode": "pureApi"}
  ],
  "activeRelayId": "relay-a"
}"#,
        )
        .unwrap();
        let store = SettingsStore::new(path.clone());

        assert_eq!(store.load().unwrap().relay_profiles.len(), 2);

        let _ = store.update(json!({ "relayProfiles": [] }));

        let after = store.load().unwrap();
        assert_eq!(
            after.relay_profiles.len(),
            2,
            "传空 relayProfiles 不得覆盖磁盘上已有的供应商配置"
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("relay-a"), "原条目不得被清空: {raw}");
        assert!(raw.contains("relay-b"), "原条目不得被清空: {raw}");
    }

    /// 回归：数组里混入一条反序列化失败的条目时，好条目必须保留，
    /// 而不是像旧实现那样整个列表退化成空。
    #[test]
    fn settings_store_update_keeps_good_profiles_when_one_entry_is_broken() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"relayProfiles":[]}"#).unwrap();
        let store = SettingsStore::new(path.clone());

        let _ = store.update(json!({
            "relayProfiles": [
                {"id": "relay-a", "name": "A", "relayMode": "pureApi"},
                {"id": 12345, "name": "broken"}
            ]
        }));

        let after = store.load().unwrap();
        assert_eq!(
            after.relay_profiles.len(),
            1,
            "坏条目应被跳过，好条目必须保留"
        );
        assert_eq!(after.relay_profiles[0].id, "relay-a");
    }

    /// 回归：一次把 relayProfiles 退化为「仅默认 profile」的 save，不得覆盖磁盘上
    /// 多条供应商配置。这就是「20 条变 1 条」那次故障的核心防护。
    #[test]
    fn settings_store_save_does_not_collapse_to_default_single_profile() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{
  "relayProfiles": [
    {"id": "relay-a", "name": "A", "relayMode": "pureApi"},
    {"id": "relay-b", "name": "B", "relayMode": "pureApi"},
    {"id": "relay-c", "name": "C", "relayMode": "pureApi"}
  ]
}"#,
        )
        .unwrap();
        let store = SettingsStore::new(path.clone());

        let collapsed = BackendSettings {
            relay_profiles: vec![RelayProfile::default()],
            ..store.load().unwrap()
        };
        let error = store
            .save(&collapsed)
            .expect_err("退化为默认单条必须被拒绝");
        assert!(
            error.to_string().contains("退化为仅剩默认"),
            "实际错误: {error}"
        );

        let after = store.load().unwrap();
        assert_eq!(after.relay_profiles.len(), 3, "原 3 条不得被覆盖");
        assert_eq!(after.relay_profiles[0].id, "relay-a");
    }

    #[test]
    fn settings_store_save_load_roundtrip_uses_custom_path() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("nested").join("settings.json"));
        let settings = BackendSettings {
            provider_sync_enabled: true,
            codex_extra_args: vec!["--force_high_performance_gpu".to_string()],
            ccs_db_path: dir.join("cc-switch.db").to_string_lossy().to_string(),
            ..BackendSettings::default()
        };

        store.save(&settings).unwrap();

        let mut expected = settings;
        expected.settings_revision = store.revision().unwrap();
        expected.tools.insert(ToolId::Codex, ToolConfig::default());
        assert_eq!(store.load().unwrap(), expected);
    }

    #[test]
    fn settings_store_model_routes_restore_target_credentials() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        let profile = |id: &str, base_url: &str, api_key: &str| RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            upstream_base_url: base_url.to_string(),
            config_contents: format!(
                "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nbase_url = \"{base_url}\"\n"
            ),
            auth_contents: format!(r#"{{"OPENAI_API_KEY":"{api_key}"}}"#),
            ..RelayProfile::default()
        };
        let mut source = profile("source", "https://source.example/v1", "sk-source");
        source.model_routes = vec![RelayModelRoute {
            model: "gpt-5.6-luna".to_string(),
            target_relay_id: "target".to_string(),
            target_model: String::new(),
        }];
        let settings = BackendSettings {
            active_relay_id: "source".to_string(),
            relay_profiles: vec![
                source,
                profile("target", "https://target.example/v1", "sk-target"),
            ],
            ..BackendSettings::default()
        };

        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();

        assert!(loaded.active_relay_uses_protocol_proxy());
        assert_eq!(
            loaded.relay_profiles[0].base_url,
            "https://source.example/v1"
        );
        assert_eq!(
            loaded.relay_profiles[1].base_url,
            "https://target.example/v1"
        );
        assert_eq!(loaded.relay_profiles[1].api_key, "sk-target");
        assert_eq!(
            loaded.relay_profiles[0].model_routes[0].target_relay_id,
            "target"
        );
    }

    #[test]
    fn settings_store_persists_and_normalizes_stepwise_protocol_and_generation_mode() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppStepwiseProtocol": "responses",
                "codexAppStepwiseGenerationMode": " manual "
            }))
            .unwrap();
        assert_eq!(updated.codex_app_stepwise_protocol, "responses");
        assert_eq!(
            store.load().unwrap().codex_app_stepwise_protocol,
            "responses"
        );
        assert_eq!(updated.codex_app_stepwise_generation_mode, "manual");
        assert_eq!(
            store.load().unwrap().codex_app_stepwise_generation_mode,
            "manual"
        );

        let invalid = store
            .update(json!({
                "codexAppStepwiseProtocol": "not-a-protocol",
                "codexAppStepwiseGenerationMode": "not-a-mode"
            }))
            .unwrap();
        assert_eq!(invalid.codex_app_stepwise_protocol, "chat_completions");
        assert_eq!(invalid.codex_app_stepwise_generation_mode, "auto");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(store.path).unwrap()).unwrap();
        assert_eq!(saved["codexAppStepwiseProtocol"], "chat_completions");
        assert_eq!(saved["codexAppStepwiseGenerationMode"], "auto");
    }

    #[test]
    fn settings_store_save_load_roundtrip_preserves_aggregate_relay_settings() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        let settings = BackendSettings {
            relay_profiles: vec![
                RelayProfile {
                    id: "relay-a".to_string(),
                    name: "中转 A".to_string(),
                    ..RelayProfile::default()
                },
                RelayProfile {
                    id: "relay-b".to_string(),
                    name: "中转 B".to_string(),
                    ..RelayProfile::default()
                },
                RelayProfile {
                    id: "agg".to_string(),
                    name: "聚合".to_string(),
                    relay_mode: RelayMode::Aggregate,
                    ..RelayProfile::default()
                },
            ],
            active_relay_id: "agg".to_string(),
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: "agg".to_string(),
                name: "聚合".to_string(),
                session_provider: RelaySessionProvider::Openai,
                model_aggregation_enabled: true,
                sync_default_member_login: false,
                strategy: AggregateRelayStrategy::WeightedRoundRobin,
                members: vec![
                    AggregateRelayMember {
                        relay_id: "relay-a".to_string(),
                        weight: 1,
                    },
                    AggregateRelayMember {
                        relay_id: "relay-b".to_string(),
                        weight: 3,
                    },
                ],
                routes: Vec::new(),
                default_relay_id: String::new(),
            }],
            active_aggregate_relay_id: "agg".to_string(),
            ..BackendSettings::default()
        };

        store.save(&settings).unwrap();

        let loaded = store.load().unwrap();
        let mut expected = normalize_settings_config_sections(settings);
        expected.settings_revision = store.revision().unwrap();
        let active_aggregate = loaded.active_aggregate_relay_profile().unwrap();
        assert_eq!(loaded, expected);
        assert_eq!(
            active_aggregate.strategy,
            AggregateRelayStrategy::WeightedRoundRobin
        );
        assert_eq!(active_aggregate.members[1].relay_id, "relay-b");
        assert_eq!(active_aggregate.members[1].weight, 3);
        assert_eq!(
            active_aggregate.session_provider,
            RelaySessionProvider::Openai
        );
        assert!(active_aggregate.model_aggregation_enabled);
        assert_eq!(
            loaded.active_relay_session_provider(),
            RelaySessionProvider::Openai
        );
        assert!(loaded.active_relay_uses_protocol_proxy());
    }

    #[test]
    fn active_relay_session_provider_reads_standard_profile_config() {
        let mut settings = BackendSettings {
            relay_profiles: vec![RelayProfile {
                config_contents: "model_provider = \"openai\"\n".to_string(),
                ..RelayProfile::default()
            }],
            ..BackendSettings::default()
        };

        assert_eq!(
            settings.active_relay_session_provider(),
            RelaySessionProvider::Openai
        );

        settings.relay_profiles[0].config_contents = "model_provider = \"custom\"\n".to_string();
        assert_eq!(
            settings.active_relay_session_provider(),
            RelaySessionProvider::Custom
        );
    }

    #[test]
    fn settings_store_update_only_mutates_present_known_fields() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        let initial = BackendSettings {
            provider_sync_enabled: false,
            ..BackendSettings::default()
        };
        store.save(&initial).unwrap();

        let updated = store
            .update(json!({
            "providerSyncEnabled": true,
            "codexAppPath": "C:\\Portable\\Codex\\Codex.exe",
            "enhancementsEnabled": false,
            "codexAppSessionDelete": false,
            "codexAppConversationView": true,
            "codexAppThreadIdBadge": true,
            "codexAppNativeFastUnlock": true,
            "codexAppPetRealMouseLook": true,
            "codexGoalsEnabled": true,
            "relayBaseUrl": "https://relay.example.test/v1",
            "relayApiKey": "sk-relay",
            "codexExtraArgs": ["--force_high_performance_gpu", "", "  ", " --enable-gpu "],
            "unknownKey": "ignored"
            }))
            .unwrap();

        assert!(updated.provider_sync_enabled);
        assert_eq!(updated.codex_app_path, r"C:\Portable\Codex\Codex.exe");
        assert!(!updated.enhancements_enabled);
        assert!(!updated.codex_app_session_delete);
        assert!(updated.codex_app_conversation_view);
        assert!(updated.codex_app_thread_id_badge);
        assert!(updated.codex_app_native_fast_unlock);
        assert!(updated.codex_app_pet_real_mouse_look);
        assert!(updated.codex_goals_enabled);
        assert_eq!(updated.relay_base_url, "https://relay.example.test/v1");
        assert_eq!(updated.relay_api_key, "sk-relay");
        assert_eq!(
            updated.codex_extra_args,
            vec![
                "--force_high_performance_gpu".to_string(),
                "--enable-gpu".to_string(),
            ]
        );
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_update_persists_image_overlay_settings() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppImageOverlayEnabled": true,
                "codexAppImageOverlayPath": "C:\\Users\\me\\Pictures\\overlay.png",
                "codexAppImageOverlayOpacity": 42,
                "codexAppImageOverlayFitMode": "fill"
            }))
            .unwrap();

        assert!(updated.codex_app_image_overlay_enabled);
        assert_eq!(
            updated.codex_app_image_overlay_path,
            r"C:\Users\me\Pictures\overlay.png"
        );
        assert_eq!(updated.codex_app_image_overlay_opacity, 42);
        assert_eq!(updated.codex_app_image_overlay_fit_mode, "fill");
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_defaults_invalid_image_overlay_fit_mode_to_fit() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppImageOverlayFitMode": "unknown"
            }))
            .unwrap();

        assert_eq!(updated.codex_app_image_overlay_fit_mode, "fit");
    }

    #[test]
    fn settings_store_update_persists_dream_skin_settings() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppDreamSkinEnabled": true,
                "codexAppDreamSkinTheme": "miku",
                "codexAppDreamSkinImagePath": " C:\\Users\\me\\Pictures\\dream.webp "
            }))
            .unwrap();

        assert!(updated.codex_app_dream_skin_enabled);
        assert_eq!(updated.codex_app_dream_skin_theme, "miku");
        assert_eq!(
            updated.codex_app_dream_skin_image_path,
            r"C:\Users\me\Pictures\dream.webp"
        );
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_defaults_invalid_dream_skin_theme_to_pink() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppDreamSkinTheme": "unknown"
            }))
            .unwrap();

        assert_eq!(updated.codex_app_dream_skin_theme, "pink");
    }

    #[test]
    fn legacy_market_theme_ids_resolve_to_layout_presets() {
        assert_eq!(
            resolve_dream_skin_style_preset("preset-cyber-neon", "dream-original"),
            "cyber-neon"
        );
        assert_eq!(
            resolve_dream_skin_style_preset("codex-snow-skin", ""),
            "codex-snow"
        );
        assert_eq!(
            resolve_dream_skin_style_preset("custom-theme", "dream-original"),
            "dream-original"
        );
    }

    #[test]
    fn settings_store_update_persists_stepwise_settings() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "codexAppStepwiseEnabled": true,
                "codexAppStepwiseGenerationMode": "manual",
                "codexAppAnswerOutlineEnabled": false,
                "codexAppStepwiseDirectSend": true,
                "codexAppStepwiseBaseUrl": "https://api.example.test/v1/",
                "codexAppStepwiseApiKey": " sk-stepwise ",
                "codexAppStepwiseApiKeyEnv": "",
                "codexAppStepwiseModel": " stepwise-mini ",
                "codexAppStepwiseMaxItems": 12,
                "codexAppStepwiseMaxInputChars": 25000,
                "codexAppStepwiseMaxOutputTokens": 50,
                "codexAppStepwiseTimeoutMs": 70000
            }))
            .unwrap();

        assert!(updated.codex_app_stepwise_enabled);
        assert_eq!(updated.codex_app_stepwise_generation_mode, "manual");
        assert!(!updated.codex_app_answer_outline_enabled);
        assert!(updated.codex_app_stepwise_direct_send);
        assert_eq!(
            updated.codex_app_stepwise_base_url,
            "https://api.example.test/v1"
        );
        assert_eq!(updated.codex_app_stepwise_api_key, "sk-stepwise");
        assert_eq!(
            updated.codex_app_stepwise_api_key_env,
            default_stepwise_api_key_env()
        );
        assert_eq!(updated.codex_app_stepwise_model, "stepwise-mini");
        assert_eq!(updated.codex_app_stepwise_max_items, 6);
        assert_eq!(updated.codex_app_stepwise_max_input_chars, 24000);
        assert_eq!(updated.codex_app_stepwise_max_output_tokens, 100);
        assert_eq!(updated.codex_app_stepwise_timeout_ms, 60000);
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_update_persists_weixin_connect_settings() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "weixinConnectEnabled": true,
                "weixinConnectBaseUrl": "https://ilink.example.test/",
                "weixinConnectToken": " token ",
                "weixinConnectAccountId": " bot-1 ",
                "weixinConnectAllowFrom": " user@im.wechat ",
                "weixinConnectRouteTag": " route ",
                "weixinConnectWorkDir": " /workspace ",
                "weixinConnectModel": " gpt-test ",
                "weixinConnectSandbox": "workspace-write",
                "weixinConnectCodexPath": " /usr/local/bin/codex "
            }))
            .unwrap();

        assert!(updated.weixin_connect_enabled);
        assert_eq!(
            updated.weixin_connect_base_url,
            "https://ilink.example.test"
        );
        assert_eq!(updated.weixin_connect_token, "token");
        assert_eq!(updated.weixin_connect_account_id, "bot-1");
        assert_eq!(updated.weixin_connect_allow_from, "user@im.wechat");
        assert_eq!(updated.weixin_connect_route_tag, "route");
        assert_eq!(updated.weixin_connect_work_dir, "/workspace");
        assert_eq!(updated.weixin_connect_model, "gpt-test");
        assert_eq!(updated.weixin_connect_sandbox, "workspace-write");
        assert_eq!(updated.weixin_connect_codex_path, "/usr/local/bin/codex");
        assert_eq!(store.load().unwrap(), updated);
    }

    /// #2028 存量迁移：系统保护目录（WindowsApps）里的 CLI 无法被第三方进程执行，
    /// 旧版点「使用桌面版内置 CLI」写入过这类路径；归一化时置空，
    /// 让连接启动时自动选用桌面版维护的标准 CLI。
    #[test]
    fn windows_store_codex_path_is_cleared_for_auto_resolution() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "weixinConnectCodexPath":
                    r"C:\Program Files\WindowsApps\OpenAI.Codex_26.915.4065.0_x64__2p2nqsd0c76g0\app\resources\codex.exe"
            }))
            .unwrap();

        assert_eq!(updated.weixin_connect_codex_path, "");
        assert_eq!(store.load().unwrap().weixin_connect_codex_path, "");
    }

    #[test]
    fn legacy_launch_mode_is_ignored_without_changing_provider_modes() {
        for value in [
            json!("relay"),
            json!("patch"),
            json!("obsolete"),
            json!(true),
        ] {
            let mut old = serde_json::to_value(BackendSettings::default()).unwrap();
            old["launchMode"] = value;
            old["relayProfiles"][0]["relayMode"] = json!("pureApi");
            let loaded: BackendSettings = serde_json::from_value(old).unwrap();
            assert!(loaded.enhancements_enabled);
            assert_eq!(loaded.relay_profiles[0].relay_mode, RelayMode::PureApi);
            assert!(
                serde_json::to_value(loaded)
                    .unwrap()
                    .get("launchMode")
                    .is_none()
            );
        }
    }

    #[test]
    fn settings_update_cleans_legacy_launch_mode_and_preserves_provider_and_voice_configuration() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let mut old = serde_json::to_value(BackendSettings::default()).unwrap();
        old["launchMode"] = json!("relay");
        old["relayProfiles"][0]["relayMode"] = json!("pureApi");
        old["dictation"]["enabled"] = json!(true);
        old["dictation"]["apiKey"] = json!("fake-asr-private-key");
        old["preservedUnknownSetting"] = json!("keep");
        std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.relay_profiles[0].relay_mode, RelayMode::PureApi);
        assert!(
            serde_json::to_value(&loaded)
                .unwrap()
                .get("launchMode")
                .is_none()
        );
        let updated = store
            .update(json!({"launchMode": "relay", "enhancementsEnabled": false}))
            .unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("launchMode").is_none());
        assert_eq!(saved["preservedUnknownSetting"], "keep");
        assert_eq!(updated.relay_profiles[0].relay_mode, RelayMode::PureApi);
        assert!(updated.dictation.enabled);
        assert_eq!(updated.dictation.api_key, "fake-asr-private-key");
        assert!(!updated.enhancements_enabled);
        store.save(&updated).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(saved.get("launchMode").is_none());
    }

    #[test]
    fn settings_update_persists_dictation_provider_and_migrates_legacy_sensevoice_marker() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let mut legacy = serde_json::to_value(BackendSettings::default()).unwrap();
        legacy["dictation"]["enabled"] = json!(true);
        legacy["dictation"]["baseUrl"] = json!("local://sensevoice");
        legacy["dictation"]["model"] = json!("SenseVoiceSmall (INT8)");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.dictation.provider, DictationProvider::SenseVoice);

        let updated = store
            .update(json!({
                "dictation": {
                    "provider": "sensevoice",
                    "baseUrl": "local://sensevoice",
                    "enabled": true
                }
            }))
            .unwrap();
        assert_eq!(updated.dictation.provider, DictationProvider::SenseVoice);
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["dictation"]["provider"], "sensevoice");
    }

    #[test]
    fn settings_store_update_persists_relay_profiles_and_active_profile() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayProfiles": [
                    {
                        "id": "relay-a",
                        "name": "中转 A",
                        "baseUrl": "https://relay-a.example/v1",
                        "apiKey": "sk-a"
                    },
                    {
                        "id": "relay-b",
                        "name": "中转 B",
                        "baseUrl": "https://relay-b.example/v1",
                        "apiKey": "sk-b"
                    }
                ],
                "activeRelayId": "relay-b",
                "relayTestModel": "claude-sonnet-4"
            }))
            .unwrap();

        let active = updated.active_relay_profile();
        assert_eq!(updated.relay_profiles.len(), 2);
        assert_eq!(active.id, "relay-b");
        assert_eq!(active.name, "中转 B");
        assert_eq!(updated.relay_test_model, "claude-sonnet-4");

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        assert!(saved["relayProfiles"][1].get("baseUrl").is_none());
        assert!(saved["relayProfiles"][1].get("apiKey").is_none());
    }

    #[test]
    fn settings_store_update_does_not_persist_relay_profile_derived_fields() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayProfiles": [
                    {
                        "id": "relay-a",
                        "name": "供应商 A",
                        "model": "gpt-5.4",
                        "baseUrl": "https://relay.example/v1",
                        "apiKey": "sk-a",
                        "configContents": "model = \"gpt-5.4\"\n",
                        "authContents": "{\"OPENAI_API_KEY\":\"sk-a\"}"
                    }
                ],
                "activeRelayId": "relay-a"
            }))
            .unwrap();

        assert_eq!(updated.relay_profiles[0].id, "relay-a");
        assert_eq!(updated.relay_profiles[0].name, "供应商 A");

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap())
                .unwrap();
        let saved_profile = &saved["relayProfiles"][0];
        assert!(saved_profile.get("model").is_none());
        assert!(saved_profile.get("baseUrl").is_none());
        assert!(saved_profile.get("apiKey").is_none());
        assert_eq!(saved_profile["configContents"], "model = \"gpt-5.4\"\n");
        assert_eq!(
            saved_profile["authContents"],
            "{\"OPENAI_API_KEY\":\"sk-a\"}"
        );
    }

    #[test]
    fn settings_store_update_moves_context_tables_out_of_common_config() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayCommonConfigContents": "[mcp_servers.context7]\ncommand = \"npx\"\n"
            }))
            .unwrap();

        assert!(updated.relay_common_config_contents.is_empty());
        assert_eq!(
            updated.relay_context_config_contents,
            "[mcp_servers.context7]\ncommand = \"npx\"\n"
        );
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_update_extracts_context_config_from_common_config() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayCommonConfigContents": "model_reasoning_effort = \"high\"\n\n[mcp_servers.context7]\ncommand = \"npx\"\n\n[plugins.\"superpowers@openai-curated\"]\nenabled = true\n"
            }))
            .unwrap();

        assert_eq!(
            updated.relay_common_config_contents,
            "model_reasoning_effort = \"high\"\n"
        );
        assert!(
            updated
                .relay_context_config_contents
                .contains("[mcp_servers.context7]")
        );
        assert!(
            updated
                .relay_context_config_contents
                .contains("[plugins.\"superpowers@openai-curated\"]")
        );
        assert_eq!(store.load().unwrap(), updated);
    }

    #[test]
    fn settings_store_update_persists_aggregate_relay_profiles_and_active_id() {
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));

        let updated = store
            .update(json!({
                "relayProfiles": [
                    { "id": "relay-a", "name": "中转 A" },
                    { "id": "relay-b", "name": "中转 B" },
                    { "id": "agg", "name": "聚合", "relayMode": "aggregate" }
                ],
                "activeRelayId": "agg",
                "aggregateRelayProfiles": [
                    {
                        "id": "agg",
                        "name": "聚合",
                        "modelAggregationEnabled": true,
                        "strategy": "weightedRoundRobin",
                        "members": [
                            { "relayId": "relay-a", "weight": 1 },
                            { "relayId": "relay-b", "weight": 4 }
                        ]
                    }
                ],
                "activeAggregateRelayId": "agg"
            }))
            .unwrap();

        let active_aggregate = updated.active_aggregate_relay_profile().unwrap();
        assert_eq!(updated.active_relay_id, "agg");
        assert_eq!(updated.active_aggregate_relay_id, "agg");
        assert_eq!(
            active_aggregate.strategy,
            AggregateRelayStrategy::WeightedRoundRobin
        );
        assert!(active_aggregate.model_aggregation_enabled);
        assert_eq!(active_aggregate.members.len(), 2);
        assert_eq!(active_aggregate.members[1].relay_id, "relay-b");
        assert_eq!(active_aggregate.members[1].weight, 4);
        assert!(updated.active_relay_uses_protocol_proxy());
    }

    #[test]
    fn active_relay_profile_uses_legacy_single_relay_when_profiles_are_default() {
        let settings = BackendSettings {
            relay_base_url: "https://legacy.example/v1".to_string(),
            relay_api_key: "sk-legacy".to_string(),
            ..BackendSettings::default()
        };

        let active = settings.active_relay_profile();

        assert_eq!(active.id, "default");
        assert_eq!(active.name, "默认中转");
        assert_eq!(active.base_url, "https://legacy.example/v1");
        assert_eq!(active.api_key, "sk-legacy");
        assert_eq!(active.relay_mode, RelayMode::MixedApi);
        assert!(active.official_mix_api_key);
    }

    #[test]
    fn settings_store_update_preserves_existing_unknown_fields() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        std::fs::write(
            &path,
            r#"{"providerSyncEnabled":false,"customField":{"nested":true}}"#,
        )
        .unwrap();

        let updated = store
            .update(json!({
                "providerSyncEnabled": true
            }))
            .unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert!(updated.provider_sync_enabled);
        assert_eq!(saved["providerSyncEnabled"], json!(true));
        assert_eq!(saved["codexExtraArgs"], Value::Null);
        assert_eq!(saved["customField"], json!({"nested": true}));
    }

    #[test]
    fn settings_store_update_removes_obsolete_setting_fields() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        std::fs::write(
            &path,
            r#"{"providerSyncEnabled":false,"codexAppPluginAutoExpand":true,"computerUseGuardEnabled":true,"customField":1}"#,
        )
        .unwrap();

        store
            .update(json!({
                "providerSyncEnabled": true
            }))
            .unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert!(saved.get("codexAppPluginAutoExpand").is_none());
        assert!(saved.get("computerUseGuardEnabled").is_none());
        assert_eq!(saved["customField"], json!(1));
    }

    #[test]
    fn settings_store_update_persists_codex_extra_args_and_preserves_unknown_fields() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        std::fs::write(
            &path,
            r#"{"providerSyncEnabled":false,"customField":{"nested":true}}"#,
        )
        .unwrap();

        let updated = store
            .update(json!({
                "codexExtraArgs": ["--force_high_performance_gpu", "--enable-features=UseOzonePlatform"]
            }))
            .unwrap();
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(
            updated.codex_extra_args,
            vec![
                "--force_high_performance_gpu".to_string(),
                "--enable-features=UseOzonePlatform".to_string(),
            ]
        );
        assert_eq!(
            saved["codexExtraArgs"],
            json!([
                "--force_high_performance_gpu",
                "--enable-features=UseOzonePlatform"
            ])
        );
        assert_eq!(saved["customField"], json!({"nested": true}));
    }

    #[test]
    fn settings_store_update_with_non_object_payload_does_not_write_file() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        let original = r#"{"providerSyncEnabled":false,"customField":"keep me"}"#;
        std::fs::write(&path, original).unwrap();

        let updated = store.update(json!(null)).unwrap();

        assert!(!updated.provider_sync_enabled);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn relay_profile_standard_openai_protocol_defaults_off_for_legacy_profiles() {
        // 旧 profile 没有该字段：反序列化后默认关闭。
        let mut enabled = RelayProfile::default();
        enabled.standard_openai_protocol = true;
        let mut legacy = serde_json::to_value(&enabled).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("standardOpenaiProtocol");
        let profile: RelayProfile = serde_json::from_value(legacy).unwrap();
        assert!(!profile.standard_openai_protocol);
    }

    #[test]
    fn raw_settings_snapshot_restores_original_bytes_and_rejects_invalid_input() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let original = b"{\n \"providerSyncEnabled\":false,\"unknownFutureField\":{\"x\":1}}\n";
        fs::write(&path, original).unwrap();
        let store = SettingsStore::new(path.clone());
        let snapshot = store.snapshot_raw_bytes().unwrap();
        fs::write(&path, b"{\"providerSyncEnabled\":true}").unwrap();
        store.restore_raw_snapshot(snapshot.as_deref(), &store.revision().unwrap()).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
        for invalid in [b"{broken".as_slice(), b"[]".as_slice()] {
            assert!(store.restore_raw_snapshot(Some(invalid), &store.revision().unwrap()).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        let oversized = vec![b' '; MAX_SETTINGS_FILE_BYTES + 1];
        assert!(store.restore_raw_snapshot(Some(&oversized), &store.revision().unwrap()).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[test]
    fn raw_settings_snapshot_restores_a_previously_absent_file() {
        let dir = temp_dir();
        let path = dir.join("settings.json");
        let store = SettingsStore::new(path.clone());
        assert!(store.snapshot_raw_bytes().unwrap().is_none());
        store.save(&BackendSettings::default()).unwrap();
        store.restore_raw_snapshot(None, &store.revision().unwrap()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn relay_profile_standard_openai_protocol_round_trip_keeps_existing_providers() {
        // 关闭时导出不写该字段，round-trip 不改变既有 provider。
        let value = serde_json::to_value(RelayProfile::default()).unwrap();
        assert!(value.get("standardOpenaiProtocol").is_none());

        let mut enabled = RelayProfile::default();
        enabled.standard_openai_protocol = true;
        let value = serde_json::to_value(&enabled).unwrap();
        assert_eq!(value["standardOpenaiProtocol"], json!(true));
        let round_tripped: RelayProfile = serde_json::from_value(value).unwrap();
        assert!(round_tripped.standard_openai_protocol);
    }

    #[test]
    fn web_search_history_compat_is_default_off_and_round_trips_per_provider() {
        let legacy = serde_json::to_value(RelayProfile::default()).unwrap();
        assert!(legacy.get("webSearchHistoryCompat").is_none());
        let disabled: RelayProfile = serde_json::from_value(legacy).unwrap();
        assert!(!disabled.web_search_history_compat_enabled());
        let enabled = RelayProfile {
            relay_mode: RelayMode::PureApi,
            web_search_history_compat: true,
            ..RelayProfile::default()
        };
        let value = serde_json::to_value(&enabled).unwrap();
        assert_eq!(value["webSearchHistoryCompat"], json!(true));
        let restored: RelayProfile = serde_json::from_value(value).unwrap();
        assert!(restored.web_search_history_compat_enabled());
        let mut settings = BackendSettings {
            active_relay_id: "off".to_string(),
            relay_profiles: vec![
                RelayProfile { id: "off".to_string(), ..RelayProfile::default() },
                RelayProfile { id: "on".to_string(), ..enabled },
            ],
            ..BackendSettings::default()
        };
        assert!(!settings.active_relay_transport_uses_protocol_proxy());
        settings.active_relay_id = "on".to_string();
        assert!(settings.active_relay_transport_uses_protocol_proxy());
        let dir = temp_dir();
        let store = SettingsStore::new(dir.join("settings.json"));
        store.save(&settings).unwrap();
        settings = store.load().unwrap();
        assert!(settings.relay_profiles[1].web_search_history_compat);
        settings.relay_profiles[0].web_search_history_compat = true;
        settings.relay_profiles[1].web_search_history_compat = false;
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert!(loaded.relay_profiles[0].web_search_history_compat);
        assert!(!loaded.relay_profiles[1].web_search_history_compat);
    }

    #[test]
    fn web_search_history_compat_does_not_enable_official_or_chat_paths() {
        for (mode, protocol, mixed, expected) in [
            (RelayMode::Official, RelayProtocol::Responses, false, false),
            (RelayMode::Official, RelayProtocol::Responses, true, true),
            (RelayMode::Aggregate, RelayProtocol::Responses, false, false),
            (RelayMode::PureApi, RelayProtocol::ChatCompletions, false, false),
            (RelayMode::PureApi, RelayProtocol::Responses, false, true),
        ] {
            let profile = RelayProfile {
                relay_mode: mode,
                protocol,
                official_mix_api_key: mixed,
                web_search_history_compat: true,
                ..RelayProfile::default()
            };
            assert_eq!(profile.web_search_history_compat_enabled(), expected);
        }
    }
    // 自动接管的两个设置字段必须走既有 settings.json 链路持久化：
    // 守护线程在另一个线程里读同一个文件，写不进就等于开关无效。
    #[test]
    fn auto_takeover_settings_round_trip_through_settings_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(temp.path().join("settings.json"));

        let initial = BackendSettings::default();
        assert!(!initial.codex_app_auto_takeover);
        assert_eq!(
            initial.codex_app_auto_takeover_when_active,
            default_auto_takeover_when_active()
        );
        assert_eq!(initial.codex_app_auto_takeover_when_active, "defer");
        store.save(&initial).unwrap();

        let updated = store
            .update(json!({
                "codexAppAutoTakeover": true,
                "codexAppAutoTakeoverWhenActive": "immediate"
            }))
            .unwrap();
        assert!(updated.codex_app_auto_takeover);
        assert_eq!(updated.codex_app_auto_takeover_when_active, "immediate");

        // 重新读盘：字段名必须与前端/守护线程约定的一致（camelCase）。
        let reloaded = store.load().unwrap();
        assert!(reloaded.codex_app_auto_takeover);
        assert_eq!(reloaded.codex_app_auto_takeover_when_active, "immediate");
    }

}
