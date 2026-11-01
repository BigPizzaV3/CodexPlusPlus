//! 解析供应商 `/models` 响应里每个模型带的元数据：上下文窗口、思考等级、默认思考等级（#2807）。
//!
//! 中转站和网关常改模型的窗口和思考档位，响应里的字段五花八门：OpenAI 兼容的
//! `context_length` / `context_window` / `max_input_tokens`，OpenRouter 的
//! `top_provider.context_length`，Copilot 的 `capabilities.limits.max_context_window_tokens`，
//! Gemini 的 `inputTokenLimit`，还有 `context_windows` 档位数组、`reasoning_efforts` 等。
//! 这里按固定优先级各取一个；字段不存在、类型不对、数值不合理（0、负数、超大）的都跳过，
//! 不报错，只记进 [`IgnoredField`] 供界面提示。思考等级只认 Codex 支持的档位名。

use serde::Serialize;
use serde_json::{Map, Value};

/// Codex 支持的思考档位（由低到高），与前端 `REASONING_LEVELS` 一致。
pub const REASONING_LEVELS: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

/// 档位写进模型目录时带的说明，与前端 `REASONING_LEVEL_DESCRIPTIONS` 一致。
pub fn reasoning_level_description(level: &str) -> &'static str {
    match level {
        "none" => "Fast responses without reasoning",
        "minimal" => "Minimal reasoning, fastest responses",
        "low" => "Fast responses with lighter reasoning",
        "medium" => "Balances speed and reasoning depth for everyday tasks",
        "high" => "Greater reasoning depth for complex problems",
        "xhigh" => "Extra high reasoning depth for complex problems",
        "max" => "Maximum reasoning depth for the hardest problems",
        "ultra" => "Maximum reasoning with automatic task delegation",
        _ => "",
    }
}

/// 合理的上下文窗口下限：和界面的校验一致，小于 1000 的当作填错。
pub const MIN_CONTEXT_WINDOW: u64 = 1_000;
/// 合理的上下文窗口上限：一亿 token，超过的当作上游数据有误。
pub const MAX_CONTEXT_WINDOW: u64 = 100_000_000;

/// 上下文窗口按优先级查的字段（`.` 表示嵌套）。总窗口类的在前，只限输入的在后。
const WINDOW_FIELDS: &[&str] = &[
    "context_window",
    "context_length",
    "max_context_tokens",
    "max_context_length",
    "max_context_window",
    "contextWindow",
    "contextLength",
    "context_windows",
    "top_provider.context_length",
    "capabilities.limits.max_context_window_tokens",
    "limits.context_window",
    "limit.context",
    "max_input_tokens",
    "input_token_limit",
    "inputTokenLimit",
    "capabilities.limits.max_prompt_tokens",
];

/// 可选窗口档位数组：取最大值当窗口，各档位一并列出供参考。
const WINDOW_OPTIONS_FIELD: &str = "context_windows";

/// 思考等级按优先级查的字段。
const LEVEL_FIELDS: &[&str] = &[
    "supported_reasoning_levels",
    "reasoning_efforts",
    "supported_reasoning_efforts",
    "reasoning_levels",
    "reasoning.efforts",
    "reasoning.supported_efforts",
    "reasoning.levels",
    "reasoning.supported_levels",
    "capabilities.reasoning_efforts",
    "capabilities.supports.reasoning_effort",
];

/// 默认思考等级按优先级查的字段。
const DEFAULT_LEVEL_FIELDS: &[&str] = &[
    "reasoning_default_effort",
    "default_reasoning_effort",
    "default_reasoning_level",
    "reasoning.default_effort",
    "reasoning.default_level",
    "reasoning.default",
];

/// 一个模型从上游元数据里读到的信息。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamModelMeta {
    pub model: String,
    /// 上下文窗口（token）；没有或不合理时为 None。
    pub context_window: Option<u64>,
    /// 窗口取自哪个字段，如 `context_length`、`top_provider.context_length`。
    pub context_window_source: String,
    /// `context_windows` 里的合理档位（升序去重）；没有为空。
    pub context_window_options: Vec<u64>,
    /// 支持的思考档位（按由低到高排序去重）；没有为空。
    pub reasoning_levels: Vec<String>,
    pub reasoning_levels_source: String,
    /// 默认思考档位；只在它属于 `reasoning_levels` 时保留。
    pub default_reasoning_level: Option<String>,
    pub default_reasoning_source: String,
    /// 认不出或不合理、被跳过的字段值。
    pub ignored: Vec<IgnoredField>,
}

/// 被跳过的一个字段值及原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IgnoredField {
    pub field: String,
    pub value: String,
    pub reason: IgnoredReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum IgnoredReason {
    /// 窗口不是正整数，或超出合理范围。
    InvalidWindow,
    /// 档位名不是 Codex 支持的。
    UnknownLevel,
    /// 默认档位不在支持的档位里。
    DefaultNotInLevels,
    /// 字段类型不对（比如思考等级不是数组）。
    InvalidType,
}

/// 解析整个 `/models` 响应：认得 `{"data": [...]}`、`{"models": [...]}`、裸数组等形态。
/// 没有模型名的条目跳过；同名模型（忽略大小写）只取第一个。
pub fn parse_upstream_model_metas(payload: &Value) -> Vec<UpstreamModelMeta> {
    let mut metas: Vec<UpstreamModelMeta> = Vec::new();
    for item in model_items(payload) {
        let Some(meta) = parse_model_item(item) else {
            continue;
        };
        if !metas
            .iter()
            .any(|known| known.model.eq_ignore_ascii_case(&meta.model))
        {
            metas.push(meta);
        }
    }
    metas
}

/// 找到响应里的模型条目（对象）。
fn model_items(payload: &Value) -> Vec<&Map<String, Value>> {
    if let Some(array) = payload.as_array() {
        return array.iter().filter_map(Value::as_object).collect();
    }
    let Some(object) = payload.as_object() else {
        return Vec::new();
    };
    for key in ["data", "models", "items"] {
        if let Some(value) = object.get(key) {
            let nested = model_items(value);
            if !nested.is_empty() {
                return nested;
            }
        }
    }
    if model_id(object).is_some() {
        return vec![object];
    }
    Vec::new()
}

/// 模型名：`id` > `model` > `name`（Gemini 的 `name` 带 `models/` 前缀，去掉）。
fn model_id(object: &Map<String, Value>) -> Option<String> {
    ["id", "model", "name"].iter().find_map(|key| {
        let value = object.get(*key)?.as_str()?.trim();
        let value = if *key == "name" {
            value.strip_prefix("models/").unwrap_or(value)
        } else {
            value
        };
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// 解析单个模型条目；没有模型名返回 None。
pub fn parse_model_item(object: &Map<String, Value>) -> Option<UpstreamModelMeta> {
    let model = model_id(object)?;
    let mut meta = UpstreamModelMeta {
        model,
        ..UpstreamModelMeta::default()
    };
    read_window(object, &mut meta);
    read_levels(object, &mut meta);
    read_default_level(object, &mut meta);
    Some(meta)
}

/// 按 `a.b.c` 取嵌套字段；null 当作没有。
fn field<'a>(object: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut current = object.get(parts.next()?)?;
    for part in parts {
        current = current.as_object()?.get(part)?;
    }
    (!current.is_null()).then_some(current)
}

/// 短的原值文字，放进提示里。
fn value_text(value: &Value) -> String {
    let text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.chars().count() > 60 {
        let mut short: String = text.chars().take(60).collect();
        short.push('…');
        short
    } else {
        text
    }
}

fn ignore(meta: &mut UpstreamModelMeta, field: &str, value: &Value, reason: IgnoredReason) {
    meta.ignored.push(IgnoredField {
        field: field.to_string(),
        value: value_text(value),
        reason,
    });
}

/// 一个窗口值：正整数（整数值的小数也认）或 `"1000000"` / `"200K"` / `"1M"` 这样的字符串，
/// 且在合理范围内。
pub fn window_value(value: &Value) -> Option<u64> {
    let window = match value {
        Value::Number(number) => number.as_u64().or_else(|| {
            number
                .as_f64()
                .filter(|float| float.is_finite() && *float > 0.0 && float.fract() == 0.0)
                .filter(|float| *float <= MAX_CONTEXT_WINDOW as f64)
                .map(|float| float as u64)
        }),
        Value::String(text) => crate::model_suffix::parse_window_token(text),
        _ => None,
    }?;
    is_sane_window(window).then_some(window)
}

pub fn is_sane_window(window: u64) -> bool {
    (MIN_CONTEXT_WINDOW..=MAX_CONTEXT_WINDOW).contains(&window)
}

fn read_window(object: &Map<String, Value>, meta: &mut UpstreamModelMeta) {
    // 档位数组单独读：即使窗口取自更靠前的字段，也把档位列出来供参考。
    if let Some(value) = field(object, WINDOW_OPTIONS_FIELD) {
        match value.as_array() {
            Some(items) => {
                let mut options = Vec::new();
                for item in items {
                    match window_value(item) {
                        Some(window) => options.push(window),
                        None => ignore(
                            meta,
                            WINDOW_OPTIONS_FIELD,
                            item,
                            IgnoredReason::InvalidWindow,
                        ),
                    }
                }
                options.sort_unstable();
                options.dedup();
                meta.context_window_options = options;
            }
            None => ignore(
                meta,
                WINDOW_OPTIONS_FIELD,
                value,
                IgnoredReason::InvalidType,
            ),
        }
    }
    for path in WINDOW_FIELDS {
        let window = if *path == WINDOW_OPTIONS_FIELD {
            meta.context_window_options.last().copied()
        } else {
            let Some(value) = field(object, path) else {
                continue;
            };
            let window = window_value(value);
            if window.is_none() {
                ignore(meta, path, value, IgnoredReason::InvalidWindow);
            }
            window
        };
        if let Some(window) = window {
            meta.context_window = Some(window);
            meta.context_window_source = path.to_string();
            return;
        }
    }
}

/// 档位名归一：小写、去空白，`x-high` 这类写法认作 `xhigh`；不是 Codex 支持的返回 None。
pub fn normalize_level(raw: &str) -> Option<&'static str> {
    let lowered = raw.trim().to_ascii_lowercase();
    let level = match lowered.as_str() {
        "x-high" | "x_high" | "extra-high" | "extra_high" | "extrahigh" => "xhigh",
        other => other,
    };
    REASONING_LEVELS
        .iter()
        .find(|known| **known == level)
        .copied()
}

/// 一组档位按由低到高排序去重，认不出的丢掉。
pub fn normalize_levels<S: AsRef<str>>(levels: &[S]) -> Vec<String> {
    REASONING_LEVELS
        .iter()
        .filter(|known| {
            levels
                .iter()
                .any(|level| normalize_level(level.as_ref()) == Some(**known))
        })
        .map(|known| known.to_string())
        .collect()
}

/// 数组里一个档位：字符串，或 `{"effort": "high"}` 这类对象。
fn level_text(item: &Value) -> Option<&str> {
    match item {
        Value::String(text) => Some(text),
        Value::Object(object) => ["effort", "level", "name", "value", "id"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str)),
        _ => None,
    }
}

fn read_levels(object: &Map<String, Value>, meta: &mut UpstreamModelMeta) {
    for path in LEVEL_FIELDS {
        let Some(value) = field(object, path) else {
            continue;
        };
        let items: Vec<Value> = match value {
            Value::Array(items) => items.clone(),
            // 逗号分隔的字符串也认：`"low,medium,high"`。
            Value::String(text) => text
                .split([',', ' ', '|'])
                .filter(|part| !part.trim().is_empty())
                .map(|part| Value::String(part.trim().to_string()))
                .collect(),
            // 布尔值只表示「支持思考」，不是档位列表，不算类型错误。
            Value::Bool(_) => continue,
            other => {
                ignore(meta, path, other, IgnoredReason::InvalidType);
                continue;
            }
        };
        let mut picked: Vec<&'static str> = Vec::new();
        for item in &items {
            match level_text(item).and_then(normalize_level) {
                Some(level) => picked.push(level),
                None => ignore(meta, path, item, IgnoredReason::UnknownLevel),
            }
        }
        let levels = normalize_levels(&picked);
        if !levels.is_empty() {
            meta.reasoning_levels = levels;
            meta.reasoning_levels_source = path.to_string();
            return;
        }
    }
}

fn read_default_level(object: &Map<String, Value>, meta: &mut UpstreamModelMeta) {
    for path in DEFAULT_LEVEL_FIELDS {
        let Some(value) = field(object, path) else {
            continue;
        };
        let Some(level) = value.as_str().and_then(normalize_level) else {
            let reason = if value.is_string() {
                IgnoredReason::UnknownLevel
            } else {
                IgnoredReason::InvalidType
            };
            ignore(meta, path, value, reason);
            continue;
        };
        if !meta.reasoning_levels.iter().any(|known| known == level) {
            ignore(meta, path, value, IgnoredReason::DefaultNotInLevels);
            continue;
        }
        meta.default_reasoning_level = Some(level.to_string());
        meta.default_reasoning_source = path.to_string();
        return;
    }
}
