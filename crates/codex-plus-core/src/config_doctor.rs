//! 配置体检：先用本地规则把「一定有问题」的配置挑出来，再可选地把脱敏摘要交给
//! 一个 OpenAI 兼容的免费模型，让它用人话解释并给修复建议。
//!
//! 两条链路刻意分开：
//!
//! - [`run_rules`] 纯本地、离线可用、可测，不联网也不读凭据，出问题也只是一条 finding；
//! - [`run_ai`] 只在用户显式开启并点了按钮时跑，送出去的只有脱敏摘要（[`build_snapshot`]
//!   之后再过一遍 [`redact_secrets`]），永远不含 API Key。
//!
//! 体检结论只提出建议，不替用户改配置：写配置的地方在 `relay_config`，这里保持只读。

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::{BackendSettings, RelayProfile};

/// 体检用模型配置的落盘文件名（与 settings.json 分开存，避免污染整份设置）。
pub const CONFIG_FILE: &str = "config-doctor.json";

/// 单次 AI 体检的最长等待。免费额度慢的时候宁可超时报错，也不要挂住界面。
const AI_TIMEOUT: Duration = Duration::from_secs(90);
/// 摘要上限：免费模型上下文普遍不大，超了直接截断，别把请求撑爆。
const MAX_SNAPSHOT_CHARS: usize = 24_000;
/// 最多采纳几条 AI 结论，防止模型啰嗦刷屏。
const MAX_AI_ITEMS: usize = 8;

// ───────────────────────── 设置 ─────────────────────────

/// 体检用哪一路模型：
/// - [`SOURCE_CURRENT`]（默认）：直接复用 Codex 当前在用的供应商与模型，零额外配置；
/// - [`SOURCE_LOCAL`]：本机 Ollama / LM Studio，自动探测，免费；
/// - [`SOURCE_CUSTOM`]：自己填 OpenAI 兼容地址（想用哪家免费额度都行）。
pub const SOURCE_CURRENT: &str = "current";
pub const SOURCE_LOCAL: &str = "local";
pub const SOURCE_CUSTOM: &str = "custom";

/// 体检用的模型接入配置。`source = custom` 时 `base_url` 是 OpenAI 兼容前缀（拼 `/chat/completions`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DoctorSettings {
    /// 只有显式开启才允许把摘要发到外面。
    pub enabled: bool,
    /// 见 [`SOURCE_CURRENT`] / [`SOURCE_LOCAL`] / [`SOURCE_CUSTOM`]，空串按 current 处理。
    pub source: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    /// 上次选的预设，只用来回填下拉框。
    pub preset_id: String,
}

impl Default for DoctorSettings {
    fn default() -> Self {
        Self { enabled: false, source: SOURCE_CURRENT.into(), base_url: String::new(), model: String::new(), api_key: String::new(), preset_id: String::new() }
    }
}

/// 给前端的视图：**永不**回传明文 Key，只给「有没有」和打码后的样子。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSettingsView {
    pub enabled: bool,
    pub source: String,
    pub base_url: String,
    pub model: String,
    pub preset_id: String,
    pub has_key: bool,
    pub api_key_masked: String,
}

impl DoctorSettings {
    /// 归一化：历史上的配置没有 `source`，按「复用当前供应商」处理。
    pub fn source(&self) -> &str {
        match self.source.trim() {
            SOURCE_LOCAL => SOURCE_LOCAL,
            SOURCE_CUSTOM => SOURCE_CUSTOM,
            _ => SOURCE_CURRENT,
        }
    }

    /// 真正能跑 AI 体检。复用当前供应商不需要额外填任何东西。
    pub fn ready(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.source() {
            SOURCE_CUSTOM => !self.base_url.trim().is_empty() && !self.model.trim().is_empty(),
            _ => true,
        }
    }

    /// 拼成 `…/chat/completions`，容忍用户多写/少写结尾斜杠。
    pub fn endpoint(&self) -> String {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            String::new()
        } else {
            format!("{base}/chat/completions")
        }
    }

    pub fn view(&self) -> DoctorSettingsView {
        DoctorSettingsView {
            enabled: self.enabled,
            source: self.source().to_string(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            preset_id: self.preset_id.clone(),
            has_key: !self.api_key.trim().is_empty(),
            api_key_masked: mask_secret(&self.api_key),
        }
    }
}

/// 打码：短 key 全遮，长 key 留头 4 位与末 2 位，中间固定 `***`。
pub fn mask_secret(secret: &str) -> String {
    let secret = secret.trim();
    if secret.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 8 {
        return "***".into();
    }
    let head: String = chars[..4].iter().collect();
    let tail: String = chars[chars.len() - 2..].iter().collect();
    format!("{head}***{tail}")
}

pub fn settings_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(CONFIG_FILE)
}

/// 读不到或格式坏了都当默认值：体检配置坏掉不该让管理器起不来。
pub fn load_settings(path: &Path) -> DoctorSettings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<DoctorSettings>(&text).ok())
        .unwrap_or_default()
}

pub fn save_settings(path: &Path, settings: &DoctorSettings) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, format!("{text}\n"))?;
    Ok(())
}

// ───────────────────────── 预设 ─────────────────────────

/// 一个开箱可填的 OpenAI 兼容接入点。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
    pub needs_key: bool,
    pub note: &'static str,
    pub docs_url: &'static str,
}

/// 预设只解决「填什么」，不解决「谁付钱」：本地跑不要钱，云端免费额度各有时效，
/// 所以每条都把限流写进 note，别让用户以为是无条件的免费。
///
/// 顺序按「拿到就能用」排：本地 → 长期免费的免费档 → 有每日额度的 → 限流更紧的。
/// 已退役/必需绑卡的（GitHub Models、Cerebras、Together、Vercel AI Gateway）不放进预设。
pub fn presets() -> Vec<DoctorPreset> {
    vec![
        DoctorPreset {
            id: "ollama",
            label: "Ollama（本机）",
            base_url: "http://127.0.0.1:11434/v1",
            model: "qwen3:8b",
            needs_key: false,
            note: "本机推理，完全免费；先 ollama pull 对应模型",
            docs_url: "https://docs.ollama.com/api/openai-compatibility",
        },
        DoctorPreset {
            id: "lmstudio",
            label: "LM Studio（本机）",
            base_url: "http://127.0.0.1:1234/v1",
            model: "qwen/qwen3-8b",
            needs_key: false,
            note: "本地开 Local Server 即用，模型名随加载变",
            docs_url: "https://lmstudio.ai/docs/app/api/endpoints/openai",
        },
        DoctorPreset {
            id: "zhipu",
            label: "智谱 GLM（免费模型）",
            base_url: "https://open.bigmodel.cn/api/paas/v4",
            model: "GLM-4.7-Flash",
            needs_key: true,
            note: "Flash 系列输入输出全免费，国内直连",
            docs_url: "https://open.bigmodel.cn/usercenter/apikeys",
        },
        DoctorPreset {
            id: "cloudflare",
            label: "Cloudflare Workers AI",
            base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1",
            model: "@cf/meta/llama-3.1-8b-instruct",
            needs_key: true,
            note: "每天 10k Neurons 免费；地址里的 account_id 要换成自己的",
            docs_url: "https://dash.cloudflare.com/profile/api-tokens",
        },
        DoctorPreset {
            id: "siliconflow",
            label: "硅基流动 SiliconFlow",
            base_url: "https://api.siliconflow.cn/v1",
            model: "Qwen/Qwen3-8B",
            needs_key: true,
            note: "小模型长期免费，约 30 RPM；注册送额度",
            docs_url: "https://cloud.siliconflow.cn/account/ak",
        },
        DoctorPreset {
            id: "gemini",
            label: "Google Gemini 兼容层",
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            model: "gemini-3.8-flash",
            needs_key: true,
            note: "免费层按项目限流；内容可能被用于改进产品",
            docs_url: "https://aistudio.google.com/apikey",
        },
        DoctorPreset {
            id: "groq",
            label: "Groq",
            base_url: "https://api.groq.com/openai/v1",
            model: "openai/gpt-oss-120b",
            needs_key: true,
            note: "免卡注册；30 RPM、每天 1000 次，8K TPM 最紧",
            docs_url: "https://console.groq.com/keys",
        },
        DoctorPreset {
            id: "openrouter",
            label: "OpenRouter 免费模型",
            base_url: "https://openrouter.ai/api/v1",
            model: "google/gemma-4-31b-it:free",
            needs_key: true,
            note: "只有 :free 后缀免费，约 20 RPM、每天 50 次",
            docs_url: "https://openrouter.ai/settings/keys",
        },
        DoctorPreset {
            id: "mistral",
            label: "Mistral（Free mode）",
            base_url: "https://api.mistral.ai/v1",
            model: "mistral-small-latest",
            needs_key: true,
            note: "免费档免信用卡，具体限流后台看",
            docs_url: "https://console.mistral.ai/api-keys",
        },
        DoctorPreset {
            id: "dashscope",
            label: "阿里百炼（兼容模式）",
            base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
            model: "qwen3.5-flash",
            needs_key: true,
            note: "新用户每模型约 100 万 token，90 天有效",
            docs_url: "https://bailian.console.aliyun.com/",
        },
        DoctorPreset {
            id: "custom",
            label: "自定义（任意 OpenAI 兼容）",
            base_url: "",
            model: "",
            needs_key: false,
            note: "自己填 base_url 与模型名",
            docs_url: "",
        },
    ]
}

// ───────────────────────── 结论模型 ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warn,
    Info,
}

impl Level {
    fn parse(raw: &str) -> Level {
        match raw.trim().to_ascii_lowercase().as_str() {
            "error" | "fatal" | "critical" | "high" => Level::Error,
            "warn" | "warning" | "medium" => Level::Warn,
            _ => Level::Info,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub level: Level,
    pub title: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub error: usize,
    pub warn: usize,
    pub info: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiReport {
    /// `skipped` / `ok` / `failed`
    pub status: String,
    pub message: String,
    pub model: String,
    pub endpoint: String,
    pub elapsed_ms: u64,
    pub summary: String,
    pub items: Vec<Finding>,
}

impl AiReport {
    pub fn skipped(reason: &str, settings: &DoctorSettings) -> Self {
        Self {
            status: "skipped".into(),
            message: reason.to_string(),
            model: settings.model.clone(),
            endpoint: settings.endpoint(),
            elapsed_ms: 0,
            summary: String::new(),
            items: Vec::new(),
        }
    }

    pub fn failed(message: String, settings: &DoctorSettings, elapsed_ms: u64) -> Self {
        Self {
            status: "failed".into(),
            message,
            model: settings.model.clone(),
            endpoint: settings.endpoint(),
            elapsed_ms,
            summary: String::new(),
            items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub checked_at: String,
    pub findings: Vec<Finding>,
    pub summary: Summary,
    pub ai: AiReport,
}

/// 把规则结论与 AI 结论拼成一份报告（AI 侧只在这里汇总，不参与计数）。
pub fn report(findings: Vec<Finding>, ai: AiReport) -> DoctorReport {
    let summary = Summary {
        error: findings.iter().filter(|item| item.level == Level::Error).count(),
        warn: findings.iter().filter(|item| item.level == Level::Warn).count(),
        info: findings.iter().filter(|item| item.level == Level::Info).count(),
    };
    DoctorReport { checked_at: now_rfc3339(), findings, summary, ai }
}

/// 「这轮没跑 AI」的标准报告：前端按 status 渲染提示文案。
pub fn skipped_report(settings: &DoctorSettings, reason: &str) -> AiReport {
    AiReport::skipped(reason, settings)
}

fn now_rfc3339() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let rem = (seconds % 86_400) as u32;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// ───────────────────────── 本地规则 ─────────────────────────

fn finding(level: Level, id: impl Into<String>, title: impl Into<String>, detail: impl Into<String>) -> Finding {
    Finding {
        id: id.into(),
        level,
        title: title.into(),
        detail: detail.into(),
        fix: None,
        scope: None,
    }
}

impl Finding {
    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

/// 模型清单按行/逗号拆开，顺手去掉空行、`#` 注释与首尾空白。
pub fn model_entries(model_list: &str) -> Vec<String> {
    model_list
        .lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn parse_u64_field(raw: &str) -> Option<u64> {
    let digits: String = raw.trim().chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u64>().ok()
}

/// 把 `"90"` / `"90%"` / `90` 统一成百分比整数。
fn parse_percent(value: &Value) -> Option<u32> {
    let raw = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().trim_end_matches('%').trim().parse::<f64>().ok()?,
        _ => return None,
    };
    if !(1.0..=100.0).contains(&raw) {
        return None;
    }
    Some(raw.round() as u32)
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']').to_ascii_lowercase();
    host == "localhost" || host == "::1" || host.starts_with("127.") || host.ends_with(".localhost")
}

/// 只从 URL 里取 `scheme://host:port/path` 的形状信息，不解析查询串。
fn split_url(raw: &str) -> Option<(String, String, String)> {
    let (scheme, rest) = raw.split_once("://")?;
    let (authority, path) = match rest.find('/') {
        Some(index) => (&rest[..index], &rest[index..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), authority.to_string(), path.to_string()))
}

/// 全量本地体检入口。
pub fn run_rules(settings: &BackendSettings, home: &Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    if !settings.relay_profiles_enabled {
        findings.push(
            finding(Level::Warn, "relay.disabled", "供应商配置整体关闭", "「连接与供应商」开关关着，Codex 会用官方登录态跑，下面的供应商配置都不会生效。")
                .fix("确认要走自定义接入时，把该开关打开。"),
        );
    }

    let profiles = &settings.relay_profiles;
    if profiles.is_empty() {
        findings.push(
            finding(Level::Error, "relay.no_profiles", "没有任何供应商配置", "列表是空的，Codex 启动时会退回官方默认配置。")
                .fix("在「连接与供应商」里新增一个供应商。"),
        );
    }

    // 重复 id：保存后互相覆盖，是最难查的一类。
    let mut seen_ids = HashSet::new();
    for profile in profiles {
        if !profile.id.trim().is_empty() && !seen_ids.insert(profile.id.trim().to_string()) {
            findings.push(
                finding(Level::Error, "relay.duplicate_id", format!("供应商 id 重复：{}", profile.id), "两条配置共用一个 id，保存时后写的会盖掉前一条。")
                    .fix("删掉重复的那条，或重新导入一份配置。")
                    .scope(profile_scope(profile)),
            );
        }
    }

    // 同一个 base_url 配了两条：切供应商时容易切到「看起来一样」的另一条。
    let mut seen_urls: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for profile in profiles {
        let url = profile.base_url.trim().trim_end_matches('/').to_ascii_lowercase();
        if url.is_empty() {
            continue;
        }
        if let Some(first) = seen_urls.get(&url) {
            findings.push(
                finding(Level::Info, "relay.duplicate_url", "两个供应商用了同一个接入地址", format!("「{first}」和「{}」的 base_url 相同，切过去看起来不会有变化。", profile.name))
                    .fix("确认是否真的需要两条；不需要就删掉一条。")
                    .scope(profile_scope(profile)),
            );
        } else {
            seen_urls.insert(url, profile.name.clone());
        }
    }

    let active_id = active_profile(settings).id;
    for profile in profiles {
        check_profile(profile, profile.id == active_id, &mut findings);
    }
    check_aggregate(settings, &mut findings);
    check_generated_config(settings, home, &mut findings);
    findings
}

/// 当前在用哪条供应商：`active_relay_id` 命中谁就是谁；一条都没命中时取列表第一条，
/// 免得所有供应商都被当成「没在用」，真问题全降级成提示。
fn active_profile(backend: &BackendSettings) -> RelayProfile {
    backend
        .relay_profiles
        .iter()
        .find(|profile| profile.id == backend.active_relay_id)
        .or_else(|| backend.relay_profiles.first())
        .cloned()
        .unwrap_or_else(|| backend.active_relay_profile())
}

fn profile_scope(profile: &RelayProfile) -> String {
    let name = profile.name.trim();
    if name.is_empty() { "供应商".to_string() } else { name.to_string() }
}

fn check_profile(profile: &RelayProfile, is_active: bool, findings: &mut Vec<Finding>) {
    let scope = profile_scope(profile);

    let base_url = profile.base_url.trim();
    if base_url.is_empty() {
        // 没在用的供应商填一半很常见，别拿它当致命错误刷屏。
        findings.push(
            finding(if is_active { Level::Error } else { Level::Info }, "profile.base_url.empty", "接入地址是空的", "base_url 为空时请求会发不出去。")
                .fix("填供应商的 API 地址，例如 https://api.example.com/v1。")
                .scope(&scope),
        );
    } else if let Some((scheme, authority, path)) = split_url(base_url) {
        if scheme != "http" && scheme != "https" {
            findings.push(
                finding(Level::Error, "profile.base_url.scheme", format!("接入地址协议不对：{scheme}"), "只支持 http / https。")
                    .fix("改成 https:// 开头的完整地址。")
                    .scope(&scope),
            );
        }
        if scheme == "http" {
            let host = authority.rsplit('@').next().unwrap_or(&authority).to_string();
            if !is_loopback(&host) {
                findings.push(
                    finding(Level::Warn, "profile.base_url.insecure", "接入地址是明文 http", "API Key 会以明文发到网络上，同网段的人可以看到。")
                        .fix("供应商支持 https 时换成 https。")
                        .scope(&scope),
                );
            }
        }
        if path.contains("/chat/completions") || path.contains("/responses") {
            findings.push(
                finding(Level::Error, "profile.base_url.over_path", "接入地址写到了具体接口", "base_url 只要写到 `/v1`，Codex 会自己拼 `/responses` / `/chat/completions`。")
                    .fix("删掉地址末尾的 /chat/completions 或 /responses。")
                    .scope(&scope),
            );
        }
        if path.ends_with("//") || base_url.ends_with('/') {
            findings.push(
                finding(Level::Info, "profile.base_url.trailing_slash", "接入地址结尾多了斜杠", "多数中转能容忍，但拼出来的路径会变成 `//responses`，个别网关会 404。")
                    .fix("把结尾的 `/` 去掉。")
                    .scope(&scope),
            );
        }
        if base_url.contains(' ') {
            findings.push(
                finding(Level::Error, "profile.base_url.space", "接入地址里有空格", "地址是从别处复制来的，首尾或中间混进了空白字符。")
                    .fix("删掉空格后重新保存。")
                    .scope(&scope),
            );
        }
    } else {
        findings.push(
            finding(Level::Error, "profile.base_url.invalid", "接入地址不是完整 URL", format!("`{base_url}` 缺少 http(s):// 前缀，请求会被当成相对路径。"))
                .fix("补全成 https://域名/v1 的形式。")
                .scope(&scope),
        );
    }

    let has_key = !profile.api_key.trim().is_empty()
        || profile.api_keys.iter().any(|key| !key.api_key.trim().is_empty());
    if !has_key && !profile.uses_no_auth() && !profile.official_mix_api_key {
        findings.push(
            finding(if is_active { Level::Error } else { Level::Info }, "profile.api_key.missing", "没有填 API Key", "这条供应商需要鉴权，但 Key 是空的，请求会直接 401。")
                .fix("填入 Key；确认服务端不需要鉴权时，改用「纯 API + 免鉴权」。")
                .scope(&scope),
        );
    }
    if profile.active_api_key_id.trim().is_empty() && profile.api_keys.len() > 1 {
        findings.push(
            finding(Level::Warn, "profile.api_key.no_active", "配了多个 Key 但没选当前用哪个", "多 Key 轮换会退回第一条，轮换设置看起来没生效。")
                .fix("在 Key 列表里点选一条作为当前 Key。")
                .scope(&scope),
        );
    }

    let models = model_entries(&profile.model_list);
    if models.is_empty() {
        findings.push(
            finding(Level::Info, "profile.model_list.empty", "模型列表是空的", "Codex 会显示接入点返回的模型之外只有默认模型，切换模型时选项很少。")
                .fix("在「模型列表」里每行写一个模型名。")
                .scope(&scope),
        );
    }

    // 剥掉窗口后缀后的 slug 必须唯一，否则 catalog 里同一个模型会被写两次。
    let mut slugs: HashSet<String> = HashSet::new();
    for entry in &models {
        let (slug, window) = crate::model_suffix::parse_model_suffix(entry);
        if slug.is_empty() {
            findings.push(
                finding(Level::Warn, "profile.model_list.blank_slug", "模型列表里有空条目", format!("条目 `{entry}` 剥掉后缀后模型名是空的。"))
                    .fix("删掉这一行，或补上模型名。")
                    .scope(&scope),
            );
            continue;
        }
        if !slugs.insert(slug.clone()) {
            findings.push(
                finding(Level::Warn, "profile.model_list.duplicate", format!("模型重复：{slug}"), "同一个模型写了两行（可能一行带窗口后缀、一行不带），catalog 里会冲突。")
                    .fix("只保留一行。")
                    .scope(&scope),
            );
        }
        if let Some(window) = window {
            if window < 1_000 {
                findings.push(
                    finding(Level::Error, "profile.model_list.window_too_small", format!("{slug} 的窗口小得不合理"), format!("后缀解析出 {window} token，正常模型不会这么小。"))
                        .fix("检查后缀写法，`16K` / `128K` / `1M` 或纯数字。")
                        .scope(&scope),
                );
            }
            if window > 10_000_000 {
                findings.push(
                    finding(Level::Warn, "profile.model_list.window_too_large", format!("{slug} 的窗口大得离谱"), format!("后缀解析出 {window} token，超过 1000 万，多半是多写了一个数量级。"))
                        .fix("确认单位，`1M` 已经是 100 万。")
                        .scope(&scope),
                );
            }
        }
    }

    let context_window = profile.context_window.trim();
    let parsed_window = if context_window.is_empty() { None } else { parse_u64_field(context_window) };
    if !context_window.is_empty() && parsed_window.is_none() {
        findings.push(
            finding(Level::Error, "profile.context_window.invalid", "上下文窗口不是数字", format!("填的是 `{context_window}`，解析不出 token 数。"))
                .fix("只填数字，例如 128000。")
                .scope(&scope),
        );
    }
    if let Some(window) = parsed_window
        && !(1_000..=10_000_000).contains(&window)
    {
        findings.push(
            finding(Level::Warn, "profile.context_window.range", "上下文窗口超出常规范围", format!("{window} token 不在 1000 ~ 10000000 之间。"))
                .fix("按模型实际窗口填写，例如 128000、200000。")
                .scope(&scope),
        );
    }

    let limit = profile.auto_compact_limit.trim();
    if !limit.is_empty() {
        match parse_u64_field(limit) {
            None => findings.push(
                finding(Level::Error, "profile.auto_compact.invalid", "自动压缩阈值不是数字", format!("填的是 `{limit}`。"))
                    .fix("只填数字，且小于上下文窗口。")
                    .scope(&scope),
            ),
            Some(value) => {
                if let Some(window) = parsed_window {
                    if value >= window {
                        findings.push(
                            finding(Level::Error, "profile.auto_compact.too_late", "自动压缩阈值不低于上下文窗口", format!("阈值 {value} ≥ 窗口 {window}，等到触发时已经超窗，压缩可能来不及。"))
                                .fix("把阈值压到窗口的 80%~90%。")
                                .scope(&scope),
                        );
                    } else if value * 100 < window * 30 {
                        findings.push(
                            finding(Level::Warn, "profile.auto_compact.too_early", "自动压缩触发过早", format!("阈值 {value} 不到窗口 {window} 的 30%，会话还没做什么就会被压缩，容易丢上下文。"))
                                .fix("把阈值调到窗口的 80%~90%。")
                                .scope(&scope),
                        );
                    }
                }
            }
        }
    }

    check_model_map("modelWindows", &profile.model_windows, &models, &scope, findings, true);
    check_model_map("modelAutoCompact", &profile.model_auto_compact, &models, &scope, findings, false);

    if !profile.model_metadata.trim().is_empty() {
        match serde_json::from_str::<Value>(profile.model_metadata.trim()) {
            Ok(Value::Object(_)) => {}
            Ok(_) => findings.push(
                finding(Level::Error, "profile.model_metadata.shape", "模型元数据不是对象", "应该是一个 `{ \"模型名\": {...} }` 的 JSON 对象。")
                    .fix("改成对象形态的 JSON。")
                    .scope(&scope),
            ),
            Err(error) => findings.push(
                finding(Level::Error, "profile.model_metadata.invalid", "模型元数据不是合法 JSON", error.to_string())
                    .fix("检查逗号、引号和括号。")
                    .scope(&scope),
            ),
        }
    }

    let test_model = profile.test_model.trim();
    if !test_model.is_empty()
        && !models.is_empty()
        && !models.iter().any(|entry| crate::model_suffix::parse_model_suffix(entry).0.eq_ignore_ascii_case(test_model))
    {
        findings.push(
            finding(Level::Info, "profile.test_model.not_listed", "测试模型不在模型列表里", format!("`{test_model}` 不在 model_list 中，连接测试可能 404。"))
                .fix("改用列表里的模型，或把它加进列表。")
                .scope(&scope),
        );
    }

    let vlm_model = profile.vlm_model.trim();
    let vlm_base = profile.vlm_base_url.trim();
    if vlm_model.is_empty() && !vlm_base.is_empty() {
        findings.push(
            finding(Level::Warn, "profile.vlm.half", "只填了图片识别的地址", "没填模型名，识图请求发不出去。")
                .fix("补上 vlmModel，或把地址清空。")
                .scope(&scope),
        );
    }
    if !vlm_model.is_empty() && vlm_base.is_empty() {
        findings.push(
            finding(Level::Warn, "profile.vlm.half", "只填了图片识别的模型", "没填地址时会复用主供应商地址，识图能力依赖主地址是否支持该模型。")
                .fix("确认主供应商支持这个模型，或补上 vlmBaseUrl。")
                .scope(&scope),
        );
    }

    if !profile.config_contents.trim().is_empty() {
        if let Err(error) = profile.config_contents.parse::<toml_edit::DocumentMut>() {
            findings.push(
                finding(Level::Error, "profile.config_contents.invalid", "自定义 config.toml 不是合法 TOML", error.to_string())
                    .fix("修好这段自定义配置，或先清空它。")
                    .scope(&scope),
            );
        }
    }
    if !profile.auth_contents.trim().is_empty()
        && let Err(error) = serde_json::from_str::<Value>(profile.auth_contents.trim())
    {
        findings.push(
            finding(Level::Error, "profile.auth_contents.invalid", "自定义 auth.json 不是合法 JSON", error.to_string())
                .fix("修好这段 JSON，或先清空它。")
                .scope(&scope),
        );
    }
}

/// 窗口值在 settings.json 里是**数字字符串**（`"1048576"`），也有人直接写数字；
/// 两种都认，别把正常配置报成警告。
fn parse_window_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(parse_u64_field))
}

/// `modelWindows` / `modelAutoCompact` 这类「模型名 → 值」映射的共用检查。
///
/// 同一个供应商里写错很多条是常态（一次粘贴一整段），所以按供应商聚合，
/// 一条结论里列前几个名字，而不是每错一条刷一行。
fn check_model_map(
    label: &str,
    raw: &str,
    models: &[String],
    scope: &str,
    findings: &mut Vec<Finding>,
    windows: bool,
) {
    let raw = raw.trim();
    if raw.is_empty() {
        return;
    }
    let value: Value = match serde_json::from_str(raw) {
        Ok(value) => value,
        Err(error) => {
            findings.push(
                finding(Level::Error, format!("profile.{label}.invalid"), format!("{label} 不是合法 JSON"), error.to_string())
                    .fix(format!("改成 `{{ \"模型名\": 值 }}` 的 JSON 对象。"))
                    .scope(scope),
            );
            return;
        }
    };
    let Some(object) = value.as_object() else {
        findings.push(
            finding(Level::Error, format!("profile.{label}.shape"), format!("{label} 不是对象"), "顶层必须是 JSON 对象。")
                .fix(format!("改成 `{{ \"模型名\": 值 }}` 的 JSON 对象。"))
                .scope(scope),
        );
        return;
    };
    let known: BTreeSet<String> = models
        .iter()
        .map(|entry| crate::model_suffix::parse_model_suffix(entry).0.to_ascii_lowercase())
        .collect();
    let mut bad: Vec<String> = Vec::new();
    let mut orphans: Vec<String> = Vec::new();
    for (model, value) in object {
        let slug = crate::model_suffix::parse_model_suffix(model).0.to_ascii_lowercase();
        if !models.is_empty() && !known.contains(&slug) {
            orphans.push(model.clone());
        }
        let ok = if windows {
            parse_window_value(value).map(|window| (1_000..=10_000_000).contains(&window)).unwrap_or(false)
        } else {
            parse_percent(value).is_some()
        };
        if !ok {
            bad.push(format!("{model}（{value}）"));
        }
    }
    let summarize = |items: &[String]| {
        let head: Vec<String> = items.iter().take(3).cloned().collect();
        if items.len() > 3 {
            format!("{} 等 {} 条", head.join("、"), items.len())
        } else {
            head.join("、")
        }
    };
    if !bad.is_empty() {
        findings.push(
            finding(
                Level::Warn,
                format!("profile.{label}.range"),
                if windows { format!("{label} 里 {} 条窗口值不合理", bad.len()) } else { format!("{label} 里 {} 条百分比不合理", bad.len()) },
                summarize(&bad),
            )
            .fix(if windows { "按模型实际窗口填 1000 ~ 10000000 的数字。" } else { "填 1~100 的百分比，例如 \"90%\"。" })
            .scope(scope),
        );
    }
    if !orphans.is_empty() {
        findings.push(
            finding(
                Level::Info,
                format!("profile.{label}.orphan"),
                format!("{label} 里 {} 条没有对应模型", orphans.len()),
                summarize(&orphans),
            )
            .fix("删掉这些条目，或把对应模型加进模型列表。")
            .scope(scope),
        );
    }
}

fn check_aggregate(settings: &BackendSettings, findings: &mut Vec<Finding>) {
    let relay_ids: HashSet<String> = settings
        .relay_profiles
        .iter()
        .map(|profile| profile.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect();
    for aggregate in &settings.aggregate_relay_profiles {
        let name = if aggregate.name.trim().is_empty() { "聚合供应商".to_string() } else { aggregate.name.clone() };
        if aggregate.members.is_empty() {
            findings.push(
                finding(Level::Error, "aggregate.no_members", "聚合供应商没有任何成员", "没有成员就没法转发请求。")
                    .fix("至少加入一个普通供应商。")
                    .scope(&name),
            );
            continue;
        }
        let mut seen = HashSet::new();
        for member in &aggregate.members {
            let id = member.relay_id.trim();
            if id.is_empty() {
                findings.push(
                    finding(Level::Error, "aggregate.member.blank", "聚合成员没选供应商", "有一条成员是空的。")
                        .fix("删掉空成员或补选一个供应商。")
                        .scope(&name),
                );
            } else if !relay_ids.contains(id) {
                findings.push(
                    finding(Level::Error, "aggregate.member.missing", "聚合成员指向不存在的供应商", format!("成员 `{id}` 在供应商列表里找不到，多半是删掉了上游配置。"))
                        .fix("重新指定成员，或把被删的供应商加回来。")
                        .scope(&name),
                );
            } else if !seen.insert(id.to_string()) {
                findings.push(
                    finding(Level::Warn, "aggregate.member.duplicate", "聚合成员重复", format!("`{id}` 出现了多次，权重会被算重。"))
                        .fix("删掉重复的成员。")
                        .scope(&name),
                );
            }
        }
    }
}

/// 看实际生成出来的 `~/.codex/config.toml` 与模型 catalog：这是 Codex 真正读的东西，
/// 和 settings.json 里的草稿可能不一致（例如上次 apply 失败）。
fn check_generated_config(settings: &BackendSettings, home: &Path, findings: &mut Vec<Finding>) {
    let config_path = home.join("config.toml");
    let text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(_) => {
            if settings.relay_profiles_enabled {
                findings.push(
                    finding(Level::Warn, "config.missing", "还没有生成过 config.toml", "Codex 现在读的是它自己的默认配置，管理器里的设置还没落到磁盘。")
                        .fix("保存一次供应商配置，让它重新生成。")
                        .scope("生成结果"),
                );
            }
            return;
        }
    };
    let document = match text.parse::<toml_edit::DocumentMut>() {
        Ok(document) => document,
        Err(error) => {
            findings.push(
                finding(Level::Error, "config.invalid", "config.toml 不是合法 TOML", error.to_string())
                    .fix("清掉自定义 config 里写坏的地方；解析失败时 Codex 会启动不起来。")
                    .scope("生成结果"),
            );
            return;
        }
    };

    let Some(pointer) = document.get("model_catalog_json").and_then(|item| item.as_str()) else {
        findings.push(
            finding(Level::Info, "catalog.absent", "config.toml 里没有 model_catalog_json", "说明没有按模型声明上下文窗口，Codex 会用内置目录。")
                .fix("想按模型区分窗口时，在供应商里填模型列表与窗口。")
                .scope("生成结果"),
        );
        return;
    };
    let catalog_path = Path::new(pointer);
    let catalog_path = if catalog_path.is_absolute() { catalog_path.to_path_buf() } else { home.join(catalog_path) };
    let catalog_text = match std::fs::read_to_string(&catalog_path) {
        Ok(text) => text,
        Err(error) => {
            findings.push(
                finding(Level::Error, "catalog.missing", "模型目录文件不存在", format!("config.toml 指向 `{pointer}`，但读不出来：{error}"))
                    .fix("重新保存供应商配置，让 Codex++ 重新生成目录文件。")
                    .scope("生成结果"),
            );
            return;
        }
    };
    let catalog: Value = match serde_json::from_str(&catalog_text) {
        Ok(value) => value,
        Err(error) => {
            findings.push(
                finding(Level::Error, "catalog.invalid", "模型目录文件不是合法 JSON", error.to_string())
                    .fix("重新保存供应商配置再试。")
                    .scope("生成结果"),
            );
            return;
        }
    };
    let models = catalog.get("models").and_then(Value::as_array).cloned().unwrap_or_default();
    if models.is_empty() {
        findings.push(
            finding(Level::Warn, "catalog.empty", "模型目录里没有模型", "catalog 生成出来了但 models 是空的，Codex 里会看不到这些模型。")
                .fix("检查模型列表是否为空。")
                .scope("生成结果"),
        );
        return;
    }
    let missing: Vec<String> = models
        .iter()
        .filter_map(|model| model.get("slug").and_then(Value::as_str).map(str::to_string))
        .filter(|slug| {
            models
                .iter()
                .find(|model| model.get("slug").and_then(Value::as_str) == Some(slug.as_str()))
                .map(|model| model.get("context_window").and_then(Value::as_u64).unwrap_or(0) == 0)
                .unwrap_or(false)
        })
        .collect();
    if !missing.is_empty() {
        findings.push(
            finding(Level::Info, "catalog.no_window", "部分模型没有声明上下文窗口", format!("{} 个模型缺少 context_window：{}", missing.len(), missing.join("、")))
                .fix("给这些模型补窗口，自动压缩才会按真实窗口算。")
                .scope("生成结果"),
        );
    }
}

// ───────────────────────── 脱敏与摘要 ─────────────────────────

/// 兜底脱敏：即使调用方不小心把字段塞满，也不会把长 token 发给外部模型。
pub fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // 见过 `Bearer` 之后紧跟的那一段一律当凭据处理：它本身可能不够长，不合通用启发式。
    let mut expect_secret = false;
    for token in text.split_inclusive(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ':' | ';' | ')' | '(')) {
        let trimmed = token.trim_end_matches(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ':' | ';' | ')' | '('));
        let suffix = &token[trimmed.len()..];
        let lower = trimmed.to_ascii_lowercase();
        let is_scheme = matches!(lower.as_str(), "bearer" | "basic" | "token" | "apikey" | "api_key" | "authorization");
        if (expect_secret && trimmed.len() >= 8) || looks_secret(trimmed) {
            out.push_str("«已脱敏»");
        } else {
            out.push_str(trimmed);
        }
        expect_secret = is_scheme;
        out.push_str(suffix);
    }
    out
}

fn looks_secret(token: &str) -> bool {
    let value = token.trim();
    if value.len() < 12 {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if lower.starts_with("sk-") || lower.starts_with("sk_") || lower.starts_with("gsk_") || lower.starts_with("aiza") {
        return true;
    }
    if lower.starts_with("bearer") {
        return true;
    }
    // 连续的长 token（无标点、无中文）：API Key 的典型形状。
    value.len() >= 24
        && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && value.chars().any(|c| c.is_ascii_digit())
        && value.chars().any(|c| c.is_ascii_alphabetic())
}

/// 送给模型的摘要：只有结构、数量、地址形状与规则结论，没有任何凭据。
pub fn build_snapshot(settings: &BackendSettings, home: &Path, findings: &[Finding]) -> Value {
    let profiles: Vec<Value> = settings
        .relay_profiles
        .iter()
        .map(|profile| {
            let (scheme, host) = split_url(profile.base_url.trim())
                .map(|(scheme, authority, _)| (scheme, sanitize_host(&authority)))
                .unwrap_or_default();
            let models = model_entries(&profile.model_list);
            json!({
                "name": profile.name,
                "baseUrl": json!({ "scheme": scheme, "host": host }),
                "protocol": format!("{:?}", profile.protocol),
                "relayMode": format!("{:?}", profile.relay_mode),
                "noAuth": profile.no_auth,
                "hasApiKey": !profile.api_key.trim().is_empty() || profile.api_keys.iter().any(|key| !key.api_key.trim().is_empty()),
                "keyCount": profile.api_keys.len(),
                "contextWindow": profile.context_window,
                "autoCompactLimit": profile.auto_compact_limit,
                "modelCount": models.len(),
                "models": models.iter().take(20).collect::<Vec<_>>(),
            })
        })
        .collect();

    let flags = json!({
        "relayProfilesEnabled": settings.relay_profiles_enabled,
        "enhancementsEnabled": settings.enhancements_enabled,
        "providerSyncEnabled": settings.provider_sync_enabled,
        "codexAppDisableWorkView": settings.codex_app_disable_work_view,
        "codexAppStandaloneQuotaUnlock": settings.codex_app_standalone_quota_unlock,
    });

    let config_path = home.join("config.toml");
    let generated = json!({
        "configTomlExists": config_path.exists(),
        "catalogPointer": std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
            .and_then(|document| document.get("model_catalog_json").and_then(|item| item.as_str()).map(str::to_string)),
        "homeLabel": "~/.codex",
    });

    let rule_view: Vec<Value> = findings
        .iter()
        .map(|item| json!({ "id": item.id, "level": item.level, "title": item.title, "detail": item.detail, "scope": item.scope }))
        .collect();

    json!({
        "profiles": profiles,
        "activeProfile": settings.active_relay_profile().name,
        "flags": flags,
        "generated": generated,
        "ruleFindings": rule_view,
    })
}

/// 地址只保留 host:port，去掉用户信息与路径（路径里有时会塞 key）。
fn sanitize_host(authority: &str) -> String {
    let host = authority.rsplit('@').next().unwrap_or(authority);
    host.chars().take(64).collect()
}

pub fn ai_system_prompt() -> String {
    [
        "你是 Codex / OpenAI 兼容中转配置的排障专家。",
        "用户会给你一份脱敏后的配置摘要（JSON），里面已经有本地规则挑出来的问题。",
        "请补充规则没覆盖到的隐患，并用人话解释原因和修复办法。",
        "只输出一个 JSON 对象，不要 markdown 代码块，不要多余解释：",
        "{\"summary\":\"一句话总体结论\",\"items\":[{\"level\":\"error|warn|info\",\"title\":\"短标题\",\"detail\":\"为什么有问题\",\"fix\":\"怎么改\"}]}",
        "items 最多 8 条，按严重程度排序；没有别的问题就给空数组。",
        "不要编造摘要里没有的配置项，不要建议上传任何密钥或日志。",
    ]
    .join("\n")
}

/// 摘要压缩成字符串并截断，保证请求体大小可控。
pub fn snapshot_prompt_text(snapshot: &Value) -> String {
    let text = serde_json::to_string_pretty(snapshot).unwrap_or_else(|_| "{}".into());
    let text = redact_secrets(&text);
    if text.chars().count() <= MAX_SNAPSHOT_CHARS {
        return text;
    }
    text.chars().take(MAX_SNAPSHOT_CHARS).collect::<String>() + "\n…（已截断）"
}

pub fn ai_request_body(settings: &DoctorSettings, snapshot: &Value) -> Value {
    json!({
        "model": settings.model.trim(),
        "temperature": 0.2,
        "stream": false,
        "max_tokens": 1200,
        "messages": [
            { "role": "system", "content": ai_system_prompt() },
            { "role": "user", "content": snapshot_prompt_text(snapshot) },
        ],
    })
}

/// 从模型回复里抠出 `{summary, items}`。模型经常裹一层 ```json，这里两种都认；
/// 实在解析不出来就把整段当成 summary，至少不丢信息。
pub fn parse_ai_reply(text: &str) -> (String, Vec<Finding>) {
    let cleaned = strip_code_fence(text);
    let candidate = match (cleaned.find('{'), cleaned.rfind('}')) {
        (Some(start), Some(end)) if end > start => &cleaned[start..=end],
        _ => cleaned.as_str(),
    };
    let Ok(value) = serde_json::from_str::<Value>(candidate) else {
        return (cleaned.trim().to_string(), Vec::new());
    };
    let summary = value
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let title = item.get("title").and_then(Value::as_str)?.trim();
                    if title.is_empty() {
                        return None;
                    }
                    let level = Level::parse(item.get("level").and_then(Value::as_str).unwrap_or("info"));
                    let detail = item.get("detail").and_then(Value::as_str).unwrap_or_default().trim();
                    let fix = item.get("fix").and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty());
                    let mut finding = finding(level, "ai.suggestion", title.to_string(), detail.to_string());
                    finding.fix = fix.map(str::to_string);
                    Some(finding)
                })
                .take(MAX_AI_ITEMS)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (summary, items)
}

fn strip_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let body = trimmed.trim_start_matches("```");
    let body = body.split_once('\n').map(|(_, rest)| rest).unwrap_or(body);
    body.trim_end_matches("```").trim().to_string()
}


/// 本机模型（Ollama / LM Studio）绝不能踩系统代理：走代理等于连不上自己。
/// 远程地址按目标 URL 解析系统代理 / PAC，和插件市场那些请求一致。
fn build_client(settings: &DoctorSettings, endpoint: &str) -> anyhow::Result<reqwest::Client> {
    let loopback = split_url(settings.base_url.trim())
        .map(|(_, authority, _)| is_loopback(authority.rsplit('@').next().unwrap_or(&authority)))
        .unwrap_or(false);
    if loopback {
        return Ok(reqwest::Client::builder()
            .user_agent("CodexPlusPlus/config-doctor")
            .connect_timeout(Duration::from_secs(5))
            .timeout(AI_TIMEOUT)
            .no_proxy()
            .build()?);
    }
    crate::http_client::proxied_client_for_url_with_timeouts(
        "CodexPlusPlus/config-doctor",
        endpoint,
        Duration::from_secs(10),
        AI_TIMEOUT,
    )
}

// ───────────────────────── 三路来源 ─────────────────────────

/// 「复用当前供应商」这一路的可用性，给前端决定默认选项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentSource {
    pub available: bool,
    /// Codex 当前默认模型（config.toml 的 `model`）；空串说明还没配过。
    pub model: String,
    /// 当前供应商名字，只用于显示。
    pub profile: String,
    pub label: String,
}

/// 本机自动探测到的模型服务。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSource {
    pub found: bool,
    pub base_url: String,
    pub model: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSources {
    pub current: CurrentSource,
    pub local: LocalSource,
}

/// 当前 Codex 用的模型：以生成出来的 config.toml 为准（用户在 Codex 里换过模型，这里才是真的）。
pub fn current_source(backend: &BackendSettings, home: &Path) -> CurrentSource {
    let model = std::fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|document| document.get("model").and_then(|item| item.as_str()).map(str::to_string))
        .unwrap_or_default();
    let profile = active_profile(backend).name;
    let available = !model.trim().is_empty();
    CurrentSource {
        available,
        label: if available {
            format!("复用当前供应商（{profile} · {model}）")
        } else {
            "复用当前供应商（还没读到默认模型）".to_string()
        },
        model,
        profile,
    }
}

const LOCAL_CANDIDATES: [(&str, &str); 2] = [
    ("http://127.0.0.1:11434/v1", "Ollama（本机）"),
    ("http://127.0.0.1:1234/v1", "LM Studio（本机）"),
];

async fn probe_models(base_url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(700))
        .build()
        .ok()?;
    let value: Value = client
        .get(format!("{}/models", base_url.trim_end_matches('/')))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    value.get("data")?.as_array()?.first()?.get("id")?.as_str().map(str::to_string)
}

/// 探测本机 Ollama / LM Studio。两个并行探，最慢 ~700ms，插件没装就返回 `found: false`。
pub async fn detect_local_source() -> LocalSource {
    let (first, second) = tokio::join!(
        probe_models(LOCAL_CANDIDATES[0].0),
        probe_models(LOCAL_CANDIDATES[1].0)
    );
    for ((base, label), model) in LOCAL_CANDIDATES.iter().zip([first, second]) {
        if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
            return LocalSource { found: true, base_url: base.to_string(), model: model.clone(), label: label.to_string() };
        }
    }
    LocalSource { found: false, base_url: String::new(), model: String::new(), label: String::new() }
}

/// 复用当前供应商跑一次：走 Codex++ 自己的协议代理，和 Codex 用的路由/Key/模型完全一致，
/// 不需要用户再填任何东西。
async fn run_ai_current(settings: &DoctorSettings, snapshot: &Value) -> AiReport {
    let started = Instant::now();
    let params = json!({
        "input": snapshot_prompt_text(snapshot),
        "instructions": ai_system_prompt(),
        "maxOutputTokens": 1200,
    });
    match crate::extension_llm::complete(&params).await {
        Ok(reply) => {
            let text = reply.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            let model = reply.get("model").and_then(Value::as_str).unwrap_or_default().to_string();
            if text.trim().is_empty() {
                return AiReport::failed("当前供应商返回了空回复，可以在下面换成「本机模型」或「自定义」。".into(), settings, started.elapsed().as_millis() as u64);
            }
            let (summary, items) = parse_ai_reply(&text);
            AiReport {
                status: "ok".into(),
                message: format!("{} 条建议", items.len()),
                model,
                endpoint: "当前供应商".into(),
                elapsed_ms: started.elapsed().as_millis() as u64,
                summary,
                items,
            }
        }
        Err(error) => AiReport::failed(
            format!("用当前供应商没跑通：{error}。可以换成「本机模型」或「自定义」来源。"),
            settings,
            started.elapsed().as_millis() as u64,
        ),
    }
}

/// 按 `source` 分发：默认复用当前供应商；选本机就先探测再当自定义源用。
pub async fn run_ai_for(settings: &DoctorSettings, snapshot: &Value) -> AiReport {
    if !settings.enabled {
        return AiReport::skipped("AI 体检没开启，只跑了本地规则", settings);
    }
    match settings.source() {
        SOURCE_CUSTOM => run_ai(settings, snapshot).await,
        SOURCE_LOCAL => {
            let local = detect_local_source().await;
            if !local.found {
                return AiReport::failed("没在本机找到 Ollama 或 LM Studio（端口 11434 / 1234 都没响应）。".into(), settings, 0);
            }
            let mut custom = settings.clone();
            custom.source = SOURCE_CUSTOM.into();
            custom.base_url = local.base_url;
            custom.model = local.model;
            custom.api_key.clear();
            run_ai(&custom, snapshot).await
        }
        _ => run_ai_current(settings, snapshot).await,
    }
}

/// 调用 OpenAI 兼容接口。任何失败都变成 `ai.status = failed`，绝不 panic。
pub async fn run_ai(settings: &DoctorSettings, snapshot: &Value) -> AiReport {
    if !settings.ready() {
        return AiReport::skipped("AI 体检没开启，或还没填接入地址 / 模型", settings);
    }
    let started = Instant::now();
    let endpoint = settings.endpoint();
    let body = ai_request_body(settings, snapshot);
    let client = match build_client(settings, &endpoint) {
        Ok(client) => client,
        Err(error) => return AiReport::failed(format!("创建 HTTP 客户端失败：{error}"), settings, started.elapsed().as_millis() as u64),
    };
    let mut request = client.post(&endpoint).timeout(AI_TIMEOUT).json(&body);
    if !settings.api_key.trim().is_empty() {
        request = request.bearer_auth(settings.api_key.trim());
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return AiReport::failed(
                format!("连不上 `{endpoint}`：{error}。本地模型请确认服务已经启动。"),
                settings,
                started.elapsed().as_millis() as u64,
            );
        }
    };
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        let brief: String = text.chars().take(300).collect();
        return AiReport::failed(format!("模型返回 HTTP {status}：{brief}"), settings, started.elapsed().as_millis() as u64);
    }
    let reply: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            let brief: String = text.chars().take(300).collect();
            return AiReport::failed(format!("模型返回的不是 JSON：{brief}"), settings, started.elapsed().as_millis() as u64);
        }
    };
    let content = reply
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .or_else(|| reply.get("output_text").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    if content.trim().is_empty() {
        return AiReport::failed("模型返回了空回复".into(), settings, started.elapsed().as_millis() as u64);
    }
    let (summary, items) = parse_ai_reply(&content);
    AiReport {
        status: "ok".into(),
        message: format!("{} 条建议", items.len()),
        model: settings.model.clone(),
        endpoint,
        elapsed_ms: started.elapsed().as_millis() as u64,
        summary,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_secret_hides_middle() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("short"), "***");
        assert_eq!(mask_secret("sk-abcdefghijkl"), "sk-a***kl");
    }

    #[test]
    fn endpoint_tolerates_trailing_slash() {
        let settings = DoctorSettings {
            base_url: "http://127.0.0.1:11434/v1/".into(),
            model: "qwen3:8b".into(),
            ..Default::default()
        };
        assert_eq!(settings.endpoint(), "http://127.0.0.1:11434/v1/chat/completions");
    }

    #[test]
    fn redact_removes_key_shapes() {
        let text = r#"{"key":"sk-1234567890abcdefghijklmn","url":"https://api.example.com/v1"}"#;
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("1234567890abcdefghijklmn"), "{redacted}");
        assert!(redacted.contains("api.example.com"), "{redacted}");
    }

    #[test]
    fn parse_reply_accepts_code_fence() {
        let text = "```json\n{\"summary\":\"总体还行\",\"items\":[{\"level\":\"warn\",\"title\":\"阈值偏高\",\"detail\":\"可能超窗\",\"fix\":\"调到 90%\"}]}\n```";
        let (summary, items) = parse_ai_reply(text);
        assert_eq!(summary, "总体还行");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].level, Level::Warn);
        assert_eq!(items[0].fix.as_deref(), Some("调到 90%"));
    }

    #[test]
    fn parse_reply_falls_back_to_raw_text() {
        let (summary, items) = parse_ai_reply("配置看起来没问题，但建议把阈值调到 90%。");
        assert!(summary.contains("90%"));
        assert!(items.is_empty());
    }
}
